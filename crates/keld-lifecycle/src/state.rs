use keld_flow::LifecycleId;
use keld_source::Span;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProofId(pub u32);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProvenanceId(pub u32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleFact {
    Known(LifecycleId),
    Dynamic,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RefState {
    Live {
        lifecycle: LifecycleFact,
        provenance: ProvenanceId,
    },
    Invalidated {
        cause: Span,
    },
    Retired {
        cause: Span,
    },
    OutOfScope,
}
