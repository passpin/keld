from pathlib import Path

control = Path("crates/keld-flow/tests/control_flow.rs")
text = control.read_text()
addition = r'''

#[test]
fn while_lowers_to_condition_scope_branch_and_back_edge() {
    let flow = lower_text_for_test(
        "fn main() -> Int { var i = 0; while i < 3 { i += 1 }; return i }\n",
    )
    .expect("loop source reaches Flow");
    let function = flow.function_named("main").expect("main exists");

    let (branch_block, body, exit) = function
        .blocks
        .iter()
        .find_map(|block| match &block.terminator {
            Terminator::Branch {
                then_block,
                else_block,
                ..
            } => Some((block.id, *then_block, *else_block)),
            _ => None,
        })
        .expect("while condition branch exists");

    let condition = function
        .blocks
        .iter()
        .find(|block| matches!(
            &block.terminator,
            Terminator::ExitScopes {
                storage_scopes,
                lifecycles,
                next: ExitTarget::Goto(target),
            } if *target == branch_block && storage_scopes.len() == 1 && lifecycles.is_empty()
        ))
        .expect("condition full-expression exits its child scope");

    assert_ne!(condition.storage_scope, function.blocks[branch_block.0 as usize].storage_scope);
    assert_eq!(
        function.blocks[branch_block.0 as usize].storage_scope,
        function.blocks[exit.0 as usize].storage_scope
    );
    assert!(matches!(
        &function.blocks[body.0 as usize].terminator,
        Terminator::ExitScopes {
            next: ExitTarget::Goto(target),
            ..
        } if *target == condition.id
    ));
}

#[test]
fn nested_loop_control_targets_the_innermost_loop() {
    let flow = lower_text_for_test(
        "fn main() -> Int { var outer = 0; while outer < 2 { outer += 1; var inner = 0; while inner < 3 { inner += 1; if inner == 1 { continue }; break } }; return outer }\n",
    )
    .expect("nested loops reach Flow");
    let function = flow.function_named("main").expect("main exists");

    let mut loops = function
        .blocks
        .iter()
        .filter_map(|branch| {
            let Terminator::Branch {
                then_block,
                else_block,
                ..
            } = &branch.terminator
            else {
                return None;
            };
            let condition = function.blocks.iter().find(|candidate| matches!(
                &candidate.terminator,
                Terminator::ExitScopes {
                    storage_scopes,
                    lifecycles,
                    next: ExitTarget::Goto(target),
                } if *target == branch.id && storage_scopes.len() == 1 && lifecycles.is_empty()
            ))?;
            Some((branch.id, condition.id, *then_block, *else_block))
        })
        .collect::<Vec<_>>();
    loops.sort_by_key(|(branch, ..)| branch.0);
    assert_eq!(loops.len(), 2, "{:#?}", function.blocks);

    let (_, inner_condition, _, inner_exit) = loops[1];
    assert!(function.blocks.iter().any(|block| matches!(
        &block.terminator,
        Terminator::ExitScopes {
            next: ExitTarget::Goto(target),
            ..
        } if *target == inner_condition
    )), "inner continue must jump to inner condition");
    assert!(function.blocks.iter().any(|block| matches!(
        &block.terminator,
        Terminator::ExitScopes {
            next: ExitTarget::Goto(target),
            ..
        } if *target == inner_exit
    )), "inner break must jump to inner exit");
}
'''
if "fn while_lowers_to_condition_scope_branch_and_back_edge()" in text:
    raise RuntimeError("control-flow loop tests already exist")
control.write_text(text + addition)
print("added Flow loop CFG tests")

storage = Path("crates/keld-flow/tests/storage_scopes.rs")
text = storage.read_text()
text = text.replace(
    "use keld_flow::{ExitTarget, StorageScopeId, Terminator, lower_text_for_test};",
    "use keld_flow::{ExitTarget, FlowOp, StorageScopeId, Terminator, lower_text_for_test};",
    1,
)
addition = r'''

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
'''
if "fn continue_exits_iteration_storage_and_lifecycle_before_back_edge()" in text:
    raise RuntimeError("storage loop test already exists")
storage.write_text(text + addition)
print("added Flow loop cleanup test")
