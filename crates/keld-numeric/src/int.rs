#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntUnaryOp {
    Neg,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntBinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Shl,
    Shr,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NumericFault {
    Arithmetic,
    DivisionByZero,
    Shift,
}

/// Evaluates one checked unary Int operation.
///
/// # Errors
///
/// Returns `Arithmetic` when the mathematical result is outside the Int range.
pub fn eval_unary(op: IntUnaryOp, value: i64) -> Result<i64, NumericFault> {
    match op {
        IntUnaryOp::Neg => value.checked_neg().ok_or(NumericFault::Arithmetic),
    }
}

/// Evaluates one checked binary Int operation.
///
/// # Errors
///
/// Returns the specified arithmetic, division-by-zero, or invalid-shift fault
/// instead of invoking host-language overflow or undefined behavior.
pub fn eval_binary(op: IntBinaryOp, lhs: i64, rhs: i64) -> Result<i64, NumericFault> {
    match op {
        IntBinaryOp::Add => lhs.checked_add(rhs).ok_or(NumericFault::Arithmetic),
        IntBinaryOp::Sub => lhs.checked_sub(rhs).ok_or(NumericFault::Arithmetic),
        IntBinaryOp::Mul => lhs.checked_mul(rhs).ok_or(NumericFault::Arithmetic),
        IntBinaryOp::Div => checked_div(lhs, rhs),
        IntBinaryOp::Rem => checked_rem(lhs, rhs),
        IntBinaryOp::Shl => checked_shl(lhs, rhs),
        IntBinaryOp::Shr => checked_shr(lhs, rhs),
    }
}

fn checked_div(lhs: i64, rhs: i64) -> Result<i64, NumericFault> {
    if rhs == 0 {
        return Err(NumericFault::DivisionByZero);
    }
    lhs.checked_div(rhs).ok_or(NumericFault::Arithmetic)
}

fn checked_rem(lhs: i64, rhs: i64) -> Result<i64, NumericFault> {
    if rhs == 0 {
        return Err(NumericFault::DivisionByZero);
    }
    if lhs == i64::MIN && rhs == -1 {
        return Ok(0);
    }
    Ok(lhs % rhs)
}

fn checked_shl(lhs: i64, rhs: i64) -> Result<i64, NumericFault> {
    let amount = valid_shift_amount(rhs)?;
    let mathematical = i128::from(lhs) * (1_i128 << amount);
    i64::try_from(mathematical).map_err(|_| NumericFault::Arithmetic)
}

fn checked_shr(lhs: i64, rhs: i64) -> Result<i64, NumericFault> {
    let amount = valid_shift_amount(rhs)?;
    Ok(lhs >> amount)
}

fn valid_shift_amount(rhs: i64) -> Result<u32, NumericFault> {
    let amount = u32::try_from(rhs).map_err(|_| NumericFault::Shift)?;
    if amount < i64::BITS {
        Ok(amount)
    } else {
        Err(NumericFault::Shift)
    }
}
