use keld_flow::{ExitTarget, FlowOp, Terminator, lower_text_for_test};

#[test]
fn return_from_nested_lifecycle_has_an_explicit_exit_edge() {
    let flow = lower_text_for_test(
        "fn main() -> Int { lifecycle outer { lifecycle inner { return 7 } } }\n",
    )
    .unwrap();
    let function = flow.function_named("main").unwrap();

    assert!(function.blocks.iter().any(|block| matches!(
        block.terminator,
        Terminator::ExitScopes {
            ref lifecycles,
            next: ExitTarget::Return(_),
            ..
        } if lifecycles.len() == 2 && lifecycles[0].0 > lifecycles[1].0
    )));
}

#[test]
fn explicit_lifecycles_have_distinct_parented_ids() {
    let flow = lower_text_for_test(
        "fn main() -> Int { lifecycle outer { lifecycle inner { return 0 } } }\n",
    )
    .unwrap();
    let function = flow.function_named("main").unwrap();
    let starts = function
        .linear_ops()
        .into_iter()
        .filter_map(|operation| match operation {
            FlowOp::BeginLifecycle {
                lifecycle, parent, ..
            } => Some((*lifecycle, *parent)),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(function.current_lifecycle.0, 0);
    assert_eq!(starts.len(), 2);
    assert_eq!(starts[0].1.0, 0);
    assert_eq!(starts[1].1, starts[0].0);
}

#[test]
fn when_resolves_link_into_live_and_absent_blocks() {
    let flow = lower_text_for_test(
        "entity Enemy {\nhealth: Int\n}\nfn main() -> Int { let target: link Enemy? = none; when target as live { return live.health } else { return 0 } }\n",
    )
    .unwrap();
    let function = flow.function_named("main").unwrap();

    assert!(
        function
            .blocks
            .iter()
            .any(|block| matches!(block.terminator, Terminator::ResolveLink { .. }))
    );
}

#[test]
fn direct_entity_identity_condition_uses_branch_identity() {
    let flow = lower_text_for_test(
        "entity Enemy {\nhealth: Int\n}\nfn same(a: Enemy, b: Enemy) -> Int { if a == b { return 1 } else { return 0 } }\nfn main() -> Int { lifecycle level { let a = Enemy(health: 1); return same(a, a) } }\n",
    )
    .unwrap();
    let function = flow.function_named("same").unwrap();

    assert!(
        function
            .blocks
            .iter()
            .any(|block| matches!(block.terminator, Terminator::BranchIdentity { .. }))
    );
}

#[test]
fn stable_dump_contains_no_host_addresses_or_debug_format() {
    let source =
        "fn add(a: Int, b: Int) -> Int { return a + b }\nfn main() -> Int { return add(1, 2) }\n";
    let first = lower_text_for_test(source).unwrap().dump();
    let second = lower_text_for_test(source).unwrap().dump();

    assert_eq!(first, second);
    assert!(first.contains("function f0 add"));
    assert!(first.contains("block b0"));
    assert!(!first.contains("0x"));
    assert!(!first.contains("FlowOp"));
}


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
