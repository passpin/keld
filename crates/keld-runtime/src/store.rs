use crate::lifecycle::{EntityKey, Lifecycle};
use crate::slot::{Segment, Slot};
use crate::{EntityId, Generation, Link, RuntimeLifecycleId, RuntimeTypeId, SlotIndex, StoreBrand};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

const SEGMENT_SIZE: usize = 64;
static NEXT_BRAND: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoreError {
    Allocation,
    BrandExhausted,
    InvalidOperation(StoreInvariantError),
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Allocation => formatter.write_str("runtime store allocation failed"),
            Self::BrandExhausted => formatter.write_str("runtime store brands are exhausted"),
            Self::InvalidOperation(error) => {
                write!(formatter, "invalid runtime store operation: {error}")
            }
        }
    }
}

impl std::error::Error for StoreError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoreInvariantError {
    ForeignIdentity,
    StaleEntity,
    InvalidLifecycle,
    NonAncestorKeep,
    RootEnd,
    ActiveChild,
    AlreadyFinished,
}

impl fmt::Display for StoreInvariantError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ForeignIdentity => "identity belongs to another store",
            Self::StaleEntity => "entity identity is no longer live",
            Self::InvalidLifecycle => "lifecycle is invalid or inactive",
            Self::NonAncestorKeep => "keep target is not a strict active ancestor",
            Self::RootEnd => "the root lifecycle cannot end explicitly",
            Self::ActiveChild => "lifecycle still has an active child",
            Self::AlreadyFinished => "store cleanup already completed",
        })
    }
}

pub struct Store<P> {
    brand: StoreBrand,
    segments: Vec<Segment<P>>,
    reusable: Vec<SlotIndex>,
    permanently_retired: Vec<SlotIndex>,
    lifecycles: Vec<Lifecycle>,
    finished: bool,
    max_generation: u32,
}

impl<P> Store<P> {
    /// Creates an empty store with a process-unique identity brand.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::BrandExhausted`] if no unique brand remains, or
    /// [`StoreError::Allocation`] if root lifecycle storage cannot be reserved.
    pub fn new() -> Result<Self, StoreError> {
        Self::with_max_generation(u32::MAX)
    }

    fn with_max_generation(max_generation: u32) -> Result<Self, StoreError> {
        let brand = next_brand()?;
        let mut lifecycles = Vec::new();
        lifecycles
            .try_reserve_exact(1)
            .map_err(|_| StoreError::Allocation)?;
        lifecycles.push(Lifecycle::root(brand));
        Ok(Self {
            brand,
            segments: Vec::new(),
            reusable: Vec::new(),
            permanently_retired: Vec::new(),
            lifecycles,
            finished: false,
            max_generation,
        })
    }

    #[must_use]
    pub const fn root_lifecycle(&self) -> RuntimeLifecycleId {
        RuntimeLifecycleId {
            brand: self.brand,
            index: 0,
        }
    }

    /// Starts a child lifecycle.
    ///
    /// # Errors
    ///
    /// Returns an allocation error or [`StoreInvariantError::InvalidLifecycle`].
    pub fn begin_lifecycle(
        &mut self,
        parent: RuntimeLifecycleId,
    ) -> Result<RuntimeLifecycleId, StoreError> {
        let parent_index = self.lifecycle_index(parent)?;
        let index = u32::try_from(self.lifecycles.len()).map_err(|_| StoreError::Allocation)?;
        self.lifecycles
            .try_reserve(1)
            .map_err(|_| StoreError::Allocation)?;
        let active_children = self.lifecycles[parent_index]
            .active_children
            .checked_add(1)
            .ok_or(StoreError::Allocation)?;
        let id = RuntimeLifecycleId {
            brand: self.brand,
            index,
        };
        self.lifecycles.push(Lifecycle {
            id,
            parent: Some(parent),
            active: true,
            active_children: 0,
            adoptions: Vec::new(),
        });
        self.lifecycles[parent_index].active_children = active_children;
        Ok(id)
    }

