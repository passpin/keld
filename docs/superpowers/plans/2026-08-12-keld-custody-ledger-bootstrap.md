# Keld Custody Ledger Bootstrap Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the first executable Keld compiler milestone: parse one source
file, type-check the bootstrap subset, prove Custody Ledger entity and lifecycle
rules, lower to validated executable IR, and check or run it through the
interpreter CLI.

**Architecture:** Use a dependency-acyclic Rust workspace. Syntax produces a
lossless normalized token tree and milestone AST; semantics owns names and
types; flow owns evaluation order and CFG identity; lifecycle verification owns
entity proofs and cleanup planning; executable IR owns concrete operations;
runtime owns slots and lifecycle state; the interpreter is the semantic oracle.
Shared source/diagnostic and integer-operation crates are leaf dependencies so
constant evaluation and runtime execution cannot drift.

**Tech Stack:** Rust 1.97.0, Cargo workspace resolver 3, Rust edition 2024,
standard library only, hand-written lexer and event parser, deterministic unit,
compile-pass, compile-fail, model, and CLI tests.

## Global Constraints

- Pin `rustc` and Cargo to `1.97.0`; the version is installed and verified in
  the implementation environment.
- Forbid `unsafe` in every bootstrap crate.
- Use no third-party crates in this milestone. Add a dependency only after a
  separate design and benchmark justify it.
- Target Windows x86-64 development first, but keep all compiler and interpreter
  semantics platform-independent.
- Parse the full Keld 0.1 grammar. Semantically accept only the first buildable
  milestone and emit `KLD0004` for parsed post-milestone constructs.
- Accept `Int`, `Bool`, structs, entities, links, optionals required by links,
  immutable `let`, functions, `if`, `when`, lifecycles, `keep`, `retire`, field
  access, direct entity-field mutation, calls, and return. Struct values and
  local bindings are immutable in this milestone.
- Omit `keld-storage` in this milestone because every accepted value is
  implicit-copy and `take` is rejected. Add that stage with List/Text rather
  than creating a pass with no storage decisions.
- Reject imports, enums, `var`, loops, `break`, `continue`, `match`, recursion,
  Text, List, `take`, generics, typed errors, unsafe, FFI, and packages with a
  focused unsupported-feature diagnostic.
- The bootstrap entrypoint is exactly `fn main() -> Int`. Successful `run`
  writes the returned decimal Int plus one newline and exits zero. Static
  rejection exits one, runtime fault exits two, and command misuse exits 64.
- Default Int arithmetic is checked in every build. Preserve the specified
  behavior for overflow, zero division, `Int.MIN / -1`, `Int.MIN % -1`, and
  invalid shifts.
- Entity allocation, retirement, movement, and lifecycle destruction are
  structural operations. No executable-IR `View` may cross one.
- Every task uses red-green TDD, runs its focused tests, runs all affected crate
  tests, and makes one reviewable commit.

---

## File and Crate Ownership Map

| Path | Sole responsibility |
|---|---|
| `crates/keld-source/` | normalized source text, spans, source map, diagnostics |
| `crates/keld-numeric/` | normative Int literal and checked-operation behavior |
| `crates/keld-syntax/` | lexer, virtual terminators, lossless tree, AST wrappers |
| `crates/keld-semantics/` | feature gate, declarations, names, types, typed HIR |
| `crates/keld-flow/` | explicit evaluation order, CFG, locals, provenance sites |
| `crates/keld-lifecycle/` | entity proof dataflow, effects, cleanup planning |
| `crates/keld-ir/` | executable operations, validator, stable text dump |
| `crates/keld-runtime/` | store brand, slots, generations, lifecycle membership |
| `crates/keld-interpreter/` | executable-IR call stack and value execution |
| `crates/keld-cli/` | command parsing, pipeline orchestration, rendered output |
| `crates/keld-cli/tests/fixtures/` | end-to-end pass/fail Keld programs |
| `docs/spec/` | normative language behavior; implementation never overrides it |

The dependency direction is:

```text
keld-source   keld-numeric
      \        /       \
      keld-syntax       \
           \             \
          keld-semantics  \
                \         \
               keld-flow   \
                    \       \
               keld-lifecycle
                       \
                      keld-ir     keld-runtime
                           \       /
                         keld-interpreter
                                  |
                               keld-cli
```

Every crate manifest uses this package/lint shape, substituting only the crate
name and the path dependencies in the following table:

```toml
[package]
name = "keld-source"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[lints]
workspace = true
```

| Crate | Regular path dependencies |
|---|---|
| `keld-source` | none |
| `keld-numeric` | none |
| `keld-syntax` | `keld-source` |
| `keld-semantics` | `keld-source`, `keld-numeric`, `keld-syntax` |
| `keld-flow` | `keld-source`, `keld-numeric`, `keld-semantics` |
| `keld-lifecycle` | `keld-source`, `keld-semantics`, `keld-flow` |
| `keld-ir` | `keld-source`, `keld-numeric`, `keld-semantics`, `keld-flow`, `keld-lifecycle` |
| `keld-runtime` | none |
| `keld-interpreter` | `keld-source`, `keld-numeric`, `keld-semantics`, `keld-ir`, `keld-runtime` |
| `keld-cli` | pipeline crates except numeric/runtime, owned by interpreter |

Use `{ path = "../crate-name" }` for every local dependency. Do not expose a
dependency only to reuse an unrelated helper; move only truly shared semantic
types to their owning leaf crate.

## Stable Diagnostic Allocation

| Range | Owner |
|---|---|
| `KLD0001`-`KLD0009` | source, lexer, parser, unsupported feature gate |
| `KLD0101`-`KLD0199` | name and type semantics |
| `KLD1001`-`KLD1009` | normative entity and lifecycle verification |
| `KLD9001`-`KLD9099` | invalid executable IR and internal compiler invariants |

### Task 1: Workspace, Source Text, Spans, and Diagnostics

**Files:**

- Create: `Cargo.toml`
- Create: `rust-toolchain.toml`
- Create: `.gitignore`
- Create: `crates/keld-source/Cargo.toml`
- Create: `crates/keld-source/src/lib.rs`
- Create: `crates/keld-source/src/source.rs`
- Create: `crates/keld-source/src/span.rs`
- Create: `crates/keld-source/src/diagnostic.rs`
- Create: `crates/keld-source/tests/source_and_diagnostics.rs`

**Interfaces:**

- Produces: `SourceId`, `Span`, `SourceText`, `SourceMap`, `DiagnosticCode`,
  `Severity`, `Label`, `Diagnostic`, `sort_diagnostics`.
- `SourceText::from_bytes(SourceId, Vec<u8>) -> Result<SourceText, Diagnostic>`
  validates UTF-8, removes one leading UTF-8 BOM, and normalizes CRLF to LF.
- `SourceText::from_str(SourceId, &str) -> Result<SourceText, Diagnostic>` applies
  the same BOM, size check, and CRLF normalization to already valid UTF-8 test
  and embedding input.
- `SourceMap::add(PathBuf, Vec<u8>) -> Result<SourceId, Diagnostic>` assigns with
  checked ID increment and publishes the source only after validation succeeds.
- `SourceText::line_col(BytePos) -> Option<(u32, u32)>` returns one-based
  coordinates for an in-range UTF-8 boundary; columns count Unicode scalar
  values, with each tab counting as one column.
- All later crates depend on `keld-source`; it depends only on `std`.

- [ ] **Step 1: Write the failing source and diagnostic tests**

```rust
use keld_source::{sort_diagnostics, Diagnostic, DiagnosticCode, SourceId,
                  SourceText, Span};

#[test]
fn source_normalizes_bom_and_crlf() {
    let source = SourceText::from_bytes(SourceId(7),
        b"\xEF\xBB\xBFlet x = 1\r\nreturn x\r\n".to_vec()).unwrap();
    assert_eq!(source.text(), "let x = 1\nreturn x\n");
    let span = Span::new(SourceId(7), 10, 11).unwrap();
    assert_eq!(source.line_col(span.start()), Some((2, 1)));
}

#[test]
fn diagnostics_sort_by_source_then_span_then_code() {
    let mut items = vec![
        Diagnostic::error(DiagnosticCode("KLD0102"),
                          Span::new(SourceId(0), 9, 10).unwrap(), "b"),
        Diagnostic::error(DiagnosticCode("KLD0101"),
                          Span::new(SourceId(0), 2, 3).unwrap(), "a"),
    ];
    sort_diagnostics(&mut items);
    assert_eq!(items[0].code.0, "KLD0101");
}
```

- [ ] **Step 2: Run the focused test and verify the red state**

Run: `cargo test -p keld-source --test source_and_diagnostics`

Expected: FAIL because the workspace and `keld-source` crate do not exist.

- [ ] **Step 3: Create the workspace and source interfaces**

Use this root workspace configuration:

```toml
[workspace]
resolver = "3"
members = ["crates/*"]

[workspace.package]
edition = "2024"
rust-version = "1.97"
license = "MIT OR Apache-2.0"

[workspace.lints.rust]
unsafe_code = "forbid"

[workspace.lints.clippy]
all = "warn"
pedantic = "warn"
```

Use this toolchain pin and ignore file:

```toml
[toolchain]
channel = "1.97.0"
profile = "minimal"
components = ["rustfmt", "clippy"]
```

```gitignore
/target/
```

Use these public core types:

```rust
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourceId(pub u32);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BytePos(pub u32);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Span { source: SourceId, start: BytePos, end: BytePos }

impl Span {
    pub const fn new(source: SourceId, start: u32, end: u32) -> Option<Self>;
    pub const fn source(self) -> SourceId;
    pub const fn start(self) -> BytePos;
    pub const fn end(self) -> BytePos;
    pub fn cover(self, other: Self) -> Option<Self>;
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct DiagnosticCode(pub &'static str);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Severity { Error, Warning }

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Label { pub span: Span, pub message: String }

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    pub code: DiagnosticCode,
    pub severity: Severity,
    pub primary: Label,
    pub secondary: Vec<Label>,
    pub help: Option<String>,
}
```

`SourceText` stores normalized `Arc<str>` plus line-start byte offsets.
`SourceMap` assigns monotonically increasing `SourceId` values and never uses a
filesystem path as identity. It retains a separate display path for rendering
and permits distinct source IDs for repeated reads of the same path. Reject
invalid UTF-8 as `KLD0001`. Keep rendering
out of this crate; expose data, line/column lookup, and source slices only.
Reject normalized input larger than `u32::MAX` bytes as `KLD0001` before
constructing byte positions. `Span::new` rejects reversed bounds; `cover`
rejects spans from different sources; `SourceText::slice` and `line_col` reject
out-of-range positions and non-UTF-8 boundaries, and `slice` also rejects a
different source ID.

Add `crates/keld-source/` to the architecture document's repository map.

- [ ] **Step 4: Run formatting, lints, and tests**

Run: `cargo fmt --all --check`

Run: `cargo clippy -p keld-source --all-targets -- -D warnings`

Run: `cargo test -p keld-source`

Expected: all commands exit zero; both integration tests pass.

- [ ] **Step 5: Commit the source foundation**

