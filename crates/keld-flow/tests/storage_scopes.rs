use keld_flow::{ExitTarget, FlowOp, StorageScopeId, Terminator, lower_text_for_test};
use keld_semantics::LocalId;

#[test]
fn branch_locals_exit_before_the_merge() {
    let flow = lower_text_for_test(
        "fn main() -> Int { if true { let text: Text = \"inner\"; } else { let n = 0; }; return 0; }\n",
    )
    .expect("source reaches Flow");
    let main = flow.function_named("main").expect("main exists");

    let text_local = LocalId(0);
    assert_ne!(main.local_scopes[text_local.0 as usize], StorageScopeId(0));
    assert!(main.blocks.iter().any(|block| matches!(
        &block.terminator,
        Terminator::ExitScopes {
            storage_scopes,
            next: ExitTarget::Goto(_),
            ..
        } if storage_scopes == &[main.local_scopes[text_local.0 as usize]]
    )));
    assert!(flow.dump().contains("storage_exit [s1]"));
}

#[test]
fn return_exits_storage_scopes_inside_out() {
    let flow = lower_text_for_test(
        "fn main() -> Int { lifecycle level { let outer: Text = \"a\"; if true { let inner: Text = \"b\"; return 1; }; }; return 0; }\n",
    )
    .expect("source reaches Flow");
    let main = flow.function_named("main").expect("main exists");
    let exit = main
        .blocks
        .iter()
        .find_map(|block| match &block.terminator {
            Terminator::ExitScopes {
                storage_scopes,
                next: ExitTarget::Return(Some(_)),
                ..
            } => Some(storage_scopes),
            _ => None,
        })
        .expect("return exit exists");
    assert!(
        exit.windows(2)
            .all(|pair| { main.storage_scope_parents[pair[0].0 as usize] == Some(pair[1]) })
    );
}


#[test]
fn continue_exits_iteration_storage_and_lifecycle_before_back_edge() {
    let flow = lower_text_for_test(
        "entity E { value: Int }\nfn main() -> Int { var i = 0; while i < 2 { lifecycle iteration { let e = E(value: i); i += 1; continue } }; return i }\n",
    )
    .expect("loop lifecycle reaches Flow");
    let main = flow.function_named("main").expect("main exists");
    let lifecycle = main
        .linear_ops()
        .into_iter()
        .find_map(|operation| match operation {
            FlowOp::BeginLifecycle { lifecycle, .. } => Some(*lifecycle),
            _ => None,
        })
        .expect("iteration lifecycle exists");

    assert!(main.blocks.iter().any(|block| matches!(
        &block.terminator,
        Terminator::ExitScopes {
            storage_scopes,
            lifecycles,
            next: ExitTarget::Goto(_),
        } if !storage_scopes.is_empty() && lifecycles == &[lifecycle]
    )), "continue must clean both lexical storage and the iteration lifecycle");
}
