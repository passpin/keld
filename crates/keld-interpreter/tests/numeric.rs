use keld_interpreter::{RuntimeFaultKind, Value, run_text_for_test};
use keld_numeric::{IntBinaryOp, NumericFault, eval_binary};

#[test]
fn runtime_division_by_zero_has_the_source_fault() {
    let result = run_text_for_test(
        "fn divide(a: Int, b: Int) -> Int { return a / b }\nfn main() -> Int { return divide(7, 0) }\n",
    );
    let fault = result.unwrap_err();

    assert_eq!(fault.kind, RuntimeFaultKind::DivisionByZero);
    assert!(fault.span.start().0 < fault.span.end().0);
}

#[test]
fn checked_arithmetic_and_short_circuit_control_flow_execute() {
    let result = run_text_for_test(
        "fn positive(value: Int) -> Bool { return value > 0 }\nfn main() -> Int { if false && positive(1 / 0) { return 0 } else { return (7 * 6) - 2 } }\n",
    )
    .unwrap();

    assert_eq!(result.value, Value::Int(40));
}

#[test]
fn runtime_numeric_edges_match_the_shared_checked_evaluator() {
    let cases = [
        (IntBinaryOp::Add, i64::MAX, 1),
        (IntBinaryOp::Add, i64::MIN, -1),
        (IntBinaryOp::Add, i64::MIN, 1),
        (IntBinaryOp::Add, -1, 1),
        (IntBinaryOp::Sub, i64::MIN, 1),
        (IntBinaryOp::Sub, i64::MAX, -1),
        (IntBinaryOp::Sub, i64::MAX, 1),
        (IntBinaryOp::Sub, 0, 1),
        (IntBinaryOp::Mul, i64::MAX, 2),
        (IntBinaryOp::Mul, i64::MIN, -1),
        (IntBinaryOp::Mul, i64::MAX, 0),
        (IntBinaryOp::Mul, -1, -1),
        (IntBinaryOp::Div, 7, 0),
        (IntBinaryOp::Div, i64::MIN, -1),
        (IntBinaryOp::Div, -7, 3),
        (IntBinaryOp::Div, -7, -3),
        (IntBinaryOp::Rem, 7, 0),
        (IntBinaryOp::Rem, i64::MIN, -1),
        (IntBinaryOp::Rem, -7, 3),
        (IntBinaryOp::Rem, 7, -3),
        (IntBinaryOp::Shl, 1, -1),
        (IntBinaryOp::Shl, 1, 0),
        (IntBinaryOp::Shl, -1, 1),
        (IntBinaryOp::Shl, 1, 62),
        (IntBinaryOp::Shl, 1, 63),
        (IntBinaryOp::Shl, 0, 63),
        (IntBinaryOp::Shl, -1, 63),
        (IntBinaryOp::Shl, 1, 64),
        (IntBinaryOp::Shr, 1, -1),
        (IntBinaryOp::Shr, 1, 0),
        (IntBinaryOp::Shr, -2, 1),
        (IntBinaryOp::Shr, i64::MIN, 62),
        (IntBinaryOp::Shr, i64::MIN, 63),
        (IntBinaryOp::Shr, 1, 64),
    ];

    for (operation, lhs, rhs) in cases {
        let operator = match operation {
            IntBinaryOp::Add => "+",
            IntBinaryOp::Sub => "-",
            IntBinaryOp::Mul => "*",
            IntBinaryOp::Div => "/",
            IntBinaryOp::Rem => "%",
            IntBinaryOp::Shl => "<<",
            IntBinaryOp::Shr => ">>",
        };
        let source = format!(
            "fn calculate(a: Int, b: Int) -> Int {{ return a {operator} b }}\nfn main() -> Int {{ return calculate({lhs}, {rhs}) }}\n"
        );
        match (eval_binary(operation, lhs, rhs), run_text_for_test(&source)) {
            (Ok(expected), Ok(result)) => assert_eq!(result.value, Value::Int(expected)),
            (Err(expected), Err(actual)) => assert_eq!(actual.kind, fault_kind(expected)),
            (expected, actual) => panic!("numeric parity mismatch: {expected:?} vs {actual:?}"),
        }
    }
}

#[test]
fn unary_minimum_overflow_is_a_runtime_arithmetic_fault() {
    let fault = run_text_for_test(
        "fn negate(value: Int) -> Int { return -value }\nfn main() -> Int { return negate(-9223372036854775808) }\n",
    )
    .unwrap_err();

    assert_eq!(fault.kind, RuntimeFaultKind::Arithmetic);
}

fn fault_kind(fault: NumericFault) -> RuntimeFaultKind {
    match fault {
        NumericFault::Arithmetic => RuntimeFaultKind::Arithmetic,
        NumericFault::DivisionByZero => RuntimeFaultKind::DivisionByZero,
        NumericFault::Shift => RuntimeFaultKind::Shift,
    }
}
