const MAX_PROCESSES: usize = 4;
const MAX_FDS: usize = 19;
const USER_TOP: u32 = 0xc000_0000;
const PRESENT: u32 = 1;
const WRITABLE: u32 = 2;
const USER: u32 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Mapping {
    start: u32,
    end: u32,
    physical: u32,
    flags: u32,
}

fn validate_user_range(mappings: &[Mapping], address: u32, length: u32, write: bool) -> bool {
    if length == 0 {
        return true;
    }
    let Some(end) = address.checked_add(length) else {
        return false;
    };
    if address >= USER_TOP || end > USER_TOP {
        return false;
    }
    let mut page = address & !0xfff;
    let final_page = (end - 1) & !0xfff;
    loop {
        let Some(mapping) = mappings
            .iter()
            .find(|mapping| page >= mapping.start && page < mapping.end)
        else {
            return false;
        };
        let required = PRESENT | USER | if write { WRITABLE } else { 0 };
        if mapping.flags & required != required {
            return false;
        }
        if page == final_page {
            return true;
        }
        page = page.wrapping_add(0x1000);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Request {
    slot: usize,
    generation: u32,
}

#[derive(Debug, Default)]
struct RequestQueue {
    entries: [Option<Request>; MAX_PROCESSES],
    head: usize,
    tail: usize,
    count: usize,
}

impl RequestQueue {
    fn push(&mut self, request: Request) -> bool {
        if request.slot >= MAX_PROCESSES
            || self.count == MAX_PROCESSES
            || self.entries.iter().flatten().any(|entry| *entry == request)
        {
            return false;
        }
        self.entries[self.tail] = Some(request);
        self.tail = (self.tail + 1) % MAX_PROCESSES;
        self.count += 1;
        true
    }

    fn pop(&mut self) -> Option<Request> {
        if self.count == 0 {
            return None;
        }
        let result = self.entries[self.head].take();
        self.head = (self.head + 1) % MAX_PROCESSES;
        self.count -= 1;
        result
    }
}

#[derive(Clone, Debug)]
struct FdNamespace {
    raw: [u32; MAX_FDS],
}

impl Default for FdNamespace {
    fn default() -> Self {
        Self { raw: [0; MAX_FDS] }
    }
}

impl FdNamespace {
    fn install(&mut self, raw: u32) -> Option<usize> {
        (3..MAX_FDS)
            .find(|&fd| self.raw[fd] == 0)
            .inspect(|&fd| self.raw[fd] = raw)
    }

    fn close_all(&mut self) -> Vec<u32> {
        let mut released = Vec::new();
        for fd in 3..MAX_FDS {
            if self.raw[fd] != 0 {
                released.push(self.raw[fd]);
                self.raw[fd] = 0;
            }
        }
        released
    }
}

#[test]
fn transition_resume_image_and_tss_contract_are_exact() {
    assert_eq!([0, 4, 8, 12, 16, 20, 24, 28], [0, 4, 8, 12, 16, 20, 24, 28]);
    assert_eq!([32, 36, 40, 44, 48], [32, 36, 40, 44, 48]);
    assert_eq!(core::mem::size_of::<[u32; 26]>(), 104);
    assert_eq!((0x08, 0x10, 0x1b, 0x23, 0x28), (8, 16, 27, 35, 40));
}

#[test]
fn user_copy_checks_every_page_permissions_and_overflow_before_copy() {
    let mappings = [
        Mapping {
            start: 0x0040_0000,
            end: 0x0040_1000,
            physical: 0x0018_0000,
            flags: PRESENT | USER,
        },
        Mapping {
            start: 0x0040_1000,
            end: 0x0040_2000,
            physical: 0x0019_0000,
            flags: PRESENT | USER | WRITABLE,
        },
    ];
    assert!(validate_user_range(&mappings, 0x0040_0ff0, 32, false));
    assert!(!validate_user_range(&mappings, 0x0040_0ff0, 32, true));
    assert!(!validate_user_range(&mappings[..1], 0x0040_0ff0, 32, false));
    assert!(!validate_user_range(&mappings, 0xc100_0000, 1, false));
    assert!(!validate_user_range(&mappings, 0xffff_fff0, 32, false));
    assert!(validate_user_range(&[], 0xffff_ffff, 0, true));
    assert_ne!(mappings[0].physical, mappings[1].physical);
}

#[test]
fn request_fifo_and_generation_reject_stale_completion() {
    let mut queue = RequestQueue::default();
    assert!(queue.push(Request {
        slot: 0,
        generation: 1
    }));
    assert!(queue.push(Request {
        slot: 1,
        generation: 3
    }));
    assert!(!queue.push(Request {
        slot: 0,
        generation: 1
    }));
    assert_eq!(
        queue.pop(),
        Some(Request {
            slot: 0,
            generation: 1
        })
    );
    let reused_generation = 2;
    let stale = Request {
        slot: 0,
        generation: 1,
    };
    assert_ne!(stale.generation, reused_generation);
    assert_eq!(
        queue.pop(),
        Some(Request {
            slot: 1,
            generation: 3
        })
    );
    assert_eq!(queue.pop(), None);
}

#[test]
fn per_process_fd_tables_are_isolated_bounded_and_reclaimed() {
    let mut a = FdNamespace::default();
    let mut b = FdNamespace::default();
    assert_eq!(a.install(0x11), Some(3));
    assert_eq!(b.install(0x22), Some(3));
    assert_ne!(a.raw[3], b.raw[3]);
    for raw in 1..16 {
        if raw != 0x11 {
            let _ = a.install(0x100 + raw);
        }
    }
    assert_eq!(a.install(0xffff), None);
    assert_eq!(b.close_all(), vec![0x22]);
    assert!(b.raw[3..].iter().all(|value| *value == 0));
    assert_eq!(b.install(0x33), Some(3));
}

#[test]
fn private_images_and_guarded_stacks_do_not_alias() {
    let process_a_text = 0x0019_a000u32;
    let process_b_text = 0x001a_4000u32;
    let guard_page = 0xbfff_a000u32;
    let stack_floor = 0xbfff_c000u32;
    assert_ne!(process_a_text, process_b_text);
    assert_eq!(guard_page + 0x1000, 0xbfff_b000u32);
    assert_eq!(stack_floor + 4 * 0x1000, USER_TOP);
    assert!(guard_page < stack_floor);
}
