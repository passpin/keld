# Keld

Keld is an experimental statically typed language centered on compile-time
Custody Ledger proofs for entity lifecycles, aliases, links, and deterministic
cleanup. This checkout contains the completed interpreter/storage milestone;
the approved LLVM Native-1 milestone is documented but is not available yet.

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

## CLI

```powershell
$env:CARGO_BUILD_TARGET='x86_64-pc-windows-gnu'
cargo run -p keld-cli -- check crates/keld-cli/tests/fixtures/cyclic_graph.keld
cargo run -p keld-cli -- run --engine interpreter crates/keld-cli/tests/fixtures/cyclic_graph.keld
cargo run -p keld-cli -- dump-ir crates/keld-cli/tests/fixtures/keep_survives.keld
```

The currently implemented command shapes are:

```text
keld check <file>
keld run --engine interpreter <file>
keld dump-ir <file>
```

`check` is silent on success. Interpreter `run` prints the returned `Int` and
a newline. Exit statuses are 0 for success, 1 for static/read failure, 2 for a
Keld runtime fault, and 64 for command misuse. `dump-ir` writes the validated
executable IR and is useful when inspecting the compiler/interpreter contract.

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
are deferred. Resources, packages, WebAssembly, transpilation, and LLVM native
code generation are also deferred; Native-1 is approved, but its `build` and
`run --engine native` commands are not implemented in this checkout.

Architecture and verification evidence are in
[`docs/compiler-architecture.md`](docs/compiler-architecture.md) and
[`docs/milestone-acceptance.md`](docs/milestone-acceptance.md). Normative value
and List rules are in [`docs/spec/storage-values.md`](docs/spec/storage-values.md)
and numeric rules are in [`docs/spec/numeric-safety.md`](docs/spec/numeric-safety.md).
