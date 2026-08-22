# Keld Language Design

Status: approved language baseline; implementation not started

Date: 2026-08-11

Last revised: 2026-08-12

Working language version: 0.1 design core

## 1. Purpose

Keld is a statically typed, high-level language for native applications whose
central feature is deterministic, memory-safe management of values and mutable
entity graphs without tracing garbage collection or reference counting.

Keld targets:

- games and simulations;
- native desktop tools;
- editors and creative applications; and
- predictable-latency servers.

Keld does not initially target kernels, device drivers, tiny embedded systems,
hard real-time control, or dynamic scripting.

The language optimizes for this order:

1. memory safety;
2. understandable source semantics and diagnostics;
3. deterministic reclamation;
4. predictable application performance; and
5. implementation feasibility.

No claim of formal proof is made by this document. The state machine and static
rules below are the basis for later preservation and progress proofs.

## 2. Locked Product Decisions

- Source files use the `.keld` extension.
- The command-line tool is `keld`.
- Types are decided at compile time, with strong local inference.
- Keld is a high-level application language, not a systems language.
- Ordinary source has no borrow operators, lifetime parameters, allocators,
  retain/release operations, slots, or generations. Whole-value storage transfer
  is the one explicit operation, written `take`.
- Entity memory uses Custody Ledger Memory with acyclic lifecycle groups.
- `List[T]`, `Text`, and aggregates containing them are single-home values.
- Default integer arithmetic is checked in every build; runtime faults never
  rely on backend undefined behavior.
- Sequential execution is the first semantic and implementation core.
- Native ahead-of-time compilation is primary.
- WebAssembly is a secondary output target.
- LLVM is the first production backend behind a backend-neutral executable IR.
- The compiler is implemented in Rust.
- An interpreter for executable IR is the semantic test oracle.
- The normative core grammar is defined separately from compiler code.

## 3. Language-Level Memory Concepts

### 3.1 Values

Values have structural identity. Examples include numbers, booleans, structs,
enums, `List[T]`, and immutable `Text` values.

The compiler may place a value inline, on the stack, in registers, or in hidden
storage. This placement is not observable in safe Keld.

Implicit-copy values duplicate normally. Managed storage values are
single-home: named transfer uses `take`, independent duplication uses `.copy()`,
and ordinary function parameters create compiler-checked call loans. List and
Text may appear inside structs and entity fields because their cleanup is
compiler-generated, non-throwing, and non-reentrant.

External resources such as files and sockets remain deferred. Keld 0.1 does not
permit them inside entity fields or provide user-defined destruction. The
normative value-storage rules are in
[`docs/spec/storage-values.md`](../../spec/storage-values.md).

### 3.2 Entities

An `entity` has stable identity and mutable state.

```keld
entity Enemy {
    health: Int
    target: link Enemy?
}
```

Creating an entity places it in the current lifecycle:

```keld
lifecycle level {
    let boss = Enemy(health: 100, target: none)
}
```

An ordinary local entity binding is a compiler-proven live `EntityRef`. It may
be aliased freely within its proven lifetime. The binding is not a raw pointer,
is not the entity's cleanup authority, and does not keep the entity alive.

All direct aliases carry the same hidden identity provenance. Retiring through
one alias invalidates every direct alias with that provenance. The compiler does
not treat aliases as independent owners.

Direct entity references may exist in locals, parameters, and compiler-bounded
temporary results. Persistent fields and containers must use `link`.

At runtime an `EntityRef` contains or can reconstruct the entity's store brand,
slot, and generation. The compiler may replace it with a direct address only
inside a proven access window.

### 3.3 Links

A `link T` is a copyable, non-owning relationship to an entity of type `T`.
A `link T?` may additionally begin with no target.

The `?` applies to the link value: `link Enemy?` means `(link Enemy)?`, not a
link to an optional entity. A non-optional `link Enemy` must contain an identity,
but resolving it is still conditional because that identity may have retired.
`link Enemy?` additionally permits the stored value `none`. Both forms use
`when` for resolution.

Links never extend an entity's lifetime. Arbitrary link cycles are permitted.
Resolving a link may fail because its target was retired.

```keld
boss.target = player

when boss.target as target {
    target.health -= 10
}
```

`when link-expression as name` validates the expression exactly once. The bound
name is an `EntityRef` valid only within the successful branch. Individual
payload accesses through that reference create shorter-lived hidden views.
Failure enters an optional `else` branch when present.

