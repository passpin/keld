# Keld LLVM Native-1: Windows x86-64 Parity

Native-1 adds an ahead-of-time LLVM backend for `x86_64-w64-windows-gnu`.
Validated executable IR is the sole semantic authority: the backend does not
infer ownership, lifecycle, loan, cleanup, or storage policy. Generated
programs are standalone console executables accompanied by one versioned
`keld_runtime_v1.dll`.

## Frozen boundary

- LLVM is pinned to 22.1.8 (`llvm-sys = 221.0.1`, strict-versioning,
  force-dynamic) and emits COFF for the GNU Windows x86-64 triple.
- GNU `gcc.exe` is invoked through `std::process::Command`; `KELD_MINGW_GCC`
  overrides `PATH`, and `gcc -dumpmachine` must identify x86-64 MinGW. There is
  no MSVC fallback.
- The runtime DLL exports only the versioned `keld_rt_v1_*` C ABI. No Rust
  layout, panic, allocator, trait-object, or unwinding ABI crosses the boundary.
- Fixed records (`KeldHandle`, `KeldEntity`, `KeldLink`, `KeldLifecycle`,
  `KeldValue`, and `KeldFault`), status values, fault IDs, and deterministic
  source locations are frozen in the native ABI crate.
- The safe runtime core owns Text, List, struct values, generational handles,
  copy/drop, and synchronous projected-place resolution. Raw FFI and LLVM C
  API calls are isolated in narrow adapter crates.
- The backend re-validates `keld_ir::Module` before creating LLVM state,
  lowers every instruction and terminator exhaustively, and never imports
  lifecycle/storage/flow policy crates directly.

## Runtime and CLI behavior

Generated functions use status/out-result calling conventions and call-bounded
`KeldPlace` descriptors. Language faults are uncatchable, record only the first
failure, print the existing interpreter fault format, and exit 2. Internal
failures exit 70. Successful `main` prints its Int and exits 0. `keld build`
stages a non-overwriting executable, and `keld run --engine native` uses O0 in a
unique temporary directory while forwarding child streams and status.

Native-1 is limited to one-source-file Windows x86-64 AOT executables. JIT,
Wasm, other targets, PDB/CodeView, installers, incremental caching, ABI v2,
and new language syntax are deferred.