```powershell
git add Cargo.toml rust-toolchain.toml .gitignore crates/keld-source
git commit -m "build: bootstrap Keld source infrastructure"
```

### Task 2: Shared Checked Int Semantics

**Files:**

- Create: `crates/keld-numeric/Cargo.toml`
- Create: `crates/keld-numeric/src/lib.rs`
- Create: `crates/keld-numeric/src/literal.rs`
- Create: `crates/keld-numeric/src/int.rs`
- Create: `crates/keld-numeric/tests/int_boundaries.rs`

**Interfaces:**

- Produces: `ParsedIntLiteral`, `IntUnaryOp`, `IntBinaryOp`, `NumericFault`,
  `parse_int_literal`, `eval_unary`, and `eval_binary`.
- `keld-semantics` uses the literal and evaluation API for constant folding.
- `keld-interpreter` uses the same evaluation API for runtime instructions.

- [ ] **Step 1: Write exhaustive boundary tests before the crate exists**

```rust
use keld_numeric::{eval_binary, parse_int_literal, IntBinaryOp, NumericFault,
                   ParsedIntLiteral};

#[test]
fn recognizes_the_int_min_magnitude_without_accepting_it_as_positive() {
    assert_eq!(parse_int_literal("9_223_372_036_854_775_808"),
               ParsedIntLiteral::IntMinMagnitude);
    assert_eq!(parse_int_literal("9223372036854775809"),
               ParsedIntLiteral::OutOfRange);
}

#[test]
fn signed_division_edges_match_the_spec() {
    assert_eq!(eval_binary(IntBinaryOp::Div, 7, 0),
               Err(NumericFault::DivisionByZero));
    assert_eq!(eval_binary(IntBinaryOp::Div, i64::MIN, -1),
               Err(NumericFault::Arithmetic));
    assert_eq!(eval_binary(IntBinaryOp::Rem, i64::MIN, -1), Ok(0));
}

#[test]
fn shifts_validate_amount_before_computing() {
    assert_eq!(eval_binary(IntBinaryOp::Shl, 1, 64), Err(NumericFault::Shift));
    assert_eq!(eval_binary(IntBinaryOp::Shr, 1, -1), Err(NumericFault::Shift));
    assert_eq!(eval_binary(IntBinaryOp::Shl, -1, 1), Ok(-2));
}
```

- [ ] **Step 2: Run the tests and verify the missing-crate failure**

Run: `cargo test -p keld-numeric --test int_boundaries`

Expected: FAIL because package `keld-numeric` does not exist.

- [ ] **Step 3: Implement one normative Int engine**

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParsedIntLiteral { Value(i64), IntMinMagnitude, OutOfRange }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntUnaryOp { Neg }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntBinaryOp { Add, Sub, Mul, Div, Rem, Shl, Shr }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NumericFault { Arithmetic, DivisionByZero, Shift }

pub fn parse_int_literal(raw: &str) -> ParsedIntLiteral;
pub fn eval_unary(op: IntUnaryOp, value: i64) -> Result<i64, NumericFault>;
pub fn eval_binary(op: IntBinaryOp, lhs: i64, rhs: i64)
    -> Result<i64, NumericFault>;
```

Parse decimal digits incrementally after removing only valid between-digit
underscores. Stop accumulating after the magnitude exceeds `2^63`; no bigint
dependency is needed to decide acceptance for the Int-only milestone. Implement
add/sub/mul/neg with `checked_*`. Check division by zero before the signed minimum
case. Special-case `MIN % -1` to zero. Validate shifts against `0..64`, implement
signed left shift as `i128::from(lhs) * (1_i128 << amount)` followed by an i64
range check, and use arithmetic right shift. The wider intermediate is required
for defined cases such as `0 << 63` and `-1 << 63`.

- [ ] **Step 4: Add a table test over every operator boundary and run checks**

The table must include `MIN`, `MIN + 1`, `-1`, `0`, `1`, `MAX - 1`, and `MAX`
against each operator's safe and faulting cases. For shift operations test
amounts `-1`, `0`, `1`, `62`, `63`, and `64`.

```rust
let cases = [
    (IntBinaryOp::Add, i64::MAX, 1, Err(NumericFault::Arithmetic)),
    (IntBinaryOp::Add, i64::MIN, 1, Ok(i64::MIN + 1)),
    (IntBinaryOp::Sub, i64::MIN, 1, Err(NumericFault::Arithmetic)),
    (IntBinaryOp::Sub, i64::MAX, 1, Ok(i64::MAX - 1)),
    (IntBinaryOp::Mul, i64::MAX, 0, Ok(0)),
    (IntBinaryOp::Mul, i64::MIN, -1, Err(NumericFault::Arithmetic)),
    (IntBinaryOp::Div, -1, 1, Ok(-1)),
    (IntBinaryOp::Rem, i64::MIN, -1, Ok(0)),
    (IntBinaryOp::Shl, 1, 62, Ok(1_i64 << 62)),
    (IntBinaryOp::Shl, 1, 63, Err(NumericFault::Arithmetic)),
    (IntBinaryOp::Shl, 0, 63, Ok(0)),
    (IntBinaryOp::Shl, -1, 63, Ok(i64::MIN)),
    (IntBinaryOp::Shr, i64::MIN, 63, Ok(-1)),
    (IntBinaryOp::Shr, 1, 64, Err(NumericFault::Shift)),
];
for (op, lhs, rhs, expected) in cases {
    assert_eq!(eval_binary(op, lhs, rhs), expected, "{op:?} {lhs} {rhs}");
}
```

Run: `cargo fmt --all --check`

Run: `cargo clippy -p keld-numeric --all-targets -- -D warnings`

Run: `cargo test -p keld-numeric`

Expected: all commands exit zero.

- [ ] **Step 5: Commit the shared numeric semantics**

```powershell
git add crates/keld-numeric
git commit -m "feat: define checked Keld Int semantics"
```

### Task 3: Lossless Lexer and Virtual Terminators

**Files:**

- Create: `crates/keld-syntax/Cargo.toml`
- Create: `crates/keld-syntax/src/lib.rs`
- Create: `crates/keld-syntax/src/token.rs`
- Create: `crates/keld-syntax/src/lexer.rs`
- Create: `crates/keld-syntax/tests/lexer.rs`

**Interfaces:**

- Consumes: `keld_source::{Diagnostic, SourceText, Span}`.
- Produces: `Keyword`, `Punct`, `TokenKind`, `Token`, `TokenId`, `Lexed`, and
  `lex(&SourceText) -> Lexed`.
- Every original normalized byte belongs to exactly one non-synthetic token.
  Synthetic `Term` tokens have empty spans and `synthetic == true`.

- [ ] **Step 1: Write lexer tests for losslessness and terminators**

```rust
use keld_source::{SourceId, SourceText};
use keld_syntax::{lex, TokenKind};

#[test]
fn preserves_trivia_and_inserts_terms() {
    let source = SourceText::from_str(SourceId(0),
        "let x = (1 +\n 2) // sum\nif true {\n x\n}\n").unwrap();
    let lexed = lex(&source);
    assert!(lexed.diagnostics.is_empty());
    assert_eq!(lexed.reconstruct(&source), source.text());
    let terms = lexed.tokens.iter()
        .filter(|token| token.kind == TokenKind::Term)
        .count();
    assert_eq!(terms, 3);
}

#[test]
fn longest_match_wins_for_shift_and_comparison_tokens() {
    let source = SourceText::from_str(SourceId(0), "a << 1 <= b >> 2 >= c\n").unwrap();
    let kinds = lex(&source).significant_kinds();
    assert!(kinds.contains(&TokenKind::Punct(keld_syntax::Punct::Shl)));
    assert!(kinds.contains(&TokenKind::Punct(keld_syntax::Punct::LessEq)));
}
```

- [ ] **Step 2: Run the lexer tests and verify the red state**

Run: `cargo test -p keld-syntax --test lexer`

Expected: FAIL because `keld-syntax` does not exist.

- [ ] **Step 3: Implement tokens and the scanner**

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TokenKind {
    Ident, Int, String, Keyword(Keyword), Punct(Punct), Term,
    Whitespace, LineComment, BlockComment, Error, Eof,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Punct {
    LParen, RParen, LBrace, RBrace, LBracket, RBracket,
    Comma, Dot, Colon, Question, Semicolon, Underscore,
    Eq, FatArrow, Arrow, Plus, Minus, Star, Slash, Percent,
    PlusEq, MinusEq, StarEq, SlashEq, PercentEq,
    Bang, BangEq, EqEq, Less, LessEq, Greater, GreaterEq,
    Shl, Shr, AndAnd, OrOr,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TokenId(pub u32);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
    pub synthetic: bool,
}

pub struct Lexed {
    pub tokens: Vec<Token>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn lex(source: &SourceText) -> Lexed;
```

Define all keywords from `grammar.md`, including reserved post-0.1 words. Define
punctuation for every EBNF terminal. Track parenthesis and bracket depth, not
brace depth, for qualifying newlines. Insert a synthetic `Term` before `}` and
at EOF only when the prior significant token can end a statement and no Term is
already present. Diagnose malformed integer underscores, bad string escapes,
unterminated strings, unterminated block comments, and unknown characters with
`KLD0002`.

- [ ] **Step 4: Add the complete lexer matrix and run checks**

Add one data-driven test case for every keyword, punctuation token, required
escape, legal identifier form, lone `_` wildcard use, comment form, and each Term
insertion/suppression rule from `grammar.md` Sections 2-6.

```rust
const KEYWORDS: &[&str] = &[
    "any", "as", "break", "continue", "else", "entity", "enum", "extern",
    "false", "fn", "handle", "if", "in", "keep", "let", "lifecycle",
    "link", "match", "module", "none", "pub", "raises", "retire",
    "retires", "return", "struct", "take", "true", "try", "unsafe", "use",
    "var", "when", "while", "async", "await", "dynamic", "impl",
    "interface", "resource", "shared",
];
for word in KEYWORDS {
    let source = SourceText::from_str(SourceId(0), word).unwrap();
    assert!(matches!(lex(&source).significant_kinds()[0], TokenKind::Keyword(_)),
            "{word}");
}

let term_cases = [
    ("let x = 1\nlet y = 2\n", 2),
    ("let x = (1 +\n2)\n", 1),
    ("let x = 1 +\n2\n", 1),
    ("if true {\n1\n}\n", 2),
    ("return\n", 1),
];
for (text, expected) in term_cases {
    let source = SourceText::from_str(SourceId(0), text).unwrap();
    let actual = lex(&source).tokens.iter()
        .filter(|token| token.kind == TokenKind::Term).count();
    assert_eq!(actual, expected, "{text:?}");
}
```

Run: `cargo fmt --all --check`

Run: `cargo clippy -p keld-syntax --all-targets -- -D warnings`

Run: `cargo test -p keld-syntax --test lexer`

Expected: all commands exit zero.

- [ ] **Step 5: Commit the lexer**

```powershell
git add crates/keld-syntax
git commit -m "feat: add lossless Keld lexer"
```

### Task 4: Full Grammar Parser and Milestone AST

**Files:**