```keld
when boss.target as target {
    attack(target)
} else {
    choose_new_target(boss)
}
```

Keld does not provide an unchecked force-dereference operation in safe code.

### 3.4 Lifecycles

A `lifecycle` is a semantic cleanup group.

```keld
lifecycle game {
    let player = Enemy(health: 100, target: none)

    lifecycle level {
        let enemy = Enemy(health: 30, target: player)
        keep enemy in game
    }
}
```

Rules:

- The program and each task have an implicit root lifecycle.
- Explicit lifecycles nest into an acyclic tree.
- Every live entity belongs to exactly one lifecycle.
- A new entity joins the innermost active lifecycle.
- Ending a lifecycle deterministically retires all remaining members.
- Child lifecycles end before their parent.
- `keep entity in lifecycle` moves an entity to a strict named ancestor
  lifecycle.
- `retire entity` ends one entity early.

Keeping an entity removes its old membership and appends it as the newest
adoption in the target ancestor. Reverse-adoption cleanup order is therefore
defined by the time an entity most recently entered that lifecycle.

A function call does not create a lifecycle implicitly. The callee inherits the
call site's current lifecycle as a hidden argument, so an entity created by an
ordinary function joins the caller's current lifecycle. An explicit lifecycle
inside the function still has its own lexical extent.

`keep` only extends lifetime. Moving an entity into a shorter-lived or unrelated
lifecycle is rejected. This restriction makes the operation locally
understandable and keeps live-reference proofs monotonic.

Moving an entity between lifecycles does not move its allocation or change its
identity. Existing links remain valid.

### 3.5 Retirement

```keld
retire enemy
```

After `retire enemy`, direct use of `enemy` is a compile-time error. Links to the
entity remain ordinary values, but later resolution produces absence.

Retirement is deterministic. There is no background collector and no reference
count whose value controls retirement.

## 4. Custody Ledger Runtime Model

The concepts in this section are implementation semantics. They are not
ordinary source-language features.

### 4.1 Store

Each sequential task owns a hidden store. A store contains:

- stable segmented slot storage;
- a generation value for each reusable slot;
- lifecycle membership lists;
- type-directed cleanup metadata; and
- free and permanently retired slot lists.

An entity allocation does not move while live.

### 4.2 Slot states

Each slot is in exactly one state:

```text
Empty(generation)
Live(generation, type, lifecycle, payload)
Dying(generation, type, lifecycle, payload)
Retired
```

Permitted state transitions are:

```text
Empty(g) -> Live(g, type, lifecycle, payload)
Live(g, type, lifecycle, payload) -> Dying(g, type, lifecycle, payload)
Dying(g, type, lifecycle, payload) -> Empty(g + 1)
Dying(MAX, type, lifecycle, payload) -> Retired
```

No other transition is valid. A generation never wraps. A slot whose generation
is exhausted is permanently retired.

### 4.3 Runtime link representation

The conceptual link representation is:

```text
Link = (store_brand, slot_index, generation, expected_type)
```

An implementation may erase `expected_type` when static layout and module
metadata prove it redundant.

A link resolves only when:

- the store brand matches;
- the slot exists;
- the slot is `Live`;
- the generation matches; and
- the stored type is compatible with the requested type.

Otherwise resolution yields absence. It never yields a dangling address.

### 4.4 Hidden access windows

The compiler lowers entity payload access into bounded read or edit windows.
The runtime object created inside a window is a `View`, not an `EntityRef`.

- A read window permits any number of reads.
- An edit window permits reads and mutations within the current sequential task.
- Aliasing inside one edit window is permitted.
- Structural operations cannot execute while either window is active.
- A `View` is a direct payload address and cannot escape its window.
- An `EntityRef` may span multiple windows while its static live proof remains
  valid.
- Local sequential windows normally lower to no runtime synchronization.

Structural operations are allocation, retirement, lifecycle movement, lifecycle
destruction, and any operation that may re-enter the same store structurally.

The compiler chooses the smallest practical window from typed control flow. A
structural operation ends prior windows. Later payload access may begin a new
window only from an `EntityRef` whose live proof survived that operation.

Structural effects update proofs as follows:

- allocation preserves existing live proofs;
- `keep` preserves proofs because it only extends lifetime;
- retiring an exact entity invalidates every reference that may alias it;
- ending a lifecycle invalidates every reference proven to belong to it or a
  descendant; and
- an opaque broad-retirement effect invalidates every compatible reference in
  the affected store.

