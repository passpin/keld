from pathlib import Path

path = Path("crates/keld-lifecycle/tests/provenance_facts.rs")
text = path.read_text()
marker = '''#[test]
fn retirement_summaries_remain_exact_and_broad() {
'''
if text.count(marker) != 1:
    raise RuntimeError("retirement test marker is not unique")
addition = r'''#[test]
fn loop_carried_nonfresh_identity_is_may_alias_before_refinement() {
    let module = verified(
        "entity Item { value: Int }\nfn inspect(left: Item, right: Item, choose_right: Bool) -> Bool { var carried = left; var i = 0; while i < 1 { if choose_right { carried = right }; i = i + 1 }; return carried == left }\nfn main() -> Int { return 0 }\n",
    );
    let function = function_id(&module, "inspect");
    let function_data = &module.flow.functions[function.0 as usize];
    let identity_block = function_data
        .blocks
        .iter()
        .find(|block| matches!(block.terminator, Terminator::BranchIdentity { .. }))
        .expect("post-loop identity comparison exists");
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
        .expect("identity comparison publishes facts");
    let carried = facts.local_reference(copied_locals[0]).unwrap();
    let left = facts.local_reference(copied_locals[1]).unwrap();

    assert_eq!(facts.alias_relation(carried, left), AliasRelation::MayAlias);
}

#[test]
fn repeated_when_resolution_forgets_prior_iteration_identity_refinement() {
    let module = verified(
        "entity Item { value: Int }\nentity World { target: link Item? }\nfn inspect(seed: Item, world: World) -> Int { var i = 0; while i < 2 { when world.target as current { if seed != current { i = i + 1; continue } else { return 7 } }; return 0 }; return 1 }\nfn main() -> Int { return 0 }\n",
    );
    let function = function_id(&module, "inspect");
    let function_data = &module.flow.functions[function.0 as usize];
    let identity_block = function_data
        .blocks
        .iter()
        .find(|block| matches!(block.terminator, Terminator::BranchIdentity { .. }))
        .expect("resolved identity comparison exists");
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
        .expect("resolved comparison publishes facts");
    let seed = facts.local_reference(copied_locals[0]).unwrap();
    let current = facts.local_reference(copied_locals[1]).unwrap();

    assert_eq!(facts.alias_relation(seed, current), AliasRelation::MayAlias);
}

'''
path.write_text(text.replace(marker, addition + marker, 1))
print("added remaining Task 4 provenance regressions")
