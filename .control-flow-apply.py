from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"{label}: expected one match, found {count}: {old[:120]!r}")
    return text.replace(old, new, 1)


verify_path = Path("crates/keld-lifecycle/src/verify.rs")
text = verify_path.read_text()
text = replace_once(
    text,
    '''struct Catalog {
    parameter: BTreeMap<LocalId, ProvenanceId>,
    value: Vec<Option<ProvenanceId>>,
    resolve: BTreeMap<BlockId, ProvenanceId>,
    entities: Vec<Option<DefId>>,
}
''',
    '''struct Catalog {
    parameter: BTreeMap<LocalId, ProvenanceId>,
    value: Vec<Option<ProvenanceId>>,
    resolve: BTreeMap<BlockId, ProvenanceId>,
    merge: BTreeMap<(BlockId, LocalId), ProvenanceId>,
    entities: Vec<Option<DefId>>,
}
''',
    "Catalog",
)
text = replace_once(
    text,
    '''    fn solve_fixed_point(&mut self) -> Vec<Option<AbstractState>> {
''',
    '''    fn join_at(&self, block: BlockId, states: &[AbstractState]) -> AbstractState {
        let mut joined = AbstractState::join(states, self.function.span);
        for (&(merge_block, local), &merge_provenance) in &self.catalog.merge {
            if merge_block != block {
                continue;
            }
            let local_index = local.0 as usize;
            let references = states
                .iter()
                .map(|state| state.locals[local_index])
                .collect::<Vec<_>>();
            let Some(first) = references.first().copied().flatten() else {
                joined.locals[local_index] = None;
                continue;
            };
            if references.iter().any(Option::is_none) {
                joined.locals[local_index] = None;
                continue;
            }
            let references = references
                .into_iter()
                .map(Option::unwrap)
                .collect::<Vec<_>>();
            if references.iter().all(|reference| *reference == first) {
                joined.locals[local_index] = Some(first);
                continue;
            }
            if references
                .iter()
                .any(|reference| reference.entity != first.entity)
            {
                joined.locals[local_index] = None;
                continue;
            }
            let inputs = states
                .iter()
                .zip(&references)
                .map(|(state, reference)| (state, reference.provenance))
                .collect::<Vec<_>>();
            joined.install_merge(merge_provenance, &inputs, self.function.span);
            joined.locals[local_index] = Some(RefValue {
                provenance: merge_provenance,
                entity: first.entity,
            });
        }
        joined
    }

    fn clear_block_entity_values(&self, operations: &[FlowOp], state: &mut AbstractState) {
        for operation in operations {
            let Some(value) = defined_value(operation) else {
                continue;
            };
            if self
                .catalog
                .value
                .get(value.0 as usize)
                .is_some_and(Option::is_some)
            {
                state.values[value.0 as usize] = None;
            }
        }
    }

    fn solve_fixed_point(&mut self) -> Vec<Option<AbstractState>> {
''',
    "join_at insertion",
)
block_prefix = '''            let block = &self.function.blocks[block_index];
            for (index, operation) in block.operations.iter().enumerate() {
'''
block_replacement = '''            let block = &self.function.blocks[block_index];
            self.clear_block_entity_values(&block.operations, &mut state);
            for (index, operation) in block.operations.iter().enumerate() {
'''
count = text.count(block_prefix)
if count != 2:
    raise RuntimeError(f"block transfer prefix: expected two matches, found {count}")
