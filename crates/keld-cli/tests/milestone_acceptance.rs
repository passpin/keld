use keld_cli::{Compilation, DriverFailure, compile_source, run_source};
use keld_interpreter::{RuntimeFaultKind, Value};
use keld_ir::{Instruction, Module, Terminator as IrTerminator, ViewId, validate};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

struct CriterionProof {
    number: u8,
    requirement: &'static str,
    proof: &'static str,
}

const DONE_CRITERIA: [CriterionProof; 16] = [
    CriterionProof {
        number: 1,
        requirement: "cyclic graph without ownership syntax",
        proof: "crates/keld-cli/tests/fixtures/cyclic_graph.keld",
    },
    CriterionProof {
        number: 2,
        requirement: "lifecycle end invalidates member links",
        proof: "crates/keld-runtime/tests/lifecycle_cleanup.rs::lifecycle_end_stales_links_and_finish_runs_once",
    },
    CriterionProof {
        number: 3,
        requirement: "keep to ancestor survives",
        proof: "crates/keld-cli/tests/fixtures/keep_survives.keld",
    },
    CriterionProof {
        number: 4,
        requirement: "retired direct use is static error",
        proof: "crates/keld-cli/tests/fixtures/fail_retired_use.keld",
    },
    CriterionProof {
        number: 5,
        requirement: "stale link takes absence path",
        proof: "crates/keld-cli/tests/fixtures/stale_link.keld",
    },
    CriterionProof {
        number: 6,
        requirement: "double retirement is rejected",
        proof: "crates/keld-lifecycle/tests/retirement.rs::retiring_a_must_alias_makes_both_names_retired",
    },
    CriterionProof {
        number: 7,
        requirement: "direct reference cannot escape into persistent storage",
        proof: "crates/keld-lifecycle/tests/links_and_escape.rs::persistent_direct_reference_field_requires_a_link",
    },
    CriterionProof {
        number: 8,
        requirement: "cleanup order is deterministic",
        proof: "crates/keld-runtime/tests/lifecycle_cleanup.rs::lifecycle_cleanup_is_reverse_adoption_order",
    },
    CriterionProof {
        number: 9,
        requirement: "runtime model sequences cover store transitions",
        proof: "crates/keld-runtime/tests/model_sequences.rs::store_matches_reference_model_for_generated_sequences",
    },
    CriterionProof {
        number: 10,
        requirement: "library interpreter and CLI results agree",
        proof: "crates/keld-cli/tests/milestone_acceptance.rs::criterion_10_library_and_cli_observations_match",
    },
    CriterionProof {
        number: 11,
        requirement: "grammar goldens cover milestone syntax",
        proof: "crates/keld-syntax/tests/{lexer,parser_golden}.rs",
    },
    CriterionProof {
        number: 12,
        requirement: "may-alias use after retirement is rejected",
        proof: "crates/keld-cli/tests/fixtures/fail_may_alias.keld",
    },
    CriterionProof {
        number: 13,
        requirement: "identity-refined distinct parameters are accepted",
        proof: "crates/keld-cli/tests/fixtures/alias_distinct.keld",
    },
    CriterionProof {
        number: 14,
        requirement: "broad retirement invalidates and permits reacquisition",
        proof: "crates/keld-cli/tests/fixtures/broad_retirement.keld",
    },
    CriterionProof {
        number: 15,
        requirement: "views do not cross structural operations",
        proof: "crates/keld-cli/tests/milestone_acceptance.rs::criterion_15_views_close_before_structural_operations",
    },
    CriterionProof {
        number: 16,
        requirement: "numeric stages agree on values and faults",
        proof: "crates/keld-cli/tests/milestone_acceptance.rs::criterion_16_numeric_stages_have_identical_boundaries",
    },
];

