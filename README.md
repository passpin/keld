# Keld

Keld is an experimental statically typed language centered on compile-time
Custody Ledger proofs for entity lifecycles, aliases, links, and deterministic
cleanup. This checkout contains the interpreter/storage pipeline and the
Windows GNU LLVM Native-1 backend.

## Build and test

Rust 1.97.0, rustfmt, and clippy are pinned by `rust-toolchain.toml`. The
repository uses the GNU Windows target in CI and on the supported development
machines.

```powershell
$env:CARGO_BUILD_TARGET='x86_64-pc-windows-gnu'
cargo build --workspace
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --workspace --release
```

Native builds require the pinned LLVM prefix from `scripts/bootstrap-llvm.ps1`
and an x86-64 MinGW GNU toolchain. Activate the prefix before native commands:

```powershell
. scripts/activate-llvm.ps1
cargo build -p keld-native-ffi --release --target x86_64-pc-windows-gnu
```

## CLI

```powershell
$env:CARGO_BUILD_TARGET='x86_64-pc-windows-gnu'
cargo run -p keld-cli -- check crates/keld-cli/tests/fixtures/cyclic_graph.keld
cargo run -p keld-cli -- run --engine interpreter crates/keld-cli/tests/fixtures/cyclic_graph.keld
cargo run -p keld-cli -- run --engine native crates/keld-cli/tests/fixtures/cyclic_graph.keld
cargo run -p keld-cli -- build crates/keld-cli/tests/fixtures/cyclic_graph.keld -o cyclic_graph.exe
cargo run -p keld-cli -- dump-ir crates/keld-cli/tests/fixtures/keep_survives.keld
```

The currently implemented command shapes are:

```text
keld check <file>
keld build <source> -o <program.exe>
keld run --engine interpreter <file>
keld run --engine native <file>
keld dump-ir <file>
```

`check` is silent on success. Interpreter `run` prints the returned `Int` and
a newline. Exit statuses are 0 for success, 1 for static/read failure, 2 for a
Keld runtime fault, and 64 for command misuse. `dump-ir` writes the validated
executable IR and is useful when inspecting the compiler/interpreter/native
contract. `build` uses O2 and refuses to overwrite an executable; it places the
matching `keld_runtime_v1.dll` beside the output. `run --engine native` uses O0,
forwards the child process streams and status, and removes its private
temporary directory after the child exits. Runtime and toolchain paths can be
overridden with `KELD_RUNTIME_DLL`, `KELD_RUNTIME_IMPORT_LIBRARY`,
`KELD_LLVM_PREFIX`, `LLVM_SYS_221_PREFIX`, and `KELD_MINGW_GCC`.

Allocation-failure schedules are available only to the test-only
`keld-native-ffi-test` package. Its version-1 line records use
`site_id=<u32> phase=<name> attempt=<u64>` and are read from
`KELD_TEST_CONTROL`; normalized status, fault, and allocation observations are
written to `KELD_TEST_OBSERVATION`. Production `keld_runtime_v1.dll` ignores
these variables.

## Implemented interpreter surface

The accepted source surface currently includes:

- `Unit`, `Bool`, checked signed `Int`, immutable UTF-8 `Text`, `List[T]`, and
  allocation-free `Optional` values;
- value structs, entities, optional links, entity field reads/writes, and
  generational lifecycle identity;
- immutable `let` and replaceable `var` bindings, explicit moves through
  `take`, structural duplication through `.copy()`, normal and consuming
  parameters, and owned managed returns;
- functions, blocks, conditionals, assignments, calls, checked unary/binary
  arithmetic, boolean operators, scalar comparisons, and explicit runtime
  faults;
- lifecycle creation/end, entity allocation, link resolution with `when`,
  `keep`, `retire`, deterministic reverse cleanup, and validated executable IR;
- the complete current List operation set: construction, length, push, index
  loans, copy-get, replace, remove, try-remove, clear, reserve, try-reserve,
  nested Lists, projected places, and structural-copy cleanup; and
- Text byte length, emptiness, equality/inequality, concatenation, copies,
  moves, owned returns, struct/entity fields, and nested cleanup.

Text indexing and substring operations are intentionally not part of this
milestone.

## Explicitly deferred or rejected gates

The parser recognizes the broader Keld grammar, but the semantic gate still
rejects imports/`use`, enums, externs/foreign declarations, generics, loops,
`break`, `continue`, `match`, exceptions/typed errors, unsafe modules, and
recursion. Map, Set, Slice, iterators, Text integer indexing, and substrings
are deferred. Resources, packages, WebAssembly, transpilation, and targets
other than Windows x86-64 GNU remain deferred. Native-1 does not add new
source syntax; it lowers only validated executable IR.

Architecture and verification evidence are in
[`docs/compiler-architecture.md`](docs/compiler-architecture.md) and
[`docs/milestone-acceptance.md`](docs/milestone-acceptance.md). Normative value
and List rules are in [`docs/spec/storage-values.md`](docs/spec/storage-values.md)
and numeric rules are in [`docs/spec/numeric-safety.md`](docs/spec/numeric-safety.md).