- Create: `crates/keld-syntax/src/kind.rs`
- Create: `crates/keld-syntax/src/green.rs`
- Create: `crates/keld-syntax/src/parser/mod.rs`
- Create: `crates/keld-syntax/src/parser/item.rs`
- Create: `crates/keld-syntax/src/parser/statement.rs`
- Create: `crates/keld-syntax/src/parser/expression.rs`
- Create: `crates/keld-syntax/src/ast.rs`
- Modify: `crates/keld-syntax/src/lib.rs`
- Create: `crates/keld-syntax/tests/parser_golden.rs`
- Create: `crates/keld-syntax/tests/parser_recovery.rs`

**Interfaces:**

- Consumes: `Lexed` from Task 3.
- Produces: immutable `SyntaxNode`, `SyntaxElement`, `SyntaxKind`, `ParsedFile`,
  and typed AST wrapper accessors in `keld_syntax::ast`.
- `parse(Lexed) -> ParsedFile` never panics on user input and retains every
  token exactly once in source order.
- `ParsedFile::ast() -> ast::SourceFile` is the only syntax interface consumed
  by `keld-semantics`.

- [ ] **Step 1: Write precedence, grammar, and recovery tests**

```rust
use keld_source::{SourceId, SourceText};
use keld_syntax::{lex, parse, SyntaxKind};

#[test]
fn parses_shift_below_addition_and_above_comparison() {
    let source = SourceText::from_str(SourceId(0),
        "fn main() -> Int { return 1 + 2 << 3 < 40 }\n").unwrap();
    let parsed = parse(lex(&source));
    assert!(parsed.diagnostics.is_empty());
    assert_eq!(parsed.root.descendant_kinds().filter(|k| **k == SyntaxKind::ShiftExpr).count(), 1);
    assert_eq!(parsed.root.descendant_kinds()
        .filter(|kind| **kind == SyntaxKind::ComparisonExpr).count(), 1);
}

#[test]
fn recovers_at_term_and_keeps_the_following_function() {
    let source = SourceText::from_str(SourceId(0),
        "fn broken( { return 0 }\nfn main() -> Int { return 1 }\n").unwrap();
    let parsed = parse(lex(&source));
    assert!(!parsed.diagnostics.is_empty());
    assert_eq!(parsed.ast().functions().count(), 2);
}
```

- [ ] **Step 2: Run both parser tests and verify the red state**

Run: `cargo test -p keld-syntax --test parser_golden --test parser_recovery`

Expected: FAIL because parser modules and syntax tree types do not exist.

- [ ] **Step 3: Implement the immutable token-backed syntax tree**

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyntaxKind {
    SourceFile, ModuleDecl, UseDecl, StructDecl, EntityDecl, EnumDecl,
    FunctionDecl, ExternFunctionDecl, FieldDecl, VariantDecl, Parameter,
    Type, Block, BindingStmt, AssignmentStmt, KeepStmt, RetireStmt,
    ReturnStmt, BreakStmt, ContinueStmt, ExprStmt, LifecycleStmt, WhenStmt,
    IfStmt, WhileStmt, TryStmt, HandleClause, LogicalOrExpr,
    LogicalAndExpr, EqualityExpr, ComparisonExpr, ShiftExpr, AdditiveExpr,
    MultiplicativeExpr, UnaryExpr, TakeExpr, CallExpr, FieldExpr, IndexExpr,
    MatchExpr, MatchArm, Pattern, LiteralExpr, NameExpr, Error,
}

#[derive(Clone, Debug)]
pub enum SyntaxElement { Node(SyntaxNode), Token(TokenId) }

#[derive(Clone, Debug)]
pub struct SyntaxNode {
    pub kind: SyntaxKind,
    pub span: Span,
    pub children: Vec<SyntaxElement>,
}

pub struct ParsedFile {
    pub lexed: Lexed,
    pub root: SyntaxNode,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn parse(lexed: Lexed) -> ParsedFile;
```

Use an event parser with `Start(kind)`, `Token(TokenId)`, `Finish`, and
`Error(Diagnostic)` events, then build the immutable tree once. Trivia tokens
are consumed into the current node. Use Pratt binding powers matching the
normative precedence table. Comparison and equality productions accept at most
one operator. Recover top-level forms at `Term`, item keywords, or EOF; recover
statements at `Term` or `}`; recover delimited lists at comma or their closing
delimiter. Emit `KLD0003` with the unexpected token and expected token set.
Track recursive block, parenthesized-expression, unary, pattern, and type depth;
at depth 257 emit `KLD0003`, consume to the current recovery boundary, and never
recurse further. Add cases at depths 256 and 257 so arbitrary input cannot cause
a parser stack overflow.

- [ ] **Step 4: Implement AST wrappers without a second mutable tree**

Provide wrappers for every EBNF item, statement, expression, type, pattern, and
clause. Wrappers contain `&SyntaxNode` and expose iterators or optional children;
they do not copy source strings. At minimum, later crates require these exact
entry points:

```rust
impl<'a> ast::SourceFile<'a> {
    pub fn module(self) -> Option<ast::ModuleDecl<'a>>;
    pub fn uses(self) -> impl Iterator<Item = ast::UseDecl<'a>>;
    pub fn items(self) -> impl Iterator<Item = ast::Item<'a>>;
}

impl<'a> ast::FunctionDecl<'a> {
    pub fn name(self) -> ast::Name<'a>;
    pub fn parameters(self) -> impl Iterator<Item = ast::Parameter<'a>>;
    pub fn return_type(self) -> Option<ast::TypeRef<'a>>;
    pub fn retirement_targets(self) -> impl Iterator<Item = ast::RetirementTarget<'a>>;
    pub fn body(self) -> ast::Block<'a>;
}
```

- [ ] **Step 5: Generate one golden case per EBNF production and run checks**

Each case asserts no diagnostics, the root child-kind sequence, and
`ParsedFile::reconstruct() == SourceText::text()`. Add negative cases for
comparison chaining, mixed positional/named arguments, missing delimiters, and
`else`/`handle` after a virtual Term.

```rust
let accepted = [
    "module game.world\nfn main() -> Int { return 0 }\n",
    "struct Point {\nx: Int\ny: Int\n}\nfn main() -> Int { return 0 }\n",
    "entity Enemy {\ntarget: link Enemy?\n}\nfn main() -> Int { return 0 }\n",
    "fn remove(e: Enemy) -> Int retires e { retire e; return 0 }\n",
    "fn sweep(e: Enemy) -> Int retires any Enemy { retire e; return 0 }\n",
    "fn main() -> Int { if true { return 1 } else { return 0 } }\n",
    "fn main() -> Int { match 1 {\n_ => 0\n}\n}\n",
];
for text in accepted {
    let source = SourceText::from_str(SourceId(0), text).unwrap();
    let parsed = parse(lex(&source));
    assert!(parsed.diagnostics.is_empty(), "{text}\n{:?}", parsed.diagnostics);
    assert_eq!(parsed.reconstruct(&source), source.text());
}
```

Run: `cargo fmt --all --check`

Run: `cargo clippy -p keld-syntax --all-targets -- -D warnings`

Run: `cargo test -p keld-syntax`

Expected: all commands exit zero.

- [ ] **Step 6: Commit the full grammar parser**

```powershell
git add crates/keld-syntax
git commit -m "feat: parse the Keld core grammar"
```

### Task 5: Feature Gate, Names, Types, and Typed HIR

**Files:**

- Create: `crates/keld-semantics/Cargo.toml`
- Create: `crates/keld-semantics/src/lib.rs`
- Create: `crates/keld-semantics/src/ids.rs`
- Create: `crates/keld-semantics/src/types.rs`
- Create: `crates/keld-semantics/src/symbols.rs`
- Create: `crates/keld-semantics/src/features.rs`
- Create: `crates/keld-semantics/src/hir.rs`
- Create: `crates/keld-semantics/src/analyze.rs`
- Create: `crates/keld-semantics/tests/feature_gate.rs`
- Create: `crates/keld-semantics/tests/types.rs`
- Create: `crates/keld-semantics/tests/entrypoint.rs`

**Interfaces:**

- Consumes: `keld_syntax::ParsedFile`, `keld_numeric` literal semantics, and
  source diagnostics.
- Produces: stable IDs, `TypeStore`, `TypedModule`, typed HIR, `FunctionEffects`,
  and `Analysis { module: Option<TypedModule>, diagnostics }`.
- Produces no CFG and performs no lifecycle-state proof.

- [ ] **Step 1: Write tests for supported types and rejected parsed features**

```rust
use keld_semantics::analyze_text;

#[test]
fn resolves_entity_names_as_direct_refs_and_link_fields_as_links() {
    let analysis = analyze_text(r#"
        entity Enemy {
            health: Int
            target: link Enemy?
        }
        fn main() -> Int {
            lifecycle level {
                let enemy = Enemy(health: 7, target: none)
                return enemy.health
            }
        }
    "#);
    assert!(analysis.diagnostics.is_empty());
}

#[test]
fn parsed_but_deferred_var_has_a_focused_diagnostic() {
    let analysis = analyze_text("fn main() -> Int { var x = 1; return x }\n");
    assert_eq!(analysis.diagnostics[0].code.0, "KLD0004");
    assert!(analysis.diagnostics[0].primary.message.contains("`var`"));
}
```

- [ ] **Step 2: Run semantic tests and verify the missing-crate failure**

Run: `cargo test -p keld-semantics --test feature_gate --test types --test entrypoint`

Expected: FAIL because `keld-semantics` does not exist.

- [ ] **Step 3: Define stable semantic IDs and types**

```rust
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)] pub struct DefId(pub u32);
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)] pub struct FieldId(pub u32);
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)] pub struct FunctionId(pub u32);
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)] pub struct LocalId(pub u32);
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)] pub struct ParameterIndex(pub u32);
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)] pub struct TypeId(pub u32);
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)] pub struct HirLifecycleId(pub u32);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TypeKind {
    Unit,
    Bool,
    Int,
    Struct(DefId),
    EntityRef(DefId),
    Link { entity: DefId, optional: bool },
    Optional(TypeId),
    Error,
}

pub struct FunctionEffects {
    pub retires: Vec<LocalId>,
    pub retires_any: Vec<DefId>,
}
```

Intern types in `TypeStore` for stable equality. Collect all struct, entity, and
function signatures before checking bodies. Reject duplicate declarations,
unknown names, duplicate fields, wrong constructor arguments, field mistakes,
type mismatches, and non-Bool conditions with codes `KLD0101` through `KLD0107`.
Mark a direct `EntityRef` in a persistent struct or entity field for the
lifecycle verifier; it emits normative `KLD1002` in Task 7. Do not erase the
field or issue a competing type diagnostic. Build the by-value struct dependency graph and reject
every self or mutual cycle, or a longest by-value path over 256 structs, as
`KLD0110`; links break layout cycles. This cap also bounds recursive value drop
depth in the interpreter.
Assign `DefId`, `FunctionId`, and `FieldId` in normalized source declaration
order. Lookup maps never determine IDs, diagnostic ordering, or serialized
output.

- [ ] **Step 4: Implement the milestone feature gate and entry contract**

Walk the parsed AST before body typing. Emit one primary `KLD0004` per outermost
unsupported construct and do not cascade into its children. Check imports,
enums, `var`, loops, `break`, `continue`, `match`, generics, `take`, string/Text,
List, `raises`, `try`, `unsafe`, and `extern`.

After call resolution, detect strongly connected components with an iterative
Tarjan or Kosaraju traversal. Emit `KLD0004` for every direct or mutual recursive
cycle. Require one non-public or public function named `main`, with zero
parameters, return type `Int`, and no retirement clauses; diagnose violations as
`KLD0109`.

- [ ] **Step 5: Define typed HIR and check bodies bidirectionally**

```rust
pub struct TypedModule {
    pub definitions: Vec<Definition>,
    pub functions: Vec<HirFunction>,
    pub types: TypeStore,
    pub main: FunctionId,
}