    /// Allocates an entity in an active lifecycle.
    ///
    /// # Errors
    ///
    /// Returns an allocation error or [`StoreInvariantError::InvalidLifecycle`].
    pub fn allocate(
        &mut self,
        definition: RuntimeTypeId,
        lifecycle: RuntimeLifecycleId,
        payload: P,
    ) -> Result<EntityId, StoreError> {
        let lifecycle_index = self.lifecycle_index(lifecycle)?;
        self.lifecycles[lifecycle_index]
            .adoptions
            .try_reserve(1)
            .map_err(|_| StoreError::Allocation)?;
        self.ensure_reusable_slot()?;
        let slot_index = self.reusable.pop().ok_or(StoreError::InvalidOperation(
            StoreInvariantError::StaleEntity,
        ))?;
        let generation = match self.slot(slot_index) {
            Some(Slot::Empty { generation }) => *generation,
            _ => {
                return Err(StoreError::InvalidOperation(
                    StoreInvariantError::StaleEntity,
                ));
            }
        };
        let adoption = self.lifecycles[lifecycle_index].adoptions.len();
        let key = EntityKey {
            slot: slot_index,
            generation,
        };
        self.lifecycles[lifecycle_index].adoptions.push(Some(key));
        let Some(slot) = self.slot_mut(slot_index) else {
            return Err(StoreError::InvalidOperation(
                StoreInvariantError::StaleEntity,
            ));
        };
        *slot = Slot::Live {
            generation,
            definition,
            lifecycle,
            adoption,
            payload,
        };
        Ok(EntityId {
            brand: self.brand,
            slot: slot_index,
            generation,
            definition,
        })
    }

    /// Creates a persistent weak link to a live entity.
    ///
    /// # Errors
    ///
    /// Returns an invariant error if the identity is foreign or stale.
    pub fn link(&self, entity: EntityId) -> Result<Link, StoreError> {
        self.validate_entity(entity)?;
        Ok(Link {
            brand: self.brand,
            slot: entity.slot,
            generation: entity.generation,
            expected: entity.definition,
        })
    }

    #[must_use]
    pub fn resolve(&self, link: Link) -> Option<EntityId> {
        if link.brand != self.brand {
            return None;
        }
        match self.slot(link.slot) {
            Some(Slot::Live {
                generation,
                definition,
                ..
            }) if *generation == link.generation && *definition == link.expected => {
                Some(EntityId {
                    brand: self.brand,
                    slot: link.slot,
                    generation: *generation,
                    definition: *definition,
                })
            }
            _ => None,
        }
    }

    /// Reads a live payload through a non-escaping closure.
    ///
    /// # Errors
    ///
    /// Returns an invariant error if the identity is foreign or stale.
    pub fn read<R>(&self, entity: EntityId, access: impl FnOnce(&P) -> R) -> Result<R, StoreError> {
        let slot = self.validate_entity(entity)?;
        let Some(Slot::Live { payload, .. }) = self.slot(slot) else {
            return Err(StoreError::InvalidOperation(
                StoreInvariantError::StaleEntity,
            ));
        };
        Ok(access(payload))
    }

    /// Edits a live payload through a non-escaping closure.
    ///
    /// # Errors
    ///
    /// Returns an invariant error if the identity is foreign or stale.
    pub fn edit<R>(
        &mut self,
        entity: EntityId,
        access: impl FnOnce(&mut P) -> R,
    ) -> Result<R, StoreError> {
        let slot = self.validate_entity(entity)?;
        let Some(Slot::Live { payload, .. }) = self.slot_mut(slot) else {
            return Err(StoreError::InvalidOperation(
                StoreInvariantError::StaleEntity,
            ));
        };
        Ok(access(payload))
    }

