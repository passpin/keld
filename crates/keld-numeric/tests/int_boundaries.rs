use keld_numeric::{
    IntBinaryOp, IntUnaryOp, NumericFault, ParsedIntLiteral, eval_binary, eval_unary,
    parse_int_literal,
};

#[test]
fn parses_int_literals_at_the_signed_boundaries() {
    assert_eq!(parse_int_literal("0"), ParsedIntLiteral::Value(0));
    assert_eq!(
        parse_int_literal("9_223_372_036_854_775_807"),
        ParsedIntLiteral::Value(i64::MAX)
    );
    assert_eq!(
        parse_int_literal("9_223_372_036_854_775_808"),
        ParsedIntLiteral::IntMinMagnitude
    );
    assert_eq!(
        parse_int_literal("9223372036854775809"),
        ParsedIntLiteral::OutOfRange
    );
}

#[test]
fn rejects_non_decimal_or_malformed_literal_text() {
    for raw in ["", "_1", "1_", "1__0", "-1", "12a", "１２"] {
        assert_eq!(
            parse_int_literal(raw),
            ParsedIntLiteral::OutOfRange,
            "{raw:?}"
        );
    }
}

#[test]
fn unary_negation_is_checked() {
    assert_eq!(eval_unary(IntUnaryOp::Neg, 0), Ok(0));
    assert_eq!(eval_unary(IntUnaryOp::Neg, 1), Ok(-1));
    assert_eq!(eval_unary(IntUnaryOp::Neg, i64::MAX), Ok(-i64::MAX));
    assert_eq!(
        eval_unary(IntUnaryOp::Neg, i64::MIN),
        Err(NumericFault::Arithmetic)
    );
}

#[test]
fn arithmetic_boundaries_return_values_or_precise_faults() {
    let cases = [
        (IntBinaryOp::Add, i64::MAX, 1, Err(NumericFault::Arithmetic)),
        (
            IntBinaryOp::Add,
            i64::MIN,
            -1,
            Err(NumericFault::Arithmetic),
        ),
        (IntBinaryOp::Add, i64::MIN, 1, Ok(i64::MIN + 1)),
        (IntBinaryOp::Add, -1, 1, Ok(0)),
        (IntBinaryOp::Sub, i64::MIN, 1, Err(NumericFault::Arithmetic)),
        (
            IntBinaryOp::Sub,
            i64::MAX,
            -1,
            Err(NumericFault::Arithmetic),
        ),
        (IntBinaryOp::Sub, i64::MAX, 1, Ok(i64::MAX - 1)),
        (IntBinaryOp::Sub, 0, 1, Ok(-1)),
        (IntBinaryOp::Mul, i64::MAX, 2, Err(NumericFault::Arithmetic)),
        (
            IntBinaryOp::Mul,
            i64::MIN,
            -1,
            Err(NumericFault::Arithmetic),
        ),
        (IntBinaryOp::Mul, i64::MAX, 0, Ok(0)),
        (IntBinaryOp::Mul, -1, -1, Ok(1)),
    ];

    for (op, lhs, rhs, expected) in cases {
        assert_eq!(eval_binary(op, lhs, rhs), expected, "{op:?} {lhs} {rhs}");
    }
}

#[test]
fn division_and_remainder_edges_match_the_spec() {
    let cases = [
        (IntBinaryOp::Div, 7, 0, Err(NumericFault::DivisionByZero)),
        (
            IntBinaryOp::Div,
            i64::MIN,
            -1,
            Err(NumericFault::Arithmetic),
        ),
        (IntBinaryOp::Div, -7, 3, Ok(-2)),
        (IntBinaryOp::Div, -7, -3, Ok(2)),
        (IntBinaryOp::Rem, 7, 0, Err(NumericFault::DivisionByZero)),
        (IntBinaryOp::Rem, i64::MIN, -1, Ok(0)),
        (IntBinaryOp::Rem, -7, 3, Ok(-1)),
        (IntBinaryOp::Rem, 7, -3, Ok(1)),
    ];

    for (op, lhs, rhs, expected) in cases {
        assert_eq!(eval_binary(op, lhs, rhs), expected, "{op:?} {lhs} {rhs}");
    }
}

#[test]
fn shifts_validate_amount_before_computing() {
    let cases = [
        (IntBinaryOp::Shl, 1, -1, Err(NumericFault::Shift)),
        (IntBinaryOp::Shl, 1, 0, Ok(1)),
        (IntBinaryOp::Shl, -1, 1, Ok(-2)),
        (IntBinaryOp::Shl, 1, 62, Ok(1_i64 << 62)),
        (IntBinaryOp::Shl, 1, 63, Err(NumericFault::Arithmetic)),
        (IntBinaryOp::Shl, 0, 63, Ok(0)),
        (IntBinaryOp::Shl, -1, 63, Ok(i64::MIN)),
        (IntBinaryOp::Shl, 1, 64, Err(NumericFault::Shift)),
        (IntBinaryOp::Shr, 1, -1, Err(NumericFault::Shift)),
        (IntBinaryOp::Shr, 1, 0, Ok(1)),
        (IntBinaryOp::Shr, -2, 1, Ok(-1)),
        (IntBinaryOp::Shr, i64::MIN, 62, Ok(-2)),
        (IntBinaryOp::Shr, i64::MIN, 63, Ok(-1)),
        (IntBinaryOp::Shr, 1, 64, Err(NumericFault::Shift)),
    ];

    for (op, lhs, rhs, expected) in cases {
        assert_eq!(eval_binary(op, lhs, rhs), expected, "{op:?} {lhs} {rhs}");
    }
}
