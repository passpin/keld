use keld_interpreter::{RuntimeFaultKind, ValueKind, run_text_for_test};

fn run_int(source: &str) -> i64 {
    let result = run_text_for_test(source).expect("Control Flow-1 program must execute");
    match result.value.kind() {
        ValueKind::Int(value) => *value,
        other => panic!("expected Int result, got {other:?}"),
    }
}

#[test]
fn break_and_continue_fixture_executes_to_eight() {
    let source = include_str!("../../keld-cli/tests/fixtures/control_flow_loop.keld");
    assert_eq!(run_int(source), 8);
}

#[test]
fn repeated_managed_allocation_fixture_executes_to_six() {
    let source = include_str!("../../keld-cli/tests/fixtures/control_flow_allocations.keld");
    assert_eq!(run_int(source), 6);
}

#[test]
fn zero_iteration_loop_skips_its_body() {
    assert_eq!(
        run_int("fn main() -> Int { var value = 7; while false { value += 100 }; return value }\n"),
        7
    );
}

#[test]
fn nested_break_and_continue_target_the_innermost_loop() {
    assert_eq!(
        run_int(
            "fn main() -> Int { var outer = 0; var total = 0; while outer < 3 { outer += 1; var inner = 0; while inner < 4 { inner += 1; if inner == 1 { continue }; if inner == 3 { break }; total += inner } }; return total }\n",
        ),
        6
    );
}

#[test]
fn while_condition_is_re_evaluated_after_each_backedge() {
    assert_eq!(
        run_int("fn main() -> Int { var i = 0; while i < 4 { i += 1 }; return i }\n"),
        4
    );
}

#[test]
fn explicit_iteration_lifecycle_exits_before_the_next_iteration() {
    assert_eq!(
        run_int(
            "fn main() -> Int { var i = 0; var total = 0; while i < 2 { lifecycle iteration { let text = \"x\"; total += text.byte_length; i += 1 } }; return total }\n",
        ),
        2
    );
}


#[test]
fn plain_loop_does_not_create_an_implicit_lifecycle() {
    assert_eq!(
        run_int(
            "entity Item { value: Int }\nentity Anchor { target: link Item? }\nfn main() -> Int { lifecycle game { let anchor = Anchor(target: none); var i = 0; while i < 1 { let item = Item(value: 7); anchor.target = item; i += 1 }; when anchor.target as live { return live.value } else { return 0 } } }\n",
        ),
        7
    );
}

#[test]
fn keep_to_ancestor_survives_break_out_of_inner_lifecycle() {
    assert_eq!(
        run_int(
            "entity Item { value: Int }\nentity Anchor { target: link Item? }\nfn main() -> Int { lifecycle game { let anchor = Anchor(target: none); var i = 0; while i < 1 { lifecycle frame { let item = Item(value: 9); anchor.target = item; keep item in game; break } }; when anchor.target as live { return live.value } else { return 0 } } }\n",
        ),
        9
    );
}

#[test]
fn checked_fault_in_condition_occurs_before_body() {
    let fault = run_text_for_test(
        "fn zero() -> Int { return 0 }\nfn main() -> Int { while 1 / zero() == 0 { return 99 }; return 0 }\n",
    )
    .unwrap_err();
    assert_eq!(fault.kind, RuntimeFaultKind::DivisionByZero);
}
