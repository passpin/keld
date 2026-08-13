# Keld Managed Cleanup Correctness Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Correct the six concrete managed-cleanup, loan, List, and IR-validation defects found in the completed milestone without changing the language surface or approved storage semantics.

**Architecture:** Keep storage verification as the sole owner of Home/value/loan/drop decisions. Make the interpreter resolve all managed reads through address-free loan places and use preallocated cleanup scratch rather than allocating during cleanup. Make `keld-ir::validate` reject malformed consuming operands, projected call sources, and non-live managed returns before the interpreter can execute them.

**Tech Stack:** Rust workspace, `keld-storage`, `keld-ir`, `keld-interpreter`, Cargo tests, strict Clippy, rustfmt.

## Global Constraints

- Preserve the approved single-home, reverse-successful-initialization, `MaybeLive`, replacement, reservation, and nested managed-value semantics.
- Add a regression test before each production fix and observe the expected failure.
- Keep the language surface unchanged; test controls and validator fixtures are test infrastructure only.
- Do not turn invalid IR into an ordinary runtime fault; validation must reject it.
- Run the focused test after each fix, then the full workspace suite, strict Clippy, formatting, and `git diff --check`.

---

### Task 1: Resolve loaned Text operands

**Files:**
- Modify: `crates/keld-interpreter/tests/storage.rs`
- Modify: `crates/keld-interpreter/src/machine.rs`

- [x] Add focused tests for `List[Text]` projected operands in `+` and equality; run them and observe the current undefined-loan failure.
- [x] Route `TextConcat` and `Compare` operands through the existing loan-aware `with_register_value` helper.
- [x] Run the focused storage tests.

### Task 2: Preserve managed field loans from owned temporaries

**Files:**
- Modify: `crates/keld-interpreter/tests/storage.rs`
- Modify: `crates/keld-storage/src/verify.rs`
- Modify: `crates/keld-interpreter/src/machine.rs` if lowering/storage role behavior requires no-copy read handling.

- [x] Add a test that reads a managed field from an owned temporary while structural-copy allocation is injected to fail; run it and observe the current allocation fault.
- [x] Classify owned aggregate managed-field reads as address-free loans rooted in the owned temporary rather than `BorrowedUnknown`/trivial values.
- [x] Ensure the resulting IR destination is a Loan and runtime field access resolves through the temporary’s place.
- [x] Run storage and focused interpreter tests.

### Task 3: Make cleanup and `List.clear` allocation-free

**Files:**
- Modify: `crates/keld-interpreter/src/cleanup.rs`
- Modify: `crates/keld-interpreter/src/frame.rs` or machine-owned execution scratch as needed.
- Modify: `crates/keld-interpreter/src/machine.rs`
- Modify: `crates/keld-interpreter/src/list.rs`
- Modify: `crates/keld-interpreter/tests/cleanup.rs`
- Modify: `crates/keld-interpreter/tests/list_surface.rs`

- [x] Add deterministic allocation-failure regressions proving cleanup and clear do not use structural/list allocation paths; run them red.
- [x] Reuse preallocated cleanup scratch and move list elements through an iterative, capacity-preserving path without allocating a temporary removal vector.
- [x] Retain reverse cleanup order and nested aggregate/Optional behavior.
- [x] Run cleanup and List focused suites.

### Task 4: Reject invalid consuming IR operands

**Files:**
- Modify: `crates/keld-ir/tests/storage_validation.rs`
- Modify: `crates/keld-ir/src/validate.rs`

- [x] Add a malformed-IR test that rewrites a loan read into `Take`; verify validation currently accepts it.
- [x] Require consuming sources and destinations to have the exact Home/DropSlot roles required by each instruction, including `Take`, List replacement/push, aggregate construction, and consuming call arguments.
- [x] Run the focused IR validation suite.

### Task 5: Validate projected call sources

**Files:**
- Modify: `crates/keld-ir/tests/storage_validation.rs`
- Modify: `crates/keld-ir/src/validate.rs`

- [x] Add malformed call-source fixtures with an invalid projection/index type and a source that does not resolve to the argument’s type; verify the current validator accepts the malformed source.
- [x] Reuse `validate_argument_source` and cross-check each source against its corresponding argument and callee parameter.
- [x] Run the focused IR validation suite.

### Task 6: Require live managed returns

**Files:**
- Modify: `crates/keld-ir/tests/storage_validation.rs`
- Modify: `crates/keld-ir/src/validate.rs`

- [x] Add a malformed IR test returning an Empty or MaybeLive Home; verify the current validator accepts it.
- [x] Validate the selected managed return register’s role and require `HomeState::Live`; continue excluding it from ordinary end-of-function cleanup only after that check.
- [x] Run the focused IR validation suite.

### Final verification and commit

- [x] Run focused tests after each task and confirm every new test has a red-before-fix and green-after-fix result.
- [x] Run `cargo +stable-x86_64-pc-windows-gnu test --workspace --all-targets --no-fail-fast`.
- [x] Run `cargo +stable-x86_64-pc-windows-gnu clippy --workspace --all-targets --all-features -- -D warnings`.
- [x] Run `cargo +stable-x86_64-pc-windows-gnu fmt --all -- --check` and `git diff --check`.
- [x] Confirm only intended files are changed and commit the six fixes cleanly.