pub enum DefinitionKind { Struct, Entity }

pub struct Definition {
    pub id: DefId,
    pub name: String,
    pub kind: DefinitionKind,
    pub fields: Vec<FieldDefinition>,
}

pub struct FieldDefinition {
    pub id: FieldId,
    pub name: String,
    pub ty: TypeId,
    pub span: Span,
}

pub struct HirFunction {
    pub id: FunctionId,
    pub parameters: Vec<(LocalId, TypeId)>,
    pub return_type: TypeId,
    pub effects: FunctionEffects,
    pub body: HirBlock,
    pub span: Span,
}

pub struct HirBlock { pub statements: Vec<HirStmt>, pub span: Span }
pub struct HirStmt { pub kind: HirStmtKind, pub span: Span }
pub struct HirPlace { pub base: LocalId, pub fields: Vec<FieldId>, pub span: Span }
pub struct HirIf { pub condition: HirExpr, pub then_block: HirBlock,
                   pub else_block: Option<HirBlock> }
pub struct HirWhen { pub link: HirExpr, pub binding: LocalId,
                     pub live: HirBlock, pub absent: Option<HirBlock> }
pub struct HirLifecycle { pub id: HirLifecycleId, pub name: String, pub body: HirBlock }

pub struct HirExpr { pub ty: TypeId, pub span: Span, pub kind: HirExprKind }

pub enum HirUnaryOp { Int(IntUnaryOp), Not }

pub enum CompareOp { Eq, NotEq, Less, LessEq, Greater, GreaterEq }

pub enum HirBinaryOp { Int(IntBinaryOp), And, Or, Compare(CompareOp) }

pub enum HirExprKind {
    Int(i64), Bool(bool), None, Local(LocalId),
    Unary { op: HirUnaryOp, value: Box<HirExpr> },
    Binary { op: HirBinaryOp, lhs: Box<HirExpr>, rhs: Box<HirExpr> },
    Field { base: Box<HirExpr>, field: FieldId },
    Call { function: FunctionId, arguments: Vec<(ParameterIndex, HirExpr)> },
    ConstructStruct { definition: DefId, fields: Vec<(FieldId, HirExpr)> },
    ConstructEntity { definition: DefId, fields: Vec<(FieldId, HirExpr)> },
    EntityToLink(Box<HirExpr>),
}

pub enum HirStmtKind {
    Let { local: LocalId, initializer: HirExpr },
    Assign { target: HirPlace, value: HirExpr },
    CompoundAssign { target: HirPlace, op: IntBinaryOp, value: HirExpr },
    Expr(HirExpr), If(HirIf), When(HirWhen), Lifecycle(HirLifecycle),
    Keep { entity: HirExpr, lifecycle: HirLifecycleId },
    Retire(HirExpr), Return(Option<HirExpr>),
}
```

Preserve source argument and field-initializer order in HIR; retain destination
parameter indices and field IDs independently of evaluation order. Contextually type
`none` and entity-to-link conversion. Use `keld-numeric` for constant Int
evaluation and emit a compile-time diagnostic using the corresponding runtime
fault name as `KLD0120` when a constant would fault. Reject non-Unit expression statements.
Accept `ParsedIntLiteral::IntMinMagnitude` only when unary negation directly
contains that literal after ignoring parenthesis nodes; it then becomes
`i64::MIN`. Diagnose that magnitude in every other context, and every larger
magnitude, as `KLD0121` out of range. Resolve `Int.MIN` and `Int.MAX` as built-in
constants and lower them to the corresponding HIR `Int` value.
Reject optional non-link types in the bootstrap as `KLD0004`; the only accepted
optional storage is `link Entity?`. Equality accepts Int, Bool, and direct entity
identity operands of the same type. Link equality remains unsupported; resolve a
link with `when`. An assignment target must be exactly one field of a direct
entity reference. Reject local assignment, struct-field assignment, indexing,
and deeper field paths as focused `KLD0004` bootstrap exclusions.
For a read such as `world.target.health`, type the result from the link's entity
definition and preserve an `UncheckedLinkField` marker for Task 7; do not emit a
generic type error before normative `KLD1005`.

Because loops are excluded, compute structured definite return directly over
typed blocks. A non-`Unit` function must return on every reachable path:
`if` requires a final `else` and every arm, `when` requires both arms, and a
`lifecycle` adopts its body's result. Emit `KLD0111` at the function return type
when fallthrough remains possible. A `Unit` function receives implicit
fallthrough return.

- [ ] **Step 6: Run semantic validation and commit**

Run: `cargo fmt --all --check`

Run: `cargo clippy -p keld-semantics --all-targets -- -D warnings`

Run: `cargo test -p keld-semantics`

Expected: all commands exit zero, including feature-gate and entrypoint tests.

```powershell
git add crates/keld-semantics
git commit -m "feat: add Keld name and type semantics"
```

### Task 6: Typed Flow IR and Explicit Evaluation Order

**Files:**

- Create: `crates/keld-flow/Cargo.toml`
- Create: `crates/keld-flow/src/lib.rs`
- Create: `crates/keld-flow/src/cfg.rs`
- Create: `crates/keld-flow/src/op.rs`
- Create: `crates/keld-flow/src/lower.rs`
- Create: `crates/keld-flow/src/dump.rs`
- Create: `crates/keld-flow/tests/evaluation_order.rs`
- Create: `crates/keld-flow/tests/control_flow.rs`

**Interfaces:**

- Consumes: `keld_semantics::TypedModule`.
- Produces: `FlowModule`, `FlowFunction`, address-free places, basic blocks,
  operations, terminators, lexical lifecycle IDs, and allocation provenance
  sites.
- `lower(module: &TypedModule) -> FlowModule` is deterministic for identical HIR.

- [ ] **Step 1: Write CFG tests that expose source order and lifecycle exits**

```rust
use keld_flow::{lower_text_for_test, FlowOp, Terminator};

#[test]
fn call_arguments_are_materialized_left_to_right() {
    let flow = lower_text_for_test(r#"
        fn pick(a: Int, b: Int) -> Int { return b }
        fn main() -> Int { return pick(1 + 2, 3 * 4) }
    "#).unwrap();
    let ops = flow.function_named("main").linear_ops();
    assert!(matches!(ops[0], FlowOp::ConstInt { value: 1, .. }));
    assert!(matches!(ops[3], FlowOp::ConstInt { value: 3, .. }));
    assert!(matches!(ops.last().unwrap(), FlowOp::Call { .. }));
}

#[test]
fn return_from_nested_lifecycle_has_an_explicit_exit_edge() {
    let flow = lower_text_for_test(
        "fn main() -> Int { lifecycle level { return 7 } }\n").unwrap();
    let function = flow.function_named("main");
    assert!(function.blocks.iter().any(|b| matches!(b.terminator,
        Terminator::ExitScopes { ref lifecycles, .. } if lifecycles.len() == 1)));
}
```

- [ ] **Step 2: Run flow tests and verify the missing-crate failure**

Run: `cargo test -p keld-flow --test evaluation_order --test control_flow`

Expected: FAIL because `keld-flow` does not exist.

- [ ] **Step 3: Define the address-free CFG interfaces**

```rust
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)] pub struct BlockId(pub u32);
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)] pub struct ValueId(pub u32);
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)] pub struct LifecycleId(pub u32);
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)] pub struct AllocationSite(pub u32);

pub struct FlowModule {
    pub definitions: Vec<Definition>,
    pub types: TypeStore,
    pub functions: Vec<FlowFunction>,
    pub main: FunctionId,
}

pub struct FlowFunction {
    pub id: FunctionId,
    pub parameters: Vec<LocalId>,
    pub local_types: Vec<TypeId>,
    pub return_type: TypeId,
    pub effects: FunctionEffects,
    pub current_lifecycle: LifecycleId,
    pub value_types: Vec<TypeId>,
    pub lifecycle_parents: Vec<Option<LifecycleId>>,
    pub blocks: Vec<FlowBlock>,
    pub entry: BlockId,
}

pub struct FlowBlock {
    pub id: BlockId,
    pub operations: Vec<FlowOp>,
    pub terminator: Terminator,
}

pub enum ExitTarget { Goto(BlockId), Return(Option<ValueId>) }

pub enum FlowOp {
    ConstInt { dst: ValueId, value: i64, span: Span },
    ConstBool { dst: ValueId, value: bool, span: Span },
    ConstNoneLink { dst: ValueId, entity: DefId, span: Span },
    BeginLifecycle { lifecycle: LifecycleId, parent: LifecycleId, span: Span },
    CopyLocal { dst: ValueId, local: LocalId, span: Span },
    StoreLocal { local: LocalId, value: ValueId, span: Span },
    UnaryInt { dst: ValueId, op: IntUnaryOp, value: ValueId, span: Span },
    BinaryInt { dst: ValueId, op: IntBinaryOp, lhs: ValueId, rhs: ValueId, span: Span },
    Not { dst: ValueId, value: ValueId, span: Span },
    Compare { dst: ValueId, op: CompareOp, lhs: ValueId, rhs: ValueId, span: Span },
    Phi { dst: ValueId, inputs: Vec<(BlockId, ValueId)>, span: Span },
    ConstructStruct { dst: ValueId, definition: DefId,
                      fields: Vec<(FieldId, ValueId)>, span: Span },
    AllocateEntity { dst: ValueId, definition: DefId,
                     fields: Vec<(FieldId, ValueId)>,
                     lifecycle: LifecycleId, site: AllocationSite, span: Span },
    EntityToLink { dst: ValueId, entity: ValueId, span: Span },
    ReadStructField { dst: ValueId, base: ValueId, field: FieldId, span: Span },
    ReadEntityField { dst: ValueId, entity: ValueId, field: FieldId, span: Span },
    ReadUncheckedLinkField { dst: ValueId, link: ValueId, field: FieldId, span: Span },
    WriteEntityField { entity: ValueId, field: FieldId, value: ValueId, span: Span },
    Call { dst: Option<ValueId>, function: FunctionId,
           arguments: Vec<(ParameterIndex, ValueId)>,
           current_lifecycle: LifecycleId, span: Span },
    Keep { entity: ValueId, target: LifecycleId, span: Span },
    Retire { entity: ValueId, span: Span },
}

