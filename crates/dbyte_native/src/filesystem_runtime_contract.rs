//! Executable models for the bounded DBFS1 runtime contracts added in v11.10.

const MAX_DEPTH: usize = 8;
const MAX_COMPONENT: usize = 31;
const MAX_HANDLES: usize = 16;
const CACHE_ENTRIES: usize = 16;

fn parse_path(path: &[u8]) -> Result<Vec<&[u8]>, &'static str> {
    if path.is_empty() || path.len() > 255 || path[0] != b'/' {
        return Err("invalid path");
    }
    if path == b"/" {
        return Ok(Vec::new());
    }
    if path.last() == Some(&b'/') {
        return Err("invalid path");
    }
    let mut result = Vec::new();
    for component in path[1..].split(|byte| *byte == b'/') {
        if component.is_empty()
            || component.len() > MAX_COMPONENT
            || component == b"."
            || component == b".."
            || component.iter().any(|byte| *byte == 0 || *byte > 0x7f)
        {
            return Err("invalid component");
        }
        result.push(component);
        if result.len() > MAX_DEPTH {
            return Err("path too deep");
        }
    }
    Ok(result)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Handle {
    allocated: bool,
    retired: bool,
    generation: u32,
    owner: u32,
    inode: u32,
    offset: u32,
    flags: u32,
}

impl Default for Handle {
    fn default() -> Self {
        Self {
            allocated: false,
            retired: false,
            generation: 1,
            owner: 0,
            inode: 0,
            offset: 0,
            flags: 0,
        }
    }
}

#[derive(Clone, Debug)]
struct Handles {
    slots: [Handle; MAX_HANDLES],
}

impl Handles {
    fn new() -> Self {
        Self {
            slots: [Handle::default(); MAX_HANDLES],
        }
    }

