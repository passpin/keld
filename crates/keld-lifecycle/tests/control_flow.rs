use keld_flow::FlowOp;
use keld_lifecycle::verify_text_for_test;

#[test]
fn loop_analysis_reaches_entity_operations_and_publishes_facts() {
    let result = verify_text_for_test(
        "entity E { value: Int }\nfn read_many(e: E) -> Int { var i = 0; var total = 0; while i < 3 { total = total + e.value; i = i + 1 }; return total }\nfn main() -> Int { lifecycle level { let e = E(value: 2); return read_many(e) } }\n",
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let module = result.module.expect("loop program verifies");
    let function = module.flow.function_named("read_many").expect("read_many exists");

    let mut entity_operations = 0;
    for block in &function.blocks {
        for (index, operation) in block.operations.iter().enumerate() {
            if !matches!(operation, FlowOp::ReadEntityField { .. }) {
                continue;
            }
            entity_operations += 1;
            assert!(
                module.is_block_reachable(function.id, block.id),
                "entity operation block {:?} must be reachable",
                block.id
            );
            assert!(
                module
                    .entity_facts_at(function.id, block.id, u32::try_from(index).unwrap())
                    .is_some(),
                "reachable entity operation must publish converged facts"
            );
        }
    }
    assert!(entity_operations > 0, "test must contain an entity operation");
}

#[test]
fn retirement_on_one_loop_exit_rejects_post_loop_use() {
    let result = verify_text_for_test(
        "entity E { value: Int }\nfn maybe_retire(e: E, retire_now: Bool) -> Int retires e { var i = 0; while i < 1 { if retire_now { retire e; break }; i = i + 1 }; return e.value }\nfn main() -> Int { return 0 }\n",
    );

    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "KLD1003"),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn loop_carried_entity_from_repeated_allocation_keeps_dynamic_identity() {
    let result = verify_text_for_test(
        "entity Item { value: Int }\nfn inspect(seed: Item) -> Int { var previous = seed; var i = 0; while i < 2 { let current = Item(value: i); if i == 1 { if previous != current { retire current; return previous.value } else { return 99 } }; previous = current; i = i + 1 }; return -1 }\nfn main() -> Int { lifecycle level { let seed = Item(value: 7); return inspect(seed) } }\n",
    );

    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
}
