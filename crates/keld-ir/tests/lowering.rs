use keld_ir::{Instruction, Terminator, ViewMode, lower, validate};
use keld_lifecycle::verify_text_for_test;

#[test]
fn entity_fields_are_lowered_to_closed_access_windows() {
    let verified = verify_text_for_test(
        "entity Counter {\nvalue: Int\n}\nfn main() -> Int { lifecycle level { let counter = Counter(value: 1); counter.value = 2; return counter.value } }\n",
    )
    .module
    .expect("program must verify");
    let module = lower(&verified);
    assert!(validate(&module).is_empty(), "{:#?}", validate(&module));
    let instructions = module.functions[0]
        .blocks
        .iter()
        .flat_map(|block| block.instructions.iter())
        .collect::<Vec<_>>();

    assert!(instructions.windows(3).any(|window| matches!(
        window,
        [
            Instruction::OpenView {
                mode: ViewMode::Edit,
                ..
            },
            Instruction::WriteField { .. },
            Instruction::CloseView { .. }
        ]
    )));
    assert!(instructions.windows(3).any(|window| matches!(
        window,
        [
            Instruction::OpenView {
                mode: ViewMode::Read,
                ..
            },
            Instruction::ReadField { .. },
            Instruction::CloseView { .. }
        ]
    )));
}

#[test]
fn lifecycle_edges_and_identity_branches_become_executable_operations() {
    let verified = verify_text_for_test(
        "entity Enemy {\nhealth: Int\n}\nfn same(a: Enemy, b: Enemy) -> Int { if a == b { return 1 } else { return 0 } }\nfn main() -> Int { lifecycle level { let enemy = Enemy(health: 1); return same(enemy, enemy) } }\n",
    )
    .module
    .expect("program must verify");
    let module = lower(&verified);
    assert!(validate(&module).is_empty(), "{:#?}", validate(&module));

    assert!(
        module
            .functions
            .iter()
            .flat_map(|function| &function.blocks)
            .any(
                |block| matches!(block.terminator, Terminator::Branch { .. })
                    && block
                        .instructions
                        .iter()
                        .any(|instruction| matches!(instruction, Instruction::Compare { .. }))
            )
    );
    assert!(
        module
            .functions
            .iter()
            .flat_map(|function| &function.blocks)
            .any(|block| block
                .instructions
                .iter()
                .any(|instruction| matches!(instruction, Instruction::EndLifecycle { .. })))
    );
}

#[test]
fn resolved_link_edges_and_phi_inputs_validate_as_edge_definitions() {
    let verified = verify_text_for_test(
        "entity Enemy {\nhealth: Int\n}\nfn truth(flag: Bool) -> Bool { return flag && true }\nfn main() -> Int { lifecycle level { let enemy = Enemy(health: 7); let saved: link Enemy? = enemy; when saved as live { if truth(true) { return live.health } else { return 0 } } else { return 0 } } }\n",
    )
    .module
    .expect("program must verify");
    let module = lower(&verified);

    assert!(validate(&module).is_empty(), "{:#?}", validate(&module));
    assert!(
        module
            .functions
            .iter()
            .flat_map(|function| &function.blocks)
            .any(|block| matches!(block.terminator, Terminator::ResolveLink { .. }))
    );
    assert!(
        module
            .functions
            .iter()
            .flat_map(|function| &function.blocks)
            .flat_map(|block| &block.instructions)
            .any(|instruction| matches!(instruction, Instruction::Phi { .. }))
    );
}
