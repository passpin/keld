use crate::access::{self, AccessPath, ValueOrigin};
use crate::{CleanupAction, FunctionStoragePlan, HomeId, StoreKind, ValueStorage, cleanup};
use crate::{EmptyReason, Home};
use keld_flow::{
    BlockId, ExitTarget, FlowFunction, FlowModule, FlowOp, IndexIdentity, Place, PlaceProjection,
    StorageReceiver, Terminator, ValueId,
};
use keld_lifecycle::{EntityOperationFacts, VerifiedFlowModule};
use keld_semantics::{
    DefId, FieldId, FunctionId, LocalId, ParameterMode, StorageClass, TypeId, TypeKind, TypeStore,
};
use keld_source::{Diagnostic, DiagnosticCode, Span, sort_diagnostics};
use std::collections::{BTreeSet, VecDeque};

const AMBIGUOUS: DiagnosticCode = DiagnosticCode("KLD2001");
const MOVED: DiagnosticCode = DiagnosticCode("KLD2002");
const BORROWED: DiagnosticCode = DiagnosticCode("KLD2003");
const PARTIAL_MOVE: DiagnosticCode = DiagnosticCode("KLD2004");
const CONFLICTING_LOANS: DiagnosticCode = DiagnosticCode("KLD2005");
const INDEXED_REPLACEMENT: DiagnosticCode = DiagnosticCode("KLD2007");
const MAYBE_LIVE: DiagnosticCode = DiagnosticCode("KLD2008");

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum LoanEffect {
    Read,
    Edit,
    Structural,
    Take,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntityEffectTarget {
    Parameter(u32),
    Any(DefId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EntityStorageEffect {
    pub target: EntityEffectTarget,
    pub projections: Vec<PlaceProjection>,
    pub effect: LoanEffect,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FunctionStorageSummary {
    pub parameters: Vec<StorageClass>,
    pub effects: Vec<LoanEffect>,
    pub entity_effects: Vec<EntityStorageEffect>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StorageAnnotations {
    pub functions: Vec<FunctionStoragePlan>,
}

#[derive(Clone, Debug)]
pub struct VerifiedStorageModule {
    pub lifecycle: VerifiedFlowModule,
    pub summaries: Vec<FunctionStorageSummary>,
    pub annotations: StorageAnnotations,
}

#[derive(Clone, Debug)]
pub struct Verification {
    pub module: Option<VerifiedStorageModule>,
    pub diagnostics: Vec<Diagnostic>,
}

#[must_use]
pub fn verify(lifecycle: VerifiedFlowModule) -> Verification {
    let flow = &lifecycle.flow;
    let mut diagnostics = Vec::new();
    let mut summaries = flow
        .functions
        .iter()
        .map(|function| initial_summary(flow, function))
        .collect::<Vec<_>>();
    for _ in 0..flow.functions.len().max(1) {
        let next = flow
            .functions
            .iter()
            .map(|function| infer_summary(&lifecycle, function, &summaries))
            .collect::<Vec<_>>();
        if next == summaries {
            break;
        }
        summaries = next;
    }
    let mut annotations = StorageAnnotations::default();
    for function in &flow.functions {
        let (mut function_diagnostics, plan) = verify_function(&lifecycle, function, &summaries);
        diagnostics.append(&mut function_diagnostics);
        annotations.functions.push(plan);
    }
    sort_diagnostics(&mut diagnostics);
    diagnostics
        .dedup_by(|left, right| left.code == right.code && left.primary.span == right.primary.span);
    let module = diagnostics.is_empty().then_some(VerifiedStorageModule {
        lifecycle,
        summaries,
        annotations,
    });
    Verification {
        module,
        diagnostics,
    }
}

#[must_use]
#[allow(clippy::missing_panics_doc)]
pub fn verify_text_for_test(text: &str) -> Verification {
    let flow = match keld_flow::lower_text_for_test(text) {
        Ok(flow) => flow,
        Err(diagnostics) => {
            return Verification {
                module: None,
                diagnostics,
            };
        }
    };
    let lifecycle = keld_lifecycle::verify(flow);
    if !lifecycle.diagnostics.is_empty() {
        return Verification {
            module: None,
            diagnostics: lifecycle.diagnostics,
        };
    }
    verify(
        lifecycle
            .module
            .expect("lifecycle module exists after validation"),
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HomeState {
    homes: Vec<Home>,
    borrowed: BTreeSet<LocalId>,
    origins: Vec<ValueOrigin>,
    pending: Vec<PendingCall>,
    indexed: Vec<IndexedReservation>,
    cleanup_orders: Vec<crate::state::ScopeCleanupState>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingCall {
    call: u32,
    function: FunctionId,
    reservations: Vec<Reservation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Reservation {
    parameter: keld_semantics::ParameterIndex,
    access: AccessPath,
    effect: LoanEffect,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct IndexedReservation {
    reservation: u32,
    list: AccessPath,
}

fn verify_function(
    lifecycle: &VerifiedFlowModule,
    function: &FlowFunction,
    summaries: &[FunctionStorageSummary],
) -> (Vec<Diagnostic>, FunctionStoragePlan) {
    let flow = &lifecycle.flow;
    let mut diagnostics = Vec::new();
    let mut plan = cleanup::new_function_plan(flow, function);
    mark_unreachable_values(lifecycle, function, &mut plan);
    let initial = initial_state(flow, function);
    let mut incoming = vec![None::<HomeState>; function.blocks.len()];
    let mut exit_states = vec![None::<HomeState>; function.blocks.len()];
    incoming[function.entry.0 as usize] = Some(initial);
    let mut queue = VecDeque::from([function.entry]);
    while let Some(block_id) = queue.pop_front() {
        if !lifecycle.is_block_reachable(function.id, block_id) {
            continue;
        }
        let Some(state) = incoming[block_id.0 as usize].clone() else {
            continue;
        };
        let block = &function.blocks[block_id.0 as usize];
        let mut state = state;
        for (operation_index, operation) in block.operations.iter().enumerate() {
            let facts = access::operation_facts(
                lifecycle,
                function,
                block_id,
                u32::try_from(operation_index).expect("flow operation index fits in u32"),
            );
            let before = state.clone();
            transfer_operation(
                flow,
                function,
                facts,
                operation,
                &mut state,
                summaries,
                &lifecycle.summaries,
                &mut diagnostics,
            );
            cleanup::record_operation(
                flow,
                function,
                operation,
                &state.origins,
                block.storage_scope,
                &mut plan,
            );
            update_cleanup_state(flow, function, operation, &mut state, block.storage_scope);
            annotate_operation_plan(
                flow,
                function,
                operation,
                &before,
                &mut plan.blocks[block_id.0 as usize].operations[operation_index],
            );
        }
        verify_terminator(flow, function, &block.terminator, &state, &mut diagnostics);
        exit_states[block_id.0 as usize] = Some(state.clone());
        let mut outgoing = state.clone();
        if let Terminator::ExitScopes { storage_scopes, .. } = &block.terminator {
            cleanup::exit_scopes(&mut outgoing.cleanup_orders, storage_scopes);
        }
        for successor in successors(&block.terminator) {
            if !lifecycle.is_block_reachable(function.id, successor) {
                continue;
            }
            let slot = &mut incoming[successor.0 as usize];
            let changed = if let Some(existing) = slot {
                let joined = join_states(existing, &outgoing);
                let changed = *existing != joined;
                *existing = joined;
                changed
            } else {
                *slot = Some(outgoing.clone());
                true
            };
            if changed {
                queue.push_back(successor);
            }
        }
    }
    annotate_exit_plan(function, &exit_states, &mut plan);
    (diagnostics, plan)
}

fn mark_unreachable_values(
    lifecycle: &VerifiedFlowModule,
    function: &FlowFunction,
    plan: &mut FunctionStoragePlan,
) {
    for block in &function.blocks {
        if lifecycle.is_block_reachable(function.id, block.id) {
            continue;
        }
        for operation in &block.operations {
            let Some(value) = cleanup::defined_value(operation) else {
                continue;
            };
            if let Some(storage) = plan.values.get_mut(value.0 as usize) {
                *storage = ValueStorage::Unreachable;
            }
        }
    }
}

fn update_cleanup_state(
    flow: &FlowModule,
    function: &FlowFunction,
    operation: &FlowOp,
    state: &mut HomeState,
    scope: keld_flow::StorageScopeId,
) {
    if let Some(dst) = cleanup::defined_value(operation)
        && state.origins.get(dst.0 as usize) == Some(&ValueOrigin::Owned)
        && flow
            .types
            .storage_class(function.value_types[dst.0 as usize])
            == keld_semantics::StorageClass::SingleHome
    {
        cleanup::activate_home(
            &mut state.cleanup_orders,
            scope,
            crate::HomeId::Temporary(dst),
        );
    }

    let mut deactivate_temporary = |value: keld_flow::ValueId| {
        if state.origins.get(value.0 as usize) == Some(&ValueOrigin::Owned) {
            cleanup::deactivate_home(&mut state.cleanup_orders, crate::HomeId::Temporary(value));
        }
    };
    match operation {
        FlowOp::StoreLocal { local, value, .. } => {
            if state.origins.get(value.0 as usize) == Some(&ValueOrigin::Owned) {
                deactivate_temporary(*value);
                if let Some(scope) = function.local_scopes.get(local.0 as usize).copied() {
                    cleanup::activate_home(
                        &mut state.cleanup_orders,
                        scope,
                        crate::HomeId::Local(*local),
                    );
                }
            }
        }
        FlowOp::TakeLocal { local, dst, .. } => {
            if state.origins.get(dst.0 as usize) == Some(&ValueOrigin::Owned) {
                cleanup::deactivate_home(&mut state.cleanup_orders, crate::HomeId::Local(*local));
            }
        }
        FlowOp::ListPush { value, .. }
        | FlowOp::ListReplace { value, .. }
        | FlowOp::ReplacePlace { value, .. }
        | FlowOp::WriteEntityField { value, .. } => deactivate_temporary(*value),
        FlowOp::ConstructStruct { fields, .. } | FlowOp::AllocateEntity { fields, .. } => {
            for (_, value) in fields {
                deactivate_temporary(*value);
            }
        }
        FlowOp::Call {
            function: callee,
            arguments,
            ..
        } => {
            for (parameter, value) in arguments {
                if flow
                    .functions
                    .get(callee.0 as usize)
                    .and_then(|function| function.parameter_modes.get(parameter.0 as usize))
                    == Some(&keld_semantics::ParameterMode::Take)
                {
                    deactivate_temporary(*value);
                }
            }
        }
        _ => {}
    }
}

fn annotate_operation_plan(
    flow: &FlowModule,
    function: &FlowFunction,
    operation: &FlowOp,
    before: &HomeState,
    plan: &mut crate::OperationStoragePlan,
) {
    plan.store = None;
    plan.post_success.clear();
    if let FlowOp::Call {
        function: callee,
        arguments,
        ..
    } = operation
    {
        for (parameter, value) in arguments {
            let is_loan = flow
                .functions
                .get(callee.0 as usize)
                .and_then(|function| function.parameter_modes.get(parameter.0 as usize))
                == Some(&keld_semantics::ParameterMode::Loan);
            if is_loan && before.origins.get(value.0 as usize) == Some(&ValueOrigin::Owned) {
                plan.post_success
                    .push(CleanupAction::Drop(HomeId::Temporary(*value)));
            }
        }
    }
    let FlowOp::StoreLocal { local, .. } = operation else {
        return;
    };
    if flow
        .types
        .storage_class(function.local_types[local.0 as usize])
        != keld_semantics::StorageClass::SingleHome
    {
        return;
    }
    plan.store = Some(match before.homes[local.0 as usize] {
        Home::Live => StoreKind::ReplaceLive,
        Home::MaybeLive => StoreKind::ReplaceMaybeLive,
        Home::Empty(_) => StoreKind::Initialize,
    });
}

fn annotate_exit_plan(
    function: &FlowFunction,
    exit_states: &[Option<HomeState>],
    plan: &mut FunctionStoragePlan,
) {
    for (block_index, state) in exit_states.iter().enumerate() {
        let Some(state) = state else {
            continue;
        };
        let terminator = &function.blocks[block_index].terminator;
        let Terminator::ExitScopes {
            storage_scopes,
            next,
            ..
        } = terminator
        else {
            continue;
        };
        let returned_home = match next {
            ExitTarget::Return(Some(value)) => Some(HomeId::Temporary(*value)),
            ExitTarget::Goto(_) | ExitTarget::Return(None) => None,
        };
        let mut actions = Vec::new();
        let post_success_drops = |home: HomeId| {
            plan.blocks[block_index].operations.iter().any(|operation| {
                operation.post_success.iter().any(
                    |action| matches!(action, CleanupAction::Drop(candidate) if *candidate == home),
                )
            })
        };
        for scope in storage_scopes {
            let Some(scope_state) = state
                .cleanup_orders
                .iter()
                .find(|scope_state| scope_state.scope == *scope)
            else {
                continue;
            };
            match &scope_state.order {
                crate::state::CleanupOrder::Divergent => {
                    plan.tracked_scopes.insert(*scope);
                    actions.push(CleanupAction::CleanupTrackedScope(*scope));
                }
                crate::state::CleanupOrder::Known(order) => {
                    for home in order.iter().rev() {
                        if Some(*home) == returned_home {
                            continue;
                        }
                        match home {
                            HomeId::Local(local) => match state.homes[local.0 as usize] {
                                Home::Live => actions.push(CleanupAction::Drop(*home)),
                                Home::MaybeLive => {
                                    plan.drop_flags.insert(*home);
                                    actions.push(CleanupAction::DropIfLive(*home));
                                }
                                Home::Empty(_) => {}
                            },
                            HomeId::Temporary(_) => {
                                if post_success_drops(*home) {
                                    continue;
                                }
                                actions.push(CleanupAction::Drop(*home));
                            }
                        }
                    }
                }
            }
        }
        plan.blocks[block_index].exit = actions;
    }
}

fn initial_summary(flow: &FlowModule, function: &FlowFunction) -> FunctionStorageSummary {
    FunctionStorageSummary {
        parameters: function
            .parameters
            .iter()
            .map(|local| {
                flow.types
                    .storage_class(function.local_types[local.0 as usize])
            })
            .collect(),
        effects: function
            .parameters
            .iter()
            .enumerate()
            .map(|(index, local)| {
                if flow
                    .types
                    .storage_class(function.local_types[local.0 as usize])
                    == StorageClass::SingleHome
                    && function.parameter_modes.get(index) == Some(&ParameterMode::Take)
                {
                    LoanEffect::Take
                } else {
                    LoanEffect::Read
                }
            })
            .collect(),
        entity_effects: Vec::new(),
    }
}

#[allow(clippy::too_many_lines)]
fn infer_summary(
    lifecycle: &VerifiedFlowModule,
    function: &FlowFunction,
    summaries: &[FunctionStorageSummary],
) -> FunctionStorageSummary {
    let flow = &lifecycle.flow;
    let mut summary = initial_summary(flow, function);
    let parameter_index = function
        .parameters
        .iter()
        .enumerate()
        .map(|(index, local)| (*local, index))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut origins = vec![None::<LocalId>; function.value_types.len()];
    for block in &function.blocks {
        if !lifecycle.is_block_reachable(function.id, block.id) {
            continue;
        }
        for (operation_index, operation) in block.operations.iter().enumerate() {
            let facts = access::operation_facts(
                lifecycle,
                function,
                block.id,
                u32::try_from(operation_index).expect("flow operation index fits in u32"),
            );
            match operation {
                FlowOp::CopyLocal { dst, local, .. } | FlowOp::TakeLocal { dst, local, .. } => {
                    origins[dst.0 as usize] = Some(*local);
                }
                FlowOp::CopyStorage { dst, .. }
                | FlowOp::ListNew { dst, .. }
                | FlowOp::ConstructStruct { dst, .. }
                | FlowOp::AllocateEntity { dst, .. } => origins[dst.0 as usize] = None,
                FlowOp::ListLength { receiver, .. }
                | FlowOp::ListIndex { receiver, .. }
                | FlowOp::ListGet { receiver, .. } => {
                    mark_receiver(
                        &mut summary,
                        &parameter_index,
                        &origins,
                        receiver,
                        LoanEffect::Read,
                    );
                    record_receiver_entity_effect(
                        flow,
                        function,
                        facts,
                        receiver,
                        LoanEffect::Read,
                        &mut summary,
                    );
                }
                FlowOp::ListPush { receiver, .. }
                | FlowOp::ListRemove { receiver, .. }
                | FlowOp::ListTryRemove { receiver, .. }
                | FlowOp::ListClear { receiver, .. }
                | FlowOp::ListReserve { receiver, .. }
                | FlowOp::ListTryReserve { receiver, .. }
                | FlowOp::ListReplace { receiver, .. } => {
                    mark_receiver(
                        &mut summary,
                        &parameter_index,
                        &origins,
                        receiver,
                        LoanEffect::Structural,
                    );
                    record_receiver_entity_effect(
                        flow,
                        function,
                        facts,
                        receiver,
                        LoanEffect::Structural,
                        &mut summary,
                    );
                }
                FlowOp::ReplacePlace { place, .. }
                | FlowOp::BeginIndexedReplacement { list: place, .. } => {
                    mark_parameter_effect(
                        &mut summary,
                        &parameter_index,
                        place.base,
                        LoanEffect::Structural,
                    );
                    record_place_entity_effect(
                        flow,
                        function,
                        facts,
                        place,
                        LoanEffect::Structural,
                        &mut summary,
                    );
                }
                FlowOp::Call {
                    function: callee,
                    arguments,
                    argument_places,
                    ..
                } => {
                    let callee_summary = summaries.get(callee.0 as usize);
                    let callee_function = flow.functions.get(callee.0 as usize);
                    for (parameter, value) in arguments {
                        let Some(local) = origins.get(value.0 as usize).copied().flatten() else {
                            continue;
                        };
                        let effect = callee_summary
                            .and_then(|summary| summary.effects.get(parameter.0 as usize))
                            .copied()
                            .or_else(|| {
                                callee_function.and_then(|function| {
                                    (function.parameter_modes.get(parameter.0 as usize)
                                        == Some(&ParameterMode::Take))
                                    .then_some(LoanEffect::Take)
                                })
                            })
                            .unwrap_or(LoanEffect::Read);
                        mark_parameter_effect(&mut summary, &parameter_index, local, effect);
                    }
                    if let Some(callee_summary) = callee_summary {
                        for (parameter, place) in argument_places {
                            let Some(place) = place else { continue };
                            let effect = callee_summary
                                .effects
                                .get(parameter.0 as usize)
                                .copied()
                                .unwrap_or(LoanEffect::Read);
                            record_place_entity_effect(
                                flow,
                                function,
                                facts,
                                place,
                                effect,
                                &mut summary,
                            );
                        }
                        for effect in &callee_summary.entity_effects {
                            propagate_entity_effect(
                                flow,
                                function,
                                facts,
                                arguments,
                                effect,
                                &mut summary,
                            );
                        }
                    }
                }
                FlowOp::ReadEntityField { entity, field, .. } => record_value_entity_effect(
                    flow,
                    function,
                    facts,
                    *entity,
                    &[PlaceProjection::Field(*field)],
                    LoanEffect::Read,
                    &mut summary,
                ),
                FlowOp::WriteEntityField { entity, field, .. } => {
                    record_value_entity_effect(
                        flow,
                        function,
                        facts,
                        *entity,
                        &[PlaceProjection::Field(*field)],
                        LoanEffect::Structural,
                        &mut summary,
                    );
                }
                FlowOp::Phi { dst, inputs, .. } => {
                    let first = inputs
                        .first()
                        .and_then(|(_, value)| origins.get(value.0 as usize).copied().flatten());
                    origins[dst.0 as usize] = first.filter(|local| {
                        inputs.iter().all(|(_, value)| {
                            origins.get(value.0 as usize).copied().flatten() == Some(*local)
                        })
                    });
                }
                _ => {}
            }
        }
    }
    summary
}

fn mark_receiver(
    summary: &mut FunctionStorageSummary,
    parameter_index: &std::collections::BTreeMap<LocalId, usize>,
    origins: &[Option<LocalId>],
    receiver: &StorageReceiver,
    effect: LoanEffect,
) {
    if let Some(place) = &receiver.place {
        mark_parameter_effect(summary, parameter_index, place.base, effect);
    } else if let Some(local) = origins.get(receiver.value.0 as usize).copied().flatten() {
        mark_parameter_effect(summary, parameter_index, local, effect);
    }
}

fn mark_parameter_effect(
    summary: &mut FunctionStorageSummary,
    parameter_index: &std::collections::BTreeMap<LocalId, usize>,
    local: LocalId,
    effect: LoanEffect,
) {
    let Some(index) = parameter_index.get(&local).copied() else {
        return;
    };
    if summary.parameters.get(index) != Some(&StorageClass::SingleHome) {
        return;
    }
    summary.effects[index] = join_effect(summary.effects[index], effect);
}

fn record_receiver_entity_effect(
    flow: &FlowModule,
    function: &FlowFunction,
    facts: &EntityOperationFacts,
    receiver: &StorageReceiver,
    effect: LoanEffect,
    summary: &mut FunctionStorageSummary,
) {
    if let Some(place) = &receiver.place {
        record_place_entity_effect(flow, function, facts, place, effect, summary);
    }
}

fn record_place_entity_effect(
    flow: &FlowModule,
    function: &FlowFunction,
    facts: &EntityOperationFacts,
    place: &Place,
    effect: LoanEffect,
    summary: &mut FunctionStorageSummary,
) {
    let Some(definition) =
        access::entity_definition(flow, function.local_types[place.base.0 as usize])
    else {
        return;
    };
    let targets = facts.local_reference(place.base).map_or_else(
        || vec![EntityEffectTarget::Any(definition)],
        |reference| entity_effect_targets(facts, reference),
    );
    for target in targets {
        push_entity_effect(summary, target, place.projections.clone(), effect);
    }
}

fn record_value_entity_effect(
    flow: &FlowModule,
    function: &FlowFunction,
    facts: &EntityOperationFacts,
    value: ValueId,
    projections: &[PlaceProjection],
    effect: LoanEffect,
    summary: &mut FunctionStorageSummary,
) {
    let Some(definition) = access::entity_definition(flow, function.value_types[value.0 as usize])
    else {
        return;
    };
    let targets = facts.value_reference(value).map_or_else(
        || vec![EntityEffectTarget::Any(definition)],
        |reference| entity_effect_targets(facts, reference),
    );
    for target in targets {
        push_entity_effect(summary, target, projections.to_vec(), effect);
    }
}

fn entity_effect_targets(
    facts: &EntityOperationFacts,
    reference: keld_lifecycle::EntityReferenceFact,
) -> Vec<EntityEffectTarget> {
    let Some(origin) = facts.origin(reference.provenance) else {
        return vec![EntityEffectTarget::Any(reference.definition)];
    };
    if origin.broad {
        return vec![EntityEffectTarget::Any(reference.definition)];
    }
    if !origin.parameters.is_empty() {
        return origin
            .parameters
            .iter()
            .copied()
            .map(EntityEffectTarget::Parameter)
            .collect();
    }
    if origin.fresh {
        Vec::new()
    } else {
        vec![EntityEffectTarget::Any(reference.definition)]
    }
}

fn push_entity_effect(
    summary: &mut FunctionStorageSummary,
    target: EntityEffectTarget,
    projections: Vec<PlaceProjection>,
    effect: LoanEffect,
) {
    if let Some(existing) = summary
        .entity_effects
        .iter_mut()
        .find(|existing| existing.target == target && existing.projections == projections)
    {
        existing.effect = join_effect(existing.effect, effect);
    } else {
        summary.entity_effects.push(EntityStorageEffect {
            target,
            projections,
            effect,
        });
    }
}

fn propagate_entity_effect(
    flow: &FlowModule,
    function: &FlowFunction,
    facts: &EntityOperationFacts,
    arguments: &[(keld_semantics::ParameterIndex, ValueId)],
    effect: &EntityStorageEffect,
    summary: &mut FunctionStorageSummary,
) {
    match effect.target {
        EntityEffectTarget::Any(definition) => push_entity_effect(
            summary,
            EntityEffectTarget::Any(definition),
            effect.projections.clone(),
            effect.effect,
        ),
        EntityEffectTarget::Parameter(parameter) => {
            let Some((_, value)) = arguments
                .iter()
                .find(|(candidate, _)| candidate.0 == parameter)
            else {
                return;
            };
            record_value_entity_effect(
                flow,
                function,
                facts,
                *value,
                &effect.projections,
                effect.effect,
                summary,
            );
        }
    }
}

fn initial_state(flow: &FlowModule, function: &FlowFunction) -> HomeState {
    let mut homes = function
        .local_types
        .iter()
        .copied()
        .map(|_| Home::Empty(EmptyReason::Uninitialized))
        .collect::<Vec<_>>();
    let mut borrowed = BTreeSet::new();
    for (index, local) in function.parameters.iter().copied().enumerate() {
        let ty = function.local_types[local.0 as usize];
        if flow.types.storage_class(ty) != StorageClass::SingleHome {
            continue;
        }
        if function.parameter_modes.get(index).copied() == Some(ParameterMode::Loan) {
            borrowed.insert(local);
        } else {
            homes[local.0 as usize] = Home::Live;
        }
    }
    let mut cleanup_orders = cleanup::initial_cleanup_orders(function);
    for (index, local) in function.parameters.iter().copied().enumerate() {
        let ty = function.local_types[local.0 as usize];
        if flow.types.storage_class(ty) == StorageClass::SingleHome
            && function.parameter_modes.get(index).copied() != Some(ParameterMode::Loan)
            && let Some(scope) = function.local_scopes.get(local.0 as usize).copied()
        {
            cleanup::activate_home(&mut cleanup_orders, scope, HomeId::Local(local));
        }
    }
    HomeState {
        homes,
        borrowed,
        origins: vec![ValueOrigin::Unknown; function.value_types.len()],
        pending: Vec::new(),
        indexed: Vec::new(),
        cleanup_orders,
    }
}

fn join_effect(left: LoanEffect, right: LoanEffect) -> LoanEffect {
    match (left, right) {
        (LoanEffect::Take, _) | (_, LoanEffect::Take) => LoanEffect::Take,
        (LoanEffect::Structural, _) | (_, LoanEffect::Structural) => LoanEffect::Structural,
        (LoanEffect::Edit, _) | (_, LoanEffect::Edit) => LoanEffect::Edit,
        _ => LoanEffect::Read,
    }
}

#[allow(clippy::too_many_lines)]
#[allow(clippy::too_many_arguments)]
fn transfer_operation(
    flow: &FlowModule,
    function: &FlowFunction,
    facts: &EntityOperationFacts,
    operation: &FlowOp,
    state: &mut HomeState,
    summaries: &[FunctionStorageSummary],
    lifecycle_summaries: &[keld_lifecycle::FunctionSummary],
    diagnostics: &mut Vec<Diagnostic>,
) {
    match operation {
        FlowOp::BeginCall { call, function, .. } => {
            state.pending.push(PendingCall {
                call: *call,
                function: *function,
                reservations: Vec::new(),
            });
        }
        FlowOp::ReserveArgument {
            call,
            parameter,
            value,
            place: Some(place),
            span,
        } => {
            let Some(current) = state.pending.last() else {
                diagnostics.push(error(
                    CONFLICTING_LOANS,
                    *span,
                    "argument reservation is outside its call",
                ));
                return;
            };
            if current.call != *call {
                diagnostics.push(error(
                    CONFLICTING_LOANS,
                    *span,
                    "argument reservation belongs to another call",
                ));
                return;
            }
            let parameter_class = summaries
                .get(current.function.0 as usize)
                .and_then(|summary| summary.parameters.get(parameter.0 as usize))
                .copied();
            if parameter_class != Some(StorageClass::SingleHome) {
                return;
            }
            let effect = parameter_effect(flow, summaries, current.function, *parameter);
            let access = AccessPath::for_place(flow, function, facts, place);
            if current
                .reservations
                .iter()
                .any(|existing| reservation_conflicts(facts, existing, &access, effect))
            {
                diagnostics.push(error(
                    CONFLICTING_LOANS,
                    *span,
                    "these arguments may access the same storage incompatibly",
                ));
            }
            if matches!(effect, LoanEffect::Structural | LoanEffect::Take)
                && state.indexed.iter().any(|reservation| {
                    access::indexed_destination_conflict(facts, &reservation.list, &access)
                })
            {
                diagnostics.push(error(
                    INDEXED_REPLACEMENT,
                    *span,
                    "the right-hand side accesses an indexed replacement destination",
                ));
            }
            if state.pending.len() > 1
                && state.pending[..state.pending.len() - 1]
                    .iter()
                    .flat_map(|parent| parent.reservations.iter())
                    .any(|existing| reservation_conflicts(facts, existing, &access, effect))
            {
                diagnostics.push(error(
                    CONFLICTING_LOANS,
                    *span,
                    "nested argument evaluation conflicts with an outer storage reservation",
                ));
            }
            state
                .pending
                .last_mut()
                .expect("current pending call remains installed")
                .reservations
                .push(Reservation {
                    parameter: *parameter,
                    access,
                    effect,
                });
            let _ = value;
        }
        FlowOp::ReserveArgument { call, span, .. } => {
            if state
                .pending
                .last()
                .is_none_or(|pending| pending.call != *call)
            {
                diagnostics.push(error(
                    CONFLICTING_LOANS,
                    *span,
                    "argument reservation belongs to another call",
                ));
            }
        }
        FlowOp::CopyLocal { dst, local, span } => {
            let ty = function.local_types[local.0 as usize];
            if flow.types.storage_class(ty) == StorageClass::SingleHome {
                if state.borrowed.contains(local) {
                    state.origins[dst.0 as usize] = ValueOrigin::Borrowed(*local);
                } else if require_live(&state.homes[local.0 as usize], *span, diagnostics) {
                    check_pending_access(
                        state,
                        Place {
                            base: *local,
                            projections: Vec::new(),
                        },
                        LoanEffect::Read,
                        *span,
                        diagnostics,
                        summaries,
                        flow,
                        function,
                        facts,
                    );
                    state.origins[dst.0 as usize] = ValueOrigin::Local(*local);
                }
            } else {
                state.origins[dst.0 as usize] = match flow.types.kind(ty) {
                    TypeKind::EntityRef(_) => ValueOrigin::Entity(*local),
                    _ => ValueOrigin::Implicit,
                };
            }
        }
        FlowOp::TakeLocal { dst, local, span } => {
            let ty = function.local_types[local.0 as usize];
            if flow.types.storage_class(ty) != StorageClass::SingleHome {
                diagnostics.push(error(
                    DiagnosticCode("KLD2009"),
                    *span,
                    "`take` requires a single-home local",
                ));
            } else if state.borrowed.contains(local) {
                diagnostics.push(error(
                    BORROWED,
                    *span,
                    "a borrowed parameter cannot be taken",
                ));
            } else if take_home(&mut state.homes[local.0 as usize], *span, diagnostics) {
                check_pending_access(
                    state,
                    Place {
                        base: *local,
                        projections: Vec::new(),
                    },
                    LoanEffect::Take,
                    *span,
                    diagnostics,
                    summaries,
                    flow,
                    function,
                    facts,
                );
                state.origins[dst.0 as usize] = ValueOrigin::Owned;
            }
        }
        FlowOp::CopyStorage { dst, source, span } => {
            let origin = state
                .origins
                .get(source.0 as usize)
                .cloned()
                .unwrap_or(ValueOrigin::Unknown);
            check_origin_access(
                flow,
                function,
                facts,
                state,
                &origin,
                LoanEffect::Read,
                *span,
                diagnostics,
                summaries,
            );
            state.origins[dst.0 as usize] = ValueOrigin::Owned;
        }
        FlowOp::ListNew { dst, .. }
        | FlowOp::ConstText { dst, .. }
        | FlowOp::AllocateEntity { dst, .. } => {
            state.origins[dst.0 as usize] = ValueOrigin::Owned;
        }
        FlowOp::ListLength {
            dst,
            receiver,
            span,
            ..
        }
        | FlowOp::ListGet {
            dst,
            receiver,
            span,
            ..
        } => {
            state.origins[dst.0 as usize] = ValueOrigin::Implicit;
            check_receiver_access(
                flow,
                function,
                facts,
                state,
                receiver,
                LoanEffect::Read,
                *span,
                diagnostics,
                summaries,
            );
        }
        FlowOp::ListIndex {
            dst,
            receiver,
            index,
            span,
        } => {
            state.origins[dst.0 as usize] =
                list_index_origin(flow, function, facts, state, receiver, *index);
            check_receiver_access(
                flow,
                function,
                facts,
                state,
                receiver,
                LoanEffect::Read,
                *span,
                diagnostics,
                summaries,
            );
        }
        FlowOp::ListPush {
            receiver,
            value,
            span,
        } => {
            require_consumable_element(
                flow,
                function.value_types[receiver.value.0 as usize],
                state
                    .origins
                    .get(value.0 as usize)
                    .cloned()
                    .unwrap_or(ValueOrigin::Unknown),
                *span,
                diagnostics,
            );
            check_receiver_access(
                flow,
                function,
                facts,
                state,
                receiver,
                LoanEffect::Structural,
                *span,
                diagnostics,
                summaries,
            );
        }
        FlowOp::ListRemove {
            dst,
            receiver,
            span,
            ..
        }
        | FlowOp::ListTryRemove {
            dst,
            receiver,
            span,
            ..
        } => {
            state.origins[dst.0 as usize] =
                list_remove_origin(flow, function.value_types[receiver.value.0 as usize]);
            check_receiver_access(
                flow,
                function,
                facts,
                state,
                receiver,
                LoanEffect::Structural,
                *span,
                diagnostics,
                summaries,
            );
        }
        FlowOp::ListClear { receiver, span }
        | FlowOp::ListReserve { receiver, span, .. }
        | FlowOp::ListTryReserve { receiver, span, .. } => {
            if let FlowOp::ListTryReserve { dst, .. } = operation {
                state.origins[dst.0 as usize] = ValueOrigin::Implicit;
            }
            check_receiver_access(
                flow,
                function,
                facts,
                state,
                receiver,
                LoanEffect::Structural,
                *span,
                diagnostics,
                summaries,
            );
        }
        FlowOp::BeginIndexedReplacement {
            reservation,
            list,
            index: _,
            span,
        } => {
            check_place_access(
                flow,
                function,
                facts,
                state,
                list,
                LoanEffect::Structural,
                *span,
                diagnostics,
                summaries,
            );
            state.indexed.push(IndexedReservation {
                reservation: *reservation,
                list: AccessPath::for_place(flow, function, facts, list),
            });
        }
        FlowOp::EndIndexedReplacement { reservation, span } => {
            if state
                .indexed
                .last()
                .is_none_or(|current| current.reservation != *reservation)
            {
                diagnostics.push(error(
                    INDEXED_REPLACEMENT,
                    *span,
                    "indexed replacement reservation does not match its active destination",
                ));
            } else {
                state.indexed.pop();
            }
        }
        FlowOp::ListReplace {
            receiver,
            value,
            span,
            ..
        } => {
            require_consumable_element(
                flow,
                function.value_types[receiver.value.0 as usize],
                state
                    .origins
                    .get(value.0 as usize)
                    .cloned()
                    .unwrap_or(ValueOrigin::Unknown),
                *span,
                diagnostics,
            );
            check_receiver_access_without_indexed(
                flow,
                function,
                facts,
                state,
                receiver,
                LoanEffect::Structural,
                *span,
                diagnostics,
                summaries,
            );
        }
        FlowOp::ReplacePlace { place, value, span } => {
            if let Some(ty) = place_type(flow, function, place) {
                require_consumable_value(
                    flow,
                    ty,
                    state
                        .origins
                        .get(value.0 as usize)
                        .cloned()
                        .unwrap_or(ValueOrigin::Unknown),
                    *span,
                    diagnostics,
                );
            }
            check_place_access(
                flow,
                function,
                facts,
                state,
                place,
                LoanEffect::Structural,
                *span,
                diagnostics,
                summaries,
            );
        }
        FlowOp::TextByteLength { dst, text, span } | FlowOp::TextIsEmpty { dst, text, span } => {
            state.origins[dst.0 as usize] = ValueOrigin::Implicit;
            let origin = state
                .origins
                .get(text.0 as usize)
                .cloned()
                .unwrap_or(ValueOrigin::Unknown);
            check_origin_access(
                flow,
                function,
                facts,
                state,
                &origin,
                LoanEffect::Read,
                *span,
                diagnostics,
                summaries,
            );
        }
        FlowOp::TextConcat {
            dst,
            lhs,
            rhs,
            span,
        } => {
            state.origins[dst.0 as usize] = ValueOrigin::Owned;
            for value in [lhs, rhs] {
                let origin = state
                    .origins
                    .get(value.0 as usize)
                    .cloned()
                    .unwrap_or(ValueOrigin::Unknown);
                check_origin_access(
                    flow,
                    function,
                    facts,
                    state,
                    &origin,
                    LoanEffect::Read,
                    *span,
                    diagnostics,
                    summaries,
                );
            }
        }
        FlowOp::StoreLocal { local, value, span } => {
            let ty = function.local_types[local.0 as usize];
            if flow.types.storage_class(ty) != StorageClass::SingleHome {
                return;
            }
            check_pending_access(
                state,
                Place {
                    base: *local,
                    projections: Vec::new(),
                },
                LoanEffect::Structural,
                *span,
                diagnostics,
                summaries,
                flow,
                function,
                facts,
            );
            match state
                .origins
                .get(value.0 as usize)
                .cloned()
                .unwrap_or(ValueOrigin::Unknown)
            {
                ValueOrigin::Owned => {
                    if state.homes[local.0 as usize].is_live()
                        && function.local_mutability.get(local.0 as usize)
                            != Some(&keld_semantics::BindingMutability::Var)
                    {
                        diagnostics.push(error(
                            DiagnosticCode("KLD2010"),
                            *span,
                            "cannot replace an initialized immutable single-home value",
                        ));
                    } else {
                        state.homes[local.0 as usize] = Home::Live;
                    }
                }
                ValueOrigin::BorrowedPlace { .. }
                | ValueOrigin::BorrowedValue { .. }
                | ValueOrigin::BorrowedUnknown => {
                    diagnostics.push(error(
                        PARTIAL_MOVE,
                        *span,
                        "cannot move storage out of a managed field",
                    ));
                }
                ValueOrigin::Borrowed(_) => diagnostics.push(error(
                    BORROWED,
                    *span,
                    "a borrowed value cannot be installed into an owned home",
                )),
                ValueOrigin::Local(_) => diagnostics.push(error(
                    AMBIGUOUS,
                    *span,
                    "a named single-home value requires `take` or `.copy()` here",
                )),
                ValueOrigin::Entity(_) | ValueOrigin::Implicit | ValueOrigin::Unknown => {}
            }
        }
        FlowOp::Call {
            call,
            dst,
            function: callee,
            arguments,
            span,
            ..
        } => {
            if state
                .pending
                .last()
                .is_none_or(|pending| pending.call != *call)
            {
                diagnostics.push(error(
                    CONFLICTING_LOANS,
                    *span,
                    "call does not match its pending argument reservations",
                ));
            }
            for (parameter, value) in arguments {
                let Some(callee_function) = flow.functions.get(callee.0 as usize) else {
                    continue;
                };
                let Some(local) = callee_function.parameters.get(parameter.0 as usize) else {
                    continue;
                };
                let ty = callee_function.local_types[local.0 as usize];
                if flow.types.storage_class(ty) != StorageClass::SingleHome {
                    continue;
                }
                let origin = state
                    .origins
                    .get(value.0 as usize)
                    .cloned()
                    .unwrap_or(ValueOrigin::Unknown);
                if callee_function.parameter_modes.get(parameter.0 as usize)
                    == Some(&ParameterMode::Take)
                {
                    match origin {
                        ValueOrigin::Local(_) => diagnostics.push(error(
                            AMBIGUOUS,
                            *span,
                            "a consuming argument requires `take` or an owned temporary",
                        )),
                        ValueOrigin::BorrowedPlace { .. }
                        | ValueOrigin::BorrowedValue { .. }
                        | ValueOrigin::BorrowedUnknown => {
                            diagnostics.push(error(
                                PARTIAL_MOVE,
                                *span,
                                "cannot move storage out of a managed field",
                            ));
                        }
                        ValueOrigin::Borrowed(_) => diagnostics.push(error(
                            BORROWED,
                            *span,
                            "a borrowed value cannot satisfy a consuming parameter",
                        )),
                        _ => {}
                    }
                }
            }
            check_call_entity_effects(
                flow,
                function,
                facts,
                state,
                *callee,
                arguments,
                summaries,
                lifecycle_summaries,
                *span,
                diagnostics,
            );
            if let Some(dst) = dst {
                state.origins[dst.0 as usize] = if flow
                    .types
                    .storage_class(function_return_type(flow, *callee))
                    == StorageClass::SingleHome
                {
                    ValueOrigin::Owned
                } else {
                    ValueOrigin::Implicit
                };
            }
            if state
                .pending
                .last()
                .is_some_and(|pending| pending.call == *call)
            {
                state.pending.pop();
            }
        }
        FlowOp::ConstructStruct {
            dst,
            definition,
            fields,
            span,
        } => {
            for (field, value) in fields {
                if let Some(field_type) = flow
                    .definitions
                    .get(definition.0 as usize)
                    .and_then(|definition| {
                        definition
                            .fields
                            .iter()
                            .find(|candidate| candidate.id == *field)
                    })
                    .map(|field| field.ty)
                {
                    require_consumable_value(
                        flow,
                        field_type,
                        state
                            .origins
                            .get(value.0 as usize)
                            .cloned()
                            .unwrap_or(ValueOrigin::Unknown),
                        *span,
                        diagnostics,
                    );
                }
            }
            state.origins[dst.0 as usize] = ValueOrigin::Owned;
        }
        FlowOp::Phi { dst, inputs, .. } => {
            let all_owned = inputs.iter().all(|(_, value)| {
                state.origins.get(value.0 as usize).cloned() == Some(ValueOrigin::Owned)
            });
            state.origins[dst.0 as usize] = if all_owned {
                ValueOrigin::Owned
            } else {
                ValueOrigin::Unknown
            };
        }
        FlowOp::ConstInt { dst, .. }
        | FlowOp::ConstBool { dst, .. }
        | FlowOp::ConstNoneLink { dst, .. }
        | FlowOp::UnaryInt { dst, .. }
        | FlowOp::BinaryInt { dst, .. }
        | FlowOp::Not { dst, .. }
        | FlowOp::Compare { dst, .. }
        | FlowOp::EntityToLink { dst, .. } => {
            state.origins[dst.0 as usize] = ValueOrigin::Implicit;
        }
        FlowOp::ReadStructField {
            dst,
            base,
            field,
            span,
        } => {
            let base_type = function.value_types[base.0 as usize];
            let base_origin = state.origins.get(base.0 as usize).cloned();
            let origin =
                read_field_origin(flow, function, facts, base_type, *base, *field, base_origin);
            if !matches!(origin, ValueOrigin::Implicit)
                && let Some(base_origin) = state.origins.get(base.0 as usize)
            {
                check_origin_access(
                    flow,
                    function,
                    facts,
                    state,
                    base_origin,
                    LoanEffect::Read,
                    *span,
                    diagnostics,
                    summaries,
                );
            }
            state.origins[dst.0 as usize] = origin;
        }
        FlowOp::ReadEntityField {
            dst,
            entity,
            field,
            span,
        } => {
            let base_type = function.value_types[entity.0 as usize];
            let base_origin = state.origins.get(entity.0 as usize).cloned();
            let origin = read_field_origin(
                flow,
                function,
                facts,
                base_type,
                *entity,
                *field,
                base_origin,
            );
            state.origins[dst.0 as usize] = origin;
            if !matches!(
                state.origins.get(dst.0 as usize),
                Some(ValueOrigin::Implicit)
            ) && let Some(base_origin) = state.origins.get(entity.0 as usize)
            {
                check_origin_access(
                    flow,
                    function,
                    facts,
                    state,
                    base_origin,
                    LoanEffect::Read,
                    *span,
                    diagnostics,
                    summaries,
                );
            }
        }
        FlowOp::ReadUncheckedLinkField {
            dst,
            link,
            field,
            span,
        } => {
            let base_type = function.value_types[link.0 as usize];
            let base_origin = state.origins.get(link.0 as usize).cloned();
            let origin =
                read_field_origin(flow, function, facts, base_type, *link, *field, base_origin);
            state.origins[dst.0 as usize] = origin;
            if !matches!(
                state.origins.get(dst.0 as usize),
                Some(ValueOrigin::Implicit)
            ) && let Some(base_origin) = state.origins.get(link.0 as usize)
            {
                check_origin_access(
                    flow,
                    function,
                    facts,
                    state,
                    base_origin,
                    LoanEffect::Read,
                    *span,
                    diagnostics,
                    summaries,
                );
            }
        }
        FlowOp::WriteEntityField {
            entity,
            field,
            value,
            span,
        } => {
            if let Some(access) = AccessPath::for_entity_value(
                flow,
                function,
                facts,
                *entity,
                vec![PlaceProjection::Field(*field)],
            ) {
                check_entity_effect_against_pending(
                    facts,
                    state,
                    &access,
                    LoanEffect::Structural,
                    *span,
                    diagnostics,
                );
            }
            let entity_type = function.value_types[entity.0 as usize];
            if let TypeKind::EntityRef(definition) = flow.types.kind(entity_type)
                && let Some(field_type) = flow
                    .definitions
                    .get(definition.0 as usize)
                    .and_then(|definition| {
                        definition
                            .fields
                            .iter()
                            .find(|candidate| candidate.id == *field)
                    })
                    .map(|field| field.ty)
            {
                require_consumable_value(
                    flow,
                    field_type,
                    state
                        .origins
                        .get(value.0 as usize)
                        .cloned()
                        .unwrap_or(ValueOrigin::Unknown),
                    *span,
                    diagnostics,
                );
            }
        }
        FlowOp::Retire { entity, span } => {
            if let Some(access) =
                AccessPath::for_entity_value(flow, function, facts, *entity, Vec::new())
            {
                check_entity_effect_against_pending(
                    facts,
                    state,
                    &access,
                    LoanEffect::Take,
                    *span,
                    diagnostics,
                );
            }
        }
        FlowOp::BeginLifecycle { .. } | FlowOp::Keep { .. } => {}
    }
}

fn verify_terminator(
    flow: &FlowModule,
    function: &FlowFunction,
    terminator: &Terminator,
    state: &HomeState,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let value = match terminator {
        Terminator::Return(value)
        | Terminator::ExitScopes {
            next: ExitTarget::Return(value),
            ..
        } => *value,
        _ => None,
    };
    let Some(value) = value else { return };
    let return_class = flow.types.storage_class(function.return_type);
    if return_class != StorageClass::SingleHome {
        return;
    }
    match state
        .origins
        .get(value.0 as usize)
        .cloned()
        .unwrap_or(ValueOrigin::Unknown)
    {
        ValueOrigin::Local(_) => diagnostics.push(error(
            AMBIGUOUS,
            function.span,
            "returning a named single-home value requires `take` or `.copy()`",
        )),
        ValueOrigin::BorrowedPlace { .. }
        | ValueOrigin::BorrowedValue { .. }
        | ValueOrigin::BorrowedUnknown => {
            diagnostics.push(error(
                PARTIAL_MOVE,
                function.span,
                "cannot move storage out of a managed field",
            ));
        }
        ValueOrigin::Borrowed(_) => diagnostics.push(error(
            BORROWED,
            function.span,
            "a borrowed single-home parameter cannot be returned as owned",
        )),
        ValueOrigin::Entity(_)
        | ValueOrigin::Owned
        | ValueOrigin::Implicit
        | ValueOrigin::Unknown => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn check_origin_access(
    flow: &FlowModule,
    function: &FlowFunction,
    facts: &EntityOperationFacts,
    state: &HomeState,
    origin: &ValueOrigin,
    effect: LoanEffect,
    span: Span,
    diagnostics: &mut Vec<Diagnostic>,
    summaries: &[FunctionStorageSummary],
) {
    let Some(access) = access::origin_access(flow, function, facts, origin) else {
        return;
    };
    if !access.loaned
        && flow
            .types
            .storage_class(function.local_types[access.place.base.0 as usize])
            == StorageClass::SingleHome
    {
        let _ = require_live(
            &state.homes[access.place.base.0 as usize],
            span,
            diagnostics,
        );
    }
    let _ = summaries;
    check_pending_access_inner(facts, state, &access.path, effect, span, diagnostics, true);
}

#[allow(clippy::too_many_arguments)]
fn check_place_access(
    flow: &FlowModule,
    function: &FlowFunction,
    facts: &EntityOperationFacts,
    state: &HomeState,
    place: &Place,
    effect: LoanEffect,
    span: Span,
    diagnostics: &mut Vec<Diagnostic>,
    summaries: &[FunctionStorageSummary],
) {
    check_place_access_inner(
        flow,
        function,
        facts,
        state,
        place,
        effect,
        span,
        diagnostics,
        summaries,
        true,
    );
}

#[allow(clippy::too_many_arguments)]
fn check_place_access_inner(
    flow: &FlowModule,
    function: &FlowFunction,
    facts: &EntityOperationFacts,
    state: &HomeState,
    place: &Place,
    effect: LoanEffect,
    span: Span,
    diagnostics: &mut Vec<Diagnostic>,
    _summaries: &[FunctionStorageSummary],
    check_indexed: bool,
) {
    let base_type = function.local_types[place.base.0 as usize];
    if flow.types.storage_class(base_type) == StorageClass::SingleHome
        && !state.borrowed.contains(&place.base)
    {
        let _ = require_live(&state.homes[place.base.0 as usize], span, diagnostics);
    }
    let access = AccessPath::for_place(flow, function, facts, place);
    check_pending_access_inner(
        facts,
        state,
        &access,
        effect,
        span,
        diagnostics,
        check_indexed,
    );
}

#[allow(clippy::too_many_arguments)]
fn check_place_access_without_indexed(
    flow: &FlowModule,
    function: &FlowFunction,
    facts: &EntityOperationFacts,
    state: &HomeState,
    place: &Place,
    effect: LoanEffect,
    span: Span,
    diagnostics: &mut Vec<Diagnostic>,
    summaries: &[FunctionStorageSummary],
) {
    check_place_access_inner(
        flow,
        function,
        facts,
        state,
        place,
        effect,
        span,
        diagnostics,
        summaries,
        false,
    );
}

#[allow(clippy::too_many_arguments)]
fn check_receiver_access(
    flow: &FlowModule,
    function: &FlowFunction,
    facts: &EntityOperationFacts,
    state: &HomeState,
    receiver: &StorageReceiver,
    effect: LoanEffect,
    span: Span,
    diagnostics: &mut Vec<Diagnostic>,
    summaries: &[FunctionStorageSummary],
) {
    if let Some(place) = &receiver.place {
        check_place_access(
            flow,
            function,
            facts,
            state,
            place,
            effect,
            span,
            diagnostics,
            summaries,
        );
    } else {
        let origin = state
            .origins
            .get(receiver.value.0 as usize)
            .cloned()
            .unwrap_or(ValueOrigin::Unknown);
        check_origin_access(
            flow,
            function,
            facts,
            state,
            &origin,
            effect,
            span,
            diagnostics,
            summaries,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn check_receiver_access_without_indexed(
    flow: &FlowModule,
    function: &FlowFunction,
    facts: &EntityOperationFacts,
    state: &HomeState,
    receiver: &StorageReceiver,
    effect: LoanEffect,
    span: Span,
    diagnostics: &mut Vec<Diagnostic>,
    summaries: &[FunctionStorageSummary],
) {
    if let Some(place) = &receiver.place {
        check_place_access_without_indexed(
            flow,
            function,
            facts,
            state,
            place,
            effect,
            span,
            diagnostics,
            summaries,
        );
    } else {
        let origin = state
            .origins
            .get(receiver.value.0 as usize)
            .cloned()
            .unwrap_or(ValueOrigin::Unknown);
        check_origin_access(
            flow,
            function,
            facts,
            state,
            &origin,
            effect,
            span,
            diagnostics,
            summaries,
        );
    }
}

fn function_return_type(flow: &FlowModule, function: FunctionId) -> TypeId {
    flow.functions
        .get(function.0 as usize)
        .map_or(TypeStore::ERROR, |function| function.return_type)
}

fn field_type(flow: &FlowModule, base_type: TypeId, field: FieldId) -> Option<TypeId> {
    let definition = match flow.types.kind(base_type) {
        TypeKind::Struct(definition) | TypeKind::EntityRef(definition) => *definition,
        TypeKind::Link { entity, .. } => *entity,
        _ => return None,
    };
    flow.definitions
        .get(definition.0 as usize)
        .and_then(|definition| {
            definition
                .fields
                .iter()
                .find(|candidate| candidate.id == field)
        })
        .map(|field| field.ty)
}

fn read_field_origin(
    flow: &FlowModule,
    function: &FlowFunction,
    facts: &EntityOperationFacts,
    base_type: TypeId,
    base: ValueId,
    field: FieldId,
    base_origin: Option<ValueOrigin>,
) -> ValueOrigin {
    let Some(field_type) = field_type(flow, base_type, field) else {
        return ValueOrigin::Unknown;
    };
    if flow.types.storage_class(field_type) != StorageClass::SingleHome {
        return ValueOrigin::Implicit;
    }
    match base_origin {
        Some(ValueOrigin::Local(local)) => {
            let place = Place {
                base: local,
                projections: vec![PlaceProjection::Field(field)],
            };
            let access = AccessPath::for_place(flow, function, facts, &place);
            ValueOrigin::BorrowedPlace {
                place,
                access,
                loaned: false,
            }
        }
        Some(ValueOrigin::Borrowed(local) | ValueOrigin::Entity(local)) => {
            let place = Place {
                base: local,
                projections: vec![PlaceProjection::Field(field)],
            };
            let access = AccessPath::for_place(flow, function, facts, &place);
            ValueOrigin::BorrowedPlace {
                place,
                access,
                loaned: true,
            }
        }
        Some(ValueOrigin::BorrowedPlace {
            mut place,
            mut access,
            loaned,
        }) => {
            place.projections.push(PlaceProjection::Field(field));
            access.push(PlaceProjection::Field(field));
            ValueOrigin::BorrowedPlace {
                place,
                access,
                loaned,
            }
        }
        Some(ValueOrigin::BorrowedValue {
            root,
            mut projections,
        }) => {
            projections.push(PlaceProjection::Field(field));
            ValueOrigin::BorrowedValue { root, projections }
        }
        Some(ValueOrigin::Owned) => ValueOrigin::BorrowedValue {
            root: base,
            projections: vec![PlaceProjection::Field(field)],
        },
        Some(ValueOrigin::BorrowedUnknown | ValueOrigin::Implicit | ValueOrigin::Unknown)
        | None => ValueOrigin::BorrowedUnknown,
    }
}

fn list_remove_origin(flow: &FlowModule, list: TypeId) -> ValueOrigin {
    let TypeKind::List(element) = flow.types.kind(list) else {
        return ValueOrigin::Unknown;
    };
    if flow.types.storage_class(*element) == StorageClass::SingleHome {
        ValueOrigin::Owned
    } else {
        ValueOrigin::Implicit
    }
}

fn list_index_origin(
    flow: &FlowModule,
    function: &FlowFunction,
    facts: &EntityOperationFacts,
    state: &HomeState,
    receiver: &StorageReceiver,
    index: ValueId,
) -> ValueOrigin {
    let Some(TypeKind::List(element)) = function
        .value_types
        .get(receiver.value.0 as usize)
        .map(|ty| flow.types.kind(*ty))
    else {
        return ValueOrigin::Unknown;
    };
    if flow.types.storage_class(*element) != StorageClass::SingleHome {
        return ValueOrigin::Implicit;
    }
    if let Some(place) = receiver.place.clone() {
        let loaned = state.borrowed.contains(&place.base);
        let mut place = place;
        place
            .projections
            .push(PlaceProjection::Index(IndexIdentity::Value(index)));
        let access = AccessPath::for_place(flow, function, facts, &place);
        return ValueOrigin::BorrowedPlace {
            place,
            access,
            loaned,
        };
    }
    if let Some(ValueOrigin::BorrowedValue {
        root,
        mut projections,
    }) = state.origins.get(receiver.value.0 as usize).cloned()
    {
        projections.push(PlaceProjection::Index(IndexIdentity::Value(index)));
        return ValueOrigin::BorrowedValue { root, projections };
    }
    let Some(origin_access) = state
        .origins
        .get(receiver.value.0 as usize)
        .and_then(|origin| access::origin_access(flow, function, facts, origin))
    else {
        return ValueOrigin::BorrowedUnknown;
    };
    let mut place = origin_access.place;
    let mut access = origin_access.path;
    place
        .projections
        .push(PlaceProjection::Index(IndexIdentity::Value(index)));
    access.push(PlaceProjection::Index(IndexIdentity::Value(index)));
    ValueOrigin::BorrowedPlace {
        place,
        access,
        loaned: origin_access.loaned,
    }
}

fn place_type(flow: &FlowModule, function: &FlowFunction, place: &Place) -> Option<TypeId> {
    let mut current = *function.local_types.get(place.base.0 as usize)?;
    for projection in &place.projections {
        current = match projection {
            PlaceProjection::Field(field) => field_type(flow, current, *field)?,
            PlaceProjection::Index(_) => match flow.types.kind(current) {
                TypeKind::List(element) => *element,
                _ => return None,
            },
        };
    }
    Some(current)
}

fn require_consumable_element(
    flow: &FlowModule,
    list: TypeId,
    origin: ValueOrigin,
    span: Span,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let TypeKind::List(element) = flow.types.kind(list) else {
        return;
    };
    require_consumable_value(flow, *element, origin, span, diagnostics);
}

#[allow(clippy::needless_pass_by_value)]
fn require_consumable_value(
    flow: &FlowModule,
    ty: TypeId,
    origin: ValueOrigin,
    span: Span,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if flow.types.storage_class(ty) != StorageClass::SingleHome {
        return;
    }
    match origin {
        ValueOrigin::Local(_) => diagnostics.push(error(
            AMBIGUOUS,
            span,
            "a named single-home list element requires `take` or `.copy()`",
        )),
        ValueOrigin::BorrowedPlace { .. }
        | ValueOrigin::BorrowedValue { .. }
        | ValueOrigin::BorrowedUnknown => {
            diagnostics.push(error(
                PARTIAL_MOVE,
                span,
                "cannot move storage out of a managed field",
            ));
        }
        ValueOrigin::Borrowed(_) => diagnostics.push(error(
            BORROWED,
            span,
            "a borrowed list element cannot be stored",
        )),
        ValueOrigin::Entity(_)
        | ValueOrigin::Owned
        | ValueOrigin::Implicit
        | ValueOrigin::Unknown => {}
    }
}

fn join_states(left: &HomeState, right: &HomeState) -> HomeState {
    HomeState {
        homes: left
            .homes
            .iter()
            .cloned()
            .zip(right.homes.iter().cloned())
            .map(|(left, right)| left.join(&right))
            .collect(),
        borrowed: left.borrowed.union(&right.borrowed).copied().collect(),
        origins: left
            .origins
            .iter()
            .cloned()
            .zip(right.origins.iter().cloned())
            .map(|(left, right)| join_origin(left, right))
            .collect(),
        pending: if left.pending == right.pending {
            left.pending.clone()
        } else {
            Vec::new()
        },
        indexed: if left.indexed == right.indexed {
            left.indexed.clone()
        } else {
            Vec::new()
        },
        cleanup_orders: cleanup::join_cleanup_orders(&left.cleanup_orders, &right.cleanup_orders),
    }
}

fn join_origin(left: ValueOrigin, right: ValueOrigin) -> ValueOrigin {
    if left == right {
        return left;
    }
    match (left, right) {
        (ValueOrigin::Implicit, ValueOrigin::Implicit) => ValueOrigin::Implicit,
        _ => ValueOrigin::Unknown,
    }
}

fn parameter_effect(
    flow: &FlowModule,
    summaries: &[FunctionStorageSummary],
    function: FunctionId,
    parameter: keld_semantics::ParameterIndex,
) -> LoanEffect {
    summaries
        .get(function.0 as usize)
        .and_then(|summary| summary.effects.get(parameter.0 as usize))
        .copied()
        .or_else(|| {
            flow.functions
                .get(function.0 as usize)
                .and_then(|function| function.parameter_modes.get(parameter.0 as usize))
                .map(|mode| match mode {
                    ParameterMode::Loan => LoanEffect::Read,
                    ParameterMode::Take => LoanEffect::Take,
                })
        })
        .unwrap_or(LoanEffect::Read)
}

fn effects_conflict(left: LoanEffect, right: LoanEffect) -> bool {
    !matches!((left, right), (LoanEffect::Read, LoanEffect::Read))
}

#[allow(clippy::needless_pass_by_value)]
#[allow(clippy::too_many_arguments)]
fn check_pending_access(
    state: &HomeState,
    place: Place,
    effect: LoanEffect,
    span: Span,
    diagnostics: &mut Vec<Diagnostic>,
    _summaries: &[FunctionStorageSummary],
    flow: &FlowModule,
    function: &FlowFunction,
    facts: &EntityOperationFacts,
) {
    let access = AccessPath::for_place(flow, function, facts, &place);
    check_pending_access_inner(facts, state, &access, effect, span, diagnostics, true);
}

fn check_pending_access_inner(
    facts: &EntityOperationFacts,
    state: &HomeState,
    access_path: &AccessPath,
    effect: LoanEffect,
    span: Span,
    diagnostics: &mut Vec<Diagnostic>,
    check_indexed: bool,
) {
    let call_conflict = state
        .pending
        .iter()
        .flat_map(|pending| pending.reservations.iter())
        .any(|reservation| reservation_conflicts(facts, reservation, access_path, effect));
    let indexed_conflict = check_indexed
        && matches!(effect, LoanEffect::Structural | LoanEffect::Take)
        && state.indexed.iter().any(|reservation| {
            access::indexed_destination_conflict(facts, &reservation.list, access_path)
        });
    if call_conflict || indexed_conflict {
        diagnostics.push(error(
            if indexed_conflict {
                INDEXED_REPLACEMENT
            } else {
                CONFLICTING_LOANS
            },
            span,
            if indexed_conflict {
                "the right-hand side accesses an indexed replacement destination"
            } else {
                "argument evaluation conflicts with a pending storage reservation"
            },
        ));
    }
}

fn reservation_conflicts(
    facts: &EntityOperationFacts,
    reservation: &Reservation,
    access_path: &AccessPath,
    effect: LoanEffect,
) -> bool {
    if !access::paths_overlap(facts, &reservation.access, access_path) {
        return false;
    }
    if effect == LoanEffect::Read
        && reservation.effect == LoanEffect::Structural
        && access::is_strict_prefix(facts, access_path, &reservation.access)
    {
        return false;
    }
    effects_conflict(reservation.effect, effect)
}

#[allow(clippy::too_many_arguments)]
fn check_call_entity_effects(
    flow: &FlowModule,
    function: &FlowFunction,
    facts: &EntityOperationFacts,
    state: &HomeState,
    callee: FunctionId,
    arguments: &[(keld_semantics::ParameterIndex, ValueId)],
    summaries: &[FunctionStorageSummary],
    lifecycle_summaries: &[keld_lifecycle::FunctionSummary],
    span: Span,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Some(summary) = summaries.get(callee.0 as usize) {
        for effect in &summary.entity_effects {
            let access = match effect.target {
                EntityEffectTarget::Any(definition) => Some(AccessPath::any_entity(
                    definition,
                    effect.projections.clone(),
                )),
                EntityEffectTarget::Parameter(parameter) => argument_value(arguments, parameter)
                    .and_then(|value| {
                        AccessPath::for_entity_value(
                            flow,
                            function,
                            facts,
                            value,
                            effect.projections.clone(),
                        )
                    }),
            };
            if let Some(access) = access {
                check_entity_effect_against_pending(
                    facts,
                    state,
                    &access,
                    effect.effect,
                    span,
                    diagnostics,
                );
            }
        }
    }
    if let Some(summary) = lifecycle_summaries.get(callee.0 as usize) {
        for parameter in &summary.retires_parameters {
            let access = argument_value(arguments, *parameter).and_then(|value| {
                AccessPath::for_entity_value(flow, function, facts, value, Vec::new())
            });
            if let Some(access) = access {
                check_entity_effect_against_pending(
                    facts,
                    state,
                    &access,
                    LoanEffect::Take,
                    span,
                    diagnostics,
                );
            }
        }
        for definition in &summary.retires_any {
            let access = AccessPath::any_entity(*definition, Vec::new());
            check_entity_effect_against_pending(
                facts,
                state,
                &access,
                LoanEffect::Take,
                span,
                diagnostics,
            );
        }
    }
}

fn argument_value(
    arguments: &[(keld_semantics::ParameterIndex, ValueId)],
    parameter: u32,
) -> Option<ValueId> {
    arguments
        .iter()
        .find(|(candidate, _)| candidate.0 == parameter)
        .map(|(_, value)| *value)
}

fn check_entity_effect_against_pending(
    facts: &EntityOperationFacts,
    state: &HomeState,
    access: &AccessPath,
    effect: LoanEffect,
    span: Span,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let conflicts = state
        .pending
        .iter()
        .flat_map(|pending| &pending.reservations)
        .any(|reservation| reservation_conflicts(facts, reservation, access, effect));
    if conflicts {
        diagnostics.push(error(
            CONFLICTING_LOANS,
            span,
            "entity access conflicts with a pending storage reservation",
        ));
    }
}

fn require_live(home: &Home, span: Span, diagnostics: &mut Vec<Diagnostic>) -> bool {
    match home {
        Home::Live => true,
        Home::Empty(EmptyReason::Moved) => {
            diagnostics.push(error(MOVED, span, "use of a moved single-home value"));
            false
        }
        Home::Empty(_) | Home::MaybeLive => {
            diagnostics.push(error(
                MAYBE_LIVE,
                span,
                "single-home value is not live on every path",
            ));
            false
        }
    }
}

fn take_home(home: &mut Home, span: Span, diagnostics: &mut Vec<Diagnostic>) -> bool {
    if !require_live(home, span, diagnostics) {
        return false;
    }
    *home = Home::Empty(EmptyReason::Moved);
    true
}

fn error(code: DiagnosticCode, span: Span, message: impl Into<String>) -> Diagnostic {
    let mut diagnostic = Diagnostic::error(code, span, message);
    diagnostic.help = Some("make the transfer explicit with `take` or `.copy()`".to_owned());
    diagnostic
}

fn successors(terminator: &Terminator) -> Vec<BlockId> {
    match terminator {
        Terminator::Goto(block)
        | Terminator::ExitScopes {
            next: ExitTarget::Goto(block),
            ..
        } => vec![*block],
        Terminator::Branch {
            then_block,
            else_block,
            ..
        }
        | Terminator::BranchIdentity {
            equal: then_block,
            not_equal: else_block,
            ..
        }
        | Terminator::ResolveLink {
            live: then_block,
            absent: else_block,
            ..
        } => {
            vec![*then_block, *else_block]
        }
        Terminator::ExitScopes {
            next: ExitTarget::Return(_),
            ..
        }
        | Terminator::Return(_)
        | Terminator::Unreachable => Vec::new(),
    }
}
