from pathlib import Path

path = Path("crates/keld-cli/tests/milestone_acceptance.rs")
text = path.read_text()

old = "use keld_ir::{Instruction, Module, ViewId, validate};\n"
new = "use keld_ir::{Instruction, Module, Terminator as IrTerminator, ViewId, validate};\n"
if text.count(old) != 1:
    raise RuntimeError("milestone acceptance IR import changed")
text = text.replace(old, new, 1)

anchor = '''const SOURCE_ACCEPTANCE: &[(&str, &str)] = &[\n'''
table = r'''struct ControlFlowProof {
    requirement: &'static str,
    proof: &'static str,
}

const CONTROL_FLOW_1_ACCEPTANCE: [ControlFlowProof; 11] = [
    ControlFlowProof {
        requirement: "ordinary while/break/continue result is 8",
        proof: "control_flow_loop.keld",
    },
    ControlFlowProof {
        requirement: "repeated managed allocation loop result is 6",
        proof: "control_flow_allocations.keld",
    },
    ControlFlowProof {
        requirement: "outside-loop break and continue are KLD0112",
        proof: "control_flow_1_static_loop_join_diagnostics_are_focused",
    },
    ControlFlowProof {
        requirement: "loop-head MaybeLive remains KLD2008",
        proof: "control_flow_1_static_loop_join_diagnostics_are_focused",
    },
    ControlFlowProof {
        requirement: "continue and break execute structured cleanup",
        proof: "keld-interpreter/tests/cleanup.rs",
    },
    ControlFlowProof {
        requirement: "loop-carried entity provenance stays conservative",
        proof: "keld-lifecycle/tests/control_flow.rs",
    },
    ControlFlowProof {
        requirement: "cyclic executable IR validates without a loop opcode",
        proof: "control_flow_1_compiler_ir_is_validated_and_cyclic",
    },
    ControlFlowProof {
        requirement: "plain loops add no lifecycle and ancestor keep survives",
        proof: "keld-interpreter/tests/control_flow.rs and keld-flow/tests/control_flow.rs",
    },
    ControlFlowProof {
        requirement: "body managed homes clean on fallthrough, continue, break, and re-entry",
        proof: "keld-storage/tests/control_flow.rs and keld-interpreter/tests/cleanup.rs",
    },
    ControlFlowProof {
        requirement: "condition effects, checked faults, and managed temporaries respect the boundary",
        proof: "lifecycle/interpreter Control Flow-1 condition regressions",
    },
    ControlFlowProof {
        requirement: "condition allocations reuse a static site and advance attempts at O0/O2",
        proof: "keld-native-backend/tests/differential.rs",
    },
];

'''
if text.count(anchor) != 1:
    raise RuntimeError("SOURCE_ACCEPTANCE anchor changed")
if "CONTROL_FLOW_1_ACCEPTANCE" in text:
    raise RuntimeError("Control Flow-1 acceptance table already present")
text = text.replace(anchor, table + anchor, 1)

marker = '''fn criterion(number: u8) -> &'static CriterionProof {\n'''
tests = r'''#[test]
fn control_flow_1_acceptance_table_is_complete_and_separate() {
    assert_eq!(DONE_CRITERIA.len(), 16, "historical bootstrap criteria stay frozen");
    assert_eq!(CONTROL_FLOW_1_ACCEPTANCE.len(), 11);
    for proof in &CONTROL_FLOW_1_ACCEPTANCE {
        assert!(!proof.requirement.is_empty());
        assert!(!proof.proof.is_empty());
    }
}

#[test]
fn control_flow_1_source_results_match_library_and_cli() {
    assert_accepted_fixture_matches_cli("control_flow_loop.keld", 8);
    assert_accepted_fixture_matches_cli("control_flow_allocations.keld", 6);
}

#[test]
fn control_flow_1_static_loop_join_diagnostics_are_focused() {
    for keyword in ["break", "continue"] {
        assert_inline_static(
            "control-flow-outside-loop.keld",
            &format!("fn main() -> Int {{ {keyword}; return 0 }}\n"),
            "KLD0112",
        );
    }
    assert_inline_static(
        "control-flow-maybe-live.keld",
        "fn main() -> Int { var value = \"x\"; var i = 0; while i < 2 { if i == 0 { let moved = take value }; i += 1 }; return value.byte_length }\n",
        "KLD2008",
    );
}

#[test]
fn control_flow_1_compiler_ir_is_validated_and_cyclic() {
    let compilation = compile_fixture("control_flow_loop.keld");
    assert!(compilation.diagnostics.is_empty(), "{:#?}", compilation.diagnostics);
    let module = compilation.ir.as_ref().expect("Control Flow-1 fixture reaches executable IR");
    assert!(validate(module).is_empty());
    assert!(module.functions.iter().any(|function| function.blocks.iter().any(|block| {
        matches!(
            &block.terminator,
            IrTerminator::Goto(target) if target.0 <= block.id.0
        )
    })), "validated executable IR must contain a CFG backedge");
}

#[test]
fn control_flow_1_self_review_proofs_are_present() {
    let flow = include_str!("../../keld-flow/tests/control_flow.rs");
    let storage = include_str!("../../keld-storage/tests/control_flow.rs");
    let cleanup = include_str!("../../keld-interpreter/tests/cleanup.rs");
    let interpreter = include_str!("../../keld-interpreter/tests/control_flow.rs");
    let lifecycle = include_str!("../../keld-lifecycle/tests/control_flow.rs");
    let differential = include_str!("../../keld-native-backend/tests/differential.rs");

    for proof in [
        "fn return_from_nested_lifecycle_has_an_explicit_exit_edge()",
    ] {
        assert!(flow.contains(proof), "missing Flow proof: {proof}");
    }
    for proof in [
        "fn loop_body_local_is_reinitialized_and_cleaned_on_each_backedge()",
        "fn assignment_after_possible_move_repairs_loop_carried_home()",
    ] {
        assert!(storage.contains(proof), "missing storage proof: {proof}");
    }
    for proof in [
        "fn continue_drops_body_local_text_on_every_iteration()",
        "fn break_drops_body_local_list_before_loop_exit()",
        "fn managed_condition_temporary_is_cleaned_at_the_condition_boundary()",
    ] {
        assert!(cleanup.contains(proof), "missing cleanup proof: {proof}");
    }
    for proof in [
        "fn plain_loop_does_not_create_an_implicit_lifecycle()",
        "fn keep_to_ancestor_survives_break_out_of_inner_lifecycle()",
        "fn checked_fault_in_condition_occurs_before_body()",
    ] {
        assert!(interpreter.contains(proof), "missing interpreter proof: {proof}");
    }
    for proof in [
        "fn loop_carried_entity_from_repeated_allocation_keeps_dynamic_identity()",
        "fn retirement_call_effect_in_condition_invalidates_post_loop_use()",
    ] {
        assert!(lifecycle.contains(proof), "missing lifecycle proof: {proof}");
    }
    assert!(
        differential.contains("fn repeated_condition_concat_uses_one_static_site_with_three_attempts()"),
        "missing repeated condition-allocation proof"
    );
}

'''
if text.count(marker) != 1:
    raise RuntimeError("criterion helper anchor changed")
text = text.replace(marker, tests + marker, 1)
path.write_text(text)
print("added separate 11-item Control Flow-1 milestone acceptance table and proofs")