# Keld Native-1 Short-Import Validation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans (recommended) to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reject runtime import archives whose COFF short-import records bind `keld_rt_v1_*` symbols to any DLL other than `keld_runtime_v1.dll`.

**Architecture:** Keep the approved Native-1 versioned DLL plus GNU import-library boundary and the existing single approved GNU import head/name/ABI proof. Extend the bounded archive parser with the x86-64 COFF short-import record format used by LLVM, then require every recognized runtime import member—long `.idata$6` or short form—to target the approved DLL before LLVM emission or linking.

**Tech Stack:** Rust workspace, `cargo +stable-x86_64-pc-windows-gnu`, LLVM 22.1.8 `llvm-dlltool`, MinGW-w64 GNU `dlltool`/`ar`, existing `keld-native-backend` Native-1 integration tests.

**Spec:** `docs/superpowers/specs/2026-08-14-keld-llvm-native-1-design.md`

## Global Constraints

- Target remains `x86_64-w64-windows-gnu`; no MSVC fallback.
- The runtime remains `keld_runtime_v1.dll` with import library `libkeld_runtime_v1.dll.a` and `keld_rt_v1_*` exports.
- The approved GNU `.idata$2` head/name/ABI proof remains required; short-import parsing supplements it and does not replace the approved architecture.
- Every recognized import member in the runtime archive must target `keld_runtime_v1.dll`; malformed supported records fail closed.
- Use the repo-local LLVM 22.1.8 prefix and `cargo +stable-x86_64-pc-windows-gnu` for verification.
- Do not start the next milestone, add unrelated features, redesign the approved ABI, or change interpreter semantics.

---

### Task 1: Add the failing mixed short-import regression

**Files:**
- Modify: `crates/keld-native-backend/tests/native_int.rs` beside `mixed_runtime_import_heads_are_rejected`.

**Interfaces:**
- Consumes: `runtime_artifacts`, the existing GNU archive tools, the repo-local LLVM prefix, `build_executable`, `const_module`, `metadata`, and `request`.
- Produces: `mixed_runtime_short_imports_are_rejected`, which appends LLVM COFF short-import records for `keld_rt_v1_print_int` and neighboring runtime symbols targeting `other_runtime.dll` to the approved GNU archive and requires rejection before output publication.

- [x] **Step 1: Write the failing test**

  Generate `other_runtime.dll` with `llvm-dlltool` from a definition that omits `keld_rt_v1_abi_version`. Extract short-import archive occurrences after the three descriptor members using GNU `ar xN`, rename the extracted temporary objects, append at least three short-import objects to the valid GNU archive, and call `build_executable`. Assert `BackendError::Toolchain`, the existing COFF-import validation message, and no output file.

- [x] **Step 2: Run the focused test to verify RED**

  Run:

  ```powershell
  $env:LLVM_SYS_221_PREFIX = (Resolve-Path '.tools/llvm/22.1.8-mingw64').Path
  $env:PATH = "$env:LLVM_SYS_221_PREFIX\bin;$env:PATH"
  cargo +stable-x86_64-pc-windows-gnu test -p keld-native-backend --test native_int mixed_runtime_short_imports_are_rejected -- --exact --nocapture
  ```

  Expected: FAIL because the current validator skips short-import members without `.idata$6` and accepts the mixed archive.

### Task 2: Parse and validate COFF short-import records

**Files:**
- Modify: `crates/keld-native-backend/src/lib.rs` in the existing bounded COFF/archive helpers and `validate_runtime_import_library`.

**Interfaces:**
- Consumes: the existing checked GNU archive traversal, PE integer readers, approved runtime DLL constant, and long-import/head validation.
- Produces: structural short-import parsing that recognizes the COFF import signature, x86-64 machine, bounded symbol/DLL strings, and rejects recognized import records not targeting the approved DLL; approved short ABI records count as ABI evidence while the GNU head/name proof remains mandatory.

- [x] **Step 1: Implement the minimal parser and validator change**

  Add a checked helper that distinguishes ordinary members from COFF short-import members, rejects malformed short-import headers and bounds, parses the symbol and DLL strings from the declared data payload, and returns them without substring searches. During the archive validation walk, reject any recognized short-import member whose DLL differs from `keld_runtime_v1.dll`; mark `keld_rt_v1_abi_version` as found when its short record targets the approved DLL. Leave ordinary non-import members alone and preserve the existing long `.idata$6` head-reference checks and error contract.

- [x] **Step 2: Run the focused red/green tests**

  Run the new short-import regression, the existing GNU mixed-head regression, the wrong-DLL decoy regression, and the backend integration suite. All must pass, and a normal approved archive must still build and execute.

### Task 3: Run Native-1/workspace gates and commit

**Files:**
- Inspect: final diff and repository status.
- Modify: no additional source files beyond the regression, validator, and this plan.

- [x] **Step 1: Run Native-1 focused gates**

  Run the backend architecture, differential, native integration, surface-audit, ABI, runtime, FFI, toolchain, and CLI suites with the repo-local LLVM prefix. Run a valid native O2 build and inspect its PE imports for `keld_runtime_v1.dll` without `other_runtime.dll`.

- [x] **Step 2: Run full workspace gates**

  Run `cargo +stable-x86_64-pc-windows-gnu test --workspace --all-targets --no-fail-fast`, strict workspace Clippy with `-D warnings`, `cargo fmt --all -- --check`, and `git diff --check`, recording completed exit statuses and failure counts.

- [x] **Step 3: Review scope and commit**

  Confirm only the plan, validator, and regression changes are present, then commit with:

  ```powershell
  git add docs/superpowers/plans/2026-08-22-keld-native-1-short-import-validation.md crates/keld-native-backend/src/lib.rs crates/keld-native-backend/tests/native_int.rs
  git commit -m "fix(native): validate COFF short imports"
  ```

  Do not merge, push, or begin another milestone.
