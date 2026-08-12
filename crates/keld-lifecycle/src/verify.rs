use crate::diagnostics::{DiagnosticSink, lifecycle_diagnostic};
use crate::effects::{FunctionSummary, ReturnProvenance};
use crate::provenance::{AbstractState, FailureKind, Origin, RefValue};
use crate::{LifecycleFact, ProofId, ProvenanceId, RefState};
use keld_flow::{
    BlockId, ExitTarget, FlowFunction, FlowModule, FlowOp, LifecycleId, Terminator, ValueId,
};
use keld_semantics::{DefId, FunctionId, LocalId, TypeKind};
use keld_source::{Diagnostic, Span};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProofAnnotation {
    pub proof: ProofId,
    pub function: FunctionId,
    pub block: BlockId,
    pub operation_index: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct VerifiedFlowModule {
    pub flow: FlowModule,
    pub summaries: Vec<FunctionSummary>,
    pub proofs: Vec<ProofAnnotation>,
}

#[derive(Clone, Debug)]
pub struct Verification {
    pub module: Option<VerifiedFlowModule>,
    pub diagnostics: Vec<Diagnostic>,
}

#[must_use]
pub fn verify(flow: FlowModule) -> Verification {
    let summaries = infer_summaries(&flow);
    let mut sink = DiagnosticSink::default();
    validate_persistent_fields(&flow, &mut sink);
    let mut proofs = Vec::new();

    for function in &flow.functions {
        let outcome = analyze_function(&flow, function, &summaries, true);
        validate_effects(
            function,
            &outcome.inference,
            function.id == flow.main,
            &mut sink,
        );
        for diagnostic in outcome.diagnostics {
            sink.push(diagnostic);
        }
        proofs.extend(outcome.proofs);
    }

    let diagnostics = sink.finish();
    let module = diagnostics.is_empty().then_some(VerifiedFlowModule {
        flow,
        summaries,
        proofs,
    });
    Verification {
        module,
        diagnostics,
    }
}

#[must_use]
pub fn verify_text_for_test(text: &str) -> Verification {
    match keld_flow::lower_text_for_test(text) {
        Ok(flow) => verify(flow),
        Err(diagnostics) => Verification {
            module: None,
            diagnostics,
        },
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct Inference {
    retired_parameters: BTreeSet<u32>,
    retires_any: BTreeSet<DefId>,
    return_origin: Origin,
    exact_causes: BTreeMap<u32, Span>,
    broad_causes: BTreeMap<DefId, Span>,
}

struct AnalysisOutcome {
    inference: Inference,
    diagnostics: Vec<Diagnostic>,
    proofs: Vec<ProofAnnotation>,
}

#[derive(Clone)]
struct Catalog {
    parameter: BTreeMap<LocalId, ProvenanceId>,
    value: Vec<Option<ProvenanceId>>,
    resolve: BTreeMap<BlockId, ProvenanceId>,
    entities: Vec<Option<DefId>>,
}

fn infer_summaries(flow: &FlowModule) -> Vec<FunctionSummary> {
    let mut summaries = vec![FunctionSummary::default(); flow.functions.len()];
    for _ in 0..=flow.functions.len() {
        let next = flow
            .functions
            .iter()
            .map(|function| {
                let inference = analyze_function(flow, function, &summaries, false).inference;
                summary_from_inference(flow, function, &inference)
            })
            .collect::<Vec<_>>();
        if next == summaries {
            return next;
        }
        summaries = next;
    }
    summaries
}

fn summary_from_inference(
    flow: &FlowModule,
    function: &FlowFunction,
    inference: &Inference,
) -> FunctionSummary {
    let return_provenance = if entity_type(flow, function.return_type).is_some() {
        ReturnProvenance::EntitySources {
            parameters: inference.return_origin.parameters.iter().copied().collect(),
            fresh_in_caller_lifecycle: inference.return_origin.fresh,
        }
    } else {
        ReturnProvenance::NonEntity
    };
    FunctionSummary {
        retires_parameters: inference.retired_parameters.iter().copied().collect(),
        retires_any: inference.retires_any.iter().copied().collect(),
        return_provenance,
    }
}

fn analyze_function(
    flow: &FlowModule,
    function: &FlowFunction,
    summaries: &[FunctionSummary],
    diagnose: bool,
) -> AnalysisOutcome {
    let catalog = build_catalog(flow, function);
    let mut analyzer = Analyzer {
        flow,
        function,
        summaries,
        catalog,
        diagnose,
        sink: DiagnosticSink::default(),
        proofs: Vec::new(),
        inference: Inference::default(),
        next_proof: 0,
    };
    analyzer.run();
    AnalysisOutcome {
        inference: analyzer.inference,
        diagnostics: analyzer.sink.finish(),
        proofs: analyzer.proofs,
    }
}

struct Analyzer<'module> {
    flow: &'module FlowModule,
    function: &'module FlowFunction,
    summaries: &'module [FunctionSummary],
    catalog: Catalog,
    diagnose: bool,
    sink: DiagnosticSink,
    proofs: Vec<ProofAnnotation>,
    inference: Inference,
    next_proof: u32,
}

impl Analyzer<'_> {
    fn run(&mut self) {
        let reachable = reachable_blocks(self.function);
        let mut indegree = vec![0_usize; self.function.blocks.len()];
        for block in &self.function.blocks {
            if !reachable.contains(&block.id) {
                continue;
            }
            for successor in successors(&block.terminator) {
                if reachable.contains(&successor) {
                    indegree[successor.0 as usize] += 1;
                }
            }
        }

        let mut incoming = vec![Vec::<AbstractState>::new(); self.function.blocks.len()];
        incoming[self.function.entry.0 as usize].push(self.initial_state());
        let mut queue = VecDeque::from([self.function.entry]);
        while let Some(block_id) = queue.pop_front() {
            let states = &incoming[block_id.0 as usize];
            if states.is_empty() {
                self.complete_predecessor(block_id, &mut indegree, &mut queue);
                continue;
            }
            let block = &self.function.blocks[block_id.0 as usize];
            let mut state = AbstractState::join(states, self.function.span);
            for (index, operation) in block.operations.iter().enumerate() {
                self.operation(
                    block_id,
                    u32::try_from(index).expect("flow operation index fits in u32"),
                    operation,
                    &mut state,
                );
            }
            for (successor, successor_state) in self.terminator(block_id, &block.terminator, state)
            {
                if !reachable.contains(&successor) {
                    continue;
                }
                incoming[successor.0 as usize].push(successor_state);
            }
            self.complete_predecessor(block_id, &mut indegree, &mut queue);
        }
    }

    fn complete_predecessor(
        &self,
        block: BlockId,
        indegree: &mut [usize],
        queue: &mut VecDeque<BlockId>,
    ) {
        for successor in successors(&self.function.blocks[block.0 as usize].terminator) {
            let count = &mut indegree[successor.0 as usize];
            *count = count.saturating_sub(1);
            if *count == 0 {
                queue.push_back(successor);
            }
        }
    }

    fn initial_state(&self) -> AbstractState {
        let mut state = AbstractState::new(
            self.function.local_types.len(),
            self.function.value_types.len(),
            self.catalog.entities.len(),
        );
        for (parameter_index, local) in self.function.parameters.iter().copied().enumerate() {
            let Some(&provenance) = self.catalog.parameter.get(&local) else {
                continue;
            };
            let Some(entity) = entity_type(self.flow, self.function.local_types[local.0 as usize])
            else {
                continue;
            };
            let mut origin = Origin::default();
            origin
                .parameters
                .insert(u32::try_from(parameter_index).expect("flow parameter index fits in u32"));
            state.set_live(provenance, LifecycleFact::Dynamic, origin);
            state.locals[local.0 as usize] = Some(RefValue { provenance, entity });
        }
        state
    }

    #[allow(clippy::too_many_lines)]
    fn operation(
        &mut self,
        block: BlockId,
        index: u32,
        operation: &FlowOp,
        state: &mut AbstractState,
    ) {
        match operation {
            FlowOp::CopyLocal { dst, local, span } => {
                if let Some(reference) = state.locals[local.0 as usize]
                    && self.require_live(state, reference, *span)
                {
                    state.values[dst.0 as usize] = Some(reference);
                    self.annotate(block, Some(index));
                }
            }
            FlowOp::StoreLocal { local, value, span } => {
                if let Some(reference) = state.values[value.0 as usize]
                    && self.require_live(state, reference, *span)
                {
                    state.locals[local.0 as usize] = Some(reference);
                    self.annotate(block, Some(index));
                }
            }
            FlowOp::AllocateEntity {
                dst,
                definition,
                lifecycle,
                ..
            } => {
                let provenance = self.catalog.value[dst.0 as usize]
                    .expect("entity allocation has catalog provenance");
                self.distinguish_fresh(state, provenance, *definition);
                state.set_live(
                    provenance,
                    LifecycleFact::Known(*lifecycle),
                    Origin {
                        fresh: true,
                        ..Origin::default()
                    },
                );
                state.values[dst.0 as usize] = Some(RefValue {
                    provenance,
                    entity: *definition,
                });
                self.annotate(block, Some(index));
            }
            FlowOp::EntityToLink { entity, span, .. }
            | FlowOp::ReadEntityField { entity, span, .. }
            | FlowOp::WriteEntityField { entity, span, .. } => {
                if self.require_value_live(state, *entity, *span).is_some() {
                    self.annotate(block, Some(index));
                }
            }
            FlowOp::ReadUncheckedLinkField { span, .. } => {
                self.report(lifecycle_diagnostic(
                    "KLD1005",
                    *span,
                    "a link cannot be dereferenced before its liveness is proven",
                    "resolve the link with `when link as value { ... }`",
                ));
            }
            FlowOp::Call {
                dst,
                function,
                arguments,
                current_lifecycle,
                span,
                ..
            } => self.call(
                block,
                index,
                *dst,
                *function,
                arguments,
                *current_lifecycle,
                *span,
                state,
            ),
            FlowOp::Keep {
                entity,
                target,
                span,
            } => self.keep(block, index, *entity, *target, *span, state),
            FlowOp::Retire { entity, span } => {
                if let Some(reference) = self.require_value_live(state, *entity, *span) {
                    self.record_effect(state, reference, *span);
                    self.retire_exact(state, reference, *span);
                    self.annotate(block, Some(index));
                }
            }
            FlowOp::ConstInt { .. }
            | FlowOp::ConstBool { .. }
            | FlowOp::ConstText { .. }
            | FlowOp::ConstNoneLink { .. }
            | FlowOp::BeginLifecycle { .. }
            | FlowOp::BeginCall { .. }
            | FlowOp::ReserveArgument { .. }
            | FlowOp::TakeLocal { .. }
            | FlowOp::CopyStorage { .. }
            | FlowOp::ListNew { .. }
            | FlowOp::ListLength { .. }
            | FlowOp::ListPush { .. }
            | FlowOp::ListPushPlace { .. }
            | FlowOp::ListLengthLocal { .. }
            | FlowOp::ListPushLocal { .. }
            | FlowOp::ListRemove { .. }
            | FlowOp::ListRemovePlace { .. }
            | FlowOp::ListRemoveLocal { .. }
            | FlowOp::TextByteLength { .. }
            | FlowOp::TextIsEmpty { .. }
            | FlowOp::TextConcat { .. }
            | FlowOp::UnaryInt { .. }
            | FlowOp::BinaryInt { .. }
            | FlowOp::Not { .. }
            | FlowOp::Compare { .. }
            | FlowOp::Phi { .. }
            | FlowOp::ConstructStruct { .. }
            | FlowOp::ReadStructField { .. } => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn call(
        &mut self,
        block: BlockId,
        index: u32,
        dst: Option<ValueId>,
        function: FunctionId,
        arguments: &[(keld_semantics::ParameterIndex, ValueId)],
        current_lifecycle: LifecycleId,
        span: Span,
        state: &mut AbstractState,
    ) {
        let mut actuals = BTreeMap::new();
        let mut has_entity_argument = false;
        for (parameter, value) in arguments {
            if let Some(reference) = state.values[value.0 as usize] {
                has_entity_argument = true;
                if self.require_live(state, reference, span) {
                    actuals.insert(parameter.0, reference);
                }
            }
        }
        let summary = self
            .summaries
            .get(function.0 as usize)
            .cloned()
            .unwrap_or_default();
        for parameter in &summary.retires_parameters {
            if let Some(reference) = actuals.get(parameter).copied() {
                self.record_effect(state, reference, span);
                self.retire_exact(state, reference, span);
            }
        }
        for entity in &summary.retires_any {
            self.inference.retires_any.insert(*entity);
            self.inference.broad_causes.entry(*entity).or_insert(span);
            self.invalidate_entity(state, *entity, span);
        }
        if let Some(dst) = dst
            && let Some(entity) = entity_type(self.flow, self.function.value_types[dst.0 as usize])
            && let ReturnProvenance::EntitySources {
                parameters,
                fresh_in_caller_lifecycle,
            } = summary.return_provenance
        {
            if parameters.len() == 1 && !fresh_in_caller_lifecycle {
                if let Some(reference) = actuals.get(&parameters[0]).copied() {
                    state.values[dst.0 as usize] = Some(reference);
                }
            } else {
                let provenance = self.catalog.value[dst.0 as usize]
                    .expect("entity call result has catalog provenance");
                let mut origin = Origin {
                    fresh: fresh_in_caller_lifecycle,
                    ..Origin::default()
                };
                for parameter in parameters {
                    if let Some(reference) = actuals.get(&parameter) {
                        origin.union_with(&state.origins[reference.provenance.0 as usize]);
                    }
                }
                if origin.fresh && origin.parameters.is_empty() {
                    self.distinguish_fresh(state, provenance, entity);
                }
                let lifecycle = if origin.fresh && origin.parameters.is_empty() {
                    LifecycleFact::Known(current_lifecycle)
                } else {
                    LifecycleFact::Dynamic
                };
                state.set_live(provenance, lifecycle, origin);
                state.values[dst.0 as usize] = Some(RefValue { provenance, entity });
            }
        }
        if has_entity_argument
            || dst.is_some_and(|value| self.catalog.value[value.0 as usize].is_some())
        {
            self.annotate(block, Some(index));
        }
    }

    fn keep(
        &mut self,
        block: BlockId,
        index: u32,
        value: ValueId,
        target: LifecycleId,
        span: Span,
        state: &mut AbstractState,
    ) {
        let Some(reference) = self.require_value_live(state, value, span) else {
            return;
        };
        let Some(LifecycleFact::Known(source)) = state.lifecycle(reference.provenance) else {
            self.report(lifecycle_diagnostic(
                "KLD1004",
                span,
                "`keep` requires a statically known source lifecycle",
                "create the entity in a named lifecycle before keeping it",
            ));
            return;
        };
        if source == target || !is_ancestor(self.function, target, source) {
            self.report(lifecycle_diagnostic(
                "KLD1004",
                span,
                "`keep` must move an entity to a strict active ancestor lifecycle",
                "choose a lifecycle that lexically contains the entity's current lifecycle",
            ));
            return;
        }
        for provenance in state.equivalence_class(reference.provenance) {
            state.refs[provenance.0 as usize] = RefState::Live {
                lifecycle: LifecycleFact::Known(target),
                provenance,
            };
        }
        self.annotate(block, Some(index));
    }

    fn terminator(
        &mut self,
        block: BlockId,
        terminator: &Terminator,
        state: AbstractState,
    ) -> Vec<(BlockId, AbstractState)> {
        match terminator {
            Terminator::Goto(target) => vec![(*target, state)],
            Terminator::Branch {
                then_block,
                else_block,
                ..
            } => vec![(*then_block, state.clone()), (*else_block, state)],
            Terminator::BranchIdentity {
                lhs,
                rhs,
                equal,
                not_equal,
            } => self.branch_identity(block, *lhs, *rhs, *equal, *not_equal, state),
            Terminator::ResolveLink {
                bind_local,
                live,
                absent,
                ..
            } => {
                let mut live_state = state.clone();
                let provenance = self.catalog.resolve[&block];
                let entity =
                    entity_type(self.flow, self.function.local_types[bind_local.0 as usize])
                        .expect("link binding has entity-reference type");
                live_state.set_live(
                    provenance,
                    LifecycleFact::Dynamic,
                    Origin {
                        resolved: true,
                        ..Origin::default()
                    },
                );
                live_state.locals[bind_local.0 as usize] = Some(RefValue { provenance, entity });
                self.annotate(block, None);
                vec![(*live, live_state), (*absent, state)]
            }
            Terminator::ExitScopes { lifecycles, next } => match next {
                ExitTarget::Goto(target) => {
                    let mut state = state;
                    self.exit_lifecycles(&mut state, lifecycles);
                    vec![(*target, state)]
                }
                ExitTarget::Return(value) => {
                    self.return_value(block, *value, lifecycles, &state);
                    Vec::new()
                }
            },
            Terminator::Return(value) => {
                self.return_value(block, *value, &[], &state);
                Vec::new()
            }
            Terminator::Unreachable => Vec::new(),
        }
    }

    fn branch_identity(
        &mut self,
        block: BlockId,
        lhs: ValueId,
        rhs: ValueId,
        equal: BlockId,
        not_equal: BlockId,
        state: AbstractState,
    ) -> Vec<(BlockId, AbstractState)> {
        let Some(lhs_ref) = self.require_value_live(&state, lhs, self.function.span) else {
            return vec![(equal, state.clone()), (not_equal, state)];
        };
        let Some(rhs_ref) = self.require_value_live(&state, rhs, self.function.span) else {
            return vec![(equal, state.clone()), (not_equal, state)];
        };
        self.annotate(block, None);
        let already_equal = state.equivalent(lhs_ref.provenance, rhs_ref.provenance);
        let already_distinct = state.are_distinct(lhs_ref.provenance, rhs_ref.provenance);
        let mut edges = Vec::new();
        if !already_distinct {
            let mut equal_state = state.clone();
            equal_state.add_equal(lhs_ref.provenance, rhs_ref.provenance);
            unify_equal_lifecycles(&mut equal_state, lhs_ref.provenance, rhs_ref.provenance);
            edges.push((equal, equal_state));
        }
        if !already_equal {
            let mut distinct_state = state;
            distinct_state.add_distinct(lhs_ref.provenance, rhs_ref.provenance);
            edges.push((not_equal, distinct_state));
        }
        edges
    }

    fn return_value(
        &mut self,
        block: BlockId,
        value: Option<ValueId>,
        exiting: &[LifecycleId],
        state: &AbstractState,
    ) {
        let Some(value) = value else {
            return;
        };
        let Some(reference) = state.values[value.0 as usize] else {
            return;
        };
        if !self.require_live(state, reference, self.function.span) {
            return;
        }
        let origin = &state.origins[reference.provenance.0 as usize];
        self.inference.return_origin.union_with(origin);
        let fresh_escapes = origin.fresh
            && matches!(state.lifecycle(reference.provenance), Some(LifecycleFact::Known(id)) if id.0 != 0 && exiting.contains(&id));
        if origin.resolved || origin.opaque || fresh_escapes {
            self.report(lifecycle_diagnostic(
                "KLD1002",
                self.function.span,
                "this entity reference cannot escape its proven lifecycle",
                "return a parameter, return a root-lifecycle entity, or `keep` the entity first",
            ));
        } else {
            self.annotate(block, None);
        }
    }

    fn exit_lifecycles(&self, state: &mut AbstractState, exiting: &[LifecycleId]) {
        for index in 0..state.refs.len() {
            let RefState::Live {
                lifecycle: LifecycleFact::Known(lifecycle),
                ..
            } = state.refs[index]
            else {
                continue;
            };
            if exiting
                .iter()
                .any(|root| lifecycle == *root || is_ancestor(self.function, *root, lifecycle))
            {
                state.refs[index] = RefState::OutOfScope;
                state.failures[index] = Some(FailureKind::Scope);
            }
        }
    }

    fn record_effect(&mut self, state: &AbstractState, reference: RefValue, span: Span) {
        let origin = &state.origins[reference.provenance.0 as usize];
        for parameter in &origin.parameters {
            self.inference.retired_parameters.insert(*parameter);
            self.inference
                .exact_causes
                .entry(*parameter)
                .or_insert(span);
        }
        if origin.resolved || origin.opaque {
            self.inference.retires_any.insert(reference.entity);
            self.inference
                .broad_causes
                .entry(reference.entity)
                .or_insert(span);
        }
    }

    fn retire_exact(&mut self, state: &mut AbstractState, reference: RefValue, span: Span) {
        let exact = state.equivalence_class(reference.provenance);
        for provenance in &exact {
            state.refs[provenance.0 as usize] = RefState::Retired { cause: span };
            state.failures[provenance.0 as usize] = Some(FailureKind::Retired);
        }
        for index in 0..state.refs.len() {
            let provenance =
                ProvenanceId(u32::try_from(index).expect("provenance catalog index fits in u32"));
            if exact.contains(&provenance)
                || self.catalog.entities[index] != Some(reference.entity)
                || !matches!(state.refs[index], RefState::Live { .. })
                || state.are_distinct(reference.provenance, provenance)
            {
                continue;
            }
            state.refs[index] = RefState::Invalidated { cause: span };
            state.failures[index] = Some(FailureKind::Alias);
        }
    }

    fn invalidate_entity(&self, state: &mut AbstractState, entity: DefId, span: Span) {
        for (index, definition) in self.catalog.entities.iter().enumerate() {
            if *definition == Some(entity) && matches!(state.refs[index], RefState::Live { .. }) {
                state.refs[index] = RefState::Invalidated { cause: span };
                state.failures[index] = Some(FailureKind::Alias);
            }
        }
    }

    fn distinguish_fresh(
        &self,
        state: &mut AbstractState,
        provenance: ProvenanceId,
        entity: DefId,
    ) {
        for (index, definition) in self.catalog.entities.iter().enumerate() {
            if *definition == Some(entity) && matches!(state.refs[index], RefState::Live { .. }) {
                state.add_distinct(
                    provenance,
                    ProvenanceId(
                        u32::try_from(index).expect("provenance catalog index fits in u32"),
                    ),
                );
            }
        }
    }

    fn require_value_live(
        &mut self,
        state: &AbstractState,
        value: ValueId,
        span: Span,
    ) -> Option<RefValue> {
        let reference = state.values[value.0 as usize]?;
        self.require_live(state, reference, span)
            .then_some(reference)
    }

    fn require_live(&mut self, state: &AbstractState, reference: RefValue, span: Span) -> bool {
        let index = reference.provenance.0 as usize;
        let (code, message, repair) = match state.refs[index] {
            RefState::Live { .. } => return true,
            RefState::Retired { .. } => (
                "KLD1001",
                "use of a retired entity reference",
                "move this use before `retire` or remove the retirement",
            ),
            RefState::Invalidated { .. } if state.failures[index] == Some(FailureKind::Alias) => (
                "KLD1008",
                "this reference may alias an entity retired here or by a call",
                "prove the identities distinct before retirement, or avoid the broad retirement",
            ),
            RefState::Invalidated { .. } | RefState::OutOfScope => (
                "KLD1003",
                "this entity is not live on every path reaching the use",
                "keep the entity live on all paths or move the use into a proven-live branch",
            ),
        };
        self.report(lifecycle_diagnostic(code, span, message, repair));
        false
    }

    fn report(&mut self, diagnostic: Diagnostic) {
        if self.diagnose {
            self.sink.push(diagnostic);
        }
    }

    fn annotate(&mut self, block: BlockId, operation_index: Option<u32>) {
        if !self.diagnose {
            return;
        }
        let proof = ProofId(self.next_proof);
        self.next_proof = self.next_proof.saturating_add(1);
        self.proofs.push(ProofAnnotation {
            proof,
            function: self.function.id,
            block,
            operation_index,
        });
    }
}

fn build_catalog(flow: &FlowModule, function: &FlowFunction) -> Catalog {
    fn allocate(entities: &mut Vec<Option<DefId>>, entity: DefId) -> ProvenanceId {
        let provenance = ProvenanceId(
            u32::try_from(entities.len()).expect("provenance catalog length fits in u32"),
        );
        entities.push(Some(entity));
        provenance
    }

    let mut entities = Vec::new();
    let mut parameter = BTreeMap::new();
    for local in &function.parameters {
        if let Some(entity) = entity_type(flow, function.local_types[local.0 as usize]) {
            let provenance = allocate(&mut entities, entity);
            parameter.insert(*local, provenance);
        }
    }
    let mut value = vec![None; function.value_types.len()];
    for (index, ty) in function.value_types.iter().copied().enumerate() {
        if let Some(entity) = entity_type(flow, ty) {
            value[index] = Some(allocate(&mut entities, entity));
        }
    }
    let mut resolve = BTreeMap::new();
    for block in &function.blocks {
        if let Terminator::ResolveLink { bind_local, .. } = block.terminator
            && let Some(entity) = entity_type(flow, function.local_types[bind_local.0 as usize])
        {
            resolve.insert(block.id, allocate(&mut entities, entity));
        }
    }
    Catalog {
        parameter,
        value,
        resolve,
        entities,
    }
}

fn validate_persistent_fields(flow: &FlowModule, sink: &mut DiagnosticSink) {
    for definition in &flow.definitions {
        for field in &definition.fields {
            if entity_type(flow, field.ty).is_some() {
                sink.push(lifecycle_diagnostic(
                    "KLD1002",
                    field.span,
                    "persistent fields cannot store direct entity references",
                    "store `link Entity?` and resolve it with `when` before use",
                ));
            }
        }
    }
}

fn validate_effects(
    function: &FlowFunction,
    inferred: &Inference,
    is_entrypoint: bool,
    sink: &mut DiagnosticSink,
) {
    if is_entrypoint {
        return;
    }
    let declared_exact = function
        .effects
        .retires
        .iter()
        .filter_map(|local| {
            function
                .parameters
                .iter()
                .position(|parameter| parameter == local)
                .map(|index| u32::try_from(index).expect("flow parameter index fits in u32"))
        })
        .collect::<BTreeSet<_>>();
    let declared_broad = function
        .effects
        .retires_any
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();

    for parameter in inferred.retired_parameters.difference(&declared_exact) {
        sink.push(lifecycle_diagnostic(
            "KLD1009",
            inferred
                .exact_causes
                .get(parameter)
                .copied()
                .unwrap_or(function.span),
            "function retirement behavior is missing from its declared effects",
            "add the retired parameter to the function's `retires` clause",
        ));
    }
    for parameter in declared_exact.difference(&inferred.retired_parameters) {
        let span = function
            .effects
            .retires
            .iter()
            .position(|local| function.parameters.get(*parameter as usize).copied() == Some(*local))
            .and_then(|index| function.effects.retires_spans.get(index).copied())
            .unwrap_or(function.span);
        sink.push(lifecycle_diagnostic(
            "KLD1009",
            span,
            "declared parameter retirement does not occur",
            "remove the redundant parameter from the `retires` clause",
        ));
    }
    for entity in inferred.retires_any.difference(&declared_broad) {
        sink.push(lifecycle_diagnostic(
            "KLD1009",
            inferred
                .broad_causes
                .get(entity)
                .copied()
                .unwrap_or(function.span),
            "broad retirement behavior is missing from the declared effects",
            "add `retires any Entity` to the function signature",
        ));
    }
    for entity in declared_broad.difference(&inferred.retires_any) {
        let span = function
            .effects
            .retires_any
            .iter()
            .position(|candidate| candidate == entity)
            .and_then(|index| function.effects.retires_any_spans.get(index).copied())
            .unwrap_or(function.span);
        sink.push(lifecycle_diagnostic(
            "KLD1009",
            span,
            "declared broad retirement does not occur",
            "remove the redundant `retires any` effect",
        ));
    }
}

fn entity_type(flow: &FlowModule, ty: keld_semantics::TypeId) -> Option<DefId> {
    match flow.types.kind(ty) {
        TypeKind::EntityRef(entity) => Some(*entity),
        _ => None,
    }
}

fn reachable_blocks(function: &FlowFunction) -> BTreeSet<BlockId> {
    let mut reachable = BTreeSet::new();
    let mut stack = vec![function.entry];
    while let Some(block) = stack.pop() {
        if !reachable.insert(block) {
            continue;
        }
        stack.extend(successors(&function.blocks[block.0 as usize].terminator));
    }
    reachable
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
        } => vec![*then_block, *else_block],
        Terminator::BranchIdentity {
            equal, not_equal, ..
        } => vec![*equal, *not_equal],
        Terminator::ResolveLink { live, absent, .. } => vec![*live, *absent],
        Terminator::ExitScopes {
            next: ExitTarget::Return(_),
            ..
        }
        | Terminator::Return(_)
        | Terminator::Unreachable => Vec::new(),
    }
}

fn is_ancestor(function: &FlowFunction, ancestor: LifecycleId, descendant: LifecycleId) -> bool {
    let mut current = function
        .lifecycle_parents
        .get(descendant.0 as usize)
        .copied()
        .flatten();
    while let Some(lifecycle) = current {
        if lifecycle == ancestor {
            return true;
        }
        current = function
            .lifecycle_parents
            .get(lifecycle.0 as usize)
            .copied()
            .flatten();
    }
    false
}

fn unify_equal_lifecycles(state: &mut AbstractState, lhs: ProvenanceId, rhs: ProvenanceId) {
    let lifecycle = match (state.lifecycle(lhs), state.lifecycle(rhs)) {
        (Some(LifecycleFact::Known(lhs)), Some(LifecycleFact::Known(rhs))) if lhs == rhs => {
            LifecycleFact::Known(lhs)
        }
        (Some(LifecycleFact::Known(known)), Some(LifecycleFact::Dynamic))
        | (Some(LifecycleFact::Dynamic), Some(LifecycleFact::Known(known))) => {
            LifecycleFact::Known(known)
        }
        _ => LifecycleFact::Dynamic,
    };
    for provenance in state.equivalence_class(lhs) {
        if matches!(state.refs[provenance.0 as usize], RefState::Live { .. }) {
            state.refs[provenance.0 as usize] = RefState::Live {
                lifecycle,
                provenance,
            };
        }
    }
}