    /// Moves custody of a live entity to a strict active ancestor lifecycle.
    ///
    /// # Errors
    ///
    /// Returns an allocation error or an invariant error for an invalid identity,
    /// lifecycle, or ancestry relation.
    pub fn keep(&mut self, entity: EntityId, target: RuntimeLifecycleId) -> Result<(), StoreError> {
        let slot_index = self.validate_entity(entity)?;
        let target_index = self.lifecycle_index(target)?;
        let (source, old_adoption) = match self.slot(slot_index) {
            Some(Slot::Live {
                lifecycle,
                adoption,
                ..
            }) => (*lifecycle, *adoption),
            _ => {
                return Err(StoreError::InvalidOperation(
                    StoreInvariantError::StaleEntity,
                ));
            }
        };
        if !self.is_strict_ancestor(target, source) {
            return Err(StoreError::InvalidOperation(
                StoreInvariantError::NonAncestorKeep,
            ));
        }
        self.validate_adoption(source, old_adoption, entity)?;
        self.lifecycles[target_index]
            .adoptions
            .try_reserve(1)
            .map_err(|_| StoreError::Allocation)?;
        let new_adoption = self.lifecycles[target_index].adoptions.len();
        self.lifecycles[target_index]
            .adoptions
            .push(Some(EntityKey {
                slot: slot_index,
                generation: entity.generation,
            }));
        self.clear_adoption(source, old_adoption, entity)?;
        let Some(Slot::Live {
            lifecycle,
            adoption,
            ..
        }) = self.slot_mut(slot_index)
        else {
            return Err(StoreError::InvalidOperation(
                StoreInvariantError::StaleEntity,
            ));
        };
        *lifecycle = target;
        *adoption = new_adoption;
        Ok(())
    }

    /// Retires one entity and transfers its payload to cleanup.
    ///
    /// # Errors
    ///
    /// Returns an invariant error if the identity is foreign or stale.
    pub fn retire_with(
        &mut self,
        entity: EntityId,
        cleanup: impl FnOnce(P),
    ) -> Result<(), StoreError> {
        let slot_index = self.validate_entity(entity)?;
        let (lifecycle, adoption) = match self.slot(slot_index) {
            Some(Slot::Live {
                lifecycle,
                adoption,
                ..
            }) => (*lifecycle, *adoption),
            _ => {
                return Err(StoreError::InvalidOperation(
                    StoreInvariantError::StaleEntity,
                ));
            }
        };
        self.validate_adoption(lifecycle, adoption, entity)?;
        self.clear_adoption(lifecycle, adoption, entity)?;
        self.mark_dying(slot_index, lifecycle, adoption)?;
        let payload = self.take_dying_payload(slot_index)?;
        cleanup(payload);
        self.finalize_retirement(slot_index);
        Ok(())
    }

    /// Ends an explicit lifecycle and cleans its remaining entities in reverse
    /// adoption order.
    ///
    /// # Errors
    ///
    /// Returns an invariant error for the root, an inactive lifecycle, or a
    /// lifecycle with an active child.
    pub fn end_lifecycle_with(
        &mut self,
        lifecycle: RuntimeLifecycleId,
        mut cleanup: impl FnMut(P),
    ) -> Result<(), StoreError> {
        let index = self.lifecycle_index(lifecycle)?;
        if lifecycle.index == 0 {
            return Err(StoreError::InvalidOperation(StoreInvariantError::RootEnd));
        }
        if self.lifecycles[index].active_children != 0 {
            return Err(StoreError::InvalidOperation(
                StoreInvariantError::ActiveChild,
            ));
        }
        self.cleanup_lifecycle(index, &mut cleanup)
    }

