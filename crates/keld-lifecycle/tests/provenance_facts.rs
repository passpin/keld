use keld_flow::{FlowOp, Terminator};
use keld_lifecycle::{
    AliasRelation, EntityOperationFacts, VerifiedFlowModule, verify_text_for_test,
};
use keld_semantics::{FunctionId, LocalId};

fn verified(text: &str) -> VerifiedFlowModule {
    let result = verify_text_for_test(text);
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    result.module.expect("valid source has a verified module")
}

fn function_id(module: &VerifiedFlowModule, name: &str) -> FunctionId {
    module
        .flow
        .function_named(name)
        .expect("test function exists")
        .id
}

fn facts_before_last_copy(
    module: &VerifiedFlowModule,
    function: FunctionId,
) -> &EntityOperationFacts {
    let function = &module.flow.functions[function.0 as usize];
    let (block, index) = function
        .blocks
        .iter()
        .flat_map(|block| {
            block
                .operations
                .iter()
                .enumerate()
                .filter_map(move |(index, operation)| {
                    matches!(operation, FlowOp::CopyLocal { .. }).then_some((block.id, index))
                })
        })
        .last()
        .expect("test function contains an entity copy");
    module
        .entity_facts_at(function.id, block, u32::try_from(index).unwrap())
        .expect("every operation has projected entity facts")
}

fn first_stored_local(module: &VerifiedFlowModule, function: FunctionId) -> LocalId {
    module.flow.functions[function.0 as usize]
        .blocks
        .iter()
        .flat_map(|block| &block.operations)
        .find_map(|operation| match operation {
            FlowOp::StoreLocal { local, .. } => Some(*local),
            _ => None,
        })
        .expect("test function stores a local")
}

#[test]
fn copied_entity_locals_publish_must_alias_parameter_origins() {
    let module = verified(
        "entity Holder {\nitems: List[Int]\n}\nfn inspect(holder: Holder) -> Int { let alias = holder; return alias.items.length }\nfn main() -> Int { return 0 }\n",
    );
    let function = function_id(&module, "inspect");
    let parameter = module.flow.functions[function.0 as usize].parameters[0];
    let alias = first_stored_local(&module, function);
    let facts = facts_before_last_copy(&module, function);
    let parameter = facts.local_reference(parameter).unwrap();
    let alias = facts.local_reference(alias).unwrap();

    assert_eq!(
        facts.alias_relation(parameter, alias),
        AliasRelation::MustAlias
    );
    assert_eq!(
        facts.origin(alias.provenance).unwrap().parameters,
        [0].into()
    );
    assert!(!facts.origin(alias.provenance).unwrap().fresh);
    assert!(!facts.origin(alias.provenance).unwrap().broad);
}

#[test]
fn fresh_same_type_allocations_publish_must_distinct_origins() {
    let module = verified(
        "entity Holder {\nitems: List[Int]\n}\nfn inspect() -> Bool { lifecycle level { let left = Holder(items: List()); let right = Holder(items: List()); return left == right } }\nfn main() -> Int { return 0 }\n",
    );
    let function = function_id(&module, "inspect");
    let function_data = &module.flow.functions[function.0 as usize];
    let locals = function_data
        .blocks
        .iter()
        .flat_map(|block| &block.operations)
        .filter_map(|operation| match operation {
            FlowOp::StoreLocal { local, .. } => Some(*local),
            _ => None,
        })
        .collect::<Vec<_>>();
    let facts = facts_before_last_copy(&module, function);
    let left = facts.local_reference(locals[0]).unwrap();
    let right = facts.local_reference(locals[1]).unwrap();

    assert_eq!(
        facts.alias_relation(left, right),
        AliasRelation::MustDistinct
    );
    assert!(facts.origin(left.provenance).unwrap().fresh);
    assert!(facts.origin(right.provenance).unwrap().fresh);
}

#[test]
fn unrefined_same_type_parameters_publish_may_alias() {
    let module = verified(
        "entity Holder {\nitems: List[Int]\n}\nfn inspect(left: Holder, right: Holder) -> Bool { return left == right }\nfn main() -> Int { return 0 }\n",
    );
    let function = function_id(&module, "inspect");
    let parameters = &module.flow.functions[function.0 as usize].parameters;
    let facts = facts_before_last_copy(&module, function);
    let left = facts.local_reference(parameters[0]).unwrap();
    let right = facts.local_reference(parameters[1]).unwrap();

    assert_eq!(facts.alias_relation(left, right), AliasRelation::MayAlias);
}

