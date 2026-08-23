//! Executable models for the DBFS1 v11.9 on-disk and mutation contracts.

const SECTOR: usize = 512;
const DISK_SECTORS: u32 = 16_384;
const FS_BASE: u32 = 64;
const INODE_COUNT: usize = 64;
const INODE_SIZE: usize = 64;
const DIRECT_BLOCKS: usize = 8;
const MAX_FILE: usize = DIRECT_BLOCKS * SECTOR;
const FNV_OFFSET: u32 = 2_166_136_261;
const FNV_PRIME: u32 = 16_777_619;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Layout {
    bitmap_start: u32,
    bitmap_sectors: u32,
    inode_start: u32,
    data_start: u32,
    data_blocks: u32,
}

fn layout(disk_sectors: u32) -> Option<Layout> {
    if disk_sectors <= FS_BASE + 17 {
        return None;
    }
    let filesystem_sectors = disk_sectors.checked_sub(FS_BASE)?;
    let numerator = filesystem_sectors.checked_sub(9)?;
    let bitmap_sectors = numerator.checked_add(4096)? / 4097;
    let inode_start = FS_BASE.checked_add(1)?.checked_add(bitmap_sectors)?;
    let data_start = inode_start.checked_add(8)?;
    let data_blocks = disk_sectors.checked_sub(data_start)?;
    (bitmap_sectors > 0 && data_blocks >= 8 && bitmap_sectors * 4096 >= data_blocks).then_some(
        Layout {
            bitmap_start: FS_BASE + 1,
            bitmap_sectors,
            inode_start,
            data_start,
            data_blocks,
        },
    )
}

fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn get32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn fnv1a(bytes: &[u8]) -> u32 {
    bytes.iter().fold(FNV_OFFSET, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(FNV_PRIME)
    })
}

fn superblock(disk_sectors: u32) -> [u8; SECTOR] {
    let map = layout(disk_sectors).unwrap();
    let mut bytes = [0_u8; SECTOR];
    bytes[..8].copy_from_slice(b"DBFS1\0\0\0");
    for (offset, value) in [
        (8, 1),
        (12, SECTOR as u32),
        (16, FS_BASE),
        (20, disk_sectors - FS_BASE),
        (24, map.bitmap_start),
        (28, map.bitmap_sectors),
        (32, map.inode_start),
        (36, INODE_COUNT as u32),
        (40, 8),
        (44, map.data_start),
        (48, map.data_blocks),
        (52, 1),
        (56, 64),
    ] {
        put32(&mut bytes, offset, value);
    }
    let checksum = fnv1a(&bytes[..60]);
    put32(&mut bytes, 60, checksum);
    bytes
}

fn validate_superblock(bytes: &[u8; SECTOR], disk_sectors: u32) -> bool {
    let Some(map) = layout(disk_sectors) else {
        return false;
    };
    bytes[..8] == *b"DBFS1\0\0\0"
        && get32(bytes, 8) == 1
        && get32(bytes, 12) == 512
        && get32(bytes, 16) == FS_BASE
        && get32(bytes, 20) == disk_sectors - FS_BASE
        && get32(bytes, 24) == map.bitmap_start
        && get32(bytes, 28) == map.bitmap_sectors
        && get32(bytes, 32) == map.inode_start
        && get32(bytes, 36) == INODE_COUNT as u32
        && get32(bytes, 40) == 8
        && get32(bytes, 44) == map.data_start
        && get32(bytes, 48) == map.data_blocks
        && get32(bytes, 52) == 1
        && get32(bytes, 56) == 64
        && get32(bytes, 60) == fnv1a(&bytes[..60])
        && bytes[64..].iter().all(|byte| *byte == 0)
}

