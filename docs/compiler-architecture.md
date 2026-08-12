# Keld Compiler Architecture

## Pipeline and stop rule

The bootstrap compiler is a one-way pipeline:

```text
bytes
  -> normalized SourceText
  -> lossless tokens and ParsedFile
  -> TypedModule
  -> FlowModule
  -> VerifiedFlowModule
  -> VerifiedStorageModule
  -> executable Module
  -> IR validation
  -> Interpreter
```

`keld-cli::compile_source` owns orchestration. Each stage receives only the
successful output of the previous stage. Diagnostics are sorted, the pipeline
stops after the first stage that reports an error, and no invalid input is
lowered or executed. `check` runs every static stage through IR validation.
`run` constructs an interpreter only from validated IR.

## Crate ownership

| Crate | Public input | Public output | Forbidden responsibility |
|---|---|---|---|
| `keld-source` | raw bytes, source IDs, paths, byte ranges | normalized UTF-8 source, spans, source maps, diagnostics | tokens, grammar, types, or execution |
| `keld-numeric` | literal text, `i64` operands, numeric operation | parsed literal classification or checked value/fault | source traversal, diagnostics, or target-dependent arithmetic |
| `keld-syntax` | `SourceText`, then `Lexed` | lossless tokens, immutable syntax tree, typed AST projections | name resolution, types, lifecycle proofs, or recovery by later stages |
| `keld-semantics` | source plus successful `ParsedFile` | definitions, types, typed HIR, entrypoint contract | CFG scheduling, entity liveness, cleanup, or executable operations |
| `keld-flow` | `TypedModule` | CFG with explicit evaluation order, locals, and provenance sites | deciding lifecycle safety, runtime storage, or backend layout |
| `keld-lifecycle` | `FlowModule` | function effects, liveness facts, proof annotations, verified flow | executable instruction selection, slot mutation, or interpretation |
| `keld-storage` | `VerifiedFlowModule` | `VerifiedStorageModule` with Home/value classifications, call effects, `MaybeLive` flags, transfer/drop actions, and per-exit cleanup actions | entity liveness, runtime mutation, or backend policy |
| `keld-ir` | `VerifiedStorageModule` | executable IR with explicit move/loan/install/drop and checked List operations, stable textual dump, and validation diagnostics | source recovery, runtime policy, or executing unvalidated modules |
| `keld-runtime` | opaque branded identities, lifecycle IDs, type IDs, payloads | segmented entity store, weak links, deterministic cleanup | source-language types, IR, diagnostics, or user-visible control flow |
| `keld-interpreter` | validated executable `Module` | explicit-frame execution result or classified fault | parsing, static recovery, accepting invalid IR, or language extensions |
| `keld-cli` | OS arguments, selected file bytes | `Compilation`, CLI output, documented exit status | new syntax, type, lifecycle, numeric, or runtime semantics |

The dependency graph is acyclic. `keld-source` and `keld-numeric` are semantic
leaves. The runtime is source-independent. The interpreter is the first crate
where validated executable IR and runtime storage meet.

## Verified storage and executable boundaries

The storage verifier is the sole owner of executable managed-storage decisions:

```text
VerifiedStorageModule
  = verified lifecycle flow
  + call effects
  + Home/value classifications
  + MaybeLive flags
  + per-operation transfer/drop actions
  + per-exit cleanup actions

keld-ir
  = explicit move/loan/install/drop
  + explicit checked List operations
  + no unresolved cleanup or alias decision
```

Uniform cleanup paths lower to direct reverse-order drops. A hidden per-scope
order tracker is emitted only for a scope whose successful-initialization order
diverges at a CFG join; `MaybeLive` uses conditional drop metadata. Direct drops,
tracked cleanup, normal scope exits, and returns therefore share one executable
cleanup contract. `List[T]` and `Text` remain single-home, aggregate cleanup is
recursive and iterative, `List()` has no element buffer, and bounds/capacity
checks execute in every build.

The interpreter milestone verifies nested `List[List[Int]]` and
`List[List[Text]]` transfer, copy, projected loans and replacements, removal,
clearing, bounds, reservation retry, and cleanup. `Map`, `Set`, `Slice`,
iterators, `for`, substrings, and Text integer indexing remain outside the
bootstrap surface. Native and WebAssembly differential execution is a later
LLVM backend gate and is not claimed by this interpreter milestone.

## Diagnostic ownership

| Codes | Owner and meaning |
|---|---|
| `KLD0001` | source decoding, size/source-ID limits, or CLI file-read failure |
| `KLD0002` | lexical failure |
| `KLD0003` | parser/recovery failure |
| `KLD0004` | parsed feature outside the bootstrap subset |
| `KLD0101`-`KLD0199` | declarations, names, types, entrypoint, layout, returns, constants |
| `KLD1001`-`KLD1009` | entity liveness, escape, lifecycle order, aliases, effects |
| `KLD2001`-`KLD2009` | single-home transfers, homes, loans, reservations, and storage effects |
| `KLD9001`-`KLD9005` | invalid executable views, registers, CFG, lifecycles, or module shape |
| `KLD9006` | invalid executable storage or container IR: homes, cleanup, projected places, Optional values, List operations, or capacity operations |

Flow lowering and the runtime do not create Keld diagnostics. Runtime store
errors and interpreter faults are classified separately from static errors.
The CLI renders diagnostics but does not reinterpret their meaning.

## Runtime slot transitions

Each store owns a process-unique brand. An entity identity contains that brand,
a slot, a generation, and an exact runtime type. Links retain the same identity
coordinates and resolve only while every component still matches a live slot.

```text
Empty(g)
  -> Live(g, type, lifecycle, adoption, payload)
  -> Dying(g, type, lifecycle, adoption, payload)
  -> cleanup(payload)
  -> Empty(g + 1)

If g cannot advance:
Dying(g) -> cleanup(payload) -> Retired permanently
```

A segment contributes 64 stable slots. Empty reusable slots and permanently
retired slots are separate. Generation is advanced before a slot can be reused,
so an old link never resolves to a replacement entity.

Every lifecycle has an append-only adoption log. `keep` clears the old entry
and appends the identity to a strict active ancestor. Ending a lifecycle first
checks all membership, marks every remaining member dying, and then cleans them
in reverse adoption order. A parent cannot end while it has an active child.

## Compiler-bug boundary

`keld-ir::validate` is mandatory after lowering and again when an interpreter is
constructed. A `KLD9001`-`KLD9006` diagnostic from compiler-produced IR is a
compiler bug, not a user program error. In particular, a field `View` must close
before a structural instruction or terminator, and executable storage/container
IR must contain no unresolved cleanup or alias decision.

With validated IR, `ForeignIdentity`, `StaleEntity`, `InvalidLifecycle`,
`NonAncestorKeep`, `RootEnd`, and `ActiveChild` store failures indicate a
compiler/interpreter bug. User-visible runtime faults are checked arithmetic,
division by zero, invalid shifts, and allocation failure. Invalid IR and broken
store invariants are never converted into ordinary Keld runtime faults.
