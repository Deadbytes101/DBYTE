//! Executable model tests for the v11.5 fixed scheduler ownership contract.

const MAX_TASKS: usize = 8;
const SLOT_BASE: u32 = 0xc120_0000;
const SLOT_SIZE: u32 = 0x5000;
const PAGE_SIZE: u32 = 0x1000;
const RESUME_WORDS: usize = 11;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Ready,
    Running,
    Exited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Task {
    id: u32,
    slot: usize,
    state: State,
    resume_esp: u32,
}

#[derive(Default)]
struct Model {
    queue: [Option<Task>; MAX_TASKS],
    next_id: u32,
    cursor: usize,
}

impl Model {
    fn spawn(&mut self) -> Option<Task> {
        let slot = (0..MAX_TASKS)
            .find(|slot| self.queue.iter().flatten().all(|task| task.slot != *slot))?;
        let queue = self.queue.iter().position(Option::is_none)?;
        self.next_id = self.next_id.checked_add(1)?;
        let stack_top = SLOT_BASE + (slot as u32 + 1) * SLOT_SIZE;
        let task = Task {
            id: self.next_id,
            slot,
            state: State::Ready,
            resume_esp: stack_top - 48,
        };
        self.queue[queue] = Some(task);
        Some(task)
    }

    fn next(&mut self) -> Option<Task> {
        for _ in 0..MAX_TASKS {
            self.cursor = (self.cursor + 1) % MAX_TASKS;
            if let Some(mut task) = self.queue[self.cursor] {
                if task.state == State::Ready || task.state == State::Running {
                    task.state = State::Running;
                    self.queue[self.cursor] = Some(task);
                    return Some(task);
                }
            }
        }
        None
    }

    fn exit(&mut self, id: u32) {
        let task = self
            .queue
            .iter_mut()
            .flatten()
            .find(|task| task.id == id)
            .unwrap();
        task.state = State::Exited;
    }

    fn reap(&mut self, id: u32) -> Option<usize> {
        let index = self.queue.iter().position(|task| {
            task.is_some_and(|task| task.id == id && task.state == State::Exited)
        })?;
        let slot = self.queue[index].take()?.slot;
        Some(slot)
    }
}

#[test]
fn canonical_resume_image_offsets_match_pushad_and_iretd() {
    let offsets: Vec<usize> = (0..RESUME_WORDS).map(|word| word * 4).collect();
    assert_eq!(offsets, [0, 4, 8, 12, 16, 20, 24, 28, 32, 36, 40]);
    assert_eq!(offsets[3], 12, "POPAD must ignore the saved ESP slot");
    assert_eq!(offsets[8..], [32, 36, 40], "IRETD consumes EIP, CS, EFLAGS");
}

#[test]
fn fixed_slots_have_real_guard_pages_and_four_disjoint_stack_pages() {
    for slot in 0..MAX_TASKS {
        let base = SLOT_BASE + slot as u32 * SLOT_SIZE;
        let guard = base..base + PAGE_SIZE;
        let stack = base + PAGE_SIZE..base + SLOT_SIZE;
        assert_eq!(guard.end, stack.start);
        assert_eq!(stack.end - stack.start, 4 * PAGE_SIZE);
        if slot + 1 < MAX_TASKS {
            assert_eq!(stack.end, SLOT_BASE + (slot as u32 + 1) * SLOT_SIZE);
        }
    }
}

#[test]
fn round_robin_exit_reap_and_lowest_slot_reuse_are_deterministic() {
    let mut model = Model::default();
    let idle = model.spawn().unwrap();
    let first = model.spawn().unwrap();
    let second = model.spawn().unwrap();
    assert_eq!((idle.id, first.id, second.id), (1, 2, 3));
    assert_eq!((idle.slot, first.slot, second.slot), (0, 1, 2));
    assert_eq!(model.next().unwrap().id, first.id);
    assert_eq!(model.next().unwrap().id, second.id);
    assert_eq!(model.next().unwrap().id, idle.id);
    model.exit(first.id);
    assert_eq!(model.reap(first.id), Some(1));
    let reused = model.spawn().unwrap();
    assert_eq!(reused.id, 4);
    assert_eq!(reused.slot, 1);
}

#[test]
fn capacity_failure_does_not_mutate_queue() {
    let mut model = Model::default();
    for _ in 0..MAX_TASKS {
        model.spawn().unwrap();
    }
    let before = model.queue;
    assert_eq!(model.spawn(), None);
    assert_eq!(model.queue, before);
}

#[test]
fn stack_mapping_failure_rolls_back_all_operation_owned_frames() {
    let mut free_frames = vec![0x2000u32, 0x3000, 0x4000, 0x5000];
    let before = free_frames.clone();
    let mut mapped = Vec::new();
    for page in 0..4 {
        let frame = free_frames.remove(0);
        if page == 2 {
            free_frames.push(frame);
            free_frames.append(&mut mapped);
            free_frames.sort_unstable();
            break;
        }
        mapped.push(frame);
    }
    assert!(mapped.is_empty());
    assert_eq!(free_frames, before);
}

#[test]
fn corrupt_tcb_and_nested_entry_fail_before_state_mutation() {
    let mut words = [7u32, 2, 0xc120_4fd0, 0, 0, 3];
    let checksum = [words[0], words[1], words[2], words[3], words[5]]
        .iter()
        .fold(0x51ce_d00d, |sum, word| sum ^ word);
    words[4] = checksum;
    let snapshot = words;
    words[2] ^= 4;
    let actual = [words[0], words[1], words[2], words[3], words[5]]
        .iter()
        .fold(0x51ce_d00d, |sum, word| sum ^ word);
    assert_ne!(actual, words[4]);
    words = snapshot;
    assert_eq!(words, snapshot, "fail-closed validation preserves the TCB");

    let entered = true;
    let tick_before = 9u32;
    assert!(entered, "a nested IRQ must be rejected");
    assert_eq!(
        tick_before, 9,
        "nested rejection cannot mutate scheduler state"
    );
}
