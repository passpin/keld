use crate::plan::{
    BlockStoragePlan, FunctionStoragePlan, LocalStorage, OperationStoragePlan, ValueStorage,
};
use crate::state::{CleanupOrder, ScopeCleanupState};
use keld_flow::{FlowFunction, FlowModule, FlowOp, Place, StorageScopeId, ValueId};
use keld_semantics::{LocalId, ParameterMode, StorageClass, TypeKind};
use std::collections::{BTreeMap, BTreeSet};

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

pub(crate) fn initial_cleanup_orders(function: &FlowFunction) -> Vec<ScopeCleanupState> {
    function
        .storage_scope_parents
        .iter()
        .enumerate()
        .map(|(index, _)| ScopeCleanupState {
            scope: StorageScopeId(u32::try_from(index).unwrap_or(u32::MAX)),
            order: CleanupOrder::Known(Vec::new()),
        })
        .collect()
}

pub(crate) fn activate_home(
    orders: &mut [ScopeCleanupState],
    scope: StorageScopeId,
    home: crate::HomeId,
) {
    let Some(scope_state) = orders.iter_mut().find(|state| state.scope == scope) else {
        return;
    };
    match &mut scope_state.order {
        CleanupOrder::Known(order) => {
            order.retain(|existing| *existing != home);
            order.push(home);
        }
        CleanupOrder::Divergent => {}
    }
}

pub(crate) fn deactivate_home(orders: &mut [ScopeCleanupState], home: crate::HomeId) {
    for scope_state in orders {
        if let CleanupOrder::Known(order) = &mut scope_state.order {
            order.retain(|existing| *existing != home);
        }
    }
}

pub(crate) fn exit_scopes(orders: &mut [ScopeCleanupState], scopes: &[StorageScopeId]) {
    for scope in scopes {
        if let Some(scope_state) = orders.iter_mut().find(|state| state.scope == *scope) {
            scope_state.order = CleanupOrder::Known(Vec::new());
        }
    }
}

pub(crate) fn join_cleanup_orders(
    left: &[ScopeCleanupState],
    right: &[ScopeCleanupState],
) -> Vec<ScopeCleanupState> {
    let count = left.len().max(right.len());
    (0..count)
        .map(|index| {
            let scope = left.get(index).or_else(|| right.get(index)).map_or(
                StorageScopeId(u32::try_from(index).unwrap_or(u32::MAX)),
                |state| state.scope,
            );
            let left_order = left
                .get(index)
                .map_or(CleanupOrder::Known(Vec::new()), |state| state.order.clone());
            let right_order = right
                .get(index)
                .map_or(CleanupOrder::Known(Vec::new()), |state| state.order.clone());
            ScopeCleanupState {
                scope,
                order: join_cleanup_order(left_order, right_order),
            }
        })
        .collect()
}

fn join_cleanup_order(left: CleanupOrder, right: CleanupOrder) -> CleanupOrder {
    let (CleanupOrder::Known(left), CleanupOrder::Known(right)) = (left, right) else {
        return CleanupOrder::Divergent;
    };
    let mut nodes = Vec::new();
    for home in left.iter().chain(right.iter()) {
        if !nodes.contains(home) {
            nodes.push(*home);
        }
    }
    let node_set = nodes.iter().copied().collect::<BTreeSet<_>>();
    let mut edges = BTreeMap::<crate::HomeId, BTreeSet<crate::HomeId>>::new();
    let mut indegree = node_set
        .iter()
        .copied()
        .map(|home| (home, 0_usize))
        .collect::<BTreeMap<_, _>>();
    for order in [&left, &right] {
        for pair in order.windows(2) {
            let successors = edges.entry(pair[0]).or_default();
            if successors.insert(pair[1]) {
                *indegree
                    .get_mut(&pair[1])
                    .expect("cleanup order node has an indegree entry") += 1;
            }
        }
    }
    let mut ready = nodes
        .iter()
        .copied()
        .filter(|home| indegree[home] == 0)
        .collect::<Vec<_>>();
    let mut result = Vec::with_capacity(nodes.len());
    while let Some(home) = ready.pop() {
        result.push(home);
        if let Some(successors) = edges.get(&home) {
            for successor in successors {
                let degree = indegree
                    .get_mut(successor)
                    .expect("cleanup order successor has an indegree entry");
                *degree -= 1;
                if *degree == 0 {
                    ready.push(*successor);
                }
            }
        }
    }
    if result.len() == nodes.len() {
        CleanupOrder::Known(result)
    } else {
        CleanupOrder::Divergent
    }
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
        ValueOrigin::Implicit | ValueOrigin::BorrowedUnknown | ValueOrigin::Unknown => {
            ValueStorage::Trivial
        }
    }
}

