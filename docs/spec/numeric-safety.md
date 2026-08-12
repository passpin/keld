# Keld Numeric and Runtime Safety

Status: normative Keld 0.1 core

This document defines integer arithmetic, conversions, indexing quantities,
runtime faults, and backend obligations. Safe Keld has defined behavior in every
build. An optimizer must not introduce target-language undefined behavior for an
operation that is defined here.

## 1. Numeric Types

Keld defines these integer types:

| Type | Range |
|---|---|
| `I8` | -2^7 through 2^7 - 1 |
| `I16` | -2^15 through 2^15 - 1 |
| `I32` | -2^31 through 2^31 - 1 |
| `I64` | -2^63 through 2^63 - 1 |
| `U8` | 0 through 2^8 - 1 |
| `U16` | 0 through 2^16 - 1 |
| `U32` | 0 through 2^32 - 1 |
| `U64` | 0 through 2^64 - 1 |

`Int` is a language alias for `I64`. `UInt` is a language alias for `U64`.
Their size does not change with the compilation target. Keld exposes no
platform-dependent integer such as `usize` in safe source.

Every integer type exposes `MIN` and `MAX` constants, such as `Int.MIN` and
`Int.MAX`. The aliases expose the constants of their underlying fixed-width
types.

The floating-point types are `F32` and `F64` and follow IEEE 754 as constrained
by Section 8.

## 2. Integer Literals and Constant Evaluation

The lexer records an integer literal without immediately choosing a machine
type. Constant evaluation uses arbitrary-precision signed integers.

- Context chooses a concrete integer type when one is expected.
- Otherwise an integer literal has type `Int`.
- A literal outside the destination type's range is a compile-time error.
- A constant expression that would fault at runtime is a compile-time error.
- Constant evaluation and runtime evaluation must produce the same value for
  every accepted expression.

Unary `-` is an operator, not part of the literal. This permits the source form
`-9223372036854775808` to be checked as the representable `Int.MIN` value. Range
checking therefore occurs after constant unary negation; the positive token in
this one boundary form is not rejected first as an `Int` operand.

Binary integer arithmetic and numeric comparison require both non-literal
operands to have the same concrete type. Lossless implicit widening is available
in assignment, argument, return, and explicitly typed expression contexts, but
Keld does not apply C-like integer promotions to choose a new type for two
differently typed variables. One operand must be converted explicitly:

```keld
let total = I64(small) + large
```

An untyped integer literal adopts the other operand's concrete integer type when
representable.

## 3. Default Integer Arithmetic

Default integer arithmetic is checked in debug and optimized builds.

| Operation | Defined result |
|---|---|
| `a + b` | mathematical sum, or `ArithmeticFault` |
| `a - b` | mathematical difference, or `ArithmeticFault` |
| `a * b` | mathematical product, or `ArithmeticFault` |
| `-a` | mathematical negation, or `ArithmeticFault` |
| `a / b` | quotient truncated toward zero |
| `a % b` | remainder with the sign of `a` |
| `a << n` | mathematical multiplication by 2^n, or a fault |
| `a >> n` | bitwise right shift after validating `n` |

Overflow means the mathematical result is outside the result type's range. It
produces `ArithmeticFault`; it never wraps silently.

Unary negation is not defined for unsigned integers. Use unsigned subtraction or
an explicit conversion when the intended result may be negative.

Compound assignments use the corresponding checked operation. For example,
`total += value` performs checked addition and changes `total` only after that
addition succeeds.

## 4. Division and Remainder

Operands evaluate left to right. Division then applies these checks in order:

1. If the divisor is zero, produce `DivisionByZeroFault`.
2. For signed division, if the dividend is `MIN` and the divisor is `-1`,
   produce `ArithmeticFault`.
3. Otherwise compute the quotient truncated toward zero.

Remainder first checks for a zero divisor. Signed remainder then uses:

```text
remainder = dividend - quotient * divisor
```

The special expression `MIN % -1` is defined as zero even though the
corresponding quotient is unrepresentable; it does not produce
`ArithmeticFault`. The backend must test this case before emitting a machine
remainder instruction whose behavior is undefined or trapping for that input.