    fn open(&mut self, owner: u32, inode: u32, flags: u32) -> Option<u32> {
        let (index, slot) = self
            .slots
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| !slot.allocated && !slot.retired)?;
        slot.allocated = true;
        slot.owner = owner;
        slot.inode = inode;
        slot.flags = flags;
        Some((slot.generation << 4) | index as u32)
    }

    fn validate(&self, id: u32, owner: u32) -> Result<&Handle, &'static str> {
        if id == 0 {
            return Err("invalid handle");
        }
        let slot = &self.slots[(id & 15) as usize];
        if !slot.allocated || slot.generation != id >> 4 {
            return Err("stale handle");
        }
        if slot.owner != owner {
            return Err("wrong owner");
        }
        Ok(slot)
    }

    fn close(&mut self, id: u32, owner: u32) -> Result<(), &'static str> {
        self.validate(id, owner)?;
        let slot = &mut self.slots[(id & 15) as usize];
        slot.allocated = false;
        slot.owner = 0;
        slot.inode = 0;
        slot.offset = 0;
        slot.flags = 0;
        if slot.generation == 0x0fff_ffff {
            slot.retired = true;
        } else {
            slot.generation += 1;
        }
        Ok(())
    }

    fn release_owner(&mut self, owner: u32) {
        let ids: Vec<u32> = self
            .slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.allocated && slot.owner == owner)
            .map(|(index, slot)| (slot.generation << 4) | index as u32)
            .collect();
        for id in ids {
            self.close(id, owner).unwrap();
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CacheEntry {
    lba: u32,
    valid: bool,
    bytes: [u8; 512],
}

impl Default for CacheEntry {
    fn default() -> Self {
        Self {
            lba: 0,
            valid: false,
            bytes: [0; 512],
        }
    }
}

#[derive(Clone, Debug)]
struct Cache {
    entries: [CacheEntry; CACHE_ENTRIES],
    reads: usize,
    hits: usize,
    misses: usize,
    replacements: usize,
}

impl Cache {
    fn new() -> Self {
        Self {
            entries: std::array::from_fn(|_| CacheEntry::default()),
            reads: 0,
            hits: 0,
            misses: 0,
            replacements: 0,
        }
    }

    fn read(&mut self, lba: u32, disk: &[[u8; 512]]) -> [u8; 512] {
        let index = lba as usize % CACHE_ENTRIES;
        if self.entries[index].valid && self.entries[index].lba == lba {
            self.hits += 1;
            return self.entries[index].bytes;
        }
        self.misses += 1;
        self.reads += 1;
        if self.entries[index].valid {
            self.replacements += 1;
        }
        let bytes = disk[lba as usize];
        self.entries[index] = CacheEntry {
            lba,
            valid: true,
            bytes,
        };
        bytes
    }

    fn write_through(&mut self, lba: u32, bytes: [u8; 512], device_success: bool) -> bool {
        if !device_success {
            return false;
        }
        let index = lba as usize % CACHE_ENTRIES;
        if self.entries[index].valid && self.entries[index].lba != lba {
            self.replacements += 1;
        }
        self.entries[index] = CacheEntry {
            lba,
            valid: true,
            bytes,
        };
        true
    }
}

#[test]
fn paths_are_absolute_ascii_and_bounded() {
    assert_eq!(parse_path(b"/").unwrap(), Vec::<&[u8]>::new());
    assert_eq!(parse_path(b"/SYSTEM/CONFIG.TXT").unwrap().len(), 2);
    for invalid in [
        b"".as_slice(),
        b"relative",
        b"/SYSTEM/",
        b"/SYSTEM//A",
        b"/./A",
        b"/../A",
        b"/A\0B",
        b"/A/B/C/D/E/F/G/H/I",
    ] {
        assert!(parse_path(invalid).is_err(), "accepted {invalid:?}");
    }
    let oversized = format!("/{}", "A".repeat(32));
    assert!(parse_path(oversized.as_bytes()).is_err());
}

#[test]
fn handles_are_owner_generation_and_capacity_safe() {
    let mut table = Handles::new();
    let first = table.open(7, 2, 1).unwrap();
    assert_eq!(table.validate(first, 8), Err("wrong owner"));
    table.close(first, 7).unwrap();
    assert_eq!(table.validate(first, 7), Err("stale handle"));
    let second = table.open(7, 2, 1).unwrap();
    assert_ne!(first, second);
    table.close(second, 7).unwrap();
    let ids: Vec<_> = (0..MAX_HANDLES)
        .map(|_| table.open(9, 3, 1).unwrap())
        .collect();
    assert!(table.open(9, 3, 1).is_none());
    table.release_owner(9);
    for id in ids {
        assert_eq!(table.validate(id, 9), Err("stale handle"));
    }
}

#[test]
fn direct_mapped_cache_hits_replaces_and_publishes_only_after_write_success() {
    let mut disk = vec![[0u8; 512]; 64];
    disk[5].fill(0x55);
    disk[21].fill(0x21);
    let mut cache = Cache::new();
    assert_eq!(cache.read(5, &disk), [0x55; 512]);
    assert_eq!(cache.read(5, &disk), [0x55; 512]);
    assert_eq!((cache.reads, cache.hits, cache.misses), (1, 1, 1));
    assert_eq!(cache.read(21, &disk), [0x21; 512]);
    assert_eq!(cache.replacements, 1);
    let before = cache.entries[5].clone();
    assert!(!cache.write_through(5, [0xaa; 512], false));
    assert_eq!(cache.entries[5], before);
    assert!(cache.write_through(5, [0xaa; 512], true));
    assert_eq!(cache.entries[5].bytes, [0xaa; 512]);
}

#[test]
fn signed_remainder_matches_i686_truncation_contract() {
    assert_eq!(-17_i32 / 5, -3);
    assert_eq!(-17_i32 % 5, -2);
    assert_eq!(17_i32 / -5, -3);
    assert_eq!(17_i32 % -5, 2);
    assert_eq!(u32::MAX / 16, 0x0fff_ffff);
    assert_eq!(u32::MAX % 16, 15);
    assert!(i32::MIN.checked_div(-1).is_none());
    assert!(1_i32.checked_div(0).is_none());
}
