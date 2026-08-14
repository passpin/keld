# Keld Final Storage Correctness Design

## Goal

Close the three medium-severity correctness gaps remaining at `af18dfa` without
expanding the storage milestone: exhaustive structural-operation validation,
provenance-aware IR entity liveness, and fallible executable allocation.

## Structural operations and Views

IR validation continues to reject structural operations while any entity field
View is open. The classifier becomes an exhaustive `match` over `Instruction`
with no wildcard arm. `ListPushPlace` and `ListRemovePlace` are structural, like
their non-projected counterparts. Every existing instruction is explicitly
classified, so adding a future instruction requires a deliberate View-safety
decision at compile time.

`ReadField`, `WriteField`, `ReplaceField`, and `CloseView` remain the operations
that complete an already-open field access window. This change does not alter
the compiler's existing closed replacement transaction.

## Executable entity identity and liveness

The IR validator adds a forward CFG dataflow for executable entity identity.
Each entity-producing register introduces an identity origin. `Copy` propagates
the source origin set, and an entity `Phi` unions the origin sets from its
incoming values. The state also carries the set of retired origins.

`RetireEntity` requires every possible origin of its operand to be live, then
marks all of those origins retired. Consequently, later use through the same
register, a copied alias, or a Phi value whose possible origins overlap the
retired identity is rejected. CFG joins union retired origins, so an entity
retired on any incoming path is not considered safely live after the join.
Entity operands used by instructions and terminators must have a known origin
set disjoint from the retired set.

This is deliberately executable-IR provenance, not a second source lifecycle
analysis. Parameters and independent producers remain distinct unless IR
`Copy`/`Phi` flow relates them. Source-level may-alias refinement, lifecycle
membership, escape proof, and declared interprocedural retirement effects stay
owned by `keld-lifecycle` and `keld-storage`.

## Allocation-fault invariant

Every allocation performed while starting or executing a normal Keld program
must either be preceded by a successful fallible reservation or be eliminated.
Allocator failure is mapped to Keld `AllocationFault`; infallible standard
allocation APIs must not remain on those paths.

The confirmed paths are repaired as follows:

- `ListGet` carries `Option<Value>` directly into the allocation-free Optional
  envelope instead of creating a temporary `Box<Value>`.
- Heap Text uses an owned `String` payload. Text constants and heap Text copies
  build their payload through a fallible reservation; concatenation moves its
  already-reserved String into the runtime value without another allocation.
- Frame home-scope tables reserve before population.
- Runtime places provide fallible clone/projection construction. Read-only loan
  access borrows place metadata, mutable access temporarily moves and restores
  it, and paths that genuinely need copied metadata map reservation failure to
  `AllocationFault`.

The audit includes `keld-interpreter` and `keld-runtime`. Runtime store growth,
List growth, structural value copying, frame/call vectors, Phi staging,
constructor staging, and cleanup scratch must retain their existing fallible
reservation or fixed-capacity guarantees. Compiler/validator allocations and
explicit test-only tracing/observation allocations are outside the Keld runtime
fault contract.

## Test strategy

Tests are added before production changes and observed failing for the intended
reason:

1. Open-View validation rejects both projected List mutation instructions.
2. Direct, copied-alias, Phi-alias, and maybe-retired CFG uses are rejected by
   IR validation before `Interpreter::new` can execute them.
3. `ListGet` performs no Optional-wrapper allocation, and injected/fallible Text
   allocation paths report `AllocationFault` while successful heap Text
   constants and copies preserve values.
4. Focused IR/interpreter/runtime suites pass, followed by the full GNU workspace
   test, strict Clippy, formatting, and diff gates.

## Non-goals

No new source features, lifecycle semantics, allocator API, backend work,
performance redesign, or milestone extension is included.
