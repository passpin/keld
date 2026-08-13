use keld_flow::{Place, PlaceProjection, StorageScopeId, ValueId};
use keld_semantics::LocalId;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum HomeId {
    Local(LocalId),
    Temporary(ValueId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LocalStorage {
    Trivial,
    Loan,
    Home { scope: StorageScopeId },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValueStorage {
    Trivial,
    EntityFlow,
    Loan(Place),
    LoanValue {
        root: ValueId,
        projections: Vec<PlaceProjection>,
    },
    OwnedTemporary {
        scope: StorageScopeId,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoreKind {
    Initialize,
    ReplaceLive,
    ReplaceMaybeLive,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CleanupAction {
    Drop(HomeId),
    DropIfLive(HomeId),
    CleanupTrackedScope(StorageScopeId),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OperationStoragePlan {
    pub store: Option<StoreKind>,
    pub post_success: Vec<CleanupAction>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BlockStoragePlan {
    pub operations: Vec<OperationStoragePlan>,
    pub exit: Vec<CleanupAction>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FunctionStoragePlan {
    pub locals: Vec<LocalStorage>,
    pub values: Vec<ValueStorage>,
    pub drop_flags: BTreeSet<HomeId>,
    pub tracked_scopes: BTreeSet<StorageScopeId>,
    pub blocks: Vec<BlockStoragePlan>,
}