An invalidated `EntityRef` cannot silently revalidate. The program must resolve a
`link` again with `when` to obtain a new proof.

### 4.5 Entity retirement algorithm

Retiring one entity performs these steps:

1. Verify that the entity is live and no access window is active.
2. Remove it from its lifecycle membership list.
3. Change its slot from `Live` to `Dying`.
4. Run compiler-generated field cleanup under restricted cleanup rules.
5. Advance the generation.
6. Return the slot to the free list, or permanently retire it at exhaustion.

Changing to `Dying` before cleanup ensures that all links stop resolving before
cleanup begins.

### 4.6 Lifecycle destruction algorithm

Ending a lifecycle performs two phases:

1. Mark phase: mark every remaining member `Dying`. All links to those members
   immediately stop resolving.
2. Cleanup phase: clean members in deterministic reverse-adoption order, advance
   generations, and release or permanently retire their slots.

Descendant lifecycles are destroyed deepest-first before their parent. Entities
previously kept in an ancestor are no longer members and survive.

Cleanup code cannot create entities, resolve links, move lifecycle membership,
retire another entity, call user or foreign code, or re-enter the store. Keld
0.1 has no arbitrary user-defined entity destructor and forbids external
resource-owning entity fields. Entity cleanup recursively releases links and
compiler-managed List, Text, and aggregate storage without callbacks.

A later resource design may add trusted, non-reentrant cleanup intrinsics. Such
intrinsics are not part of Keld 0.1 and cannot be assumed by its compiler or
runtime.

## 5. Safety Invariants

A conforming safe implementation must maintain all of these invariants:

1. Every live entity occupies exactly one live slot.
2. Every live entity belongs to exactly one active lifecycle.
3. Lifecycle parentage is acyclic.
4. Slot generations are monotonic and never wrap.
5. A link resolves only to the same live slot generation it originally named.
6. An `EntityRef` outside an access window carries identity and proof, never an
   exposed payload address.
7. A `View` can be created only from a `Live` proof and cannot outlive its hidden
   access window.
8. Structural store operations execute with no active views.
9. An invalidated, retired, or out-of-scope `EntityRef` cannot open a view.
10. Ending a lifecycle retires every remaining member exactly once.
11. Normal error exits execute the same lifecycle cleanup as normal control
    flow.
12. Safe code cannot construct a raw address, counterfeit link, or store brand.
13. In the sequential core, all mutation occurs on the owning task.
14. Every initialized single-home value has exactly one cleanup home.
15. A moved or uninitialized place cannot be read, loaned, copied, or moved.
16. Incompatible loans and transfers cannot overlap at a call.
17. Every accepted numeric operation either returns its specified value or
    reaches its specified Keld fault before executing an invalid target
    operation.

Consequences:

- use-after-free through direct bindings is rejected statically;
- stale persistent relationships resolve to absence;
- double retirement is rejected statically or trapped as a compiler/runtime bug;
- link cycles do not retain entities;
- entity reclamation is deterministic; and
- single-home storage cannot be aliased, implicitly copied, or destroyed twice;
- integer edge behavior is identical across debug and optimized builds; and
- unsynchronized cross-task mutation is impossible in the sequential core.

## 6. Static Semantics

### 6.1 Hidden semantic forms

The compiler may reason with the following hidden forms:

```text
EntityRef[T, lifecycle, proof, provenance]
View[T, access]
Link[T, optionality]
Lifecycle[identity, parent, state]
Access[store, mode, extent]
Home[T, place, state]
Loan[place, effect, call]
OwnedTemporary[T]
```

These forms appear in compiler IR and diagnostics, never as source annotations.
`Home` state is `Empty(reason)`, `Live`, or `MaybeLive`; empty reasons include
`Uninitialized` and `Moved`. A `Loan` is bounded to one call or immediate built-in
operation. `OwnedTemporary` has cleanup responsibility until it transfers into a
new home or consuming parameter.

### 6.2 Lifecycle order

Let `L1 < L2` mean that `L2` is a strict ancestor of `L1` and therefore outlives
`L1`.

`keep x in L2` is valid only when:

- `x` is live in `L1`;
- `L2` is active;
- `L1 < L2`;
- both lifecycles belong to the same hidden store; and
- no access window is active at the operation.

### 6.3 Flow-sensitive entity state

For each `EntityRef`, typed control flow tracks one of:

```text
Live(lifecycle_fact, provenance)
Invalidated(cause)
Retired
OutOfScope
```

