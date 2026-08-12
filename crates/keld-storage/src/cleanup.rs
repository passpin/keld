use crate::plan::{
    BlockStoragePlan, FunctionStoragePlan, LocalStorage, OperationStoragePlan, ValueStorage,
};
use keld_flow::{FlowFunction, FlowModule, FlowOp, Place, StorageScopeId, ValueId};
use keld_semantics::{LocalId, ParameterMode, StorageClass, TypeKind};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ValueOrigin {
    Implicit,
    Local(LocalId),
    Borrowed(LocalId),
    BorrowedPlace { place: Place, loaned: bool },
    BorrowedUnknown,
    Entity(LocalId),
    Owned,
    Unknown,
}

pub(crate) fn new_function_plan(flow: &FlowModule, function: &FlowFunction) -> FunctionStoragePlan {
    let locals = function
        .local_types
        .iter()
        .enumerate()
        .map(|(index, ty)| {
            if flow.types.storage_class(*ty) != StorageClass::SingleHome {
                return LocalStorage::Trivial;
            }
            let local = LocalId(u32::try_from(index).unwrap_or(u32::MAX));
            let parameter_mode = function
                .parameters
                .iter()
                .enumerate()
                .find(|(_, parameter)| **parameter == local)
                .and_then(|(parameter, _)| function.parameter_modes.get(parameter))
                .copied();
            if parameter_mode == Some(ParameterMode::Loan) {
                LocalStorage::Loan
            } else {
                LocalStorage::Home {
                    scope: function
                        .local_scopes
                        .get(index)
                        .copied()
                        .unwrap_or(StorageScopeId(0)),
                }
            }
        })
        .collect();
    let blocks = function
        .blocks
        .iter()
        .map(|block| BlockStoragePlan {
            operations: vec![OperationStoragePlan::default(); block.operations.len()],
            exit: Vec::new(),
        })
        .collect();
    FunctionStoragePlan {
        locals,
        values: vec![ValueStorage::Trivial; function.value_types.len()],
        blocks,
        ..FunctionStoragePlan::default()
    }
}

pub(crate) fn record_operation(
    flow: &FlowModule,
    function: &FlowFunction,
    operation: &FlowOp,
    origins: &[ValueOrigin],
    scope: StorageScopeId,
    plan: &mut FunctionStoragePlan,
) {
    let Some(dst) = defined_value(operation) else {
        return;
    };
    let Some(origin) = origins.get(dst.0 as usize) else {
        return;
    };
    let Some(value_type) = function.value_types.get(dst.0 as usize) else {
        return;
    };
    let storage = classify_value(flow, *value_type, origin, scope);
    if let Some(value) = plan.values.get_mut(dst.0 as usize) {
        *value = storage;
    }
}

fn classify_value(
    flow: &FlowModule,
    value_type: keld_semantics::TypeId,
    origin: &ValueOrigin,
    scope: StorageScopeId,
) -> ValueStorage {
    match origin {
        ValueOrigin::Implicit => ValueStorage::Trivial,
        ValueOrigin::Entity(_) => ValueStorage::EntityFlow,
        ValueOrigin::Local(local) | ValueOrigin::Borrowed(local) => ValueStorage::Loan(Place {
            base: *local,
            fields: Vec::new(),
        }),
        ValueOrigin::BorrowedPlace { place, .. } => ValueStorage::Loan(place.clone()),
        ValueOrigin::Owned => {
            if flow.types.storage_class(value_type) == StorageClass::SingleHome {
                ValueStorage::OwnedTemporary { scope }
            } else if matches!(flow.types.kind(value_type), TypeKind::EntityRef(_)) {
                ValueStorage::EntityFlow
            } else {
                ValueStorage::Trivial
            }
        }
        ValueOrigin::BorrowedUnknown | ValueOrigin::Unknown => ValueStorage::Trivial,
    }
}

fn defined_value(operation: &FlowOp) -> Option<ValueId> {
    match operation {
        FlowOp::ConstInt { dst, .. }
        | FlowOp::ConstBool { dst, .. }
        | FlowOp::ConstText { dst, .. }
        | FlowOp::ConstNoneLink { dst, .. }
        | FlowOp::CopyLocal { dst, .. }
        | FlowOp::TakeLocal { dst, .. }
        | FlowOp::CopyStorage { dst, .. }
        | FlowOp::ListNew { dst, .. }
        | FlowOp::ListLength { dst, .. }
        | FlowOp::ListLengthLocal { dst, .. }
        | FlowOp::ListRemove { dst, .. }
        | FlowOp::ListRemovePlace { dst, .. }
        | FlowOp::ListRemoveLocal { dst, .. }
        | FlowOp::TextByteLength { dst, .. }
        | FlowOp::TextIsEmpty { dst, .. }
        | FlowOp::TextConcat { dst, .. }
        | FlowOp::UnaryInt { dst, .. }
        | FlowOp::BinaryInt { dst, .. }
        | FlowOp::Not { dst, .. }
        | FlowOp::Compare { dst, .. }
        | FlowOp::Phi { dst, .. }
        | FlowOp::ConstructStruct { dst, .. }
        | FlowOp::AllocateEntity { dst, .. }
        | FlowOp::EntityToLink { dst, .. }
        | FlowOp::ReadStructField { dst, .. }
        | FlowOp::ReadEntityField { dst, .. }
        | FlowOp::ReadUncheckedLinkField { dst, .. } => Some(*dst),
        FlowOp::Call { dst: Some(dst), .. } => Some(*dst),
        FlowOp::BeginLifecycle { .. }
        | FlowOp::BeginCall { .. }
        | FlowOp::ReserveArgument { .. }
        | FlowOp::StoreLocal { .. }
        | FlowOp::ListPush { .. }
        | FlowOp::ListPushPlace { .. }
        | FlowOp::ListPushLocal { .. }
        | FlowOp::Call { dst: None, .. }
        | FlowOp::WriteEntityField { .. }
        | FlowOp::Keep { .. }
        | FlowOp::Retire { .. } => None,
    }
}
