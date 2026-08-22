//! Test-only oracle for the bounded v11.3 physical-memory contract.

const PAGE_SIZE: u64 = 4096;
const FRAME_COUNT: usize = 1 << 20;
const BITMAP_BYTES: usize = FRAME_COUNT / 8;
const MAX_MAP_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Entry {
    base: u64,
    length: u64,
    kind: u32,
}

fn parse_map(bytes: &[u8]) -> Result<Vec<Entry>, &'static str> {
    if bytes.is_empty() || bytes.len() > MAX_MAP_BYTES {
        return Err("memory map length is outside the bounded contract");
    }
    let mut cursor = 0usize;
    let mut entries = Vec::new();
    while cursor < bytes.len() {
        let header_end = cursor.checked_add(4).ok_or("memory map cursor overflow")?;
        if header_end > bytes.len() {
            return Err("truncated memory map entry size");
        }
        let payload = u32::from_le_bytes(bytes[cursor..header_end].try_into().unwrap()) as usize;
        if payload < 20 {
            return Err("memory map entry payload is shorter than 20 bytes");
        }
        let stride = payload
            .checked_add(4)
            .ok_or("memory map entry stride overflow")?;
        let next = cursor
            .checked_add(stride)
            .ok_or("memory map cursor overflow")?;
        if next > bytes.len() {
            return Err("memory map entry extends past bounded bytes");
        }
        let base = u64::from_le_bytes(bytes[cursor + 4..cursor + 12].try_into().unwrap());
        let length = u64::from_le_bytes(bytes[cursor + 12..cursor + 20].try_into().unwrap());
        let kind = u32::from_le_bytes(bytes[cursor + 20..cursor + 24].try_into().unwrap());
        let end = base
            .checked_add(length)
            .ok_or("physical range arithmetic overflow")?;
        if base > u64::from(u32::MAX) || end > (1u64 << 32) {
            return Err("physical range exceeds non-PAE 32-bit address space");
        }
        entries.push(Entry { base, length, kind });
        cursor = next;
    }
    Ok(entries)
}

#[derive(Clone, Debug)]
struct Ownership {
    used: Vec<u8>,
    owned: Vec<u8>,
    next: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReleaseError {
    Misaligned,
    Outside,
    NotOwned,
}

impl Ownership {
    fn from_entries(entries: &[Entry]) -> Self {
        let mut ownership = Self {
            used: vec![0xff; BITMAP_BYTES],
            owned: vec![0; BITMAP_BYTES],
            next: 0x100,
        };
        for entry in entries.iter().filter(|entry| entry.kind == 1) {
            ownership.mark_range(entry.base, entry.length, false);
        }
        for entry in entries.iter().filter(|entry| entry.kind != 1) {
            ownership.mark_range(entry.base, entry.length, true);
        }
        ownership.mark_range(0, 0x10_0000, true);
        ownership
    }

    fn mark_range(&mut self, base: u64, length: u64, used: bool) {
        let end = base.saturating_add(length).min(1u64 << 32);
        let first = if used {
            base / PAGE_SIZE
        } else {
            base.saturating_add(PAGE_SIZE - 1) / PAGE_SIZE
        };
        let limit = if used {
            end.saturating_add(PAGE_SIZE - 1).min(1u64 << 32) / PAGE_SIZE
        } else {
            end / PAGE_SIZE
        };
        for frame in first..limit {
            self.set(frame as usize, used);
        }
    }

    fn set(&mut self, frame: usize, used: bool) {
        let byte = frame / 8;
        let mask = 1u8 << (frame % 8);
        if used {
            self.used[byte] |= mask;
        } else {
            self.used[byte] &= !mask;
        }
    }

    fn is_used(&self, frame: usize) -> bool {
        self.used[frame / 8] & (1u8 << (frame % 8)) != 0
    }

    fn is_owned(&self, frame: usize) -> bool {
        self.owned[frame / 8] & (1u8 << (frame % 8)) != 0
    }

    fn reserve(&mut self, start: u32, end: u32) {
        self.mark_range(u64::from(start), u64::from(end - start), true);
    }

    fn allocate(&mut self) -> Option<u32> {
        while self.next < FRAME_COUNT {
            let frame = self.next;
            self.next += 1;
            if !self.is_used(frame) {
                self.set(frame, true);
                self.owned[frame / 8] |= 1u8 << (frame % 8);
                return u32::try_from(frame * PAGE_SIZE as usize).ok();
            }
        }
        None
    }

