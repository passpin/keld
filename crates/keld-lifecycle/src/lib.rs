mod diagnostics;
mod effects;
mod facts;
mod provenance;
mod state;
mod verify;

pub use effects::{FunctionSummary, ReturnProvenance};
pub use facts::{AliasRelation, EntityOperationFacts, EntityOriginFact, EntityReferenceFact};
pub use state::{LifecycleFact, ProofId, ProvenanceId, RefState};
pub use verify::{ProofAnnotation, Verification, VerifiedFlowModule, verify, verify_text_for_test};