struct ControlFlowProof {
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

const SOURCE_ACCEPTANCE: &[(&str, &str)] = &[
    ("cyclic graph", "cyclic_graph.keld"),
    ("ancestor keep", "keep_survives.keld"),
    ("stale link", "stale_link.keld"),
    ("retired use", "fail_retired_use.keld"),
    ("may alias", "fail_may_alias.keld"),
    ("unsupported feature", "fail_unsupported.keld"),
    ("identity-refined aliases", "alias_distinct.keld"),
    ("broad retirement", "broad_retirement.keld"),
    ("numeric edge", "numeric_edges.keld"),
    ("numeric runtime fault", "runtime_div_zero.keld"),
];

#[test]
fn criterion_01_cyclic_graph_needs_no_ownership_syntax() {
    criterion(1);
    assert_accepted_fixture_matches_cli("cyclic_graph.keld", 20);
}

#[test]
fn criterion_02_lifecycle_end_invalidates_member_links() {
    criterion(2);
    assert_inline_value(
        "criterion-02.keld",
        "entity Enemy { health: Int }\nentity Anchor { target: link Enemy? }\nfn main() -> Int { lifecycle game { let anchor = Anchor(target: none); lifecycle level { let enemy = Enemy(health: 1); anchor.target = enemy }; when anchor.target as live { return live.health } else { return 2 } } }\n",
        2,
    );
}

#[test]
fn criterion_03_keep_to_ancestor_survives() {
    criterion(3);
    assert_accepted_fixture_matches_cli("keep_survives.keld", 30);
}

#[test]
fn criterion_04_retired_direct_use_is_static_error() {
    criterion(4);
    assert_static_fixture("fail_retired_use.keld", "KLD1001");
}

#[test]
fn criterion_05_stale_link_takes_absence_path() {
    criterion(5);
    assert_accepted_fixture_matches_cli("stale_link.keld", 4);
}

#[test]
fn criterion_06_double_retirement_is_rejected() {
    criterion(6);
    assert_inline_static(
        "criterion-06.keld",
        "entity Enemy { health: Int }\nfn broken(enemy: Enemy) retires enemy { retire enemy; retire enemy }\nfn main() -> Int { return 0 }\n",
        "KLD1001",
    );
}

#[test]
fn criterion_07_direct_reference_cannot_escape_to_persistent_storage() {
    criterion(7);
    assert_inline_static(
        "criterion-07.keld",
        "entity Enemy { health: Int }\nentity Invalid { target: Enemy }\nfn main() -> Int { return 0 }\n",
        "KLD1002",
    );
}

#[test]
fn criterion_08_cleanup_order_has_an_executable_proof() {
    let proof = criterion(8);
    let runtime_tests = include_str!("../../keld-runtime/tests/lifecycle_cleanup.rs");
    assert!(
        runtime_tests.contains("fn lifecycle_cleanup_is_reverse_adoption_order()"),
        "missing proof: {}",
        proof.proof
    );
}

#[test]
fn criterion_09_runtime_model_and_generation_exhaustion_are_covered() {
    let proof = criterion(9);
    let model_tests = include_str!("../../keld-runtime/tests/model_sequences.rs");
    let runtime_store = include_str!("../../keld-runtime/src/store.rs");
    assert!(
        model_tests.contains("fn store_matches_reference_model_for_generated_sequences()"),
        "missing proof: {}",
        proof.proof
    );
    assert!(runtime_store.contains("fn generation_exhaustion_permanently_retires_the_slot()"));
}

#[test]
fn criterion_10_library_and_cli_observations_match() {
    criterion(10);
    assert_eq!(SOURCE_ACCEPTANCE.len(), 10);
    for &(name, fixture) in SOURCE_ACCEPTANCE {
        run_acceptance_case(name, fixture);
    }
}

#[test]
fn criterion_11_grammar_goldens_cover_the_milestone_syntax() {
    let proof = criterion(11);
    let lexer_tests = include_str!("../../keld-syntax/tests/lexer.rs");
    let parser_tests = include_str!("../../keld-syntax/tests/parser_golden.rs");
    assert!(lexer_tests.contains("fn virtual_terminators_follow_depth_and_previous_token_rules()"));
    for test in [
        "fn parses_shift_below_addition_and_above_comparison()",
        "fn accepts_every_top_level_form_and_type_clause()",
        "fn accepts_every_statement_family()",
    ] {
        assert!(
            parser_tests.contains(test),
            "missing {test}: {}",
            proof.proof
        );
    }
}

#[test]
fn criterion_12_may_alias_use_after_retirement_is_rejected() {
    criterion(12);
    assert_static_fixture("fail_may_alias.keld", "KLD1008");
}

#[test]
fn criterion_13_identity_refined_distinct_parameters_are_accepted() {
    criterion(13);
    assert_accepted_fixture_matches_cli("alias_distinct.keld", 20);
}

#[test]
fn criterion_14_broad_retirement_allows_link_reacquisition() {
    criterion(14);
    assert_accepted_fixture_matches_cli("broad_retirement.keld", 40);
}

#[test]
fn criterion_15_views_close_before_structural_operations() {
    criterion(15);
    for fixture in accepted_fixtures() {
        let compilation = compile_fixture(fixture);
        assert!(
            compilation.diagnostics.is_empty(),
            "{fixture}: {:#?}",
            compilation.diagnostics
        );
        let module = compilation.ir.as_ref().expect("accepted fixture has IR");
        assert!(validate(module).is_empty(), "{fixture}");
        assert_views_are_instruction_local(module, fixture);
    }
}

#[test]
fn criterion_16_numeric_stages_have_identical_boundaries() {
    criterion(16);
    let cases = [
        (
            "9223372036854775807 + 1",
            "+",
            "9223372036854775807",
            "1",
            RuntimeFaultKind::Arithmetic,
        ),
        ("7 / 0", "/", "7", "0", RuntimeFaultKind::DivisionByZero),
        (
            "-9223372036854775808 / -1",
            "/",
            "-9223372036854775808",
            "-1",
            RuntimeFaultKind::Arithmetic,
        ),
        ("1 << 64", "<<", "1", "64", RuntimeFaultKind::Shift),
    ];
    for (constant, operator, lhs, rhs, fault) in cases {
        assert_inline_static(
            "numeric-constant.keld",
            &format!("fn main() -> Int {{ return {constant} }}\n"),
            "KLD0120",
        );
        assert_inline_runtime_fault(
            "numeric-runtime.keld",
            &format!(
                "fn calculate(a: Int, b: Int) -> Int {{ return a {operator} b }}\nfn main() -> Int {{ return calculate({lhs}, {rhs}) }}\n"
            ),
            fault,
        );
    }
    assert_inline_value(
        "numeric-remainder.keld",
        "fn calculate(a: Int, b: Int) -> Int { return a % b }\nfn main() -> Int { return calculate(-9223372036854775808, -1) }\n",
        0,
    );
}

#[test]
fn control_flow_1_acceptance_table_is_complete_and_separate() {
    assert_eq!(
        DONE_CRITERIA.len(),
        16,
        "historical bootstrap criteria stay frozen"
    );
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
    assert!(
        compilation.diagnostics.is_empty(),
        "{:#?}",
        compilation.diagnostics
    );
    let module = compilation
        .ir
        .as_ref()
        .expect("Control Flow-1 fixture reaches executable IR");
    assert!(validate(module).is_empty());
    assert!(
        module
            .functions
            .iter()
            .any(|function| function.blocks.iter().any(|block| {
                matches!(
                    &block.terminator,
                    IrTerminator::Goto(target) if target.0 <= block.id.0
                )
            })),
        "validated executable IR must contain a CFG backedge"
    );
}

#[test]
fn control_flow_1_self_review_proofs_are_present() {
    let flow = include_str!("../../keld-flow/tests/control_flow.rs");
    let storage = include_str!("../../keld-storage/tests/control_flow.rs");
    let cleanup = include_str!("../../keld-interpreter/tests/cleanup.rs");
    let interpreter = include_str!("../../keld-interpreter/tests/control_flow.rs");
    let lifecycle = include_str!("../../keld-lifecycle/tests/control_flow.rs");
    let differential = include_str!("../../keld-native-backend/tests/differential.rs");

    for proof in ["fn return_from_nested_lifecycle_has_an_explicit_exit_edge()"] {
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
        assert!(
            interpreter.contains(proof),
            "missing interpreter proof: {proof}"
        );
    }
    for proof in [
        "fn loop_carried_entity_from_repeated_allocation_keeps_dynamic_identity()",
        "fn retirement_call_effect_in_condition_invalidates_post_loop_use()",
    ] {
        assert!(
            lifecycle.contains(proof),
            "missing lifecycle proof: {proof}"
        );
    }
    assert!(
        differential
            .contains("fn repeated_condition_concat_uses_one_static_site_with_three_attempts()"),
        "missing repeated condition-allocation proof"
    );
}

fn criterion(number: u8) -> &'static CriterionProof {
    assert_eq!(DONE_CRITERIA.len(), 16);
    for (index, criterion) in DONE_CRITERIA.iter().enumerate() {
        assert_eq!(usize::from(criterion.number), index + 1);
        assert!(!criterion.requirement.is_empty());
        assert!(!criterion.proof.is_empty());
    }
    &DONE_CRITERIA[usize::from(number - 1)]
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn keld() -> Command {
    Command::new(env!("CARGO_BIN_EXE_keld"))
}

fn compile_fixture(name: &str) -> Compilation {
    let path = fixture(name);
    let bytes = std::fs::read(&path).expect("acceptance fixture must be readable");
    compile_source(&path, bytes)
}

fn assert_accepted_fixture_matches_cli(name: &str, expected: i64) {
    let path = fixture(name);
    let bytes = std::fs::read(&path).expect("accepted fixture must be readable");
    let result = run_source(&path, bytes).expect("accepted fixture must run");
    assert_eq!(result.value, Value::Int(expected), "{name}");

    let output = keld()
        .args(["run", "--engine", "interpreter"])
        .arg(&path)
        .output()
        .expect("CLI must launch");
    assert_eq!(output.status.code(), Some(0), "{name}");
    assert_eq!(
        String::from_utf8(output.stdout).expect("CLI stdout is UTF-8"),
        format!("{expected}\n"),
        "{name}"
    );
    assert!(output.stderr.is_empty(), "{name}");
}

fn assert_static_fixture(name: &str, code: &str) {
    let compilation = compile_fixture(name);
    assert_static_code(&compilation, code);

    let output = keld()
        .arg("check")
        .arg(fixture(name))
        .output()
        .expect("CLI must launch");
    assert_eq!(output.status.code(), Some(1), "{name}");
    assert!(output.stdout.is_empty(), "{name}");
    assert!(
        String::from_utf8(output.stderr)
            .expect("CLI stderr is UTF-8")
            .contains(&format!("error[{code}]")),
        "{name}"
    );
}

fn assert_inline_static(path: &str, source: &str, code: &str) {
    let compilation = compile_source(Path::new(path), source.as_bytes().to_vec());
    assert_static_code(&compilation, code);
}

fn assert_static_code(compilation: &Compilation, code: &str) {
    assert!(
        compilation
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == code),
        "expected {code}, got {:#?}",
        compilation.diagnostics
    );
    assert!(compilation.ir.is_none(), "invalid source reached IR");
}