pub enum Terminator {
    Goto(BlockId),
    Branch { condition: ValueId, then_block: BlockId, else_block: BlockId },
    BranchIdentity { lhs: ValueId, rhs: ValueId, equal: BlockId, not_equal: BlockId },
    ResolveLink { link: ValueId, bind_local: LocalId, live: BlockId, absent: BlockId,
                  span: Span },
    ExitScopes { lifecycles: Vec<LifecycleId>, next: ExitTarget },
    Return(Option<ValueId>),
    Unreachable,
}
```

- [ ] **Step 4: Lower HIR without retaining payload addresses**

Assign one `ValueId` per evaluated subexpression. Lower calls, constructors, and
assignments strictly left to right. For assignment, evaluate the target entity
identity, then the complete replacement, then perform `WriteEntityField`.
Compound assignment reads and closes the old copyable field value before
evaluating the right side, applies the checked operation, then writes. A call in
the replacement that may retire the target causes the later write proof to fail.
`ReadEntityField` and `WriteEntityField`
retain entity identity plus `FieldId`, never an address. Plain struct reads use
`ReadStructField` and require no view. Preserve unchecked link field reads as
`ReadUncheckedLinkField`; Task 7 always rejects them as `KLD1005`, so they have
no executable-IR lowering. Represent each
function's hidden incoming current-lifecycle parameter as `LifecycleId(0)`; do
not create or destroy a lifecycle at function entry. Assign distinct lexical IDs
to explicit lifecycle blocks and emit `BeginLifecycle` at each block entry.
Route every return and fallthrough through
`ExitScopes`, with innermost lifecycle first. Preserve `a == b` and `a != b` as
`BranchIdentity` so lifecycle verification can refine alias facts.

Lower `&&` and `||` to short-circuit blocks and one `Phi` in the merge block;
never evaluate both operands unconditionally. Lower `!` to `Not`. Lower Int ordering,
Int/Bool equality, and entity identity equality to `Compare`; when entity
identity comparison directly controls an `if`, use `BranchIdentity` instead.
Constructor field pairs remain in source evaluation order; `FieldId` carries
layout identity independently of that order.

- [ ] **Step 5: Add a stable textual dump and run checks**

The dump prints functions by `FunctionId`, blocks by `BlockId`, then operations
and one terminator. Do not print pointer addresses, hash iteration order, or Rust
debug representations.

Run: `cargo fmt --all --check`

Run: `cargo clippy -p keld-flow --all-targets -- -D warnings`

Run: `cargo test -p keld-flow`

Expected: all commands exit zero.

- [ ] **Step 6: Commit typed flow lowering**

```powershell
git add crates/keld-flow
git commit -m "feat: lower Keld into typed flow IR"
```

### Task 7: Custody Ledger Proof Dataflow and Function Effects

**Files:**

- Create: `crates/keld-lifecycle/Cargo.toml`
- Create: `crates/keld-lifecycle/src/lib.rs`
- Create: `crates/keld-lifecycle/src/state.rs`
- Create: `crates/keld-lifecycle/src/provenance.rs`
- Create: `crates/keld-lifecycle/src/effects.rs`
- Create: `crates/keld-lifecycle/src/verify.rs`
- Create: `crates/keld-lifecycle/src/diagnostics.rs`
- Create: `crates/keld-lifecycle/tests/retirement.rs`
- Create: `crates/keld-lifecycle/tests/alias_refinement.rs`
- Create: `crates/keld-lifecycle/tests/lifecycle_order.rs`
- Create: `crates/keld-lifecycle/tests/effects.rs`
- Create: `crates/keld-lifecycle/tests/links_and_escape.rs`

**Interfaces:**

- Consumes: `keld_flow::FlowModule` and semantic retirement declarations.
- Produces: `VerifiedFlowModule`, `FunctionSummary`, `ProofId`, and one proof
  annotation for every entity-using operation.
- Emits only `KLD1001` through `KLD1009` for normative lifecycle failures.
- Does not construct executable runtime operations.

- [ ] **Step 1: Write compile-pass and compile-fail proof tests**

```rust
use keld_lifecycle::verify_text_for_test;

#[test]
fn retiring_a_may_alias_invalidates_the_other_parameter() {
    let result = verify_text_for_test(r#"
        entity Enemy { health: Int }
        fn bad(a: Enemy, b: Enemy) -> Int retires a {
            retire a
            return b.health
        }
        fn main() -> Int { return 0 }
    "#);
    assert_eq!(result.diagnostics[0].code.0, "KLD1008");
}

#[test]
fn identity_inequality_refines_the_valid_branch() {
    let result = verify_text_for_test(r#"
        entity Enemy { health: Int }
        fn ok(a: Enemy, b: Enemy) -> Int retires a {
            if a != b {
                retire a
                return b.health
            } else {
                retire a
                return 0
            }
        }
        fn main() -> Int { return 0 }
    "#);
    assert!(result.diagnostics.is_empty());
}

#[test]
fn persistent_direct_reference_and_unchecked_link_read_use_normative_codes() {
    let escaped = verify_text_for_test(r#"
        entity Enemy { health: Int }
        entity Bad { target: Enemy }
        fn main() -> Int { return 0 }
    "#);
    assert_eq!(escaped.diagnostics[0].code.0, "KLD1002");

    let unchecked = verify_text_for_test(r#"
        entity Enemy { health: Int }
        entity World { target: link Enemy? }
        fn read(world: World) -> Int { return world.target.health }
        fn main() -> Int { return 0 }
    "#);
    assert_eq!(unchecked.diagnostics[0].code.0, "KLD1005");
}
```

- [ ] **Step 2: Run lifecycle tests and verify the missing-crate failure**

Run: `cargo test -p keld-lifecycle --test retirement --test alias_refinement`

Run: `cargo test -p keld-lifecycle --test lifecycle_order --test effects`

Run: `cargo test -p keld-lifecycle --test links_and_escape`

Expected: FAIL because `keld-lifecycle` does not exist.

- [ ] **Step 3: Define proof state and conservative provenance**

```rust
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)] pub struct ProofId(pub u32);
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)] pub struct ProvenanceId(pub u32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleFact { Known(LifecycleId), Dynamic }

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RefState {
    Live { lifecycle: LifecycleFact, provenance: ProvenanceId },
    Invalidated { cause: Span },
    Retired { cause: Span },
    OutOfScope,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReturnProvenance {
    NonEntity,
    EntitySources { parameters: Vec<u32>, fresh_in_caller_lifecycle: bool },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FunctionSummary {
    pub retires_parameters: Vec<u32>,
    pub retires_any: Vec<DefId>,
    pub return_provenance: ReturnProvenance,
}
```

Maintain must-alias equivalence classes and a conservative may-alias relation.
Copying a direct reference joins its must-alias class. Independent entity
parameters may alias. Distinct allocation operations are must-distinct. A callee
fresh return is remapped to a fresh caller call-site provenance so two calls do
not become must-alias merely because they share a callee allocation site. A
function that returns different entity parameters on different paths records all
of them in `EntitySources`; its call result becomes a new class that may alias
each listed argument. The fresh flag adds a distinct call-site alternative.
Parameters begin with dynamic lifecycle facts. Fresh allocation uses the active
known lifecycle. Link resolution yields a dynamic lifecycle fact. A returned
parameter preserves the corresponding caller fact; a fresh callee return maps to
the caller's active lifecycle.

- [ ] **Step 4: Infer and validate function summaries to a fixed point**

Process the acyclic call graph in reverse topological order; recursion was
rejected in Task 5. Infer exact retired parameters, broad retired entity types,
and return provenance. Every function's inferred retirement behavior must
exactly match its declared `retires` clauses. An undeclared retirement is
`KLD1009`; a redundant or impossible declaration is also `KLD1009`, with the
clause as primary span. Serialize the validated summary in `VerifiedFlowModule`.

- [ ] **Step 5: Implement block dataflow and joins**

Use a worklist over `BlockId`. A read, field access, `keep`, call argument, or
retirement requires `Live`. Exact retirement marks must-alias refs `Retired` and
may-alias refs `Invalidated`. Broad retirement invalidates every live compatible
entity reference in the store. Ending a lexical lifecycle invalidates refs known
to belong to it or a descendant. Join states using the least permissive state;
different live provenances become a may-alias live state only when both remain
live. Joining two live facts with different known lifecycles produces
`Dynamic`; joining the same known lifecycle preserves it.

For `BranchIdentity`, clone the input state. On the not-equal edge add a
must-distinct fact. On the equal edge merge the two provenance classes. A
resolved link creates a new scoped live provenance only on the live edge; force
it to `OutOfScope` at the end of the `when` body.

Reject every `ReadUncheckedLinkField` as `KLD1005` with a `when` repair. This
check runs even though the operation's result type is already known.

- [ ] **Step 6: Enforce keep and direct-reference escape rules**

Require the entity's fact to be `Known(source)` and the target lifecycle to be
an active strict ancestor of `source` in the same function's lifecycle tree.
Reject `keep` on a dynamic fact as `KLD1004`; do not defer it to a runtime check.
Successful `keep` changes the lifecycle fact for every must-alias reference.
Reject direct refs returned without parameter or fresh-in-caller provenance and
reject a resolved-link reference that escapes its `when` scope. Reject every
persistent direct-reference field marked by Task 5 as `KLD1002` and require a
link; no invalid definition may reach executable IR.

- [ ] **Step 7: Run all proof tests and commit**

Run: `cargo fmt --all --check`

Run: `cargo clippy -p keld-lifecycle --all-targets -- -D warnings`

Run: `cargo test -p keld-lifecycle`

Expected: all commands exit zero. Reachable source families `KLD1001`-`KLD1005`
and `KLD1008`-`KLD1009` each have a compile-fail assertion and every diagnostic
includes one concrete repair. `KLD1006` requires a source-level loan that can
span evaluation and `KLD1007` requires user cleanup; both remain reserved until
the storage/resource milestones. Invalid IR attempts to cross a structural
operation with a view are still rejected as `KLD9001` in Task 8.

```powershell
git add crates/keld-lifecycle
git commit -m "feat: verify Custody Ledger lifecycles"
```

### Task 8: Executable IR, Lifecycle Lowering, and Validator

**Files:**

- Create: `crates/keld-ir/Cargo.toml`
- Create: `crates/keld-ir/src/lib.rs`
- Create: `crates/keld-ir/src/module.rs`
- Create: `crates/keld-ir/src/instruction.rs`
- Create: `crates/keld-ir/src/lower.rs`
- Create: `crates/keld-ir/src/validate.rs`
- Create: `crates/keld-ir/src/dump.rs`
- Create: `crates/keld-ir/tests/lowering.rs`
- Create: `crates/keld-ir/tests/view_validation.rs`
- Create: `crates/keld-ir/tests/dump_golden.rs`

**Interfaces:**

- Consumes: `keld_lifecycle::VerifiedFlowModule` only.
- Produces: backend-neutral `Module`, `Function`, `Instruction`, `Terminator`,
  `validate(&Module) -> Vec<Diagnostic>`, and `dump(&Module) -> String`.
- Every entity field access is bracketed by explicit view open/close operations.
- Every accepted IR module validates before interpretation.

- [ ] **Step 1: Write an invalid-view test before defining IR**

```rust
use keld_ir::{validate, Instruction, IrType, Register, TestModuleBuilder,
              ViewId, ViewMode};
use keld_semantics::DefId;
use keld_source::{SourceId, Span};

#[test]
fn structural_operation_with_an_open_view_is_invalid() {
    let span = Span::new(SourceId(0), 0, 0).unwrap();
    let module = TestModuleBuilder::new()
        .parameter(Register(0), IrType::Entity(DefId(0)))
        .instruction(Instruction::OpenView { view: ViewId(0), entity: Register(0),
                                             mode: ViewMode::Read, span })
        .instruction(Instruction::RetireEntity { entity: Register(0), span })
        .finish();
    let diagnostics = validate(&module);
    assert_eq!(diagnostics[0].code.0, "KLD9001");
}
```

- [ ] **Step 2: Run IR tests and verify the missing-crate failure**

Run: `cargo test -p keld-ir --test lowering --test view_validation --test dump_golden`

Expected: FAIL because `keld-ir` does not exist.

- [ ] **Step 3: Define executable operations without source ambiguity**

```rust
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)] pub struct Register(pub u32);
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)] pub struct ViewId(pub u32);
#[derive(Clone, Copy, Debug, Eq, PartialEq)] pub enum ViewMode { Read, Edit }

