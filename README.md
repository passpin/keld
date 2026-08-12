# Keld

Keld is an experimental statically typed language centered on compile-time
Custody Ledger proofs for entity lifecycles, aliases, links, and deterministic
cleanup. This repository contains the first executable interpreter milestone.

## Build and test

Rust 1.97.0, rustfmt, and clippy are pinned by `rust-toolchain.toml`.

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

Only these command shapes are accepted:

```text
keld check <file>
keld run --engine interpreter <file>
keld dump-ir <file>
```

Successful `check` is silent. Successful `run` prints the returned `Int` and a
newline. Exit statuses are 0 for success, 1 for static/read failure, 2 for a
Keld runtime fault, and 64 for command misuse.

## Supported bootstrap subset

- One UTF-8 source file with `Int`, `Bool`, structs, entities, and optional
  entity links.
- Immutable `let` bindings and functions with explicit parameter and return
  types. The entrypoint is exactly `fn main() -> Int` with no parameters or
  effect clause.
- Checked integer arithmetic, division, remainder, shifts, comparisons,
  boolean operators, range-checked literals, and constant-fault rejection.
- `if`, blocks, calls, field reads, and direct entity-field mutation.
- `lifecycle`, entity construction, `when` link resolution, `keep`, `retire`,
  and function retirement effects.
- Compile-time liveness and may-alias rejection, stable executable-IR dumps,
  and deterministic cleanup on normal return.

The parser recognizes the full Keld 0.1 grammar. Its semantic gate emits
`KLD0004` for parsed out-of-scope constructs including imports, enums, `var`,
loops, `break`, `continue`, `match`, recursion, Text, List, `take`, generics,
typed errors, unsafe modules, and foreign declarations. Resources, packages,
LLVM, WebAssembly, native code generation, and transpilation are not
implemented.

Architecture and verification evidence are in
[`docs/compiler-architecture.md`](docs/compiler-architecture.md) and
[`docs/milestone-acceptance.md`](docs/milestone-acceptance.md).
