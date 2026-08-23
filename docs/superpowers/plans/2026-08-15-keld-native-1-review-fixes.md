# Keld LLVM Native-1 Review Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task with TDD and verification checkpoints.

**Goal:** Fix the three concrete Native-1 review findings without changing the frozen Windows GNU architecture: reverse-order Phi lowering, runtime DLL/import ABI validation, and native impossible-List-capacity classification.

**Architecture:** Keep validated executable IR as the semantic authority and preserve the existing status/out-result ABI. Give every IR block a stable native exit edge so Phi inputs are independent of emission order; validate the supplied PE/import pair at the backend boundary and check the versioned ABI before context creation; share the interpreter's checked count/byte capacity rule with native List growth.

**Tech Stack:** Rust workspace, `cargo +stable-x86_64-pc-windows-gnu`, LLVM 22.1.8, MinGW-w64 PE/COFF, existing Native-1 differential harness.

## Global Constraints

- Target remains `x86_64-w64-windows-gnu`; no MSVC fallback.
- The runtime remains `keld_runtime_v1.dll` with import library `libkeld_runtime_v1.dll.a` and `keld_rt_v1_*` exports.
- The backend continues to revalidate executable IR before creating LLVM state and does not reconstruct lifecycle/storage policy.
- O0 and O2 must use the same validated IR, deterministic source locations, allocation site IDs, and failure schedules.
- Language faults remain exact interpreter-compatible faults; ABI/toolchain failures remain internal build/runtime failures.
- Do not add Linux, WebAssembly, typed errors, language features, performance work, debugger metadata, or style-only changes.

## File ownership map

- Modify `crates/keld-native-backend/src/lib.rs` for stable block exits, PE/import artifact checks, and generated ABI-version startup validation.
- Modify `crates/keld-native-backend/tests/native_int.rs` for real reverse-order Phi and runtime-artifact/ABI regressions at O0 and O2.
- Modify `crates/keld-native-backend/tests/differential.rs` for a source-level impossible-capacity differential case.
- Modify `crates/keld-native-runtime/src/lib.rs` and `crates/keld-native-runtime/tests/values.rs` for the native checked capacity rule.
- Modify `crates/keld-native-ffi/src/lib.rs` only to make the test-controls DLL return a deliberately mismatched version for the ABI regression; production still returns the frozen `ABI_VERSION`.

### Task 1: Stable predecessor labels for Phi

**RED:** Add `reverse_order_phi_after_checked_predecessor_runs_at_both_optimization_levels` to `native_int.rs`. Build a valid module with entry block 0 branching to join block 1 or block 2, place the Phi in block 1, and place a checked integer operation in block 2 before `Goto(1)`. Assert both O0 and O2 build and run with the hand-derived result `4`.

Run:

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-native-backend --test native_int reverse_order_phi_after_checked_predecessor_runs_at_both_optimization_levels -- --exact
```

Expected before the fix: LLVM verification fails because the Phi names `%bb2` while the checked predecessor exits through its continuation label.

**GREEN:** Precompute a stable `exit_bb<id>` label for every function block. Before each block terminator, branch from the current instruction continuation to that stable exit block, emit the terminator there, and use the stable labels for scalar and managed Phi incoming edges. This keeps forward references valid and preserves all existing terminator semantics.

### Task 2: Native List capacity parity

**RED:** Add `reserve_classifies_impossible_byte_capacity_as_capacity_fault` to `keld-native-runtime/tests/values.rs`, asserting `list_reserve(list, i64::MAX) == Err(NativeValueError::Capacity)`. Add a `capacity_overflow_surface` source case to `differential.rs` using `items.reserve(9223372036854775807)` and run it through the existing interpreter/native O0/O2 schedule comparison.

Run:

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-native-runtime --test values reserve_classifies_impossible_byte_capacity_as_capacity_fault -- --exact
cargo +stable-x86_64-pc-windows-gnu test -p keld-native-backend --test differential source_surface_fixtures_extend_the_same_differential_schedule -- --exact
```

Expected before the fix: the direct runtime call and native differential run report `Allocation` while the interpreter reports `Capacity`.

**GREEN:** Centralize native required-capacity validation for signed additional counts, checked length, representable source length, checked element-byte multiplication, and `isize::MAX`; return `Capacity` before any allocation-control attempt. Reuse it for `list_reserve`, `list_try_reserve`, and single-element growth so the native paths cannot diverge.

### Task 3: Runtime artifact and ABI mismatch handling

**RED:** Add native integration regressions that (a) pass a same-named DLL containing non-PE bytes and require `build_executable` to reject it before publishing an executable, and (b) run a valid test-controls DLL with `KELD_TEST_ABI_VERSION=2` and require O0/O2 executables to exit 70 without invoking context setup or printing a language result.

Run:

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-native-backend --test native_int invalid_runtime_dll_is_rejected_before_link -- --exact
cargo +stable-x86_64-pc-windows-gnu test -p keld-native-backend --test native_int native_program_rejects_runtime_abi_version_mismatch -- --exact
```

Expected before the fix: malformed bytes are staged/accepted and a version-2 test runtime still prints the program result.

**GREEN:** Add a small safe PE32+ parser at the backend boundary that checks the x86-64 PE signature and exported `keld_rt_v1_abi_version`, and validate the GNU import archive contains the expected DLL name and ABI symbol. Add the generated `keld_rt_v1_abi_version` call before context creation; branch to the existing internal exit unless it equals `ABI_VERSION`. Keep the environment override behind `test-controls` in the test-only FFI DLL.

### Task 4: Verification and commit

Run the focused red/green tests, formatter, strict Clippy, workspace tests in debug and release, Native-1 differential/surface/CLI gates, restricted-PATH child execution, PE import audit, and `git diff --check`. Inspect the final diff for scope and commit the plan plus fixes with a Native-1 review-fix message. Do not merge, push, or begin another milestone.