`lifecycle_fact` is either `Known(lifecycle)` or `Dynamic`. Fresh allocations
and their direct aliases are known. Entity parameters and references produced by
link resolution are dynamic unless their origin is otherwise proven. Dynamic
references may be read, passed, or retired, but `keep` requires a known current
lifecycle so the strict-ancestor relation can be proved at compile time.

Only `Live` can open a `View` for field access or be passed to a function that
requires a live entity. Resolving a link dynamically creates a new scoped `Live`
reference in the successful branch.

Provenance is an SSA identity or a conservative set of possible identities.
The verifier uses these rules:

- Copying an `EntityRef` creates a must-alias provenance.
- Two simultaneously live entities from distinct allocation operations are
  must-distinct.
- Independently supplied entity parameters may alias.
- Independent link resolutions may alias, even when the link expressions differ.
- In the true branch of `a != b`, the references are distinct. For entities,
  `==` and `!=` compare stable identity, not field values.

`retire x` changes `x` and all must-alias references to `Retired`. It changes
every remaining may-alias reference to `Invalidated`. An invalidated reference is
safe to discard but cannot be accessed, returned, or retired. The program must
resolve a persistent link again if it needs a new live proof.

Because `EntityRef` values cannot enter persistent fields or containers, the
alias set remains bounded by typed local flow and function summaries rather than
general heap analysis.

At control-flow joins, the state is the least permissive state valid on every
incoming edge. For example, an entity retired on only one branch is not directly
usable after the join.

### 6.4 Direct-reference escape checking

The compiler infers the provenance of `EntityRef` results:

- created in the caller's active lifecycle;
- derived from a specific entity parameter.

Public module metadata records provenance summaries without exposing lifetime
syntax. A function cannot return or store an `EntityRef` when no summary can
prove its target live at every caller. The programmer must return a `link`
instead.

An `EntityRef` obtained by resolving a link cannot escape its `when` block or be
returned from the function. A public API that finds an existing entity through a
persistent relationship returns `link T` or `link T?`, and its caller resolves
that link in its own scope.

Direct references cannot be stored in entity fields, heap containers, globals,
closures that outlive the current scope, or foreign state. Those locations use
links.

### 6.5 Function effects

Typed functions carry inferred semantic effects. Initial internal effects are:

```text
pure
reads(parameter set)
edits(parameter set)
structural(parameter set)
takes(parameter set)
allocates_storage
allocates_entity(current lifecycle)
retires(parameter)
retires_any(type, store)
io
raises(error set)
unsafe
```

Local access and allocation effects are inferred. Public declarations display
application-visible effects that can change control flow or liveness. Full
summaries are serialized in compiled module metadata.

For List and Text parameters, `reads`, `edits`, and `structural` describe
call-scoped loans. `takes` corresponds to a declared `take` parameter. The
compiler checks the complete call argument set for overlapping loans, transfers,
and entity retirement before entering the callee. An entity-field loan carries
identity provenance and a field path, never a payload address across the call.

A public function that can end the lifetime of an entity parameter must
declare that semantic effect:

```keld
fn remove(enemy: Enemy) retires enemy
```

Calling `remove(enemy)` invalidates `enemy` and all of its known direct aliases.
Keld 0.1 does not permit a public function to retire an entity parameter without
this declaration. Lifecycle extension with `keep` remains lexical in Keld 0.1
and cannot be hidden inside an ordinary function call.

A function that may resolve internal links and retire an entity not named by a
direct parameter must declare a broad retirement effect:

```keld
fn sweep(world: World) retires any Enemy
```

After `sweep(world)`, every live `Enemy` reference in the same hidden store is
`Invalidated`. References can be reacquired from links. The broad effect is
deliberately visible because it creates a compile-time liveness barrier.

Inside a function, entity parameters are assumed to may-alias unless refined by
an identity comparison. Therefore this function is rejected:

```keld
fn invalid(a: Enemy, b: Enemy) retires a {
    retire a
    b.health = 0
}
```

`b` is invalidated by `retire a`. The edit is accepted inside an `a != b` branch.
When dynamic dispatch is added after Keld 0.1, it must use the union of every
possible implementation's retirement effects.

### 6.6 Normative alias examples

This function is valid because the identity comparison establishes the only
branch in which `b` is used after retiring `a`:

```keld
fn remove_then_edit(a: Enemy, b: Enemy) retires a {
    if a != b {
        retire a
        b.health = 0
    } else {
        retire a
    }
}
```