Floating-point division by zero follows IEEE 754 and does not produce an integer
division fault.

## 5. Shifts

The shift amount is an `Int`. It must satisfy:

```text
0 <= amount < bit_width(left_operand)
```

An invalid amount produces `ShiftFault`.

Default left shift is checked arithmetic. For unsigned values it produces
`ArithmeticFault` when a nonzero high bit would be discarded. For signed values
it computes the mathematical product by 2^amount and faults only when that value
is not representable. Default signed right shift propagates the sign bit.
Unsigned right shift inserts zero bits.

Bit-rotation and deliberately discarding shifts require explicitly named
operations; they are not alternate interpretations of `<<` or `>>`.

## 6. Explicit Arithmetic Alternatives

Checked operations return an optional result and never produce an arithmetic,
division, or shift fault for the tested condition:

```keld
let sum: Int? = a.checked_add(b)
let quotient: Int? = a.checked_div(b)
let shifted: Int? = a.checked_shl(amount)
```

Required checked operations are:

```text
checked_add checked_sub checked_mul checked_neg
checked_div checked_rem checked_shl checked_shr
```

`checked_div` returns `none` for a zero divisor and for `MIN / -1`.
`checked_rem` returns `none` only for a zero divisor; `MIN % -1` returns zero.
`checked_shl` returns `none` for an invalid amount or an unrepresentable result.
`checked_shr` returns `none` for an invalid amount.

Saturating operations are defined for addition, subtraction, multiplication,
and negation:

```text
saturating_add saturating_sub saturating_mul saturating_neg
```

Wrapping operations are defined for addition, subtraction, multiplication,
negation, and left shift:

```text
wrapping_add wrapping_sub wrapping_mul wrapping_neg wrapping_shl
```

Negation variants exist only for signed integer types. `wrapping_shl` discards
high bits for a valid shift amount but still produces `ShiftFault` when the
amount is negative or at least the left operand's bit width. The other wrapping
operations compute modulo 2^bit_width. A signed wrapping result is the unique
two's-complement value with that bit pattern. Saturating operations clamp to the
nearest representable bound.

There is no wrapping or saturating division by zero. Programs that need to
handle zero use `checked_div`.

## 7. Integer Conversions

Implicit integer widening is permitted only when every source value is
representable by the destination type. Examples include `I8` to `I16` and `U8`
to `I16`. It occurs only in the conversion contexts defined in Section 2.
Signed-to-unsigned conversion is never implicit.

An explicit conversion uses the destination type as a constructor:

```keld
let small = I32(large)
```

It produces `ConversionFault` if the value is outside the destination range.
The optional form does not fault:

```keld
let small: I32? = I32.checked(large)
```

Integer-to-integer conversions preserve the mathematical value. They do not
reinterpret bits. Future bit reinterpretation APIs must be explicitly named.

## 8. Floating-Point Rules

`F32` and `F64` use IEEE 754 finite values, infinities, signed zero, and NaN.
Default floating-point arithmetic does not produce integer arithmetic faults.
Keld 0.1 defines `+`, `-`, `*`, and `/` for floats; `%` and shifts are
integer-only.

Arithmetic uses round-to-nearest, ties-to-even. Subnormal values are preserved;
safe Keld does not expose a mutable rounding mode or floating-point exception
flags. Positive and negative zero compare equal. Every ordered comparison with
NaN is false, equality with NaN is false, and inequality with NaN is true.

Compiler optimizations must preserve Keld's observable IEEE behavior unless a
future source-level relaxed-math mode explicitly permits otherwise. Relaxed math
is not part of Keld 0.1.

Converting a float to an integer truncates toward zero after checking that the
input is finite and the result is representable. `Int(value)` faults on NaN,
infinity, or an out-of-range result. `Int.checked(value)` returns `none` for
those inputs.