fn assert_inline_value(path: &str, source: &str, expected: i64) {
    let result = run_source(Path::new(path), source.as_bytes().to_vec())
        .expect("inline acceptance program must run");
    assert_eq!(result.value, Value::Int(expected));
}

fn assert_inline_runtime_fault(path: &str, source: &str, expected: RuntimeFaultKind) {
    match run_source(Path::new(path), source.as_bytes().to_vec()) {
        Err(DriverFailure::Runtime { fault, .. }) => assert_eq!(fault.kind, expected),
        result => panic!("expected {expected:?}, got {result:?}"),
    }
}

fn assert_runtime_fixture(name: &str, expected: RuntimeFaultKind) {
    let path = fixture(name);
    let bytes = std::fs::read(&path).expect("runtime fixture must be readable");
    match run_source(&path, bytes) {
        Err(DriverFailure::Runtime { fault, .. }) => assert_eq!(fault.kind, expected, "{name}"),
        result => panic!("expected {expected:?} for {name}, got {result:?}"),
    }

    let output = keld()
        .args(["run", "--engine", "interpreter"])
        .arg(path)
        .output()
        .expect("CLI must launch");
    assert_eq!(output.status.code(), Some(2), "{name}");
    assert!(output.stdout.is_empty(), "{name}");
}

