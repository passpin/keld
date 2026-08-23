# Keld Native-1 Import-Head Validation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans (recommended) to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reject GNU import archives that mix the approved `keld_runtime_v1.dll` import head with `keld_rt_v1_*` import members bound to another DLL.

**Architecture:** Keep `keld_ir::Module` as the only Native-1 semantic authority and preserve the existing versioned DLL plus GNU import-library boundary. Extend the existing bounded GNU archive/COFF parser so validation proves the head dependency of every runtime import member and rejects additional or conflicting heads before LLVM emission or linking.

**Tech Stack:** Rust workspace, `cargo +stable-x86_64-pc-windows-gnu`, LLVM 22.1.8, MinGW-w64 GNU `dlltool`/`ar`, existing `keld-native-backend` Native-1 integration tests.

**Spec:** `docs/superpowers/specs/2026-08-14-keld-llvm-native-1-design.md`

## Global Constraints

- Target remains `x86_64-w64-windows-gnu`; no MSVC fallback.
- The runtime remains `keld_runtime_v1.dll` with import library `libkeld_runtime_v1.dll.a` and `keld_rt_v1_*` exports.
- The backend continues to consume and revalidate executable IR without reconstructing lifecycle, storage, cleanup, or interpreter policy.
- Import validation must reject conflicting/additional DLL heads, including `other_runtime.dll`, before link output is published.
- Use the repo-local LLVM 22.1.8 prefix and `cargo +stable-x86_64-pc-windows-gnu` for verification.
- Do not start the next milestone, add unrelated features, redesign the approved ABI, or change interpreter semantics.

---

### Task 1: Reproduce the mixed-target archive in a Native-1 regression

**Files:**
- Modify: `crates/keld-native-backend/tests/native_int.rs` near the existing import-library validation regressions.

**Interfaces:**
- Consumes: `runtime_artifacts`, `build_executable`, `const_module`, `metadata`, and the existing `request` helper.
- Produces: one regression named for mixed runtime import heads that constructs a valid archive plus `other_runtime.dll` runtime-import members and requires backend rejection before an executable is published.

- [x] **Step 1: Write the failing test**

  Generate the normal valid runtime archive, generate a second GNU import archive whose definition names `other_runtime.dll` and contains the remaining `keld_rt_v1_*` exports, extract only its runtime-import object members (not its head/name records), append those members to the valid archive, and call `build_executable` with the mixed archive. Assert the error is `BackendError::Toolchain`, contains the runtime-import validation message, and leaves the output absent.

- [x] **Step 2: Run the focused test to verify RED**

  Run:

  ```powershell
  $env:LLVM_SYS_221_PREFIX = (Resolve-Path '.tools/llvm/22.1.8-mingw64').Path
  $env:PATH = "$env:LLVM_SYS_221_PREFIX\bin;$env:PATH"
  cargo +stable-x86_64-pc-windows-gnu test -p keld-native-backend --test native_int mixed_runtime_import_heads_are_rejected -- --exact --nocapture
  ```

  Expected: the new assertion fails because the current validator accepts the valid head/ABI members and does not inspect the conflicting head dependency on the appended `other_runtime.dll` import members.

### Task 2: Prove one approved import head for every runtime import

**Files:**
- Modify: `crates/keld-native-backend/src/lib.rs` in the existing archive/COFF validation helpers and `validate_runtime_import_library`.

**Interfaces:**
- Consumes: the existing checked GNU archive traversal, COFF section extraction, and COFF symbol-table parser.
- Produces: `validate_runtime_import_library` accepts the normal archive and rejects any `keld_rt_v1_*` import member whose undefined `_head_*` dependency is not the single head associated with `keld_runtime_v1.dll`; malformed archive/COFF records continue to fail closed.

- [x] **Step 1: Implement the minimal validator change**

  Retain the current proof that a real `.idata$7` name member binds `keld_runtime_v1.dll`, that a real `.idata$2` member defines the corresponding `_head_` symbol, and that `keld_rt_v1_abi_version` is a real import associated with that head. During the same archive walk, classify every defined `keld_rt_v1_*` import symbol and require its `__imp_` symbol plus an undefined head reference equal to the approved head. Reject an archive containing another defined `_head_*` member or any runtime import whose head reference differs or is missing. Preserve checked bounds, fail-closed parsing, the existing `BackendError::Toolchain` contract, and the current error text family.

- [x] **Step 2: Run the focused red/green tests**

  Run the new mixed-target regression and the existing wrong-DLL decoy regression as separate exact-test commands:

  ```powershell
  $env:LLVM_SYS_221_PREFIX = (Resolve-Path '.tools/llvm/22.1.8-mingw64').Path
  $env:PATH = "$env:LLVM_SYS_221_PREFIX\bin;$env:PATH"
  cargo +stable-x86_64-pc-windows-gnu test -p keld-native-backend --test native_int mixed_runtime_import_heads_are_rejected -- --exact --nocapture
  cargo +stable-x86_64-pc-windows-gnu test -p keld-native-backend --test native_int import_library_with_wrong_dll_name_hidden_by_benign_member_is_rejected -- --exact --nocapture
  ```

  Expected: both tests pass, with valid runtime artifacts and all unrelated Native-1 behavior unchanged.

### Task 3: Run Native-1 and workspace verification, then commit

**Files:**
- Inspect: the final diff and repository status; no additional source files beyond Tasks 1 and 2.

- [x] **Step 1: Run the Native-1 focused gates**

  Run the backend native integration, differential, surface-audit, ABI/runtime, and CLI Native-1 suites using the repo-local LLVM prefix. Also run the restricted-PATH native executable smoke and PE import audit required by the existing Native-1 plan, recording exit status and failure counts from completed commands.

- [x] **Step 2: Run the full workspace gates**

  Run `cargo +stable-x86_64-pc-windows-gnu test --workspace --all-targets --no-fail-fast`, strict workspace Clippy with `-D warnings`, `cargo fmt --all -- --check`, and `git diff --check` with fresh output.

- [x] **Step 3: Review scope and commit**

  Confirm only the plan, validator, and regression changes are staged, then commit them with:

  ```powershell
  git add docs/superpowers/plans/2026-08-22-keld-native-1-import-head-validation.md crates/keld-native-backend/src/lib.rs crates/keld-native-backend/tests/native_int.rs
  git commit -m "fix(native): reject mixed runtime import heads"
  ```

  Do not merge, push, or begin another milestone.