    /// Ends all remaining lifecycles and then the root lifecycle.
    ///
    /// # Errors
    ///
    /// Returns an invariant error if cleanup already completed or internal
    /// lifecycle membership is inconsistent.
    pub fn finish_with(&mut self, mut cleanup: impl FnMut(P)) -> Result<(), StoreError> {
        if self.finished {
            return Err(StoreError::InvalidOperation(
                StoreInvariantError::AlreadyFinished,
            ));
        }
        loop {
            let leaf = self.lifecycles.iter().rposition(|lifecycle| {
                lifecycle.id.index != 0 && lifecycle.active && lifecycle.active_children == 0
            });
            let Some(index) = leaf else {
                break;
            };
            self.cleanup_lifecycle(index, &mut cleanup)?;
        }
        if self.lifecycles[0].active_children != 0 {
            return Err(StoreError::InvalidOperation(
                StoreInvariantError::ActiveChild,
            ));
        }
        self.cleanup_lifecycle(0, &mut cleanup)?;
        self.finished = true;
        Ok(())
    }

    fn cleanup_lifecycle(
        &mut self,
        lifecycle_index: usize,
        cleanup: &mut impl FnMut(P),
    ) -> Result<(), StoreError> {
        let lifecycle = self.lifecycles[lifecycle_index].id;
        if !self.lifecycles[lifecycle_index].active {
            return Err(StoreError::InvalidOperation(
                StoreInvariantError::InvalidLifecycle,
            ));
        }
        let adoption_count = self.lifecycles[lifecycle_index].adoptions.len();
        for adoption in 0..adoption_count {
            if let Some(key) = self.lifecycles[lifecycle_index].adoptions[adoption] {
                self.validate_membership(lifecycle, adoption, key)?;
            }
        }
        for adoption in 0..adoption_count {
            if let Some(key) = self.lifecycles[lifecycle_index].adoptions[adoption] {
                self.mark_dying(key.slot, lifecycle, adoption)?;
            }
        }
        for adoption in (0..adoption_count).rev() {
            let key = self.lifecycles[lifecycle_index].adoptions[adoption];
            if let Some(key) = key {
                let payload = self.take_dying_payload(key.slot)?;
                cleanup(payload);
                self.finalize_retirement(key.slot);
                self.lifecycles[lifecycle_index].adoptions[adoption] = None;
            }
        }
        self.lifecycles[lifecycle_index].active = false;
        if let Some(parent) = self.lifecycles[lifecycle_index].parent {
            let parent_index = parent.index as usize;
            self.lifecycles[parent_index].active_children = self.lifecycles[parent_index]
                .active_children
                .checked_sub(1)
                .ok_or(StoreError::InvalidOperation(
                    StoreInvariantError::InvalidLifecycle,
                ))?;
        }
        Ok(())
    }

    fn ensure_reusable_slot(&mut self) -> Result<(), StoreError> {
        if self.reusable.is_empty() {
            self.extend_segment()?;
        }
        Ok(())
    }

    fn extend_segment(&mut self) -> Result<(), StoreError> {
        let base = self
            .segments
            .len()
            .checked_mul(SEGMENT_SIZE)
            .ok_or(StoreError::Allocation)?;
        let last = base
            .checked_add(SEGMENT_SIZE.saturating_sub(1))
            .ok_or(StoreError::Allocation)?;
        u32::try_from(last).map_err(|_| StoreError::Allocation)?;
        self.segments
            .try_reserve(1)
            .map_err(|_| StoreError::Allocation)?;
        self.reusable
            .try_reserve(SEGMENT_SIZE)
            .map_err(|_| StoreError::Allocation)?;
        self.permanently_retired
            .try_reserve(SEGMENT_SIZE)
            .map_err(|_| StoreError::Allocation)?;
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(SEGMENT_SIZE)
            .map_err(|_| StoreError::Allocation)?;
        for _ in 0..SEGMENT_SIZE {
            slots.push(Slot::Empty {
                generation: Generation(0),
            });
        }
        for offset in (0..SEGMENT_SIZE).rev() {
            self.reusable.push(SlotIndex(
                u32::try_from(base + offset).map_err(|_| StoreError::Allocation)?,
            ));
        }
        self.segments.push(Segment { slots });
        Ok(())
    }