This sequence is invalid because the broad retirement effect removes the live
proof for `selected`:

```keld
when selected_link as selected {
    sweep(world)
    selected.health = 0 // KLD1008
}
```

The repair is to end the old reference scope and resolve the persistent link
after the structural call:

```keld
sweep(world)
when selected_link as selected {
    selected.health = 0
}
```

The backend receives only operations already accepted by lifecycle and effect
verification.

## 7. Type System

Keld uses static, strong typing with bidirectional local inference.

Keld 0.1 core type categories:

- primitives: `Bool`, `I8` through `I64`, `U8` through `U64`, `F32`, and `F64`;
- platform-independent aliases: `Int` is `I64` and `UInt` is `U64`;
- managed storage: single-home `List[T]` and immutable single-home `Text`;
- value aggregates: `struct`;
- algebraic variants: `enum`;
- identity-bearing records: `entity`;
- persistent identity relationships: `link T` and `link T?`;
- absence: `T?`.

Named functions can be called but are not first-class values. Function types,
closures, and interfaces are planned language features but remain reserved and
ungrammatical in Keld 0.1.

Rules:

- Local bindings infer types when the initializer is sufficient.
- Function parameters and non-`Unit` return types are explicit; omitting a return
  clause means `Unit`.
- There is no implicit null.
- Pattern matching is exhaustive.
- Numeric widening is permitted only in a typed conversion context when every
  source value is representable. Differently typed non-literal operands do not
  trigger implicit integer promotion.
- Narrowing is explicit and checked unless inside an unsafe boundary.
- Dynamic typing is absent from the initial language.

Default integer arithmetic, conversion, shift, bounds, and capacity behavior is
defined by [`docs/spec/numeric-safety.md`](../../spec/numeric-safety.md). Map,
Set, Slice, iterator, and `for` semantics remain deferred until List passes its
storage-model verification milestone.

Generics use a hybrid compilation model:

- entity and link representations compile uniformly when layout is known
  independently of the type argument;
- value layouts specialize when physical representation requires it; and
- optimized builds may specialize measured hot instantiations without changing
  source or module ABI.

## 8. Errors and Control Flow

Recoverable errors use typed error effects:

```keld
fn load(path: Path) -> Document raises IoError, ParseError
```

Callers handle or propagate declared errors:

```keld
try {
    let document = load(path)
    show(document)
} handle IoError as error {
    show(error.message)
}
```

Error effects lower to explicit tagged control flow. Runtime stack unwinding is
not required. Every exit edge receives compiler-generated lifecycle cleanup.
Future lexical resource types must define their own non-throwing exit operation
before this rule is extended to them.

Safe-operation faults are non-catchable termination. The initial fault kinds
cover arithmetic overflow, integer division by zero, invalid shifts,
out-of-range conversions, bounds, capacity, and allocation. Their
conditions are deterministic for fixed inputs except that allocation depends on
target resource availability. A fault reports its kind and source location,
remains memory safe, and does not promise user cleanup. Expected failure uses an
optional checked operation or a typed error-producing API. Destructors cannot
fail.

## 9. Concurrency Boundary

Concurrency is excluded from the first formal core, but these rules reserve a
compatible design:

- Each task owns its ordinary lifecycle state.
- Values cross tasks by copy or explicit transfer.
- Links do not cross task boundaries by default because their hidden store brand
  is task-local.
- Mutable shared entities require a future explicit `shared lifecycle`.
- Shared access will enforce multiple readers or one editor.
- No access window may cross suspension, blocking foreign calls, or task yield.
- Detached tasks require explicit runtime custody; structured tasks are the
  default.

No shared lifecycle, asynchronous function, or scheduler behavior is normative
for Keld 0.1.

## 10. Unsafe and Foreign Boundaries

Unsafe facilities are isolated to modules declared `unsafe module`.

Safe modules cannot:

- create or dereference raw addresses;
- counterfeit a link or store brand;
- bypass link validation;
- invoke an unsafe function directly; or
- retain an `EntityRef` or `View` in foreign state.

An unsafe module may declare C-ABI foreign functions and implementation-specific
raw address types. Its safe exported functions must re-establish all Keld
invariants before returning.

Foreign integration follows two lifetime forms:

- call-bounded access: a foreign call receives an address valid only for that
  call, and the compiler keeps the entity access window active; and
- persistent identity: foreign state receives an opaque validated handle backed
  by a Keld link, never a direct payload address.

