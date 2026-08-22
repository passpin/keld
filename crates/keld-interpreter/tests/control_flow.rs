use keld_interpreter::{Value, run_text_for_test};

fn run_int(source: &str) -> i64 {
    let result = run_text_for_test(source).expect("Control Flow-1 program must execute");
    let Value::Int(value) = result.value else {
        panic!("expected Int result, got {:?}", result.value);
    };
    value
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