    fn validate_entity(&self, entity: EntityId) -> Result<SlotIndex, StoreError> {
        if entity.brand != self.brand {
            return Err(StoreError::InvalidOperation(
                StoreInvariantError::ForeignIdentity,
            ));
        }
        match self.slot(entity.slot) {
            Some(Slot::Live {
                generation,
                definition,
                ..
            }) if *generation == entity.generation && *definition == entity.definition => {
                Ok(entity.slot)
            }
            _ => Err(StoreError::InvalidOperation(
                StoreInvariantError::StaleEntity,
            )),
        }
    }

    fn lifecycle_index(&self, lifecycle: RuntimeLifecycleId) -> Result<usize, StoreError> {
        if lifecycle.brand != self.brand {
            return Err(StoreError::InvalidOperation(
                StoreInvariantError::InvalidLifecycle,
            ));
        }
        let index = lifecycle.index as usize;
        if self
            .lifecycles
            .get(index)
            .is_none_or(|candidate| candidate.id != lifecycle || !candidate.active)
        {
            return Err(StoreError::InvalidOperation(
                StoreInvariantError::InvalidLifecycle,
            ));
        }
        Ok(index)
    }

    fn is_strict_ancestor(&self, target: RuntimeLifecycleId, source: RuntimeLifecycleId) -> bool {
        let mut current = self
            .lifecycles
            .get(source.index as usize)
            .and_then(|lifecycle| lifecycle.parent);
        while let Some(lifecycle) = current {
            if lifecycle == target {
                return true;
            }
            current = self
                .lifecycles
                .get(lifecycle.index as usize)
                .and_then(|candidate| candidate.parent);
        }
        false
    }

    fn clear_adoption(
        &mut self,
        lifecycle: RuntimeLifecycleId,
        adoption: usize,
        entity: EntityId,
    ) -> Result<(), StoreError> {
        let index = lifecycle.index as usize;
        let expected = Some(EntityKey {
            slot: entity.slot,
            generation: entity.generation,
        });
        let Some(entry) = self
            .lifecycles
            .get_mut(index)
            .and_then(|candidate| candidate.adoptions.get_mut(adoption))
        else {
            return Err(StoreError::InvalidOperation(
                StoreInvariantError::StaleEntity,
            ));
        };
        if *entry != expected {
            return Err(StoreError::InvalidOperation(
                StoreInvariantError::StaleEntity,
            ));
        }
        *entry = None;
        Ok(())
    }

    fn validate_adoption(
        &self,
        lifecycle: RuntimeLifecycleId,
        adoption: usize,
        entity: EntityId,
    ) -> Result<(), StoreError> {
        let expected = Some(EntityKey {
            slot: entity.slot,
            generation: entity.generation,
        });
        if self
            .lifecycles
            .get(lifecycle.index as usize)
            .and_then(|candidate| candidate.adoptions.get(adoption))
            != Some(&expected)
        {
            return Err(StoreError::InvalidOperation(
                StoreInvariantError::StaleEntity,
            ));
        }
        Ok(())
    }

    fn validate_membership(
        &self,
        lifecycle: RuntimeLifecycleId,
        adoption: usize,
        key: EntityKey,
    ) -> Result<(), StoreError> {
        match self.slot(key.slot) {
            Some(Slot::Live {
                generation,
                lifecycle: owner,
                adoption: owner_adoption,
                ..
            }) if *generation == key.generation
                && *owner == lifecycle
                && *owner_adoption == adoption =>
            {
                Ok(())
            }
            _ => Err(StoreError::InvalidOperation(
                StoreInvariantError::StaleEntity,
            )),
        }
    }