text = text.replace(block_prefix, block_replacement)
text = replace_once(
    text,
    '''                    Some(existing) => {
                        let joined = AbstractState::join(
                            &[existing.clone(), successor_state],
                            self.function.span,
                        );
                        (joined != *existing).then_some(joined)
                    }
''',
    '''                    Some(existing) => {
                        let joined = self.join_at(
                            successor,
                            &[existing.clone(), successor_state],
                        );
                        (joined != *existing).then_some(joined)
                    }
''',
    "fixed-point join",
)
text = replace_once(
    text,
    '''    let mut resolve = BTreeMap::new();
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
''',
    '''    let mut resolve = BTreeMap::new();
    for block in &function.blocks {
        if let Terminator::ResolveLink { bind_local, .. } = block.terminator
            && let Some(entity) = entity_type(flow, function.local_types[bind_local.0 as usize])
        {
            resolve.insert(block.id, allocate(&mut entities, entity));
        }
    }

    let mut predecessor_counts = vec![0_usize; function.blocks.len()];
    for block in &function.blocks {
        for successor in successors(&block.terminator) {
            predecessor_counts[successor.0 as usize] += 1;
        }
    }
    let mut merge = BTreeMap::new();
    for block in &function.blocks {
        if predecessor_counts[block.id.0 as usize] <= 1 {
            continue;
        }
        for (index, ty) in function.local_types.iter().copied().enumerate() {
            let Some(entity) = entity_type(flow, ty) else {
                continue;
            };
            let local = LocalId(u32::try_from(index).expect("flow local index fits in u32"));
            merge.insert((block.id, local), allocate(&mut entities, entity));
        }
    }
    Catalog {
        parameter,
        value,
        resolve,
        merge,
        entities,
    }
}
''',
    "catalog construction",
)
text = replace_once(
    text,
    '''fn entity_type(flow: &FlowModule, ty: keld_semantics::TypeId) -> Option<DefId> {
''',
    '''fn defined_value(operation: &FlowOp) -> Option<ValueId> {
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
        | FlowOp::ListIndex { dst, .. }
        | FlowOp::ListGet { dst, .. }
        | FlowOp::ListRemove { dst, .. }
        | FlowOp::ListTryRemove { dst, .. }
        | FlowOp::ListTryReserve { dst, .. }
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
        FlowOp::Call { dst, .. } => *dst,
        FlowOp::BeginLifecycle { .. }
        | FlowOp::BeginCall { .. }
        | FlowOp::ReserveArgument { .. }
        | FlowOp::StoreLocal { .. }
        | FlowOp::ListPush { .. }
        | FlowOp::ListClear { .. }
        | FlowOp::ListReserve { .. }
        | FlowOp::BeginIndexedReplacement { .. }
        | FlowOp::EndIndexedReplacement { .. }
        | FlowOp::ListReplace { .. }
        | FlowOp::ReplacePlace { .. }
        | FlowOp::WriteEntityField { .. }
        | FlowOp::Keep { .. }
        | FlowOp::Retire { .. } => None,
    }
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

fn entity_type(flow: &FlowModule, ty: keld_semantics::TypeId) -> Option<DefId> {
''',
    "graph helpers",
)
verify_path.write_text(text)
print("updated lifecycle verifier with bounded merge provenance")

provenance_path = Path("crates/keld-lifecycle/src/provenance.rs")
text = provenance_path.read_text()
text = replace_once(
    text,
    '''    pub fn lifecycle(&self, provenance: ProvenanceId) -> Option<LifecycleFact> {
''',
    '''    pub fn install_merge(
        &mut self,
        target: ProvenanceId,
        inputs: &[(&Self, ProvenanceId)],
        fallback_cause: keld_source::Span,
    ) {
        let Some((first_state, first_provenance)) = inputs.first().copied() else {
            return;
        };
        let first_index = first_provenance.0 as usize;
        let mut reference = first_state.refs[first_index].clone();
        let mut origin = first_state.origins[first_index].clone();
        let mut failure = first_state.failures[first_index];
        for (state, provenance) in &inputs[1..] {
            let index = provenance.0 as usize;
            origin.union_with(&state.origins[index]);
            reference = join_ref(&reference, &state.refs[index], fallback_cause);
            failure = join_failure(failure, state.failures[index], &reference);
        }
        if let RefState::Live { lifecycle, .. } = reference {
            reference = RefState::Live {
                lifecycle,
                provenance: target,
            };
        }
        let target_index = target.0 as usize;
        self.refs[target_index] = reference;
        self.origins[target_index] = origin;
        self.failures[target_index] = failure;
    }

    pub fn lifecycle(&self, provenance: ProvenanceId) -> Option<LifecycleFact> {
''',
    "merge state helper",
)
provenance_path.write_text(text)
print("updated abstract-state merge helper")
