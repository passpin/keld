#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct StoreBrand(pub(crate) u64);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SlotIndex(pub u32);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Generation(pub u32);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RuntimeLifecycleId {
    pub(crate) brand: StoreBrand,
    pub(crate) index: u32,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RuntimeTypeId(pub u32);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct EntityId {
    pub(crate) brand: StoreBrand,
    pub(crate) slot: SlotIndex,
    pub(crate) generation: Generation,
    pub(crate) definition: RuntimeTypeId,
}

impl EntityId {
    #[must_use]
    pub const fn slot_index(self) -> SlotIndex {
        self.slot
    }

    #[must_use]
    pub const fn generation(self) -> Generation {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Link {
    pub(crate) brand: StoreBrand,
    pub(crate) slot: SlotIndex,
    pub(crate) generation: Generation,
    pub(crate) expected: RuntimeTypeId,
}
