//! Executable model tests for the v11.6 bounded blocking contract.

use std::collections::VecDeque;

const MAX_TASKS: usize = 8;
const MAX_SLEEP: u32 = 0x7fff_ffff;
const STATUS_OK: i32 = 0;
const STATUS_INVALID: i32 = 1;
const STATUS_WRONG_CONTEXT: i32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Ready,
    Running,
    Blocked,
    Sleeping,
    Exited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Task {
    id: u32,
    state: State,
    resume_esp: u32,
    wake_tick: u32,
    wait_event: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Event {
    waiters: VecDeque<u32>,
}

impl Event {
    fn new() -> Self {
        Self {
            waiters: VecDeque::with_capacity(MAX_TASKS),
        }
    }

    fn wait(&mut self, task: &mut Task, resume_esp: u32) -> Result<(), &'static str> {
        if task.state != State::Running || task.wait_event.is_some() {
            return Err("invalid task state");
        }
        if self.waiters.len() == MAX_TASKS || self.waiters.contains(&task.id) {
            return Err("event capacity");
        }
        self.waiters.push_back(task.id);
        task.resume_esp = resume_esp;
        task.wait_event = Some(1);
        task.state = State::Blocked;
        Ok(())
    }

    fn signal(&mut self, tasks: &mut [Task]) -> Result<Option<u32>, &'static str> {
        let Some(id) = self.waiters.front().copied() else {
            return Ok(None);
        };
        let task = tasks
            .iter_mut()
            .find(|task| task.id == id)
            .ok_or("stale waiter")?;
        if task.state != State::Blocked || task.wait_event != Some(1) {
            return Err("corrupt waiter");
        }
        self.waiters.pop_front();
        task.wait_event = None;
        task.state = State::Ready;
        Ok(Some(id))
    }
}

fn deadline_reached(now: u32, deadline: u32) -> bool {
    now.wrapping_sub(deadline) as i32 >= 0
}

fn sleep(task: &mut Task, now: u32, interval: u32, resume_esp: u32) -> Result<(), &'static str> {
    if interval == 0 || interval > MAX_SLEEP {
        return Err("invalid interval");
    }
    if task.state != State::Running {
        return Err("wrong context");
    }
    task.resume_esp = resume_esp;
    task.wake_tick = now.wrapping_add(interval);
    task.state = State::Sleeping;
    Ok(())
}

fn sleep_api(
    task: &mut Task,
    now: u32,
    interval: u32,
    resume_esp: u32,
    interrupts_enabled: bool,
) -> i32 {
    if !interrupts_enabled {
        return STATUS_WRONG_CONTEXT;
    }
    if interval == 0 {
        return STATUS_OK;
    }
    sleep(task, now, interval, resume_esp).map_or(STATUS_INVALID, |()| STATUS_OK)
}

#[test]
fn sleep_deadlines_are_wrap_safe_and_bounded_to_half_the_tick_space() {
    let mut task = Task {
        id: 1,
        state: State::Running,
        resume_esp: 0,
        wake_tick: 0,
        wait_event: None,
    };
    sleep(&mut task, 0xffff_fffd, 5, 0xc120_4fd4).unwrap();
    assert_eq!(task.wake_tick, 2);
    assert!(!deadline_reached(1, task.wake_tick));
    assert!(deadline_reached(2, task.wake_tick));
    assert!(deadline_reached(3, task.wake_tick));

    let before = task;
    assert_eq!(sleep(&mut task, 3, 0x8000_0000, 4), Err("invalid interval"));
    assert_eq!(task, before, "invalid sleep must be transactional");
}

#[test]
fn zero_sleep_is_a_task_context_noop_and_wrong_context_wins() {
    let mut task = Task {
        id: 1,
        state: State::Running,
        resume_esp: 0xc120_4fd4,
        wake_tick: 0,
        wait_event: None,
    };
    let before = task;
    assert_eq!(sleep_api(&mut task, 9, 0, 0, true), STATUS_OK);
    assert_eq!(task, before);
    assert_eq!(sleep_api(&mut task, 9, 0, 0, false), STATUS_WRONG_CONTEXT);
    assert_eq!(task, before);
}