pub enum Instruction {
    ConstInt { dst: Register, value: i64, span: Span },
    ConstBool { dst: Register, value: bool, span: Span },
    ConstNoneLink { dst: Register, entity: DefId, span: Span },
    Copy { dst: Register, src: Register, span: Span },
    CheckedUnaryInt { dst: Register, op: IntUnaryOp, src: Register, span: Span },
    CheckedBinaryInt { dst: Register, op: IntBinaryOp, lhs: Register,
                       rhs: Register, span: Span },
    Not { dst: Register, src: Register, span: Span },
    Compare { dst: Register, op: CompareOp, lhs: Register,
              rhs: Register, span: Span },
    Phi { dst: Register, inputs: Vec<(IrBlockId, Register)>, span: Span },
    ConstructStruct { dst: Register, definition: DefId,
                      fields: Vec<(FieldId, Register)>, span: Span },
    ReadStructField { dst: Register, base: Register, field: FieldId, span: Span },
    BeginLifecycle { dst: Register, parent: Register, span: Span },
    EndLifecycle { lifecycle: Register, span: Span },
    AllocateEntity { dst: Register, definition: DefId,
                     fields: Vec<(FieldId, Register)>,
                     lifecycle: Register, span: Span },
    EntityToLink { dst: Register, entity: Register, span: Span },
    OpenView { view: ViewId, entity: Register, mode: ViewMode, span: Span },
    ReadField { dst: Register, view: ViewId, field: FieldId, span: Span },
    WriteField { view: ViewId, field: FieldId, value: Register, span: Span },
    CloseView { view: ViewId, span: Span },
    KeepEntity { entity: Register, lifecycle: Register, span: Span },
    RetireEntity { entity: Register, span: Span },
    Call { dst: Option<Register>, function: FunctionId,
           arguments: Vec<(ParameterIndex, Register)>,
           current_lifecycle: Register, span: Span },
}

pub enum Terminator {
    Goto(IrBlockId),
    Branch { condition: Register, then_block: IrBlockId, else_block: IrBlockId },
    ResolveLink { link: Register, live_value: Register, live: IrBlockId,
                  absent: IrBlockId, span: Span },
    Return(Option<Register>),
    Fault { kind: FaultKind, span: Span },
    Unreachable,
}

pub enum FaultKind { Arithmetic, DivisionByZero, Shift, Allocation }

pub enum IrType {
    Unit, Bool, Int, Struct(DefId), Entity(DefId),
    Link { entity: DefId, optional: bool }, Lifecycle,
}

pub enum IrDefinitionKind { Struct, Entity }

pub struct IrDefinition {
    pub id: DefId,
    pub kind: IrDefinitionKind,
    pub fields: Vec<(FieldId, IrType)>,
}

pub struct IrBlock {
    pub id: IrBlockId,
    pub instructions: Vec<Instruction>,
    pub terminator: Terminator,
}

pub struct Function {
    pub id: FunctionId,
    pub parameters: Vec<Register>,
    pub current_lifecycle: Register,
    pub register_types: Vec<IrType>,
    pub return_type: IrType,
    pub blocks: Vec<IrBlock>,
    pub entry: IrBlockId,
}

pub struct Module {
    pub definitions: Vec<IrDefinition>,
    pub functions: Vec<Function>,
    pub main: FunctionId,
}
```

- [ ] **Step 4: Lower only verified operations**

Lower `ConstNoneLink` and `ReadStructField` directly; a struct read does not open
a view. Open, perform, and close a read view for each entity field read. Open,
perform, and close an edit view for each entity field write. Do not keep views
in locals, block parameters, calls, or returns. Materialize `BeginLifecycle` and
`EndLifecycle` from verified scope edges. Pass the active lifecycle as the
hidden final call operand. Lower each checked Int operation directly; do not
replace it with host-language arithmetic. After proof refinement is complete,
lower `BranchIdentity` to entity `Compare` plus `Branch`; executable IR performs
the runtime identity test but owns no alias reasoning.
Validate that every constructor supplies each declared `FieldId` exactly once.
The interpreter evaluates registers in the already-fixed source order, then
stores values in declaration-layout order using the definition table.
Likewise, validate that a call supplies each parameter index exactly once;
interpreter copies occur in source argument order before the callee frame is
assembled in parameter order.

- [ ] **Step 5: Implement whole-CFG validation and stable dump**

The validator tracks register definition/type, function parameter and return
types, call signatures, open views, lifecycle register state, Phi predecessor
consistency, and terminator presence. Every `Phi` must be
the first instruction group in its block, have exactly one input from every
predecessor, and agree in type with its destination. Reject use before
definition using CFG dominance (or edge availability for `Phi`), duplicate
definitions, non-contiguous register/type tables, writing through a read view, closing an
unknown view, a view live at a structural operation, a view live at a
terminator, or a non-Int `main` return as `KLD9001`-`KLD9005`. Reading through an
edit view is valid.

The dump uses explicit numeric IDs and source byte spans. Golden output must be
byte-stable across two runs in the same test.

Run: `cargo fmt --all --check`

Run: `cargo clippy -p keld-ir --all-targets -- -D warnings`

Run: `cargo test -p keld-ir`

Expected: all commands exit zero.

- [ ] **Step 6: Commit executable IR**

```powershell
git add crates/keld-ir
git commit -m "feat: lower verified Keld into executable IR"
```

### Task 9: Runtime Store, Slots, Generations, and Lifecycles

**Files:**

- Create: `crates/keld-runtime/Cargo.toml`
- Create: `crates/keld-runtime/src/lib.rs`
- Create: `crates/keld-runtime/src/id.rs`
- Create: `crates/keld-runtime/src/slot.rs`
- Create: `crates/keld-runtime/src/lifecycle.rs`
- Create: `crates/keld-runtime/src/store.rs`
- Create: `crates/keld-runtime/tests/store_transitions.rs`
- Create: `crates/keld-runtime/tests/lifecycle_cleanup.rs`
- Create: `crates/keld-runtime/tests/stale_links.rs`

**Interfaces:**

- Produces generic `Store<P>`, stable identity types, `Link`, classified
  `StoreError`, and closure-bounded read/edit access.
- Defines source-independent `RuntimeTypeId`; it does not depend on semantic,
  IR, or interpreter types.
- Structural APIs require `&mut Store`; view closures cannot re-enter them in
  safe Rust.

- [ ] **Step 1: Write state-transition and stale-link tests**

```rust
use keld_runtime::{RuntimeTypeId, Store};

#[test]
fn retired_link_never_resolves_after_slot_reuse() {
    let mut store = Store::new().unwrap();
    let root = store.root_lifecycle();
    let first = store.allocate(RuntimeTypeId(0), root, 10_u32).unwrap();
    let stale = store.link(first).unwrap();
    store.retire_with(first, |_| {}).unwrap();
    let second = store.allocate(RuntimeTypeId(0), root, 20_u32).unwrap();
    assert_eq!(first.slot_index(), second.slot_index());
    assert_ne!(first.generation(), second.generation());
    assert_eq!(store.resolve(stale), None);
}

