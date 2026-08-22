use keld_flow::FlowOp;
use keld_semantics::LocalId;
use keld_storage::{CleanupAction, HomeId, StoreKind, verify_text_for_test};

#[test]
fn loop_body_local_is_reinitialized_and_cleaned_on_each_backedge() {
    let result = verify_text_for_test(
        "fn main() -> Int { var i = 0; while i < 3 { let text = \"x\"; i = i + text.byte_length }; return i }\n",
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let module = result.module.expect("loop storage verifies");
    let function = module.lifecycle.flow.function_named("main").expect("main exists");
    let plan = &module.annotations.functions[function.id.0 as usize];
    let text_local = LocalId(1);

    let stores = function
        .blocks
        .iter()
        .enumerate()
        .flat_map(|(block_index, block)| {
            block.operations.iter().enumerate().filter_map(move |(operation_index, operation)| {
                matches!(operation, FlowOp::StoreLocal { local, .. } if *local == text_local)
                    .then_some(plan.blocks[block_index].operations[operation_index].store)
                    .flatten()
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(stores, vec![StoreKind::Initialize]);
    assert!(plan.blocks.iter().any(|block| {
        block.exit.contains(&CleanupAction::Drop(HomeId::Local(text_local)))
    }), "loop backedge must clean the body-local text home");
}

#[test]
fn loop_exit_preserves_maybe_live_for_outer_moved_home() {
    let result = verify_text_for_test(
        "fn main() -> Int { var value = \"x\"; var i = 0; while i < 2 { if i == 0 { let moved = take value }; i = i + 1 }; return value.byte_length }\n",
    );
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "KLD2008"),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn assignment_after_possible_move_repairs_loop_carried_home() {
    let result = verify_text_for_test(
        "fn main() -> Int { var value = \"x\"; var i = 0; while i < 2 { if i == 0 { let moved = take value }; value = \"reset\"; i = i + 1 }; return value.byte_length }\n",
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
}

#[test]
fn zero_iteration_loop_does_not_initialize_outer_home() {
    let result = verify_text_for_test(
        "fn main() -> Int { var value: Text; while false { value = \"ready\" }; return value.byte_length }\n",
    );
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(diagnostic.code.0, "KLD2002" | "KLD2008")),
        "{:#?}",
        result.diagnostics
    );
}
