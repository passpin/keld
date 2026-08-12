use crate::{Generation, RuntimeLifecycleId, RuntimeTypeId};

pub(crate) enum Slot<P> {
    Empty {
        generation: Generation,
    },
    Live {
        generation: Generation,
        definition: RuntimeTypeId,
        lifecycle: RuntimeLifecycleId,
        adoption: usize,
        payload: P,
    },
    Dying {
        generation: Generation,
        definition: RuntimeTypeId,
        lifecycle: RuntimeLifecycleId,
        adoption: usize,
        payload: Option<P>,
    },
    Retired,
}

pub(crate) struct Segment<P> {
    pub slots: Vec<Slot<P>>,
}