#[test]
fn lifecycle_cleanup_is_reverse_adoption_order() {
    let mut store = Store::new().unwrap();
    let root = store.root_lifecycle();
    store.allocate(RuntimeTypeId(0), root, 1).unwrap();
    let child = store.begin_lifecycle(root).unwrap();
    let kept = store.allocate(RuntimeTypeId(0), child, 2).unwrap();
    store.keep(kept, root).unwrap();
    store.allocate(RuntimeTypeId(0), root, 3).unwrap();
    let mut cleaned = Vec::new();
    store.end_lifecycle_with(child, |payload| cleaned.push(payload)).unwrap();
    store.finish_with(|payload| cleaned.push(payload)).unwrap();
    assert_eq!(cleaned, vec![3, 2, 1]);
}
```

- [ ] **Step 2: Run runtime tests and verify the missing-crate failure**

Run: `cargo test -p keld-runtime --test store_transitions --test lifecycle_cleanup`

Run: `cargo test -p keld-runtime --test stale_links`

Expected: FAIL because `keld-runtime` does not exist.

- [ ] **Step 3: Define stable identity and slot state**

```rust
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)] pub struct StoreBrand(u64);
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)] pub struct SlotIndex(pub u32);
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)] pub struct Generation(pub u32);
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RuntimeLifecycleId { brand: StoreBrand, index: u32 }
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)] pub struct RuntimeTypeId(pub u32);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct EntityId {
    brand: StoreBrand,
    slot: SlotIndex,
    generation: Generation,
    definition: RuntimeTypeId,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Link {
    brand: StoreBrand,
    slot: SlotIndex,
    generation: Generation,
    expected: RuntimeTypeId,
}

enum Slot<P> {
    Empty { generation: Generation },
    Live { generation: Generation, definition: RuntimeTypeId,
           lifecycle: RuntimeLifecycleId, adoption: usize, payload: P },
    Dying { generation: Generation, definition: RuntimeTypeId,
            lifecycle: RuntimeLifecycleId, adoption: usize, payload: Option<P> },
    Retired,
}
```

Store slots in fixed-capacity `Vec<Slot<P>>` segments. Before adding a segment,
use `try_reserve` for the outer vector and `try_reserve_exact(SEGMENT_SIZE)` for
the empty segment, then fill it without exceeding that capacity. Reserve the
same additional slot count in both reusable and retired-index lists before
publishing the segment, so retirement never allocates. A segment's
buffer therefore never reallocates and a live slot address remains stable.
Maintain separate reusable and permanently retired slot lists. Generation
increment uses `checked_add`; a maximum-generation slot becomes `Retired` and is
never reused.
Bootstrap entity definitions have no inheritance, so runtime type compatibility
is exact `RuntimeTypeId` equality. Identity fields and constructors remain
private; `EntityId` exposes read-only `slot_index()` and `generation()` accessors
for tests and diagnostics.

`Store::new` obtains an opaque process-unique brand from a private `AtomicU64`.
Use a checked compare/exchange loop with relaxed ordering; return
`StoreError::BrandExhausted` instead of wrapping. A caller cannot choose a brand
or construct an `EntityId`, `Link`, or lifecycle ID.

Allocate from the reusable-slot stack before extending the slot vector, and
push newly emptied slots onto that stack. This deterministic LIFO rule makes the
single-slot reuse assertion normative without exposing physical addresses.

Each lifecycle keeps an append-only `Vec<Option<EntityKey>>` adoption log. A
live slot records its log index. Retirement marks that entry `None`; `keep`
reserves and appends in the target, marks the old entry `None`, then updates the
slot. Cleanup scans the log backward and skips tombstones. This preserves exact
reverse-adoption order without `swap_remove` reordering or linear-time index
repairs.

- [ ] **Step 4: Implement non-reentrant store operations**

```rust
impl<P> Store<P> {
    pub fn new() -> Result<Self, StoreError>;
    pub fn root_lifecycle(&self) -> RuntimeLifecycleId;
    pub fn begin_lifecycle(&mut self, parent: RuntimeLifecycleId)
        -> Result<RuntimeLifecycleId, StoreError>;
    pub fn allocate(&mut self, definition: RuntimeTypeId, lifecycle: RuntimeLifecycleId,
                    payload: P) -> Result<EntityId, StoreError>;
    pub fn link(&self, entity: EntityId) -> Result<Link, StoreError>;
    pub fn resolve(&self, link: Link) -> Option<EntityId>;
    pub fn read<R>(&self, entity: EntityId, f: impl FnOnce(&P) -> R)
        -> Result<R, StoreError>;
    pub fn edit<R>(&mut self, entity: EntityId, f: impl FnOnce(&mut P) -> R)
        -> Result<R, StoreError>;
    pub fn keep(&mut self, entity: EntityId, target: RuntimeLifecycleId)
        -> Result<(), StoreError>;
    pub fn retire_with(&mut self, entity: EntityId, cleanup: impl FnOnce(P))
        -> Result<(), StoreError>;
    pub fn end_lifecycle_with(&mut self, lifecycle: RuntimeLifecycleId,
        cleanup: impl FnMut(P)) -> Result<(), StoreError>;
    pub fn finish_with(&mut self, cleanup: impl FnMut(P)) -> Result<(), StoreError>;
}

pub enum StoreError {
    Allocation,
    BrandExhausted,
    InvalidOperation(StoreInvariantError),
}

pub enum StoreInvariantError {
    ForeignIdentity,
    StaleEntity,
    InvalidLifecycle,
    NonAncestorKeep,
    RootEnd,
    AlreadyFinished,
}
```

Retirement removes membership, changes to Dying, invokes cleanup without giving
the closure store access, then advances generation. Lifecycle end first marks
every remaining member Dying, then cleans in reverse adoption order.
`end_lifecycle_with` rejects a lifecycle with an active child: verified IR ends
children explicitly in innermost-first order. `keep` accepts strict ancestors only, removes the old
membership entry, appends the entity as the newest target adoption, and
preserves identity. `end_lifecycle_with` rejects the root; `finish_with` ends it
exactly once after ending any still-active descendants deepest-first.
All impossible compiler-generated operations return `StoreError`; the
interpreter converts invariant variants to `KLD9001`, never a user-catchable
fault. All storage growth uses `try_reserve` before mutation. Allocation failure
is the resource variant and becomes Keld `AllocationFault`. Entity allocation
reserves its lifecycle membership entry and any new segment metadata before
publishing the slot. `keep` reserves the target membership entry before removing
the old entry, so either failure leaves custody unchanged.

- [ ] **Step 5: Test generation exhaustion with a test-only limit**

Inside the runtime crate's unit tests, construct a Store whose internal
`max_generation` is two. Allocate/retire the same slot through generations zero,
one, and two, then assert the slot is permanently retired and a later allocation
uses a different slot. Do not expose generation limits in the public API.

Run: `cargo fmt --all --check`

Run: `cargo clippy -p keld-runtime --all-targets -- -D warnings`

Run: `cargo test -p keld-runtime`

Expected: all commands exit zero.

- [ ] **Step 6: Commit the runtime store**

```powershell
git add crates/keld-runtime
git commit -m "feat: implement Custody Ledger runtime store"
```

### Task 10: Executable-IR Interpreter

**Files:**

- Create: `crates/keld-interpreter/Cargo.toml`
- Create: `crates/keld-interpreter/src/lib.rs`
- Create: `crates/keld-interpreter/src/value.rs`
- Create: `crates/keld-interpreter/src/frame.rs`
- Create: `crates/keld-interpreter/src/machine.rs`
- Create: `crates/keld-interpreter/src/fault.rs`
- Create: `crates/keld-interpreter/tests/numeric.rs`
- Create: `crates/keld-interpreter/tests/entities.rs`
- Create: `crates/keld-interpreter/tests/functions.rs`

**Interfaces:**

- Consumes: a validated `keld_ir::Module` and `keld_runtime::Store`.
- Produces: `Interpreter`, `Value`, `RuntimeFault`, `ExecutionResult`, and
  deterministic execution of the zero-argument main function.
- Uses `keld_numeric` for every checked Int instruction.

- [ ] **Step 1: Write interpreter tests for numeric faults and stale links**

```rust
use keld_interpreter::{run_text_for_test, RuntimeFaultKind, Value};

#[test]
fn runtime_division_by_zero_has_the_source_fault() {
    let result = run_text_for_test(r#"
        fn divide(a: Int, b: Int) -> Int { return a / b }
        fn main() -> Int { return divide(7, 0) }
    "#);
    let fault = result.unwrap_err();
    assert_eq!(fault.kind, RuntimeFaultKind::DivisionByZero);
    assert!(fault.span.start().0 < fault.span.end().0);
}

#[test]
fn stale_link_resolution_uses_the_absent_edge() {
    let result = run_text_for_test(r#"
        entity Enemy { health: Int }
        fn main() -> Int {
            lifecycle level {
                let enemy = Enemy(health: 4)
                let saved: link Enemy = enemy
                retire enemy
                when saved as live { return 1 } else { return 4 }
            }
        }
    "#).unwrap();
    assert_eq!(result.value, Value::Int(4));
}
```

- [ ] **Step 2: Run interpreter tests and verify the missing-crate failure**

Run: `cargo test -p keld-interpreter --test numeric --test entities --test functions`

Expected: FAIL because `keld-interpreter` does not exist.

- [ ] **Step 3: Define values, payloads, frames, and faults**

```rust
#[derive(Debug, Eq, PartialEq)]
pub enum Value {
    Unit,
    Int(i64),
    Bool(bool),
    Struct { definition: DefId, fields: Vec<Value> },
    Entity(EntityId),
    Link(Option<Link>),
}

#[derive(Debug, Eq, PartialEq)]
pub struct EntityPayload { pub definition: DefId, pub fields: Vec<Value> }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeFaultKind { Arithmetic, DivisionByZero, Shift, Allocation }

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeFault { pub kind: RuntimeFaultKind, pub span: Span }

pub struct ExecutionResult { pub value: Value }

pub struct Interpreter<'m> {
    module: &'m Module,
    store: Store<EntityPayload>,
    frames: Vec<Frame>,
}
```

Each frame owns registers, current block/instruction indices, the hidden current
lifecycle, the predecessor block used to select `Phi` inputs, and an `ActiveView`
map from `ViewId` to entity identity and mode.
The interpreter's view map is an oracle representation, not an escaping payload
address; each field instruction enters a closure-bounded Store read or edit.
Do not use derived recursive `Clone` for runtime values. Implement fallible
`try_copy_value` with an explicit work stack; it copies Int, Bool, entity
identity, links, and nested structs while mapping every reserve failure to
`AllocationFault`.

- [ ] **Step 4: Implement the explicit instruction loop**

Fetch one instruction, execute it, then advance. Calls push a frame and pass the
same current lifecycle unless the call operand says otherwise. Return pops a
frame and writes the destination register. `BeginLifecycle`, `KeepEntity`,
`RetireEntity`, and `EndLifecycle` call the corresponding runtime operations.
Cleanup drops payload values without user callbacks; Task 5's layout-depth cap
bounds destructor recursion. `ResolveLink`
chooses exactly one successor and writes the live entity register only on the
live edge.

On block entry, execute the leading `Phi` group atomically from a snapshot of
the predecessor registers before executing non-`Phi` instructions. Entry blocks
cannot contain `Phi`; a missing or duplicate predecessor input is invalid IR.

Map `NumericFault` to `RuntimeFaultKind` and retain the instruction span. Treat
IR validation, `BrandExhausted`, or invalid Store operations as internal
`InterpreterError`, not Keld runtime faults. Map `StoreError::Allocation` and
failed interpreter `try_reserve` calls to `RuntimeFaultKind::Allocation` at the
currently executing source span. Convert semantic `DefId` values to
`RuntimeTypeId(definition.0)` only at the interpreter/runtime boundary. Before
`run_main`, call `validate`; refuse invalid modules. After a successful main
return, end the implicit root lifecycle before reporting the Int result. A
runtime fault does not promise that cleanup.

- [ ] **Step 5: Add interpreter/constant-evaluator parity tests**

For every boundary row from Task 2, build a one-operation IR function and compare
the interpreter result or fault with `keld_numeric::eval_binary`. Separately test
constant source expressions through semantic analysis. Include the `MIN % -1`
zero result and ensure the corresponding division faults.

Run: `cargo fmt --all --check`

Run: `cargo clippy -p keld-interpreter --all-targets -- -D warnings`

Run: `cargo test -p keld-interpreter`

Expected: all commands exit zero.

- [ ] **Step 6: Commit the interpreter**

```powershell
git add crates/keld-interpreter
git commit -m "feat: execute Keld IR in the interpreter"
```

### Task 11: Compiler Driver, CLI, Diagnostics, and End-to-End Programs

**Files:**

- Create: `crates/keld-cli/Cargo.toml`
- Create: `crates/keld-cli/src/lib.rs`
- Create: `crates/keld-cli/src/main.rs`
- Create: `crates/keld-cli/src/driver.rs`
- Create: `crates/keld-cli/src/args.rs`
- Create: `crates/keld-cli/src/render.rs`
- Create: `crates/keld-cli/tests/cli.rs`
- Create: `crates/keld-cli/tests/fixtures/cyclic_graph.keld`
- Create: `crates/keld-cli/tests/fixtures/keep_survives.keld`
- Create: `crates/keld-cli/tests/fixtures/stale_link.keld`
- Create: `crates/keld-cli/tests/fixtures/runtime_div_zero.keld`
- Create: `crates/keld-cli/tests/fixtures/fail_retired_use.keld`
- Create: `crates/keld-cli/tests/fixtures/fail_may_alias.keld`
- Create: `crates/keld-cli/tests/fixtures/fail_unsupported.keld`
- Create: `crates/keld-cli/tests/fixtures/alias_distinct.keld`
- Create: `crates/keld-cli/tests/fixtures/broad_retirement.keld`
- Create: `crates/keld-cli/tests/fixtures/numeric_edges.keld`

**Interfaces:**

- Owns pipeline orchestration but no language decisions.
- Produces library entry points `check_source`, `compile_source`, `run_source`,
  and a binary named `keld`.
- Stops after the first stage with errors; never lowers or runs invalid input.

- [ ] **Step 1: Write black-box CLI tests before the binary exists**

```rust
use std::process::Command;

#[test]
fn run_cyclic_graph_prints_the_main_result() {
    let output = Command::new(env!("CARGO_BIN_EXE_keld"))
        .args(["run", "--engine", "interpreter",
               "tests/fixtures/cyclic_graph.keld"])
        .output().unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "20\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn static_failure_is_exit_one_with_a_stable_code() {
    let output = Command::new(env!("CARGO_BIN_EXE_keld"))
        .args(["check", "tests/fixtures/fail_retired_use.keld"])
        .output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8(output.stderr).unwrap().contains("error[KLD1001]"));
}
```

- [ ] **Step 2: Run CLI tests and verify the missing-crate failure**

Run: `cargo test -p keld-cli --test cli`

Expected: FAIL because `keld-cli` does not exist.

- [ ] **Step 3: Implement the one-way driver pipeline**

```rust
pub struct Compilation {
    pub parsed: ParsedFile,
    pub typed: Option<TypedModule>,
    pub flow: Option<FlowModule>,
    pub verified: Option<VerifiedFlowModule>,
    pub ir: Option<keld_ir::Module>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn check_source(path: &Path, bytes: Vec<u8>) -> Compilation;
pub fn compile_source(path: &Path, bytes: Vec<u8>) -> Compilation;
pub fn run_source(path: &Path, bytes: Vec<u8>)
    -> Result<ExecutionResult, DriverFailure>;
```

`check_source` executes source, lex, parse, semantics, flow, lifecycle, IR lower,
and IR validation because checking includes all static safety obligations.
`compile_source` is the same bootstrap pipeline and returns the validated IR.
At each stage, sort diagnostics and return immediately if any error exists.

- [ ] **Step 4: Implement commands and exact exit behavior**

Accept only:

```text
keld check <file>
keld run --engine interpreter <file>
keld dump-ir <file>
```

Parse arguments with `std::env::args_os`. Reject missing/extra arguments, unknown
commands, and engines with a one-line usage message and exit 64. `check` is silent
on success. `run` writes the main Int plus newline. A runtime Keld fault renders
its kind and source location and exits two. `dump-ir` writes the stable IR dump.
Failure to read the selected file renders `KLD0001` with the display path and
operating-system message and exits one; it is not command misuse.

Render static diagnostics as:

```text
path:line:column: error[KLD1001]: `enemy` is no longer live
  --> primary label
  = note: secondary label
  = help: concrete repair
```

- [ ] **Step 5: Add the representative programs**

`cyclic_graph.keld` must create two enemies with links to each other, retire one,
take the absent branch when resolving the stale link, and return the remaining
enemy's health `20`.

```keld
entity Enemy {
    health: Int
    target: link Enemy?
}

fn main() -> Int {
    lifecycle level {
        let first = Enemy(health: 10, target: none)
        let second = Enemy(health: 20, target: first)
        first.target = second
        retire first
        when second.target as stale {
            return stale.health
        } else {
            return second.health
        }
    }
}
```

`keep_survives.keld` must allocate an entity in an inner lifecycle, store its
link in an outer entity, keep it in the outer lifecycle, resolve it after the
inner lifecycle ends, and return its health.

```keld
entity Enemy { health: Int }
entity Anchor { target: link Enemy? }

fn main() -> Int {
    lifecycle game {
        let anchor = Anchor(target: none)
        lifecycle level {
            let enemy = Enemy(health: 30)
            anchor.target = enemy
            keep enemy in game
        }
        when anchor.target as survivor {
            return survivor.health
        } else {
            return 0
        }
    }
}
```

`runtime_div_zero.keld` must call a non-constant helper `divide(7, 0)` so static
constant evaluation accepts the program and runtime exits two with
`DivisionByZeroFault`.

```keld
fn divide(value: Int, divisor: Int) -> Int {
    return value / divisor
}

fn main() -> Int {
    return divide(7, 0)
}
```

`alias_distinct.keld` proves the accepted identity-refinement branch:

```keld
entity Enemy { health: Int }

fn remove_then_read(a: Enemy, b: Enemy) -> Int retires a {
    if a != b {
        retire a
        return b.health
    } else {
        retire a
        return 0
    }
}

fn main() -> Int {
    lifecycle level {
        let first = Enemy(health: 10)
        let second = Enemy(health: 20)
        return remove_then_read(first, second)
    }
}
```

`broad_retirement.keld` proves that a broad effect invalidates old direct proofs
while later link resolution can establish a new proof:

```keld
entity Enemy { health: Int }
entity World { target: link Enemy? }

fn sweep(world: World, enabled: Bool) -> Int retires any Enemy {
    if enabled {
        when world.target as enemy {
            retire enemy
        }
    }
    return 0
}

fn main() -> Int {
    lifecycle level {
        let selected = Enemy(health: 40)
        let world = World(target: selected)
        let ignored = sweep(world, false)
        when world.target as selected_again {
            return selected_again.health + ignored
        } else {
            return 0
        }
    }
}
```

`numeric_edges.keld` proves runtime and constant handling of the signed minimum
remainder edge:

```keld
fn remainder(value: Int, divisor: Int) -> Int {
    return value % divisor
}

fn main() -> Int {
    return remainder(-9223372036854775808, -1)
}
```

The three fail fixtures must assert respectively `KLD1001`, `KLD1008`, and
`KLD0004` with exact stderr golden files embedded in the test module.

- [ ] **Step 6: Run CLI and workspace checks, then commit**

Run: `cargo fmt --all --check`

Run: `cargo clippy --workspace --all-targets -- -D warnings`

Run: `cargo test --workspace`

Run: `cargo run -p keld-cli -- check crates/keld-cli/tests/fixtures/cyclic_graph.keld`

Run:

```powershell
cargo run -p keld-cli -- run --engine interpreter `
  crates/keld-cli/tests/fixtures/cyclic_graph.keld
```

Expected final command stdout: `20` followed by one newline; every command exits
zero.

```powershell
git add crates/keld-cli
git commit -m "feat: add Keld check and interpreter CLI"
```

### Task 12: Model Tests and Milestone Acceptance Gate

**Files:**

- Create: `crates/keld-runtime/tests/model_sequences.rs`
- Create: `crates/keld-cli/tests/milestone_acceptance.rs`
- Create: `docs/compiler-architecture.md`
- Create: `docs/milestone-acceptance.md`
- Create: `README.md`

**Interfaces:**

- Consumes all completed bootstrap crates.
- Produces no new language behavior.
- Establishes the command-backed acceptance gate for all 16 first-milestone done
  criteria.

- [ ] **Step 1: Write a deterministic reference-model sequence test**

```rust
#[test]
fn store_matches_reference_model_for_generated_sequences() {
    for seed in 0_u64..256 {
        let mut rng = XorShift64::new(seed + 1);
        let mut real = TestStore::new();
        let mut model = ModelStore::new();
        for _ in 0..1_000 {
            let operation = Operation::generate(&mut rng, &model);
            assert_eq!(real.apply(operation.clone()), model.apply(operation),
                       "seed={seed}");
            real.assert_invariants();
        }
    }
}
```

Implement `XorShift64`, `ModelStore`, and `Operation` inside the integration test
with no dependency. Generate allocate, link, resolve, retire, begin lifecycle,
keep to ancestor, and end lifecycle. Compare returned identities, resolutions,
cleanup order, slot state, lifecycle membership, and generation. Invalid
operations must return the same classified error in both implementations.
Operations refer to logical test handles, not raw runtime IDs. `TestStore` maps
results to an `ObservedIdentity { slot, generation, definition }`, so the model
never observes or attempts to reproduce the runtime's opaque store brand.

- [ ] **Step 2: Add a done-criteria acceptance test table**

Create one named test for each Section 16 done criterion. Each row records the
fixture or unit-test function proving it. The acceptance test must execute CLI
fixtures for graph cycles, keep, stale links, static retirement failures,
may-alias rejection, distinct-branch acceptance, broad retirement, and numeric
edges. It must call IR validation for every compiled pass fixture and assert no
active View reaches a structural instruction.

```rust
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
assert_eq!(SOURCE_ACCEPTANCE.len(), 10);
for (name, fixture) in SOURCE_ACCEPTANCE {
    run_acceptance_case(name, fixture);
}
```

`docs/milestone-acceptance.md` maps all 16 numbered design criteria to these ten
black-box cases or to an exact unit/model test path and test function. It also
records the one full-workspace command that executes each proof.

For every accepted CLI fixture, call the `run_source` library API first, then run
the binary and assert that stdout is the decimal serialization of the same Int
result. This is the direct-interpreter versus command-line equivalence check.

- [ ] **Step 3: Document the implemented pipeline and boundaries**

`docs/compiler-architecture.md` must list every crate, its public inputs and
outputs, forbidden responsibilities, diagnostic range, and the exact pipeline
stop rule. Include the stable runtime slot transitions and the rule that runtime
errors indicating invalid IR are compiler bugs. `README.md` must contain only
verified commands: build, test, check, run, dump IR, and the exact supported
bootstrap subset.

```markdown
# Keld Compiler Architecture
## Pipeline and stop rule
## Crate ownership
## Diagnostic ownership
## Runtime slot transitions
## Compiler-bug boundary

# Bootstrap milestone acceptance
## Source-level acceptance cases
## Static proof cases
## Runtime model cases
## Numeric parity cases
## Full verification command
```

- [ ] **Step 4: Run the full debug and release verification gate**

Run: `cargo fmt --all --check`

Run: `cargo clippy --workspace --all-targets -- -D warnings`

Run: `cargo test --workspace`

Run: `cargo test --workspace --release`

Run:

```powershell
cargo run --release -p keld-cli -- run --engine interpreter `
  crates/keld-cli/tests/fixtures/cyclic_graph.keld
```

Run: `cargo run --release -p keld-cli -- dump-ir crates/keld-cli/tests/fixtures/keep_survives.keld`

Expected: all commands exit zero; cyclic graph stdout is exactly `20` plus one
newline; IR dump output is non-empty and validates in its golden test.

- [ ] **Step 5: Confirm repository scope and commit the acceptance gate**

Run: `git status --short`

Expected: only Task 12 files are modified or untracked.

```powershell
git add crates/keld-runtime/tests/model_sequences.rs `
  crates/keld-cli/tests/milestone_acceptance.rs docs/compiler-architecture.md `
  docs/milestone-acceptance.md README.md
git commit -m "test: verify the Keld bootstrap milestone"
```

## Execution Handoff

Plan execution must begin in an isolated worktree created with
`superpowers:using-git-worktrees`. Then choose one execution workflow:

1. **Subagent-Driven** — explicitly authorize agent delegation; use
   `superpowers:subagent-driven-development`, one fresh worker per task, with
   specification and quality review between tasks.
2. **Inline Execution** — use `superpowers:executing-plans` in the isolated
   worktree, execute sequential task batches, and stop at review checkpoints.
