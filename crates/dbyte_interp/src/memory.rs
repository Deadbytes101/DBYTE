use std::collections::BTreeMap;

pub const ZERO_SIZE_DANGLING: u64 = 0x1000;
pub const MAX_ALIGNMENT: u64 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    System,
    Arena,
    Fixed,
    Kernel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryError {
    OutOfMemory,
    UnsupportedAlignment,
    SizeOverflow,
    ContractViolation,
    InvalidPointer,
    DoubleFree,
    UseAfterFree,
    OutOfBounds,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllocationEvent {
    Alloc {
        id: u64,
        address: u64,
        size: u64,
        align: u64,
    },
    Resize {
        id: u64,
        old_address: u64,
        address: u64,
        old_size: u64,
        size: u64,
    },
    Free {
        id: u64,
        address: u64,
        size: u64,
    },
    ArenaFreeNoop {
        id: u64,
        address: u64,
        size: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AllocationState {
    Live,
    Released,
}

#[derive(Debug, Clone)]
struct Allocation {
    id: u64,
    size: u64,
    align: u64,
    bytes: Vec<u8>,
    state: AllocationState,
}

/// Deterministic byte-addressable provider used by both Concrete HIR execution
/// and the canonical bytecode VM. Addresses are language-runtime addresses;
/// they are never host pointers.
#[derive(Debug, Clone)]
pub struct DeterministicMemory {
    kind: ProviderKind,
    base: u64,
    capacity: u64,
    high_water: u64,
    budget: Option<u64>,
    next_id: u64,
    allocations: BTreeMap<u64, Allocation>,
    free_extents: BTreeMap<u64, u64>,
    telemetry: Vec<AllocationEvent>,
}

impl DeterministicMemory {
    pub fn system() -> Self {
        Self::new(ProviderKind::System, 0x1000_0000, u32::MAX as u64)
    }

    pub fn arena(capacity: u64) -> Self {
        Self::new(ProviderKind::Arena, 0x2000_0000, capacity)
    }

    pub fn fixed(capacity: u64) -> Self {
        Self::new(ProviderKind::Fixed, 0x3000_0000, capacity)
    }

    pub fn kernel(capacity: u64) -> Self {
        Self::new(ProviderKind::Kernel, 0xc100_0000, capacity)
    }

    fn new(kind: ProviderKind, base: u64, capacity: u64) -> Self {
        let mut free_extents = BTreeMap::new();
        if matches!(kind, ProviderKind::Fixed | ProviderKind::Kernel) && capacity != 0 {
            free_extents.insert(base, capacity);
        }
        Self {
            kind,
            base,
            capacity,
            high_water: 0,
            budget: None,
            next_id: 1,
            allocations: BTreeMap::new(),
            free_extents,
            telemetry: Vec::new(),
        }
    }

    pub fn set_budget(&mut self, bytes: Option<u64>) {
        self.budget = bytes;
    }

    pub fn telemetry(&self) -> &[AllocationEvent] {
        &self.telemetry
    }

    pub fn live_allocations(&self) -> usize {
        self.allocations
            .values()
            .filter(|allocation| allocation.state == AllocationState::Live)
            .count()
    }

    pub fn alloc(&mut self, size: u64, align: u64) -> Result<u64, MemoryError> {
        validate_alignment(align)?;
        if size == 0 {
            return Ok(ZERO_SIZE_DANGLING);
        }
        if self.budget.is_some_and(|budget| {
            self.live_bytes()
                .checked_add(size)
                .is_none_or(|sum| sum > budget)
        }) {
            return Err(MemoryError::OutOfMemory);
        }
        let next_id = self
            .next_id
            .checked_add(1)
            .ok_or(MemoryError::SizeOverflow)?;
        let address = match self.kind {
            ProviderKind::Fixed | ProviderKind::Kernel => self.allocate_extent(size, align)?,
            ProviderKind::System | ProviderKind::Arena => {
                let address = align_up(
                    self.base
                        .checked_add(self.high_water)
                        .ok_or(MemoryError::SizeOverflow)?,
                    align,
                )?;
                let end = address.checked_add(size).ok_or(MemoryError::SizeOverflow)?;
                let limit = self
                    .base
                    .checked_add(self.capacity)
                    .ok_or(MemoryError::SizeOverflow)?;
                if end > limit {
                    return Err(MemoryError::OutOfMemory);
                }
                self.high_water = end - self.base;
                address
            }
        };
        let id = self.next_id;
        self.next_id = next_id;
        self.allocations.insert(
            address,
            Allocation {
                id,
                size,
                align,
                bytes: vec![0; size as usize],
                state: AllocationState::Live,
            },
        );
        self.telemetry.push(AllocationEvent::Alloc {
            id,
            address,
            size,
            align,
        });
        Ok(address)
    }

    pub fn resize(
        &mut self,
        address: u64,
        old_size: u64,
        new_size: u64,
        align: u64,
    ) -> Result<u64, MemoryError> {
        validate_alignment(align)?;
        if old_size == 0 {
            if address != ZERO_SIZE_DANGLING {
                return Err(MemoryError::ContractViolation);
            }
            return self.alloc(new_size, align);
        }
        self.validate_live_allocation(address, old_size, align)?;
        if new_size == 0 {
            self.free(address, old_size, align)?;
            return Ok(ZERO_SIZE_DANGLING);
        }
        if new_size == old_size {
            return Ok(address);
        }

        // Allocate before mutating the old record. Failure therefore preserves
        // pointer, bytes, size and the complete provider state.
        let snapshot = self.clone();
        let new_address = self.alloc(new_size, align)?;
        let copy_len = old_size.min(new_size) as usize;
        let old_bytes = self.allocations[&address].bytes[..copy_len].to_vec();
        if let Err(error) = self.write(new_address, 0, &old_bytes) {
            *self = snapshot;
            return Err(error);
        }
        let old_id = self.allocations[&address].id;
        if let Err(error) = self.free(address, old_size, align) {
            *self = snapshot;
            return Err(error);
        }
        let new_id = self.allocations[&new_address].id;
        self.telemetry.push(AllocationEvent::Resize {
            id: new_id,
            old_address: address,
            address: new_address,
            old_size,
            size: new_size,
        });
        debug_assert_ne!(old_id, 0);
        Ok(new_address)
    }

    pub fn free(&mut self, address: u64, size: u64, align: u64) -> Result<(), MemoryError> {
        validate_alignment(align)?;
        if size == 0 {
            return if address == ZERO_SIZE_DANGLING {
                Ok(())
            } else {
                Err(MemoryError::ContractViolation)
            };
        }
        match self.allocations.get(&address) {
            Some(allocation) if allocation.state == AllocationState::Released => {
                return Err(MemoryError::DoubleFree)
            }
            Some(allocation) if allocation.size != size || allocation.align != align => {
                return Err(MemoryError::ContractViolation)
            }
            Some(_) => {}
            None => return Err(MemoryError::InvalidPointer),
        }
        let allocation = self
            .allocations
            .get_mut(&address)
            .expect("validated allocation");
        allocation.state = AllocationState::Released;
        let id = allocation.id;
        if self.kind == ProviderKind::Arena {
            self.telemetry
                .push(AllocationEvent::ArenaFreeNoop { id, address, size });
            return Ok(());
        }
        if matches!(self.kind, ProviderKind::Fixed | ProviderKind::Kernel) {
            self.insert_free_extent(address, size)?;
        }
        self.telemetry
            .push(AllocationEvent::Free { id, address, size });
        Ok(())
    }

    pub fn read(&self, address: u64, offset: u64, length: u64) -> Result<Vec<u8>, MemoryError> {
        let allocation = self.live_record(address)?;
        let end = offset
            .checked_add(length)
            .ok_or(MemoryError::SizeOverflow)?;
        if end > allocation.size {
            return Err(MemoryError::OutOfBounds);
        }
        Ok(allocation.bytes[offset as usize..end as usize].to_vec())
    }

    pub fn read_address(&self, address: u64, length: u64) -> Result<Vec<u8>, MemoryError> {
        let (base, offset) = self.resolve_address(address, length)?;
        self.read(base, offset, length)
    }

    pub fn write(&mut self, address: u64, offset: u64, bytes: &[u8]) -> Result<(), MemoryError> {
        let allocation = self.live_record_mut(address)?;
        let end = offset
            .checked_add(bytes.len() as u64)
            .ok_or(MemoryError::SizeOverflow)?;
        if end > allocation.size {
            return Err(MemoryError::OutOfBounds);
        }
        allocation.bytes[offset as usize..end as usize].copy_from_slice(bytes);
        Ok(())
    }

    pub fn write_address(&mut self, address: u64, bytes: &[u8]) -> Result<(), MemoryError> {
        let (base, offset) = self.resolve_address(address, bytes.len() as u64)?;
        self.write(base, offset, bytes)
    }

    fn resolve_address(&self, address: u64, length: u64) -> Result<(u64, u64), MemoryError> {
        let (base, allocation) = self
            .allocations
            .range(..=address)
            .next_back()
            .ok_or(MemoryError::InvalidPointer)?;
        if allocation.state != AllocationState::Live {
            return Err(MemoryError::UseAfterFree);
        }
        let offset = address
            .checked_sub(*base)
            .ok_or(MemoryError::InvalidPointer)?;
        let end = offset
            .checked_add(length)
            .ok_or(MemoryError::SizeOverflow)?;
        if end > allocation.size {
            return Err(MemoryError::OutOfBounds);
        }
        Ok((*base, offset))
    }

    fn live_bytes(&self) -> u64 {
        self.allocations
            .values()
            .filter(|allocation| allocation.state == AllocationState::Live)
            .map(|allocation| allocation.size)
            .sum()
    }

    fn validate_live_allocation(
        &self,
        address: u64,
        size: u64,
        align: u64,
    ) -> Result<(), MemoryError> {
        let allocation = self.live_record(address)?;
        if allocation.size != size || allocation.align != align {
            return Err(MemoryError::ContractViolation);
        }
        Ok(())
    }

    fn live_record(&self, address: u64) -> Result<&Allocation, MemoryError> {
        match self.allocations.get(&address) {
            Some(allocation) if allocation.state == AllocationState::Live => Ok(allocation),
            Some(_) => Err(MemoryError::UseAfterFree),
            None => Err(MemoryError::InvalidPointer),
        }
    }

    fn live_record_mut(&mut self, address: u64) -> Result<&mut Allocation, MemoryError> {
        match self.allocations.get_mut(&address) {
            Some(allocation) if allocation.state == AllocationState::Live => Ok(allocation),
            Some(_) => Err(MemoryError::UseAfterFree),
            None => Err(MemoryError::InvalidPointer),
        }
    }

    fn allocate_extent(&mut self, size: u64, align: u64) -> Result<u64, MemoryError> {
        let selected = self.free_extents.iter().find_map(|(&start, &length)| {
            let aligned = align_up(start, align).ok()?;
            let end = aligned.checked_add(size)?;
            (end <= start.checked_add(length)?).then_some((start, length, aligned, end))
        });
        let (start, length, aligned, end) = selected.ok_or(MemoryError::OutOfMemory)?;
        self.free_extents.remove(&start);
        if aligned > start {
            self.free_extents.insert(start, aligned - start);
        }
        let extent_end = start + length;
        if end < extent_end {
            self.free_extents.insert(end, extent_end - end);
        }
        Ok(aligned)
    }

    fn insert_free_extent(&mut self, mut start: u64, mut size: u64) -> Result<(), MemoryError> {
        if let Some((&left_start, &left_size)) = self.free_extents.range(..start).next_back() {
            if left_start.checked_add(left_size) == Some(start) {
                self.free_extents.remove(&left_start);
                start = left_start;
                size = size
                    .checked_add(left_size)
                    .ok_or(MemoryError::SizeOverflow)?;
            }
        }
        if let Some((&right_start, &right_size)) = self.free_extents.range(start..).next() {
            if start.checked_add(size) == Some(right_start) {
                self.free_extents.remove(&right_start);
                size = size
                    .checked_add(right_size)
                    .ok_or(MemoryError::SizeOverflow)?;
            }
        }
        self.free_extents.insert(start, size);
        Ok(())
    }
}

fn validate_alignment(align: u64) -> Result<(), MemoryError> {
    if align == 0 || !align.is_power_of_two() || align > MAX_ALIGNMENT {
        Err(MemoryError::UnsupportedAlignment)
    } else {
        Ok(())
    }
}

fn align_up(value: u64, align: u64) -> Result<u64, MemoryError> {
    value
        .checked_add(align - 1)
        .map(|value| value & !(align - 1))
        .ok_or(MemoryError::SizeOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_size_never_calls_provider() {
        let mut memory = DeterministicMemory::fixed(128);
        assert_eq!(memory.alloc(0, 16), Ok(ZERO_SIZE_DANGLING));
        assert_eq!(
            memory.resize(ZERO_SIZE_DANGLING, 0, 0, 16),
            Ok(ZERO_SIZE_DANGLING)
        );
        assert_eq!(memory.free(ZERO_SIZE_DANGLING, 0, 16), Ok(()));
        assert!(memory.telemetry().is_empty());
    }

    #[test]
    fn fixed_provider_splits_coalesces_and_reuses_lowest_extent() {
        let mut memory = DeterministicMemory::fixed(256);
        let first = memory.alloc(64, 16).unwrap();
        let second = memory.alloc(64, 16).unwrap();
        let third = memory.alloc(64, 16).unwrap();
        memory.free(second, 64, 16).unwrap();
        memory.free(first, 64, 16).unwrap();
        let merged = memory.alloc(96, 16).unwrap();
        assert_eq!(merged, first);
        memory.free(third, 64, 16).unwrap();
        memory.free(merged, 96, 16).unwrap();
        assert_eq!(memory.alloc(256, 16).unwrap(), first);
    }

    #[test]
    fn resize_failure_preserves_old_allocation_and_bytes() {
        let mut memory = DeterministicMemory::fixed(64);
        let address = memory.alloc(48, 16).unwrap();
        memory.write(address, 0, b"DBYTE").unwrap();
        let before = memory.telemetry().to_vec();
        assert_eq!(
            memory.resize(address, 48, 80, 16),
            Err(MemoryError::OutOfMemory)
        );
        assert_eq!(memory.read(address, 0, 5).unwrap(), b"DBYTE");
        assert_eq!(memory.telemetry(), before);
    }

    #[test]
    fn arena_free_is_observable_noop_and_dead_to_the_owner() {
        let mut memory = DeterministicMemory::arena(64);
        let address = memory.alloc(16, 4).unwrap();
        memory.free(address, 16, 4).unwrap();
        assert_eq!(memory.read(address, 0, 1), Err(MemoryError::UseAfterFree));
        assert!(matches!(
            memory.telemetry().last(),
            Some(AllocationEvent::ArenaFreeNoop { .. })
        ));
        assert_eq!(memory.free(address, 16, 4), Err(MemoryError::DoubleFree));
    }

    #[test]
    fn configurable_budget_is_deterministic() {
        let mut memory = DeterministicMemory::system();
        memory.set_budget(Some(32));
        let address = memory.alloc(24, 8).unwrap();
        assert_eq!(memory.alloc(16, 8), Err(MemoryError::OutOfMemory));
        memory.free(address, 24, 8).unwrap();
        assert!(memory.alloc(16, 8).is_ok());
    }
}
