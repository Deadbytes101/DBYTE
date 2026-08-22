//! Executable models for the v11.7 bounded IRQ I/O contracts.

use std::collections::VecDeque;

const MAX_TASKS: usize = 8;
const RING_CAPACITY: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WaitKind {
    None,
    Event,
    Semaphore,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Task {
    id: u32,
    blocked: bool,
    wait_kind: WaitKind,
    wait_object: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Semaphore {
    count: usize,
    maximum: usize,
    waiters: VecDeque<u32>,
}

impl Semaphore {
    fn new(initial: usize, maximum: usize) -> Result<Self, &'static str> {
        if maximum == 0 || initial > maximum {
            return Err("invalid semaphore bounds");
        }
        Ok(Self {
            count: initial,
            maximum,
            waiters: VecDeque::with_capacity(MAX_TASKS),
        })
    }

    fn wait(&mut self, task: &mut Task) -> Result<bool, &'static str> {
        if task.blocked || task.wait_kind != WaitKind::None || task.wait_object != 0 {
            return Err("task already owns a wait");
        }
        if self.count != 0 {
            self.count -= 1;
            return Ok(false);
        }
        if self.waiters.len() == MAX_TASKS || self.waiters.contains(&task.id) {
            return Err("waiter capacity");
        }
        self.waiters.push_back(task.id);
        task.blocked = true;
        task.wait_kind = WaitKind::Semaphore;
        task.wait_object = 1;
        Ok(true)
    }

    fn post(&mut self, tasks: &mut [Task]) -> Result<bool, &'static str> {
        if let Some(id) = self.waiters.front().copied() {
            let task = tasks
                .iter_mut()
                .find(|task| task.id == id)
                .ok_or("stale waiter")?;
            if !task.blocked || task.wait_kind != WaitKind::Semaphore || task.wait_object != 1 {
                return Err("corrupt waiter");
            }
            self.waiters.pop_front();
            task.blocked = false;
            task.wait_kind = WaitKind::None;
            task.wait_object = 0;
            return Ok(true);
        }
        if self.count == self.maximum {
            return Ok(false);
        }
        self.count += 1;
        Ok(true)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ByteRing {
    bytes: [u8; RING_CAPACITY],
    head: usize,
    tail: usize,
    count: usize,
    dropped: usize,
}

impl ByteRing {
    fn new() -> Self {
        Self {
            bytes: [0; RING_CAPACITY],
            head: 0,
            tail: 0,
            count: 0,
            dropped: 0,
        }
    }

    fn push(&mut self, value: u8) -> bool {
        if self.count == RING_CAPACITY {
            self.dropped += 1;
            return false;
        }
        self.bytes[self.tail] = value;
        self.tail = (self.tail + 1) % RING_CAPACITY;
        self.count += 1;
        true
    }

    fn pop(&mut self) -> Option<u8> {
        if self.count == 0 {
            return None;
        }
        let value = self.bytes[self.head];
        self.head = (self.head + 1) % RING_CAPACITY;
        self.count -= 1;
        Some(value)
    }
}

#[test]
fn semaphore_is_sticky_bounded_and_fifo() {
    let mut semaphore = Semaphore::new(0, 2).unwrap();
    let mut tasks = [
        Task {
            id: 10,
            blocked: false,
            wait_kind: WaitKind::None,
            wait_object: 0,
        },
        Task {
            id: 11,
            blocked: false,
            wait_kind: WaitKind::None,
            wait_object: 0,
        },
    ];
    assert_eq!(semaphore.wait(&mut tasks[0]), Ok(true));
    assert_eq!(semaphore.wait(&mut tasks[1]), Ok(true));
    assert_eq!(semaphore.post(&mut tasks), Ok(true));
    assert!(!tasks[0].blocked);
    assert!(tasks[1].blocked);
    assert_eq!(semaphore.post(&mut tasks), Ok(true));
    assert!(!tasks[1].blocked);
    assert_eq!(semaphore.post(&mut tasks), Ok(true));
    assert_eq!(semaphore.count, 1);
    assert_eq!(semaphore.wait(&mut tasks[0]), Ok(false));
    assert_eq!(semaphore.count, 0);
    assert_eq!(semaphore.post(&mut tasks), Ok(true));
    assert_eq!(semaphore.post(&mut tasks), Ok(true));
    let before = semaphore.clone();
    assert_eq!(semaphore.post(&mut tasks), Ok(false));
    assert_eq!(
        semaphore, before,
        "overflow rejection must be transactional"
    );
}

#[test]
fn semaphore_rejects_invalid_bounds_and_cross_primitive_ownership() {
    assert_eq!(Semaphore::new(0, 0), Err("invalid semaphore bounds"));
    assert_eq!(Semaphore::new(2, 1), Err("invalid semaphore bounds"));
    let mut semaphore = Semaphore::new(0, 1).unwrap();
    let mut task = Task {
        id: 7,
        blocked: true,
        wait_kind: WaitKind::Event,
        wait_object: 2,
    };
    let before = semaphore.clone();
    assert_eq!(semaphore.wait(&mut task), Err("task already owns a wait"));
    assert_eq!(semaphore, before);
}

#[test]
fn corrupt_waiter_fails_without_mutation() {
    let mut semaphore = Semaphore::new(0, 1).unwrap();
    let mut tasks = [Task {
        id: 3,
        blocked: false,
        wait_kind: WaitKind::None,
        wait_object: 0,
    }];
    semaphore.wait(&mut tasks[0]).unwrap();
    tasks[0].wait_kind = WaitKind::Event;
    let before_semaphore = semaphore.clone();
    let before_tasks = tasks;
    assert_eq!(semaphore.post(&mut tasks), Err("corrupt waiter"));
    assert_eq!(semaphore, before_semaphore);
    assert_eq!(tasks, before_tasks);
}

#[test]
fn byte_ring_rejects_byte_65_and_drains_fifo_exactly() {
    let mut ring = ByteRing::new();
    for value in 0..RING_CAPACITY {
        assert!(ring.push(value as u8));
    }
    let before_bytes = ring.bytes;
    assert!(!ring.push(0xff));
    assert_eq!(ring.bytes, before_bytes, "full push overwrote queued data");
    assert_eq!(ring.count, RING_CAPACITY);
    assert_eq!(ring.dropped, 1);
    for expected in 0..RING_CAPACITY {
        assert_eq!(ring.pop(), Some(expected as u8));
    }
    assert_eq!(ring.pop(), None);
    assert_eq!(ring.count, 0);
}