All integer-to-float and float-to-integer conversions are explicit. An
integer-to-float conversion uses IEEE 754 round-to-nearest, ties-to-even. `F32`
to `F64` is lossless and may be implicit in a typed conversion context; `F64` to
`F32` is explicit and uses IEEE rounding, including overflow to signed infinity.

## 9. Lengths, Indices, and Address-Space Checks

Container lengths and indices use `Int` in source. Internal capacities use the
same nonnegative `Int` range but need not be observable.

- A valid index satisfies `0 <= index < length`.
- Negative indices are invalid.
- A container's length and capacity cannot exceed `Int.MAX`.
- Target addressability may impose a smaller maximum.
- Element-size multiplication and capacity growth are checked before allocation.
- Converting a checked source `Int` size into a target address size is checked.

These rules apply on native and WebAssembly targets. A 32-bit target must reject
or fail an allocation that is valid in `Int` but not representable by its address
space. It must not truncate the size.

## 10. Runtime Faults

A fault is non-catchable termination caused by a violated safe operation
precondition. Arithmetic, division, shift, conversion, bounds, and capacity fault
conditions are deterministic for fixed inputs. `AllocationFault`
depends on target resource availability. A fault is distinct from a typed
recoverable error.

Keld 0.1 defines:

```text
ArithmeticFault
DivisionByZeroFault
ShiftFault
ConversionFault
BoundsFault
CapacityFault
AllocationFault
```

A fault report contains the fault kind and source location. The current execution
terminates without promising user cleanup. Before termination, the runtime must
remain memory safe; a fault cannot be implemented by invoking undefined
behavior.

Programs that expect invalid input use optional checked operations or typed
error-producing APIs. Ordinary arithmetic and indexing remain concise for cases
where failure is a programming defect.

## 11. Allocation and Capacity Failure

Ordinary allocation and automatic container growth produce `AllocationFault` on
allocator failure. An impossible requested capacity produces `CapacityFault`.
An implementation may first request spare growth capacity, but if that larger
allocation fails it must retry the minimum required capacity before producing
`AllocationFault` or returning recoverable failure.

`List.try_reserve(additional)` is the recoverable capacity API:

```keld
if list.try_reserve(additional) {
    list.push(value)
} else {
    report_capacity_failure()
}
```

It returns `false` and leaves the list unchanged when `length + additional` is
invalid, not addressable, or cannot be allocated. It returns `true` only after
room for that many additional elements is available. The next `additional`
successful pushes do not grow the List buffer.

## 12. Backend Contract

Executable IR represents checked and explicit arithmetic as different
operations. LLVM lowering must:

- emit or prove away every required overflow, zero, shift, conversion, bounds,
  capacity, and address-size check;
- use LLVM overflow-reporting intrinsics or equivalent widened arithmetic for
  checked addition, subtraction, and multiplication;
- guard zero and signed `MIN / -1` before LLVM division, and return zero for
  signed `MIN % -1` without executing LLVM remainder;
- validate an `Int` shift amount before converting it to the left operand's LLVM
  integer type and executing a shift instruction;
- branch to a Keld fault routine before executing a target operation with
  invalid inputs;
- use LLVM `nsw`, `nuw`, `inbounds`, and equivalent assumptions only when Keld
  analysis proves their preconditions; and
- emit no fast-math flag that weakens Keld 0.1 behavior, using constrained
  floating-point operations when the target environment otherwise cannot
  preserve it; and
- preserve the same fault ordering as left-to-right Keld evaluation.

An optimized build may eliminate a check only when its failure is impossible on
every path reaching the operation.

## 13. Required Verification

The compiler and interpreter must test:

- every minimum and maximum integer boundary;
- constant and runtime overflow for every integer width;
- zero division and remainder;
- signed `MIN / -1` and `MIN % -1`;
- negative, equal-width, and oversized shifts;
- checked, saturating, and wrapping variants;
- narrowing and float conversion boundaries;
- negative and upper-bound indices;
- capacity-growth and element-size multiplication overflow;
- deterministic injected allocation failures, including minimum-capacity retry;
- 64-bit source sizes lowered to 32-bit address spaces; and
- interpreter versus optimized native and WebAssembly behavior.
