//! Executable models for the v11.8 single-request ATA block-device contracts.

const SECTOR_SIZE: usize = 512;
const SECTOR_WORDS: usize = 256;
const SECTOR_COUNT: u32 = 16_384;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Error {
    Invalid,
    Busy,
    Timeout,
    DeviceErr,
    DeviceFault,
    Protocol,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operation {
    Read,
    Write,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Request {
    operation: Operation,
    lba: u32,
    buffer: usize,
    complete: bool,
    error: Option<Error>,
    words: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AtaModel {
    request: Option<Request>,
    completion_posts: usize,
    irq_entries: usize,
    accepted_irqs: usize,
    spurious_irqs: usize,
    slave_eois: usize,
    master_eois: usize,
}

impl AtaModel {
    fn new() -> Self {
        Self {
            request: None,
            completion_posts: 0,
            irq_entries: 0,
            accepted_irqs: 0,
            spurious_irqs: 0,
            slave_eois: 0,
            master_eois: 0,
        }
    }

    fn begin(&mut self, operation: Operation, lba: u32, buffer: usize) -> Result<(), Error> {
        if buffer == 0 || buffer & 1 != 0 || buffer > 0xffff_fe00 || lba >= SECTOR_COUNT {
            return Err(Error::Invalid);
        }
        if self.request.is_some() {
            return Err(Error::Busy);
        }
        self.request = Some(Request {
            operation,
            lba,
            buffer,
            complete: false,
            error: None,
            words: 0,
        });
        Ok(())
    }

    fn setup_failed(&mut self, error: Error) -> Error {
        self.request = None;
        error
    }

    fn irq(&mut self, busy: bool, drq: bool, error: bool, fault: bool) {
        self.irq_entries += 1;
        if let Some(request) = self.request.as_mut() {
            self.accepted_irqs += 1;
            request.error = if fault {
                Some(Error::DeviceFault)
            } else if error {
                Some(Error::DeviceErr)
            } else if busy
                || (request.operation == Operation::Read && !drq)
                || (request.operation == Operation::Write && drq)
            {
                Some(Error::Protocol)
            } else {
                None
            };
            if request.error.is_none() {
                request.words = if request.operation == Operation::Read {
                    SECTOR_WORDS
                } else {
                    0
                };
                request.complete = true;
            }
            self.completion_posts += 1;
        } else {
            self.spurious_irqs += 1;
        }
        self.slave_eois += 1;
        self.master_eois += 1;
    }
}

fn seed_sector() -> [u8; SECTOR_SIZE] {
    let mut sector = [0_u8; SECTOR_SIZE];
    let header = b"DBYTE_BLOCK_TEST_V1";
    sector[..header.len()].copy_from_slice(header);
    for (index, byte) in sector.iter_mut().enumerate().skip(header.len()) {
        *byte = (index.wrapping_mul(37).wrapping_add(0x2b) & 0xff) as u8;
    }
    sector
}

fn scratch_a() -> [u8; SECTOR_SIZE] {
    std::array::from_fn(|index| (index.wrapping_mul(13).wrapping_add(0x5a) & 0xff) as u8)
}

fn scratch_b() -> [u8; SECTOR_SIZE] {
    std::array::from_fn(|index| (index.wrapping_mul(29).wrapping_add(0xa7) & 0xff) as u8)
}

#[test]
fn single_request_ownership_and_setup_rollback_are_transactional() {
    let mut ata = AtaModel::new();
    ata.begin(Operation::Read, 8, 0x2000).unwrap();
    let before = ata.clone();
    assert_eq!(ata.begin(Operation::Write, 9, 0x3000), Err(Error::Busy));
    assert_eq!(ata, before);
    assert_eq!(ata.setup_failed(Error::Timeout), Error::Timeout);
    assert!(ata.request.is_none());
    ata.begin(Operation::Write, 9, 0x3000).unwrap();
}

#[test]
fn invalid_lba_and_buffer_never_acquire_the_device() {
    for (lba, buffer) in [
        (SECTOR_COUNT, 0x2000),
        (u32::MAX, 0x2000),
        (8, 0),
        (8, 1),
        (8, 0xffff_fe01),
    ] {
        let mut ata = AtaModel::new();
        assert_eq!(ata.begin(Operation::Read, lba, buffer), Err(Error::Invalid));
        assert!(ata.request.is_none());
    }
}

#[test]
fn read_irq_transfers_exactly_256_words_and_posts_once() {
    let mut ata = AtaModel::new();
    ata.begin(Operation::Read, 8, 0x2000).unwrap();
    ata.irq(false, true, false, false);
    let request = ata.request.as_ref().unwrap();
    assert!(request.complete);
    assert_eq!(request.words, SECTOR_WORDS);
    assert_eq!(ata.completion_posts, 1);
    assert_eq!((ata.slave_eois, ata.master_eois), (1, 1));
}

#[test]
fn write_irq_and_device_errors_have_deterministic_results() {
    let mut ata = AtaModel::new();
    ata.begin(Operation::Write, 9, 0x3000).unwrap();
    ata.irq(false, false, false, false);
    assert!(ata.request.as_ref().unwrap().complete);

    for (error, fault, expected) in [
        (true, false, Error::DeviceErr),
        (false, true, Error::DeviceFault),
    ] {
        let mut failed = AtaModel::new();
        failed.begin(Operation::Read, 8, 0x2000).unwrap();
        failed.irq(false, true, error, fault);
        assert_eq!(failed.request.unwrap().error, Some(expected));
        assert_eq!(failed.completion_posts, 1);
    }
}

#[test]
fn spurious_irq_acknowledges_both_pic_levels_without_posting() {
    let mut ata = AtaModel::new();
    ata.irq(false, false, false, false);
    assert_eq!(ata.spurious_irqs, 1);
    assert_eq!(ata.accepted_irqs, 0);
    assert_eq!(ata.completion_posts, 0);
    assert_eq!((ata.slave_eois, ata.master_eois), (1, 1));
}

#[test]
fn deterministic_disk_patterns_are_full_sector_and_distinct() {
    let seed = seed_sector();
    assert_eq!(&seed[..19], b"DBYTE_BLOCK_TEST_V1");
    assert_eq!(seed[19], (19 * 37 + 0x2b) as u8);
    let before = scratch_a();
    let after = scratch_b();
    assert_ne!(before, after);
    assert_eq!(before[511], (511 * 13 + 0x5a) as u8);
    assert_eq!(after[511], (511 * 29 + 0xa7) as u8);
}