    fn release(&mut self, address: u32) -> Result<(), ReleaseError> {
        if u64::from(address) % PAGE_SIZE != 0 {
            return Err(ReleaseError::Misaligned);
        }
        let frame = address as usize / PAGE_SIZE as usize;
        if frame >= FRAME_COUNT {
            return Err(ReleaseError::Outside);
        }
        if !self.is_owned(frame) {
            return Err(ReleaseError::NotOwned);
        }
        self.owned[frame / 8] &= !(1u8 << (frame % 8));
        self.set(frame, false);
        self.next = self.next.min(frame);
        Ok(())
    }
}

const ARENA_PAGES: usize = 1024;

#[derive(Clone, Debug)]
struct ArenaMap {
    pages: Vec<Option<u32>>,
    invalidations: usize,
}

impl ArenaMap {
    fn new() -> Self {
        Self {
            pages: vec![None; ARENA_PAGES],
            invalidations: 0,
        }
    }

    fn map(&mut self, index: usize, frame: u32) -> Result<(), &'static str> {
        if index >= ARENA_PAGES || frame == 0 || u64::from(frame) % PAGE_SIZE != 0 {
            return Err("invalid page mapping");
        }
        if self.pages[index].is_some() {
            return Err("page mapping collision");
        }
        self.pages[index] = Some(frame);
        self.invalidations += 1;
        Ok(())
    }

    fn unmap(&mut self, index: usize) -> Result<u32, &'static str> {
        let page = self.pages.get_mut(index).ok_or("outside arena")?;
        let frame = page.take().ok_or("page is not mapped")?;
        self.invalidations += 1;
        Ok(frame)
    }
}

#[derive(Clone, Debug)]
struct Block {
    start: usize,
    size: usize,
    used: bool,
    checksum: u32,
}

impl Block {
    fn seal(&mut self) {
        self.checksum =
            (self.start as u32) ^ (self.size as u32) ^ u32::from(self.used) ^ 0xd8b1_0c42;
    }

    fn valid(&self) -> bool {
        self.checksum
            == (self.start as u32) ^ (self.size as u32) ^ u32::from(self.used) ^ 0xd8b1_0c42
    }
}

#[derive(Clone, Debug)]
struct HeapModel {
    blocks: Vec<Block>,
    poisoned: bool,
}

impl HeapModel {
    fn new(size: usize) -> Self {
        let mut block = Block {
            start: 0,
            size,
            used: false,
            checksum: 0,
        };
        block.seal();
        Self {
            blocks: vec![block],
            poisoned: false,
        }
    }