A foreign call that may block, suspend, call back into Keld, or mutate Keld
storage must declare that behavior. Such a call cannot occur inside an entity
access window. Unsafe code can violate memory safety; the language makes that
boundary explicit and auditable.

## 11. Diagnostics

Diagnostics lead with the source-level rule, then show hidden implementation
terms only as optional detail.

Required diagnostic families:

### KLD1001: entity used after retirement

```text
error[KLD1001]: `enemy` is no longer live
  `retire enemy` ended it on the previous line
  persistent relationships should store `link Enemy`
```

### KLD1002: direct entity reference escapes

```text
error[KLD1002]: a direct `Enemy` reference cannot be stored here
  this location may outlive the enemy's lifecycle
  store `link Enemy` and handle possible absence when resolving it
```

### KLD1003: lifecycle may end on one path

```text
error[KLD1003]: `enemy` is not live on every path reaching this use
  it is retired in the `if` branch
```

### KLD1004: invalid lifecycle extension

```text
error[KLD1004]: `level` does not outlive the entity's current lifecycle
  `keep` may move an entity only to an active ancestor lifecycle
```

### KLD1005: unchecked link access

```text
error[KLD1005]: this link may refer to a retired entity
  resolve it with `when target as value { use(value) }`
```

### KLD1006: structural operation during access

```text
error[KLD1006]: cannot retire an entity while this entity access is active
  the active access begins here and is last used here
```

### KLD1007: forbidden cleanup operation

```text
error[KLD1007]: cleanup cannot create, resolve, keep, or retire entities
```

### KLD1008: live proof invalidated by a possible alias

```text
error[KLD1008]: `selected` may have been retired by this operation
  `sweep` can retire any `Enemy` in the current store
  resolve a persistent link after the call to obtain a new live reference
```

### KLD1009: invalid retirement effect declaration

```text
error[KLD1009]: `remove` retires parameter `enemy` but does not declare it
  add `retires enemy` to the function signature
```

The same family reports a declared retirement target that the function cannot
retire. The diagnostic points to the clause and removes it as the concrete
repair.

Diagnostics must include the operation that created the restriction, the use
that violates it, and one concrete repair when one is mechanically known.

## 12. Normative Language Specifications

Keld syntax is defined by two normative files:

- [`docs/spec/grammar.md`](../../spec/grammar.md) defines encoding, tokens,
  statement termination, precedence, and semantic parsing restrictions.
- [`docs/spec/keld.ebnf`](../../spec/keld.ebnf) defines the Keld 0.1 core
  productions.

Three additional normative files define operations whose safety cannot be
expressed by grammar:

- [`docs/spec/numeric-safety.md`](../../spec/numeric-safety.md) defines numeric
  types, arithmetic, sizes, faults, and backend obligations.
- [`docs/spec/storage-values.md`](../../spec/storage-values.md) defines
  single-home values, `take`, loans, fields, List, and Text.
- [`docs/spec/control-flow.md`](../../spec/control-flow.md) defines accepted
  `while`, `break`, and `continue` semantics, cyclic joins, and structured exits.

Examples in this design document are explanatory. When an example and the
normative grammar differ, the grammar controls and the example must be corrected.

The core grammar resolves these previously ambiguous cases:

- blocks use braces and are not indentation-sensitive;
- newlines or explicit semicolons produce normalized statement terminators;
- `link Enemy?` means optional link storage, not a link to an optional entity;
- `when` validates a link or optional value exactly once;
- `take` transfers a whole named single-home local or consuming parameter;
- `retires parameter` and `retires any Type` are part of function syntax; and
- entity `==` and `!=` compare identity and can refine alias facts.

Reserved post-0.1 features have no accepted productions. A parser must not invent
syntax for them.

## 13. Compiler Architecture

The compiler pipeline is defined by Keld semantics:

```text
Source
  -> Lossless Syntax Tree
  -> Module Semantics
       names
       types
       error effects
       storage classes and parameter effects
  -> Typed Flow IR
       explicit evaluation order
       branches, loops, and calls
       place identity and initialization state
       direct-reference provenance
  -> Storage Verifier
       transfer and use-state checks
       whole-call loan conflicts
       value cleanup on every exit
  -> Lifecycle Planner
       lifecycle tree
       entity states
       link classification
       keep and retire legality
       hidden access windows
       cleanup on every exit
  -> Executable IR
       explicit store and lifecycle operations
       explicit checked numeric and container operations
       no unresolved memory decisions
  -> Interpreter or LLVM Lowering
  -> Native object or WebAssembly object
```