    fn mark_dying(
        &mut self,
        slot_index: SlotIndex,
        expected_lifecycle: RuntimeLifecycleId,
        expected_adoption: usize,
    ) -> Result<(), StoreError> {
        let Some(slot) = self.slot_mut(slot_index) else {
            return Err(StoreError::InvalidOperation(
                StoreInvariantError::StaleEntity,
            ));
        };
        let old = std::mem::replace(slot, Slot::Retired);
        match old {
            Slot::Live {
                generation,
                definition,
                lifecycle,
                adoption,
                payload,
            } if lifecycle == expected_lifecycle && adoption == expected_adoption => {
                *slot = Slot::Dying {
                    generation,
                    definition,
                    lifecycle,
                    adoption,
                    payload: Some(payload),
                };
                Ok(())
            }
            other => {
                *slot = other;
                Err(StoreError::InvalidOperation(
                    StoreInvariantError::StaleEntity,
                ))
            }
        }
    }

    fn take_dying_payload(&mut self, slot_index: SlotIndex) -> Result<P, StoreError> {
        let Some(Slot::Dying {
            definition,
            lifecycle,
            adoption,
            payload,
            ..
        }) = self.slot_mut(slot_index)
        else {
            return Err(StoreError::InvalidOperation(
                StoreInvariantError::StaleEntity,
            ));
        };
        let _ = (*definition, *lifecycle, *adoption);
        payload.take().ok_or(StoreError::InvalidOperation(
            StoreInvariantError::StaleEntity,
        ))
    }

    fn finalize_retirement(&mut self, slot_index: SlotIndex) {
        let generation = {
            let Some(slot) = self.slot_mut(slot_index) else {
                return;
            };
            let old = std::mem::replace(slot, Slot::Retired);
            match old {
                Slot::Dying {
                    generation,
                    payload: None,
                    ..
                } => generation,
                other => {
                    *slot = other;
                    return;
                }
            }
        };
        if generation.0 < self.max_generation {
            let Some(next) = generation.0.checked_add(1) else {
                self.permanently_retired.push(slot_index);
                return;
            };
            if let Some(slot) = self.slot_mut(slot_index) {
                *slot = Slot::Empty {
                    generation: Generation(next),
                };
            }
            self.reusable.push(slot_index);
        } else {
            self.permanently_retired.push(slot_index);
        }
    }

    fn slot(&self, index: SlotIndex) -> Option<&Slot<P>> {
        let global = index.0 as usize;
        let segment = global / SEGMENT_SIZE;
        let offset = global % SEGMENT_SIZE;
        self.segments
            .get(segment)
            .and_then(|segment| segment.slots.get(offset))
    }

    fn slot_mut(&mut self, index: SlotIndex) -> Option<&mut Slot<P>> {
        let global = index.0 as usize;
        let segment = global / SEGMENT_SIZE;
        let offset = global % SEGMENT_SIZE;
        self.segments
            .get_mut(segment)
            .and_then(|segment| segment.slots.get_mut(offset))
    }
}

fn next_brand() -> Result<StoreBrand, StoreError> {
    let mut current = NEXT_BRAND.load(Ordering::Relaxed);
    loop {
        let next = current.checked_add(1).ok_or(StoreError::BrandExhausted)?;
        match NEXT_BRAND.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed)
        {
            Ok(_) => return Ok(StoreBrand(current)),
            Err(observed) => current = observed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_exhaustion_permanently_retires_the_slot() {
        let mut store = Store::with_max_generation(2).unwrap();
        let root = store.root_lifecycle();
        let mut slots = Vec::new();
        for value in 0..=2 {
            let entity = store.allocate(RuntimeTypeId(0), root, value).unwrap();
            slots.push((entity.slot_index(), entity.generation()));
            store.retire_with(entity, drop).unwrap();
        }
        let replacement = store.allocate(RuntimeTypeId(0), root, 3).unwrap();

        assert_eq!(slots[0].0, slots[1].0);
        assert_eq!(slots[1].0, slots[2].0);
        assert_eq!(slots[0].1, Generation(0));
        assert_eq!(slots[2].1, Generation(2));
        assert_ne!(replacement.slot_index(), slots[0].0);
        assert_eq!(store.permanently_retired, vec![slots[0].0]);
    }
}