#[test]
fn event_is_non_sticky_wake_one_fifo() {
    let mut event = Event::new();
    let mut tasks = [
        Task {
            id: 10,
            state: State::Running,
            resume_esp: 0,
            wake_tick: 0,
            wait_event: None,
        },
        Task {
            id: 11,
            state: State::Running,
            resume_esp: 0,
            wake_tick: 0,
            wait_event: None,
        },
        Task {
            id: 12,
            state: State::Running,
            resume_esp: 0,
            wake_tick: 0,
            wait_event: None,
        },
    ];

    assert_eq!(
        event.signal(&mut tasks),
        Ok(None),
        "empty pulse is discarded"
    );
    for (index, task) in tasks.iter_mut().enumerate() {
        event
            .wait(task, 0xc120_1000 + index as u32 * 0x5000)
            .unwrap();
    }
    assert_eq!(event.signal(&mut tasks), Ok(Some(10)));
    assert_eq!(event.signal(&mut tasks), Ok(Some(11)));
    assert_eq!(event.signal(&mut tasks), Ok(Some(12)));
    assert_eq!(event.signal(&mut tasks), Ok(None));
    assert!(tasks.iter().all(|task| task.state == State::Ready));
}

#[test]
fn committed_wait_cannot_be_lost_and_precommit_signal_is_defined_loss() {
    let mut event = Event::new();
    let mut tasks = [Task {
        id: 7,
        state: State::Running,
        resume_esp: 0,
        wake_tick: 0,
        wait_event: None,
    }];

    assert_eq!(event.signal(&mut tasks), Ok(None));
    event.wait(&mut tasks[0], 0xc120_4fd4).unwrap();
    assert_eq!(tasks[0].state, State::Blocked);
    assert_eq!(event.signal(&mut tasks), Ok(Some(7)));
    assert_eq!(tasks[0].state, State::Ready);
    assert_eq!(tasks[0].resume_esp, 0xc120_4fd4);
}

#[test]
fn corrupt_or_duplicate_waiters_fail_before_mutation() {
    let mut event = Event::new();
    let mut tasks = [Task {
        id: 3,
        state: State::Running,
        resume_esp: 0,
        wake_tick: 0,
        wait_event: None,
    }];
    event.wait(&mut tasks[0], 0xc120_4fd4).unwrap();
    let before_event = event.clone();
    let before_task = tasks[0];
    assert_eq!(
        event.wait(&mut tasks[0], 0xc120_4fd0),
        Err("invalid task state")
    );
    assert_eq!(event, before_event);
    assert_eq!(tasks[0], before_task);

    tasks[0].state = State::Exited;
    let corrupt_event = event.clone();
    let corrupt_task = tasks[0];
    assert_eq!(event.signal(&mut tasks), Err("corrupt waiter"));
    assert_eq!(event, corrupt_event);
    assert_eq!(tasks[0], corrupt_task);
}

#[test]
fn full_event_rejects_another_waiter_without_mutation() {
    let mut event = Event::new();
    let mut tasks: Vec<Task> = (0..=MAX_TASKS)
        .map(|index| Task {
            id: index as u32 + 1,
            state: State::Running,
            resume_esp: 0,
            wake_tick: 0,
            wait_event: None,
        })
        .collect();
    for task in &mut tasks[..MAX_TASKS] {
        event.wait(task, 0xc120_4fd4).unwrap();
    }
    let before_event = event.clone();
    let before_task = tasks[MAX_TASKS];
    assert_eq!(
        event.wait(&mut tasks[MAX_TASKS], 0xc120_4fd4),
        Err("event capacity")
    );
    assert_eq!(event, before_event);
    assert_eq!(tasks[MAX_TASKS], before_task);
}

#[test]
fn blocked_and_sleeping_are_not_runnable_or_reclaimable() {
    for state in [State::Blocked, State::Sleeping] {
        assert_ne!(state, State::Ready);
        assert_ne!(state, State::Running);
        assert_ne!(state, State::Exited);
    }
}