pub(crate) fn defined_value(operation: &FlowOp) -> Option<ValueId> {
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
        | FlowOp::ReadUncheckedLinkField { dst, .. }
        | FlowOp::Call { dst: Some(dst), .. } => Some(*dst),
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

#[cfg(test)]
mod tests {
    use super::{CleanupOrder, join_cleanup_order};
    use crate::HomeId;
    use keld_semantics::LocalId;

    #[derive(Clone, Copy)]
    enum Action {
        Noop,
        Initialize(HomeId),
        Take(HomeId),
    }

    fn decode(mut code: usize) -> Vec<Action> {
        (0..4)
            .map(|_| {
                let action = match code % 5 {
                    0 => Action::Noop,
                    1 => Action::Initialize(HomeId::Local(LocalId(0))),
                    2 => Action::Initialize(HomeId::Local(LocalId(1))),
                    3 => Action::Take(HomeId::Local(LocalId(0))),
                    _ => Action::Take(HomeId::Local(LocalId(1))),
                };
                code /= 5;
                action
            })
            .collect()
    }

    fn reference_stack(actions: &[Action]) -> Vec<HomeId> {
        let mut stack = Vec::new();
        for action in actions {
            match action {
                Action::Noop => {}
                Action::Initialize(home) => {
                    stack.retain(|existing| existing != home);
                    stack.push(*home);
                }
                Action::Take(home) => stack.retain(|existing| existing != home),
            }
        }
        stack
    }

    fn has_order(order: &[HomeId], left: HomeId, right: HomeId) -> Option<bool> {
        let left_index = order.iter().position(|home| *home == left)?;
        let right_index = order.iter().position(|home| *home == right)?;
        Some(left_index < right_index)
    }

    #[test]
    fn generated_join_orders_match_the_reference_stack_model() {
        for left_code in 0..625 {
            for right_code in 0..625 {
                let left = reference_stack(&decode(left_code));
                let right = reference_stack(&decode(right_code));
                let homes = [HomeId::Local(LocalId(0)), HomeId::Local(LocalId(1))];
                let conflict = homes.iter().enumerate().any(|(left_index, first)| {
                    homes.iter().skip(left_index + 1).any(|second| {
                        matches!(
                            (
                                has_order(&left, *first, *second),
                                has_order(&right, *first, *second)
                            ),
                            (Some(first_order), Some(second_order))
                                if first_order != second_order
                        )
                    })
                });
                let joined = join_cleanup_order(
                    CleanupOrder::Known(left.clone()),
                    CleanupOrder::Known(right.clone()),
                );
                assert_eq!(matches!(joined, CleanupOrder::Divergent), conflict);
                if let CleanupOrder::Known(order) = joined {
                    for path in [&left, &right] {
                        for pair in path.windows(2) {
                            assert!(
                                order.iter().position(|home| *home == pair[0])
                                    < order.iter().position(|home| *home == pair[1])
                            );
                        }
                    }
                    let cleanup_exit = order.iter().rev().collect::<Vec<_>>();
                    assert_eq!(cleanup_exit.len(), order.len());
                }
            }
        }
    }
}
