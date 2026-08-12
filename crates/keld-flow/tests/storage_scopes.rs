use keld_flow::{ExitTarget, StorageScopeId, Terminator, lower_text_for_test};
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
