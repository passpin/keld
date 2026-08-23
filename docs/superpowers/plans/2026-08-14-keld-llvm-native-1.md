# Keld LLVM Native-1 Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans to implement this plan task-by-task with TDD and verification checkpoints.

**Goal:** Produce Windows x86-64 GNU console executables from validated Keld IR with exact interpreter parity and one versioned runtime DLL.

**Architecture:** Add safe native ABI/runtime/lowering crates, isolate raw FFI and LLVM C API calls in adapter crates, and keep validated executable IR as the semantic authority. Extend the CLI only after the backend and differential gates pass.

**Tech Stack:** Rust 1.97, GNU Windows target, LLVM 22.1.8, `llvm-sys 221.0.1`, MinGW-w64 GCC, PowerShell bootstrap scripts.

## Global Constraints

- Target is `x86_64-w64-windows-gnu`; no MSVC fallback.
- LLVM package closure is checksummed and pinned; generated programs do not load LLVM DLLs.
- Runtime DLL is `keld_runtime_v1.dll` with import library `libkeld_runtime_v1.dll.a` and `keld_rt_v1_*` exports.
- Backend accepts `&keld_ir::Module`, re-validates it before LLVM state, and does not reconstruct lifecycle/storage policy.
- Lowering matches every current `Instruction` and `Terminator` exhaustively.
- Language faults preserve exact kind and source location at O0 and O2; internal failures are distinct.

## Tasks

1. Reconcile current documentation and record the historical bootstrap/storage/List acceptance evidence.
2. Lock LLVM 22.1.8, bootstrap checksums, and establish the ABI/runtime/adapter crate boundaries.
3. Lower `ConstInt`/`Return`, emit COFF, link with GNU GCC, and run minimal native Int programs.
4. Add scalar CFG, checked arithmetic, fault propagation, and deterministic location tables.
5. Add functions, calls, status/out-result conventions, and Phi lowering.
6. Add managed handles, structs, homes, drop slots, and literal cleanup instructions.
7. Add Text handles and all current Text operations.
8. Add direct List operations and nested managed values.
9. Add call-bounded places, projected List operations, and managed parameter modes.
10. Add custody entities, links, views, lifecycle operations, and cleanup callbacks.
11. Add semantic allocation-failure controls and the exhaustive IR surface audit.
12. Expose `build` and `run --engine native`, complete architecture/acceptance docs, and run the full Windows GNU gate.

Each task follows RED/GREEN TDD, runs focused tests before moving on, and ends
with a reviewable commit named by the approved plan. Native-1 is not accepted
until every task and the final restricted-PATH/import/full-surface gates pass.
