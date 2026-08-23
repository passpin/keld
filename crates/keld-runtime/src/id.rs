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

    /// Reconstructs an identity received from the versioned native ABI.
    ///
    /// The returned value is only a representation. Store operations still
    /// validate its brand, generation, slot, and definition before use.
    #[cfg(feature = "native-abi")]
    #[must_use]
    pub const fn from_raw_parts(brand: u64, slot: u32, generation: u32, definition: u32) -> Self {
        Self {
            brand: StoreBrand(brand),
            slot: SlotIndex(slot),
            generation: Generation(generation),
            definition: RuntimeTypeId(definition),
        }
    }

    #[cfg(feature = "native-abi")]
    #[must_use]
    pub const fn raw_parts(self) -> (u64, u32, u32, u32) {
        (
            self.brand.0,
            self.slot.0,
            self.generation.0,
            self.definition.0,
        )
    }
}

impl Link {
    /// Reconstructs a link received from the versioned native ABI.
    ///
    /// Store resolution still validates the brand, generation, slot, and
    /// expected definition before exposing an entity identity.
    #[cfg(feature = "native-abi")]
    #[must_use]
    pub const fn from_raw_parts(brand: u64, slot: u32, generation: u32, expected: u32) -> Self {
        Self {
            brand: StoreBrand(brand),
            slot: SlotIndex(slot),
            generation: Generation(generation),
            expected: RuntimeTypeId(expected),
        }
    }

    #[cfg(feature = "native-abi")]
    #[must_use]
    pub const fn raw_parts(self) -> (u64, u32, u32, u32) {
        (
            self.brand.0,
            self.slot.0,
            self.generation.0,
            self.expected.0,
        )
    }
}

impl RuntimeLifecycleId {
    /// Reconstructs a lifecycle identity received from the native ABI.
    /// Store lifecycle operations validate the brand and active index.
    #[cfg(feature = "native-abi")]
    #[must_use]
    pub const fn from_raw_parts(brand: u64, index: u32) -> Self {
        Self {
            brand: StoreBrand(brand),
            index,
        }
    }

    #[cfg(feature = "native-abi")]
    #[must_use]
    pub const fn raw_parts(self) -> (u64, u32) {
        (self.brand.0, self.index)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Link {
    pub(crate) brand: StoreBrand,
    pub(crate) slot: SlotIndex,
    pub(crate) generation: Generation,
    pub(crate) expected: RuntimeTypeId,
}
