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
    next: usize,
}

impl Ownership {
    fn from_entries(entries: &[Entry]) -> Self {
        let mut ownership = Self {
            used: vec![0xff; BITMAP_BYTES],
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

    fn reserve(&mut self, start: u32, end: u32) {
        self.mark_range(u64::from(start), u64::from(end - start), true);
    }

    fn allocate(&mut self) -> Option<u32> {
        while self.next < FRAME_COUNT {
            let frame = self.next;
            self.next += 1;
            if !self.is_used(frame) {
                self.set(frame, true);
                return u32::try_from(frame * PAGE_SIZE as usize).ok();
            }
        }
        None
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
}