fn run_acceptance_case(name: &str, fixture: &str) {
    match fixture {
        "cyclic_graph.keld" | "alias_distinct.keld" => {
            assert_accepted_fixture_matches_cli(fixture, 20);
        }
        "keep_survives.keld" => assert_accepted_fixture_matches_cli(fixture, 30),
        "stale_link.keld" => assert_accepted_fixture_matches_cli(fixture, 4),
        "fail_retired_use.keld" => assert_static_fixture(fixture, "KLD1001"),
        "fail_may_alias.keld" => assert_static_fixture(fixture, "KLD1008"),
        "fail_unsupported.keld" => assert_static_fixture(fixture, "KLD0004"),
        "broad_retirement.keld" => assert_accepted_fixture_matches_cli(fixture, 40),
        "numeric_edges.keld" => assert_accepted_fixture_matches_cli(fixture, 0),
        "runtime_div_zero.keld" => {
            assert_runtime_fixture(fixture, RuntimeFaultKind::DivisionByZero);
        }
        _ => panic!("unclassified acceptance fixture: {name} ({fixture})"),
    }
}

fn accepted_fixtures() -> [&'static str; 6] {
    [
        "cyclic_graph.keld",
        "keep_survives.keld",
        "stale_link.keld",
        "alias_distinct.keld",
        "broad_retirement.keld",
        "numeric_edges.keld",
    ]
}

fn assert_views_are_instruction_local(module: &Module, fixture: &str) {
    for function in &module.functions {
        for block in &function.blocks {
            let mut active = BTreeSet::<ViewId>::new();
            for instruction in &block.instructions {
                if is_structural(instruction) {
                    assert!(
                        active.is_empty(),
                        "{fixture}: function {:?} block {:?} crosses {instruction:?}",
                        function.id,
                        block.id
                    );
                }
                match instruction {
                    Instruction::OpenView { view, .. } => {
                        assert!(active.insert(*view), "{fixture}: duplicate open view");
                    }
                    Instruction::CloseView { view, .. } => {
                        assert!(active.remove(view), "{fixture}: close of inactive view");
                    }
                    _ => {}
                }
            }
            assert!(
                active.is_empty(),
                "{fixture}: view reaches block terminator"
            );
        }
    }
}

const fn is_structural(instruction: &Instruction) -> bool {
    matches!(
        instruction,
        Instruction::BeginLifecycle { .. }
            | Instruction::EndLifecycle { .. }
            | Instruction::AllocateEntity { .. }
            | Instruction::KeepEntity { .. }
            | Instruction::RetireEntity { .. }
            | Instruction::Call { .. }
    )
}
