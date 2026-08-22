use keld_flow::StorageScopeId;
use keld_semantics::LocalId;
use keld_storage::{
    CleanupAction, FunctionStoragePlan, HomeId, LocalStorage, StoreKind, ValueStorage,
    verify_text_for_test,
};

fn plan_for_main(source: &str) -> FunctionStoragePlan {
    let verified = verify_text_for_test(source);
    let module = verified.module.expect("storage verifies");
    module
        .annotations
        .functions
        .last()
        .cloned()
        .expect("main plan exists")
}

fn return_actions(plan: &FunctionStoragePlan) -> &[CleanupAction] {
    plan.blocks
        .iter()
        .rev()
        .find_map(|block| (!block.exit.is_empty()).then_some(block.exit.as_slice()))
        .expect("return cleanup exists")
}

#[test]
fn plan_distinguishes_owned_temporary_local_home_and_loan() {
    let verified = verify_text_for_test(
        "fn inspect(items: List[Int]) -> Int { return items.length }\nfn forward(take items: List[Int]) -> List[Int] { let copied = items.copy(); return take copied }\nfn main() -> Int { return 0 }\n",
    );
    let module = verified.module.expect("storage verifies");
    let inspect = &module.annotations.functions[0];
    let forward = &module.annotations.functions[1];

    assert!(matches!(inspect.locals[0], LocalStorage::Loan));
    assert!(matches!(forward.locals[0], LocalStorage::Home { .. }));
    assert!(
        forward
            .values
            .iter()
            .any(|value| matches!(value, ValueStorage::OwnedTemporary { .. }))
    );
}

#[test]
fn uniform_scope_uses_direct_reverse_drops() {
    let plan = plan_for_main(
        "fn main() -> Int { let first: Text = \"a\"; let second: Text = \"b\"; return 0; }\n",
    );
    assert_eq!(
        return_actions(&plan),
        &[
            CleanupAction::Drop(HomeId::Local(LocalId(1))),
            CleanupAction::Drop(HomeId::Local(LocalId(0))),
        ]
    );
}

#[test]
fn maybe_live_home_uses_one_conditional_flag() {
    let plan = plan_for_main(
        "fn main() -> Int { var text: Text; if true { text = \"a\"; }; return 0; }\n",
    );
    assert_eq!(
        plan.drop_flags,
        [HomeId::Local(LocalId(0))].into_iter().collect()
    );
    assert!(return_actions(&plan).contains(&CleanupAction::DropIfLive(HomeId::Local(LocalId(0)))));
}

#[test]
fn divergent_successful_initialization_order_tracks_only_that_scope() {
    let plan = plan_for_main(
        "fn main() -> Int { var a: Text; var b: Text; if true { a = \"a\"; b = \"b\"; } else { b = \"b\"; a = \"a\"; }; return 0; }\n",
    );
    assert_eq!(
        plan.tracked_scopes,
        [StorageScopeId(0)].into_iter().collect()
    );
    assert_eq!(
        return_actions(&plan),
        &[CleanupAction::CleanupTrackedScope(StorageScopeId(0))]
    );
}

#[test]
fn cyclic_reinitialization_order_uses_bounded_outer_scope_tracking() {
    let plan = plan_for_main(
        "fn main() -> Int { var a: Text = \"a\"; var b: Text = \"b\"; var i = 0; while i < 2 { if i == 0 { let old_a = take a; let old_b = take b; a = \"a\"; b = \"b\" } else { let old_b = take b; let old_a = take a; b = \"b\"; a = \"a\" }; i = i + 1 }; return a.byte_length + b.byte_length }\n",
    );

    assert_eq!(
        plan.tracked_scopes,
        [StorageScopeId(0)].into_iter().collect()
    );
    assert!(
        plan.drop_flags
            .iter()
            .all(|home| matches!(home, HomeId::Local(LocalId(0) | LocalId(1))))
    );
    assert!(plan.drop_flags.len() <= 2);
}

#[test]
fn owned_temporary_loan_drops_after_the_call_succeeds() {
    let plan = plan_for_main(
        "fn inspect(value: Text) -> Int { return value.byte_length }\nfn main() -> Int { return inspect(\"Keld\") }\n",
    );
    assert!(plan.blocks.iter().any(|block| {
        block.operations.iter().any(|operation| {
            operation
                .post_success
                .iter()
                .any(|action| matches!(action, CleanupAction::Drop(HomeId::Temporary(_))))
        })
    }));
}

#[test]
fn store_local_plan_distinguishes_initialization_and_live_replacement() {
    let plan =
        plan_for_main("fn main() -> Int { var text: Text = \"a\"; text = \"b\"; return 0; }\n");
    let stores = plan
        .blocks
        .iter()
        .flat_map(|block| block.operations.iter())
        .filter_map(|operation| operation.store)
        .collect::<Vec<_>>();
    assert_eq!(stores, vec![StoreKind::Initialize, StoreKind::ReplaceLive]);
}