Ownership of decisions is strict:

- Module semantics owns names, types, storage classes, and visible effects.
- Typed Flow IR owns source evaluation order, control-flow identity, places, and
  definite initialization state.
- The storage verifier proves transfers and loans and creates value cleanup
  paths.
- The lifecycle planner proves memory operations and creates cleanup paths.
- Executable IR owns the exact runtime operation sequence, including faults.
- The interpreter defines executable-IR behavior for tests.
- LLVM lowering emits already-proven operations and makes no lifecycle policy.

LLVM artifacts are backend details, not Keld's package or module format.
Compiled libraries store Keld module semantics, generic layout information,
provenance summaries, and lifecycle/effect summaries.

## 14. Repository Boundaries

The planned Rust workspace uses small crates with one owner each:

```text
crates/keld-cli/           command-line interface and diagnostics output
crates/keld-source/        normalized source text, spans, and diagnostics
crates/keld-syntax/        lexer, lossless parser, and syntax tree
crates/keld-semantics/     names, types, visible effects
crates/keld-flow/          typed control-flow representation
crates/keld-storage/       single-home state, loans, and value cleanup
crates/keld-lifecycle/     Custody Ledger verification and cleanup planning
crates/keld-ir/            executable IR and validation
crates/keld-interpreter/   semantic oracle
crates/keld-runtime/       storage allocation, faults, slots, and lifecycles
crates/keld-backend-llvm/  LLVM-only lowering and object emission
tests/                     cross-stage and end-to-end programs
docs/spec/                 normative grammar and language rules
docs/superpowers/specs/     approved design records
```

The first implementation plan may combine crates temporarily only when the
boundary remains explicit and splitting immediately would add no independent
test surface.

## 15. Backend Contract

LLVM is the single initial production backend.

```text
Executable IR
  -> LLVM IR
       -> native x86-64 objects
       -> wasm32 objects
```

Initial delivery order:

1. Windows x86-64 native;
2. Linux x86-64 native;
3. WebAssembly;
4. AArch64 native.

Backend rules:

- Pin one LLVM major version for each Keld compiler release.
- Isolate all LLVM dependencies inside `keld-backend-llvm`.
- Use the LLVM C API where it covers the required functionality.
- Emit aliasing metadata only from lifecycle facts already proven by Keld.
- Lower checked arithmetic, conversion, bounds, capacity, and address-size
  operations without invoking LLVM undefined behavior on a Keld faulting input.
- Preserve source locations through executable IR and backend lowering.
- Differential-test interpreter and compiled output.
- Do not serialize LLVM bitcode as Keld's stable package representation.

Direct machine-code generation, source transpilation, JVM bytecode, and CIL are
excluded from the initial implementation.

## 16. First Buildable Milestone

The first milestone proves Custody Ledger semantics before native code
generation.

Included source features:

- modules with one source file;
- `Int`, `Bool`, structs, and entities;
- checked `Int` arithmetic, division, remainder, and shifts;
- range-checked `Int` literals and constant expressions;
- local `let` bindings;
- functions with explicit parameter types and explicit non-`Unit` return types;
- `lifecycle`, entity construction, `link`, `when`, `keep`, and `retire`;
- struct field read plus entity field read and mutation;
- `if` and block control flow; and
- deterministic cleanup on normal return.

Included tooling:

- `keld check <file>`;
- `keld run --engine interpreter <file>`;
- stable diagnostic codes for lifecycle failures; and
- an executable-IR textual dump for debugging tests.

The bootstrap executable entrypoint is exactly `fn main() -> Int`. It has no
parameters or effect clauses. `keld run --engine interpreter` writes the returned
decimal Int followed by one newline. A successful execution exits with host
status zero independently of the returned Keld value. Static rejection exits
with status one, a runtime Keld fault exits with status two, and command misuse
exits with status 64.

Excluded from this milestone:

- LLVM lowering;
- WebAssembly;
- concurrency and async;
- interfaces and generics;
- imports, enums, `var`, loops, `break`, `continue`, and `match`;
- recursive function-call cycles;
- Text, List, `take`, and every other single-home storage operation;
- typed error effects;
- resource types and user-defined cleanup;
- unsafe code and FFI; and
- package management.

Done criteria:

