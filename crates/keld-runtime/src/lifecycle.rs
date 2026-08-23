use crate::{Generation, RuntimeLifecycleId, SlotIndex, StoreBrand};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EntityKey {
    pub slot: SlotIndex,
    pub generation: Generation,
}

pub(crate) struct Lifecycle {
    pub id: RuntimeLifecycleId,
    pub parent: Option<RuntimeLifecycleId>,
    pub active: bool,
    pub active_children: u32,
    pub adoptions: Vec<Option<EntityKey>>,
}

impl Lifecycle {
    pub fn root(brand: StoreBrand) -> Self {
        Self {
            id: RuntimeLifecycleId { brand, index: 0 },
            parent: None,
            active: true,
            active_children: 0,
            adoptions: Vec::new(),
        }
    }
}
