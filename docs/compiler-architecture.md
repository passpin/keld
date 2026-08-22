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
  -> Interpreter or Native-1 LLVM lowering
```

`keld-cli::compile_source` owns orchestration. Each stage receives only the
successful output of the previous stage. Diagnostics are sorted, the pipeline
stops after the first stage that reports an error, and no invalid input is
lowered or executed. `check` runs every static stage through IR validation.
`run` constructs an interpreter only from validated IR.

The native path starts at the same validated `keld_ir::Module`:

```text
validated Module + SourceMetadata
  -> keld-native-backend::build_executable
  -> keld-native-llvm (LLVM C API adapter)
  -> COFF object
  -> MinGW GCC
  -> program.exe + keld_runtime_v1.dll
```

The backend re-validates the module before creating LLVM state. It may lower
registers, blocks, and explicit runtime instructions, but it does not inspect
flow, lifecycle, storage, or interpreter state to infer policy. Raw pointers
and LLVM C API calls are confined to `keld-native-ffi` and `keld-native-llvm`;
the ABI records and runtime core are safe Rust boundaries.

Native-1's test-only allocation controls use the same validated executable IR
on both engines. `keld-ir::AllocationSchedule` orders coordinates by
`(FunctionId, IrBlockId, instruction_index)` and the ABI phase/ordinal encoding
derives frozen site IDs. The test DLL records normalized status, fault, and
semantic allocation events; the differential harness exercises every reachable
single-failure frontier (including preferred-then-exact List growth) at O0 and
O2. Production generated programs do not load the test DLL or LLVM.

## Crate ownership

| Crate | Public input | Public output | Forbidden responsibility |
|---|---|---|---|
| `keld-source` | raw bytes, source IDs, paths, byte ranges | normalized UTF-8 source, spans, source maps, diagnostics | tokens, grammar, types, or execution |
| `keld-numeric` | literal text, `i64` operands, numeric operation | parsed literal classification or checked value/fault | source traversal, diagnostics, or target-dependent arithmetic |
| `keld-syntax` | `SourceText`, then `Lexed` | lossless tokens, immutable syntax tree, typed AST projections | name resolution, types, lifecycle proofs, or recovery by later stages |
| `keld-semantics` | source plus successful `ParsedFile` | definitions, types, typed HIR, entrypoint contract | CFG scheduling, entity liveness, cleanup, or executable operations |
| `keld-flow` | `TypedModule` | CFG with explicit evaluation order, locals, and provenance sites | deciding lifecycle safety, runtime storage, or backend layout |
| `keld-lifecycle` | `FlowModule` | function retirement effects, liveness/proof annotations, immutable per-operation entity provenance and alias facts, verified flow | executable instruction selection, slot mutation, or interpretation |
| `keld-storage` | `VerifiedFlowModule` | `VerifiedStorageModule` with provenance-backed access paths, Home/value classifications, pending-call effects, `MaybeLive` flags, transfer/drop actions, and per-exit cleanup actions | entity liveness, runtime mutation, or backend policy |
| `keld-ir` | `VerifiedStorageModule` | executable IR with explicit move/loan/install/drop, transactional entity-field replacement, checked List operations, stable textual dump, and exhaustive role/liveness validation | source recovery, runtime policy, or executing unvalidated modules |
| `keld-runtime` | opaque branded identities, lifecycle IDs, type IDs, payloads | segmented entity store, weak links, deterministic cleanup | source-language types, IR, diagnostics, or user-visible control flow |
| `keld-interpreter` | validated executable `Module` | explicit-frame execution result or classified fault | parsing, static recovery, accepting invalid IR, or language extensions |
| `keld-native-abi` | fixed C-layout records | versioned value/entity/lifecycle ABI | Rust layout or unwinding across the DLL |
| `keld-native-runtime` | ABI values and fallible operations | generational handles, Text/List/struct/entity storage | source-language policy or IR reconstruction |
| `keld-native-ffi` | raw C ABI calls | `keld_runtime_v1.dll` exports and status/fault protocol | LLVM lowering or lifecycle analysis |
| `keld-native-ffi-test` | the same adapter with the test-controls feature | `keld_runtime_v1_test.dll` allocation observations | production failure injection or semantic behavior |
| `keld-native-llvm` | safe lowering requests | verified LLVM module/object emission | runtime ownership or source recovery |
| `keld-native-backend` | validated `keld_ir::Module` and source metadata | linked Windows executable | querying verifier/interpreter internals or inventing cleanup |
| `keld-cli` | OS arguments, selected file bytes | `Compilation`, CLI output, documented exit status | new syntax, type, lifecycle, numeric, or runtime semantics |

The dependency graph is acyclic. `keld-source` and `keld-numeric` are semantic
leaves. The runtime is source-independent. The interpreter is the first crate
where validated executable IR and runtime storage meet.

## Verified storage and executable boundaries

The storage verifier is the sole owner of executable managed-storage decisions:

```text
VerifiedStorageModule
  = verified lifecycle flow
  + provenance-backed storage access paths
  + call effects, including lifecycle retirement summaries
  + Home/value classifications
  + MaybeLive flags
  + per-operation transfer/drop actions
  + per-exit cleanup actions

keld-ir
  = explicit move/loan/install/drop
  + explicit replace/close/drop transactions for managed entity fields
  + explicit checked List operations
  + no unresolved cleanup or alias decision
```

`VerifiedFlowModule` carries immutable entity provenance, equivalence,
proven-distinct, and retirement facts. Storage roots entity-backed access paths
in those lifecycle facts rather than entity local identity, and applies callee
mutation and lifecycle retirement summaries while checking pending calls.
Managed entity-field replacement lowers to an explicit replacement inside a
closed edit view followed by displaced-value cleanup. IR validation classifies
every register type, producer role, and owned-value use, so storage role and
liveness enforcement remains exhaustive as instructions evolve.

Uniform cleanup paths lower to direct reverse-order drops. A hidden per-scope
order tracker is emitted only for a scope whose successful-initialization order
diverges at a CFG join; `MaybeLive` uses conditional drop metadata. Direct drops,
tracked cleanup, normal scope exits, and returns therefore share one executable
cleanup contract. `List[T]` and `Text` remain single-home, aggregate cleanup is
recursive and iterative, `List()` has no element buffer, and bounds/capacity
checks execute in every build.

Control Flow-1 permits cyclic Flow CFGs. `keld-lifecycle` and `keld-storage` own
monotone fixed-point verification across backedges using their existing
liveness, provenance, Home, loan, and cleanup domains. Flow owns structured
loop targets and `ExitScopes`; executable IR receives only the converged result
and represents loops as ordinary validated CFG cycles. Neither the interpreter
nor the LLVM backend infers loop-specific lifecycle or storage policy.

The interpreter milestone verifies nested `List[List[Int]]` and
`List[List[Text]]` transfer, copy, projected loans and replacements, removal,
clearing, bounds, reservation retry, and cleanup. `Map`, `Set`, `Slice`,
iterators, `for`, substrings, and Text integer indexing remain outside the
bootstrap surface. Native-1 executes the covered Windows GNU fixtures through
the same validated IR. The native differential and surface-audit suites cover
all 48 current instruction variants and all 6 terminators, including the
validated-IR-only forms that source lowering normalizes away. WebAssembly and
other targets remain deferred.

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