fn reseal_superblock(bytes: &mut [u8; SECTOR]) {
    let checksum = fnv1a(&bytes[..60]);
    put32(bytes, 60, checksum);
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Bitmap {
    bits: Vec<bool>,
}

impl Bitmap {
    fn new(blocks: usize) -> Self {
        Self {
            bits: vec![false; blocks],
        }
    }

    fn allocate(&mut self) -> Option<usize> {
        let index = self.bits.iter().position(|allocated| !allocated)?;
        self.bits[index] = true;
        Some(index)
    }

    fn reserve(&mut self, index: usize) -> bool {
        let Some(bit) = self.bits.get_mut(index) else {
            return false;
        };
        if *bit {
            return false;
        }
        *bit = true;
        true
    }

    fn release(&mut self, index: usize) -> bool {
        let Some(bit) = self.bits.get_mut(index) else {
            return false;
        };
        if !*bit {
            return false;
        }
        *bit = false;
        true
    }
}

fn valid_name(name: &[u8]) -> bool {
    !name.is_empty()
        && name.len() <= 31
        && name
            .iter()
            .all(|byte| *byte != 0 && *byte != b'/' && byte.is_ascii())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileModel {
    bytes: Vec<u8>,
    blocks: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Node {
    allocated: bool,
    kind: u32,
    size: usize,
    blocks: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DirRef {
    inode: usize,
    kind: u32,
    name: Vec<u8>,
}

fn validate_namespace(inodes: &[Node], directory: &[DirRef], bitmap: &[bool]) -> bool {
    if inodes.len() != INODE_COUNT
        || inodes[0].allocated
        || !inodes[1].allocated
        || inodes[1].kind != 2
        || inodes[1].size != MAX_FILE
        || inodes[1].blocks != (0..8).collect::<Vec<_>>()
    {
        return false;
    }
    let mut names = std::collections::BTreeSet::new();
    let mut references = [false; INODE_COUNT];
    let mut ownership = vec![false; bitmap.len()];
    for node in inodes.iter().filter(|inode| inode.allocated) {
        if node.blocks.len() > DIRECT_BLOCKS || node.size > MAX_FILE {
            return false;
        }
        for block in &node.blocks {
            let Some(owned) = ownership.get_mut(*block) else {
                return false;
            };
            if *owned {
                return false;
            }
            *owned = true;
        }
    }
    for entry in directory {
        if entry.inode < 2
            || entry.inode >= INODE_COUNT
            || !valid_name(&entry.name)
            || !names.insert(entry.name.clone())
            || references[entry.inode]
        {
            return false;
        }
        let node = &inodes[entry.inode];
        if !node.allocated || node.kind != 1 || entry.kind != 1 {
            return false;
        }
        references[entry.inode] = true;
    }
    for index in 2..INODE_COUNT {
        if inodes[index].allocated != references[index] {
            return false;
        }
    }
    ownership == bitmap
}

impl FileModel {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            blocks: Vec::new(),
        }
    }

    fn write(&mut self, offset: usize, source: &[u8], bitmap: &mut Bitmap) -> bool {
        let Some(end) = offset.checked_add(source.len()) else {
            return false;
        };
        if offset > self.bytes.len() || end > MAX_FILE {
            return false;
        }
        let required = end.div_ceil(SECTOR);
        let old_blocks = self.blocks.len();
        while self.blocks.len() < required {
            let Some(block) = bitmap.allocate() else {
                for block in self.blocks.drain(old_blocks..) {
                    assert!(bitmap.release(block));
                }
                return false;
            };
            self.blocks.push(block);
        }
        self.bytes.resize(end.max(self.bytes.len()), 0);
        self.bytes[offset..end].copy_from_slice(source);
        true
    }

    fn read(&self, offset: usize, count: usize) -> &[u8] {
        if offset >= self.bytes.len() {
            return &[];
        }
        &self.bytes[offset..self.bytes.len().min(offset.saturating_add(count))]
    }
}

#[test]
fn exact_eight_mib_layout_is_stable_and_non_overlapping() {
    let map = layout(DISK_SECTORS).unwrap();
    assert_eq!(map.bitmap_start, 65);
    assert_eq!(map.bitmap_sectors, 4);
    assert_eq!(map.inode_start, 69);
    assert_eq!(map.data_start, 77);
    assert_eq!(map.data_blocks, 16_307);
    assert!(layout(FS_BASE + 17).is_none());
}

#[test]
fn superblock_is_explicit_little_endian_and_rejects_each_defined_byte_change() {
    let valid = superblock(DISK_SECTORS);
    assert!(validate_superblock(&valid, DISK_SECTORS));
    for index in 0..64 {
        let mut corrupt = valid;
        corrupt[index] ^= 0x80;
        assert!(!validate_superblock(&corrupt, DISK_SECTORS), "byte {index}");
    }
    let mut tail = valid;
    tail[511] = 1;
    assert!(!validate_superblock(&tail, DISK_SECTORS));
}

#[test]
fn structurally_invalid_superblocks_fail_even_with_a_recomputed_checksum() {
    for (offset, value) in [
        (8, 2),
        (12, 4096),
        (16, 63),
        (20, 16_319),
        (24, 64),
        (28, 5),
        (32, 68),
        (36, 63),
        (40, 9),
        (44, 76),
        (48, 16_308),
        (52, 0),
        (56, 32),
    ] {
        let mut malformed = superblock(DISK_SECTORS);
        put32(&mut malformed, offset, value);
        reseal_superblock(&mut malformed);
        assert!(
            !validate_superblock(&malformed, DISK_SECTORS),
            "offset {offset}"
        );
    }
}

#[test]
fn bitmap_is_lowest_first_and_exact_release_is_reused() {
    let mut bitmap = Bitmap::new(32);
    for expected in 0..8 {
        assert!(bitmap.reserve(expected));
    }
    assert_eq!(bitmap.allocate(), Some(8));
    assert_eq!(bitmap.allocate(), Some(9));
    assert!(bitmap.release(9));
    assert_eq!(bitmap.allocate(), Some(9));
    assert!(!bitmap.reserve(9));
    assert!(!bitmap.release(31));
    assert!(!bitmap.release(32));
}

#[test]
fn names_obey_the_root_only_ascii_contract() {
    assert!(valid_name(b"PERSIST.TXT"));
    assert!(!valid_name(b""));
    assert!(!valid_name(b"A/B"));
    assert!(!valid_name(b"A\0B"));
    assert!(!valid_name(&[0x80]));
    assert!(!valid_name(&[b'X'; 32]));
}

#[test]
fn partial_cross_sector_write_preserves_bytes_and_eof_is_bounded() {
    let mut bitmap = Bitmap::new(64);
    for block in 0..8 {
        assert!(bitmap.reserve(block));
    }
    let mut file = FileModel::new();
    assert!(file.write(0, &[0x55; 510], &mut bitmap));
    assert!(file.write(510, &[1, 2, 3, 4], &mut bitmap));
    assert_eq!(&file.bytes[508..514], &[0x55, 0x55, 1, 2, 3, 4]);
    assert_eq!(file.blocks, vec![8, 9]);
    assert_eq!(file.read(512, 8), &[3, 4]);
    assert!(file.read(514, 8).is_empty());
}

#[test]
fn file_limit_overflow_and_sparse_offsets_fail_without_mutation() {
    let mut bitmap = Bitmap::new(32);
    let mut file = FileModel::new();
    let before = (file.clone(), bitmap.clone());
    assert!(!file.write(1, b"gap", &mut bitmap));
    assert_eq!((file.clone(), bitmap.clone()), before);
    assert!(!file.write(0, &vec![0; MAX_FILE + 1], &mut bitmap));
    assert_eq!((file, bitmap), before);
}

#[test]
fn allocation_exhaustion_rolls_back_blocks_owned_by_the_failed_write() {
    let mut bitmap = Bitmap::new(9);
    for block in 0..8 {
        assert!(bitmap.reserve(block));
    }
    let mut file = FileModel::new();
    let before = bitmap.clone();
    assert!(!file.write(0, &[0xaa; 1024], &mut bitmap));
    assert!(file.bytes.is_empty() && file.blocks.is_empty());
    assert_eq!(bitmap, before);
}

#[test]
fn namespace_validation_rejects_duplicate_names_blocks_and_stale_ownership() {
    let mut inodes = vec![
        Node {
            allocated: false,
            kind: 0,
            size: 0,
            blocks: vec![],
        };
        INODE_COUNT
    ];
    inodes[1] = Node {
        allocated: true,
        kind: 2,
        size: MAX_FILE,
        blocks: (0..8).collect(),
    };
    inodes[2] = Node {
        allocated: true,
        kind: 1,
        size: 128,
        blocks: vec![8],
    };
    let directory = vec![DirRef {
        inode: 2,
        kind: 1,
        name: b"PERSIST.TXT".to_vec(),
    }];
    let mut bitmap = vec![false; 32];
    bitmap[..9].fill(true);
    assert!(validate_namespace(&inodes, &directory, &bitmap));

    let mut duplicate_block = inodes.clone();
    duplicate_block[3] = Node {
        allocated: true,
        kind: 1,
        size: 1,
        blocks: vec![8],
    };
    let mut second_entry = directory.clone();
    second_entry.push(DirRef {
        inode: 3,
        kind: 1,
        name: b"OTHER".to_vec(),
    });
    assert!(!validate_namespace(
        &duplicate_block,
        &second_entry,
        &bitmap
    ));

    let mut duplicate_name = directory.clone();
    duplicate_name.push(directory[0].clone());
    assert!(!validate_namespace(&inodes, &duplicate_name, &bitmap));
    bitmap[9] = true;
    assert!(!validate_namespace(&inodes, &directory, &bitmap));
}

#[test]
fn inode_and_directory_geometry_are_bounded() {
    assert_eq!(INODE_COUNT * INODE_SIZE, 8 * SECTOR);
    assert_eq!(8 * (SECTOR / 64), 64);
    assert_eq!(DIRECT_BLOCKS * SECTOR, MAX_FILE);
}

#[test]
fn binary_filesystem_lock_is_non_recursive_and_non_sticky() {
    let mut token = true;
    let take = |state: &mut bool| {
        if !*state {
            return false;
        }
        *state = false;
        true
    };
    assert!(take(&mut token));
    assert!(!take(&mut token));
    token = true;
    assert!(take(&mut token));
}