1. A cyclic enemy-target graph executes without ownership or lifetime syntax.
2. Ending a lifecycle invalidates all links to its remaining members.
3. An entity kept in an ancestor survives the inner lifecycle.
4. Early retirement makes later direct use a compile-time error.
5. Resolving a stale link enters the absence path without invalid memory access.
6. Double retirement is rejected.
7. A direct entity reference cannot escape into a persistent field or container.
8. Lifecycle cleanup order is deterministic and tested.
9. Runtime property tests cover allocation, reuse, generation exhaustion, entity
   movement, and stale-link resolution.
10. Every accepted milestone program produces the same observable result in
    direct executable-IR tests and the command-line interpreter.
11. Grammar golden tests cover virtual terminators, operator precedence,
    `link T?`, `when`, lifecycle statements, and retirement effects.
12. Compile-fail tests reject use through may-alias parameters after retirement.
13. Compile-pass tests accept identity-refined distinct parameters.
14. A broad `retires any T` call invalidates every compatible live proof and
    allows references to be reacquired from links afterward.
15. No executable IR contains a `View` that crosses a structural operation.
16. Constant evaluation, the interpreter, and executable IR agree on overflow,
    zero division, `Int.MIN / -1`, `Int.MIN % -1`, and invalid shifts.

## 17. Storage Model Milestone

The second executable milestone proves single-home storage before LLVM lowering.
Its implementation order is fixed:

1. Add place-state analysis for empty, live, maybe-live, loaned, and moved values,
   with hidden local drop flags only at non-uniform joins.
2. Implement whole-local `take`, explicit `.copy()`, normal loan parameters,
   consuming parameters, owned returns, and aggregate cleanup.
3. Implement `List[Int]`, including checked indexing, growth, removal, and
   deterministic cleanup.
4. Implement `List[List[Int]]` and pass the List state, alias, structural-copy,
   removal, fault, and differential test matrix.
5. Add immutable Text under the same state rules.
6. Add `List[List[Text]]` only after the List-only checks pass.

Map, Set, Slice, iterator protocols, and `for` remain unavailable throughout
this milestone. LLVM work does not begin until both the Custody Ledger milestone
and this storage milestone pass their normative verification requirements.

Done criteria are the compile-pass, compile-fail, cleanup, bounds, capacity, and
cross-engine cases in `docs/spec/storage-values.md`, plus the size and allocation
cases in `docs/spec/numeric-safety.md`.

## 18. Verification Strategy

Verification proceeds in layers:

- state-transition unit tests for every slot transition;
- property tests over randomized allocation, keep, retire, resolve, and
  lifecycle-end sequences;
- compile-pass tests for valid lifecycle and link programs;
- compile-fail tests for every diagnostic family;
- parser golden tests derived from every normative EBNF production;
- alias counterexamples using equal and distinct runtime identities;
- executable-IR validation before interpretation or backend lowering;
- interpreter/runtime differential tests;
- native/interpreter differential tests once LLVM lowering exists;
- sanitizer-backed runtime stress tests for native builds; and
- model-level proofs of progress and preservation after the executable core is
  stable enough to formalize without churn.

No milestone is complete solely because the compiler builds. Its language-level
done criteria and negative safety tests must pass.

## 19. Non-Goals for Version 0.1

- Transparent reclamation of arbitrary unstructured object graphs.
- Making every reference permanently valid.
- Hiding the logical possibility that a deliberately retired target is absent.
- Supporting raw-pointer programming in ordinary modules.
- Matching low-level ownership-tree performance for every workload.
- Providing a tracing collector or reference-counted fallback.
- External resource-owning entity fields before a restricted cleanup ABI exists.
- Map, Set, Slice, or iterator semantics before List verification is complete.
- Arbitrary user-defined entity destructors.
- Claiming worldwide novelty or formal correctness before evidence exists.

## 20. Design Summary

Keld separates four concerns:

- managed value storage has one home, explicit whole-value transfer, and
  compiler-inferred call loans;
- identity is represented by copyable, non-owning links;
- lifetime is determined by deterministic, acyclic lifecycles; and
- direct access is bounded by compiler-generated windows.

Programmers see `take` only when choosing to transfer named managed storage.
They also see entities, links, lifecycles, `keep`, `retire`, and explicit
handling of missing dynamic targets. They do not write borrow or lifetime
annotations or manage slots, generations, guards, custody tokens, or allocators.

This separation is the language's defining memory-model decision. All compiler,
runtime, backend, and diagnostic work must preserve it.