#[test]
fn inequality_branch_publishes_must_distinct_parameters() {
    let module = verified(
        "entity Holder {\nvalue: Int\n}\nfn inspect(left: Holder, right: Holder) -> Int { if left != right { return left.value + right.value } else { return 0 } }\nfn main() -> Int { return 0 }\n",
    );
    let function = function_id(&module, "inspect");
    let function_data = &module.flow.functions[function.0 as usize];
    let parameters = &function_data.parameters;
    let (block, index) = function_data
        .blocks
        .iter()
        .find_map(|block| {
            block
                .operations
                .iter()
                .position(|operation| matches!(operation, FlowOp::ReadEntityField { .. }))
                .map(|index| (block.id, index))
        })
        .expect("true branch reads an entity field");
    let facts = module
        .entity_facts_at(function, block, u32::try_from(index).unwrap())
        .unwrap();
    let left = facts.local_reference(parameters[0]).unwrap();
    let right = facts.local_reference(parameters[1]).unwrap();

    assert_eq!(
        facts.alias_relation(left, right),
        AliasRelation::MustDistinct
    );
}

#[test]
fn loop_carried_allocation_uses_a_stable_distinct_merge_provenance() {
    let module = verified(
        "entity Item { value: Int }\nfn inspect(seed: Item) -> Int { var previous = seed; var i = 0; while i < 2 { let current = Item(value: i); if i == 1 { if previous != current { retire current; return previous.value } else { return 99 } }; previous = current; i = i + 1 }; return -1 }\nfn main() -> Int { lifecycle level { let seed = Item(value: 7); return inspect(seed) } }\n",
    );
    let function = function_id(&module, "inspect");
    let function_data = &module.flow.functions[function.0 as usize];
    let identity_block = function_data
        .blocks
        .iter()
        .find(|block| matches!(block.terminator, Terminator::BranchIdentity { .. }))
        .expect("loop body compares previous and current identities");
    let copied_locals = identity_block
        .operations
        .iter()
        .filter_map(|operation| match operation {
            FlowOp::CopyLocal { local, .. } => Some(*local),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(copied_locals.len(), 2, "{:#?}", identity_block.operations);
    let last_copy = identity_block
        .operations
        .iter()
        .rposition(|operation| matches!(operation, FlowOp::CopyLocal { .. }))
        .unwrap();
    let facts = module
        .entity_facts_at(function, identity_block.id, u32::try_from(last_copy).unwrap())
        .expect("identity block publishes facts before its final copy");
    let previous = facts.local_reference(copied_locals[0]);
    let current = facts.local_reference(copied_locals[1]);
    assert!(
        previous.is_some() && current.is_some(),
        "carried/current refs missing: previous={previous:?}, current={current:?}\nblock={identity_block:#?}\nfacts={facts:#?}"
    );
    let previous = previous.unwrap();
    let current = current.unwrap();

    assert_eq!(
        facts.alias_relation(previous, current),
        AliasRelation::MustDistinct
    );
}

#[test]
fn retirement_summaries_remain_exact_and_broad() {
    let module = verified(
        "entity Enemy {\nhealth: Int\n}\nentity World {\ntarget: link Enemy?\n}\nfn exact(enemy: Enemy) retires enemy { retire enemy }\nfn broad(world: World) retires any Enemy { when world.target as enemy { retire enemy } }\nfn main() -> Int { return 0 }\n",
    );
    let exact = function_id(&module, "exact");
    let broad = function_id(&module, "broad");
    let enemy = module
        .flow
        .definitions
        .iter()
        .find(|definition| definition.name == "Enemy")
        .unwrap()
        .id;

    assert_eq!(
        module.summaries[exact.0 as usize].retires_parameters,
        vec![0]
    );
    assert_eq!(module.summaries[exact.0 as usize].retires_any, Vec::new());
    assert_eq!(
        module.summaries[broad.0 as usize].retires_parameters,
        Vec::new()
    );
    assert_eq!(module.summaries[broad.0 as usize].retires_any, vec![enemy]);

    assert!(
        module.flow.functions[broad.0 as usize]
            .blocks
            .iter()
            .any(|block| matches!(block.terminator, Terminator::ResolveLink { .. }))
    );
}