    fn allocate(&mut self, size: usize, alignment: usize) -> Result<usize, &'static str> {
        if self.poisoned {
            return Err("heap is poisoned");
        }
        if alignment == 0 || alignment > 4096 || !alignment.is_power_of_two() {
            return Err("invalid alignment");
        }
        if size == 0 {
            return Ok(0);
        }
        let needed = size.checked_add(63).ok_or("allocation size overflow")? & !15;
        for index in 0..self.blocks.len() {
            if !self.blocks[index].valid() {
                self.poisoned = true;
                return Err("metadata corruption");
            }
            if !self.blocks[index].used && self.blocks[index].size >= needed {
                let start = self.blocks[index].start;
                let remainder = self.blocks[index].size - needed;
                self.blocks[index].size = needed;
                self.blocks[index].used = true;
                self.blocks[index].seal();
                if remainder >= 64 {
                    let mut tail = Block {
                        start: start + needed,
                        size: remainder,
                        used: false,
                        checksum: 0,
                    };
                    tail.seal();
                    self.blocks.insert(index + 1, tail);
                }
                return Ok(start + 48);
            }
        }
        Err("arena exhausted")
    }

    fn free(&mut self, pointer: usize) -> Result<(), &'static str> {
        if pointer == 0 {
            return Ok(());
        }
        let index = self
            .blocks
            .iter()
            .position(|block| block.start.checked_add(48) == Some(pointer))
            .ok_or("outside or interior pointer")?;
        if !self.blocks[index].valid() {
            self.poisoned = true;
            return Err("metadata corruption");
        }
        if !self.blocks[index].used {
            return Err("double free");
        }
        self.blocks[index].used = false;
        self.blocks[index].seal();
        if index + 1 < self.blocks.len() && !self.blocks[index + 1].used {
            let right = self.blocks.remove(index + 1);
            self.blocks[index].size += right.size;
            self.blocks[index].seal();
        }
        if index > 0 && !self.blocks[index - 1].used {
            let current = self.blocks.remove(index);
            self.blocks[index - 1].size += current.size;
            self.blocks[index - 1].seal();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(entries: &[Entry]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for entry in entries {
            bytes.extend_from_slice(&20u32.to_le_bytes());
            bytes.extend_from_slice(&entry.base.to_le_bytes());
            bytes.extend_from_slice(&entry.length.to_le_bytes());
            bytes.extend_from_slice(&entry.kind.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn malformed_lengths_strides_and_physical_overflow_are_rejected() {
        assert!(parse_map(&[]).is_err());
        assert!(parse_map(&vec![0; MAX_MAP_BYTES + 1]).is_err());
        assert!(parse_map(&16u32.to_le_bytes()).is_err());

        let mut truncated = 20u32.to_le_bytes().to_vec();
        truncated.extend_from_slice(&[0; 19]);
        assert!(parse_map(&truncated).is_err());

        let above = encoded(&[Entry {
            base: u64::from(u32::MAX) + 1,
            length: PAGE_SIZE,
            kind: 1,
        }]);
        assert!(parse_map(&above).is_err());
        let crosses = encoded(&[Entry {
            base: u64::from(u32::MAX) - 4095,
            length: PAGE_SIZE + 1,
            kind: 1,
        }]);
        assert!(parse_map(&crosses).is_err());

        let exact_top = encoded(&[Entry {
            base: 0xffff_f000,
            length: PAGE_SIZE,
            kind: 2,
        }]);
        assert_eq!(parse_map(&exact_top).unwrap().len(), 1);
    }

    #[test]
    fn reserved_ranges_win_independently_of_map_order() {
        let usable = Entry {
            base: 0x10_0000,
            length: 0x40_000,
            kind: 1,
        };
        let reserved = Entry {
            base: 0x12_0000,
            length: PAGE_SIZE,
            kind: 2,
        };
        let first = Ownership::from_entries(&[usable, reserved]);
        let second = Ownership::from_entries(&[reserved, usable]);
        assert_eq!(first.used, second.used);
        assert!(first.is_used(0x120));
        assert!(!first.is_used(0x121));
    }

    #[test]
    fn allocation_is_ascending_unique_and_skips_owned_regions() {
        let mut ownership = Ownership::from_entries(&[Entry {
            base: 0x10_0000,
            length: 0x20_000,
            kind: 1,
        }]);
        ownership.reserve(0x10_0000, 0x11_0000);
        let first = ownership.allocate().unwrap();
        let second = ownership.allocate().unwrap();
        assert_eq!(first, 0x11_0000);
        assert_eq!(second, 0x11_1000);
        assert_ne!(first, second);
        assert!(ownership.is_used((first / 4096) as usize));
        assert!(ownership.is_used((second / 4096) as usize));
    }

    #[test]
    fn release_rejects_invalid_ownership_and_rewinds_first_fit() {
        let mut ownership = Ownership::from_entries(&[Entry {
            base: 0x10_0000,
            length: 0x20_000,
            kind: 1,
        }]);
        let first = ownership.allocate().unwrap();
        let second = ownership.allocate().unwrap();
        assert_eq!(ownership.release(first + 1), Err(ReleaseError::Misaligned));
        assert_eq!(ownership.release(0), Err(ReleaseError::NotOwned));
        ownership.release(first).unwrap();
        assert_eq!(ownership.release(first), Err(ReleaseError::NotOwned));
        assert_eq!(ownership.allocate(), Some(first));
        assert_ne!(first, second);
    }

    #[test]
    fn arena_mapping_is_transactional_and_invalidates_on_change() {
        let mut arena = ArenaMap::new();
        arena.map(0, 0x12_0000).unwrap();
        let before = arena.clone();
        assert_eq!(arena.map(0, 0x13_0000), Err("page mapping collision"));
        assert_eq!(arena.pages, before.pages);
        assert_eq!(arena.invalidations, before.invalidations);
        assert_eq!(arena.unmap(0), Ok(0x12_0000));
        assert_eq!(arena.invalidations, 2);
        assert!(arena.unmap(0).is_err());
        assert!(arena.map(ARENA_PAGES, 0x14_0000).is_err());
    }

    #[test]
    fn heap_model_splits_reuses_coalesces_and_fails_closed() {
        let mut heap = HeapModel::new(4096);
        let before = heap.clone();
        assert_eq!(heap.allocate(0, 16), Ok(0));
        assert_eq!(heap.blocks.len(), before.blocks.len());
        heap.free(0).unwrap();
        let first = heap.allocate(64, 16).unwrap();
        let second = heap.allocate(128, 16).unwrap();
        heap.free(first).unwrap();
        assert_eq!(heap.allocate(32, 16).unwrap(), first);
        assert!(heap.free(first + 1).is_err());
        heap.free(second).unwrap();
        assert_eq!(heap.free(second), Err("double free"));
        assert!(heap.allocate(usize::MAX, 16).is_err());
        assert!(heap.allocate(1, 3).is_err());
        heap.blocks[0].checksum ^= 1;
        assert_eq!(heap.allocate(1, 1), Err("metadata corruption"));
        assert!(heap.poisoned);
        assert_eq!(heap.allocate(1, 1), Err("heap is poisoned"));

        let mut bounded = HeapModel::new(128);
        assert!(bounded.allocate(4096, 16).is_err());
    }

    #[test]
    fn memmove_overlap_direction_preserves_bytes() {
        let mut bytes: Vec<u8> = (0..32).collect();
        bytes.copy_within(0..24, 8);
        assert_eq!(&bytes[8..32], &(0u8..24).collect::<Vec<_>>());
        bytes.copy_within(8..32, 0);
        assert_eq!(&bytes[0..24], &(0u8..24).collect::<Vec<_>>());
    }
}
