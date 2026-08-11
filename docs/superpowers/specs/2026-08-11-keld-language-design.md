# Keld Language Design

Status: approved baseline awaiting written-spec review

Date: 2026-08-11

Working language version: 0.1 design core

## 1. Purpose

Keld is a statically typed, high-level language for native applications whose
central feature is deterministic, memory-safe management of mutable entity
graphs without tracing garbage collection or reference counting.

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
- Ordinary source has no ownership tokens, borrow operators, lifetime
  parameters, allocators, retain/release operations, slots, or generations.
- The memory model is Custody Ledger Memory with acyclic lifecycle groups.
- Sequential execution is the first semantic and implementation core.
- Native ahead-of-time compilation is primary.
- WebAssembly is a secondary output target.
- LLVM is the first production backend behind a backend-neutral executable IR.
- The compiler is implemented in Rust.
- An interpreter for executable IR is the semantic test oracle.

## 3. Language-Level Memory Concepts

### 3.1 Values

Values have structural identity. Examples include numbers, booleans, structs,
enums, tuples, and immutable text values.

The compiler may place a value inline, on the stack, in registers, or in hidden
storage. This placement is not observable in safe Keld.

Copying a copyable value creates an independent value. Non-copyable resources
are transferred only by operations defined by their APIs.

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

An ordinary local entity binding is a compiler-proven live reference. It may be
aliased freely within its proven lifetime. The binding is not the entity's
cleanup authority and does not keep the entity alive.

All direct aliases carry the same hidden identity provenance. Retiring through
one alias invalidates every direct alias with that provenance. The compiler does
not treat aliases as independent owners.

Direct entity references may exist in locals, parameters, and compiler-bounded
temporary results. Persistent fields and containers must use `link`.

### 3.3 Links

A `link T` is a copyable, non-owning relationship to an entity of type `T`.
A `link T?` may additionally begin with no target.

Links never extend an entity's lifetime. Arbitrary link cycles are permitted.
Resolving a link may fail because its target was retired.

```keld
boss.target = player

when boss.target as target {
    target.health -= 10
}
```

`when link-expression as name` resolves the expression exactly once. The bound
name is a direct entity reference valid only within the successful branch.
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
    let player = Player(...)

    lifecycle level {
        let enemy = Enemy(...)
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
- `keep entity in lifecycle` moves an entity to a named ancestor lifecycle.
- `retire entity` ends one entity early.

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
Empty(g) -> Live(g, ...)
Live(g, ...) -> Dying(g, ...)
Dying(g, ...) -> Empty(g + 1)
Dying(MAX, ...) -> Retired
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

The compiler lowers entity access into bounded read or edit windows.

- A read window permits any number of reads.
- An edit window permits reads and mutations within the current sequential task.
- Aliasing inside one edit window is permitted.
- Structural operations cannot execute while either window is active.
- Direct views cannot escape their window.
- Local sequential windows normally lower to no runtime synchronization.

Structural operations are allocation, retirement, lifecycle movement, lifecycle
destruction, and any operation that may re-enter the same store structurally.

The compiler chooses the smallest practical window from typed control flow. A
structural operation ends prior windows and later access begins a new one.

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
retire another entity, call user code, or re-enter the store. Keld 0.1 has no
arbitrary user-defined entity destructor.

## 5. Safety Invariants

A conforming safe implementation must maintain all of these invariants:

1. Every live entity occupies exactly one live slot.
2. Every live entity belongs to exactly one active lifecycle.
3. Lifecycle parentage is acyclic.
4. Slot generations are monotonic and never wrap.
5. A link resolves only to the same live slot generation it originally named.
6. A direct view cannot outlive its hidden access window.
7. Structural store operations execute with no active direct views.
8. A direct entity binding cannot be used after its retirement or lifecycle end.
9. Ending a lifecycle retires every remaining member exactly once.
10. Normal error exits execute the same required cleanup as normal control flow.
11. Safe code cannot construct a raw address, counterfeit link, or store brand.
12. In the sequential core, all mutation occurs on the owning task.

Consequences:

- use-after-free through direct bindings is rejected statically;
- stale persistent relationships resolve to absence;
- double retirement is rejected statically or trapped as a compiler/runtime bug;
- link cycles do not retain entities;
- entity reclamation is deterministic; and
- unsynchronized cross-task mutation is impossible in the sequential core.

## 6. Static Semantics

### 6.1 Hidden semantic forms

The compiler may reason with the following hidden forms:

```text
EntityRef<T, lifecycle, state, provenance>
Link<T, optionality>
Lifecycle<identity, parent, state>
Access<store, mode, extent>
```

These forms appear in compiler IR and diagnostics, never as source annotations.

### 6.2 Lifecycle order

Let `L1 <= L2` mean that `L2` is an ancestor of `L1` and therefore outlives
`L1`.

`keep x in L2` is valid only when:

- `x` is live in `L1`;
- `L2` is active;
- `L1 <= L2`;
- both lifecycles belong to the same hidden store; and
- no access window is active at the operation.

### 6.3 Flow-sensitive entity state

For each direct entity binding, typed control flow tracks one of:

```text
Live(lifecycle, provenance)
MaybeLive(link_origin)
Retired
OutOfScope
```

Only `Live` permits direct field access or calls requiring an entity. Resolving a
link refines `MaybeLive` to a new scoped `Live` binding in the successful branch.

Provenance is an SSA identity or a conservative set of possible identities.
Copying a direct binding copies its provenance. `retire` invalidates all direct
bindings whose provenance may name the retired entity. Because direct references
cannot enter persistent fields or containers, this alias set remains bounded by
typed local flow and function summaries rather than general heap analysis.

At control-flow joins, the state is the least permissive state valid on every
incoming edge. For example, an entity retired on only one branch is not directly
usable after the join.

### 6.4 Direct-reference escape checking

The compiler infers the provenance of direct entity results:

- created in the caller's active lifecycle;
- derived from a specific direct entity parameter; or
- scoped result of a link resolution.

Public module metadata records provenance summaries without exposing lifetime
syntax. A function cannot return or store a direct reference when no summary can
prove its target live at every caller. The programmer must return a `link`
instead.

Direct references cannot be stored in entity fields, heap containers, globals,
closures that outlive the current scope, or foreign state. Those locations use
links.

### 6.5 Function effects

Typed functions carry inferred semantic effects. Initial effects are:

```text
pure
read entities
edit entities
structural lifecycle
io
raises E
unsafe
```

Local effects are inferred. Public declarations display application-visible
effects such as `raises`; hidden access and lifecycle summaries are serialized in
compiled module metadata.

A public function that can end the lifetime of a direct entity parameter must
declare that semantic effect:

```keld
fn remove(enemy: Enemy) retires enemy
```

Calling `remove(enemy)` invalidates `enemy` and all of its known direct aliases.
Keld 0.1 does not permit a public function to retire an entity parameter without
this declaration. Lifecycle extension with `keep` remains lexical in Keld 0.1
and cannot be hidden inside an ordinary function call.

The backend receives only operations already accepted by lifecycle and effect
verification.

## 7. Type System

Keld uses static, strong typing with bidirectional local inference.

Initial type categories:

- primitives: `Bool`, fixed-width integers, platform-independent `Int`, and
  floating-point values;
- value aggregates: `struct`, tuples, and arrays;
- algebraic variants: `enum`;
- identity-bearing records: `entity`;
- persistent identity relationships: `link T` and `link T?`;
- absence: `T?`;
- functions and closures; and
- interfaces.

Rules:

- Local bindings infer types when the initializer is sufficient.
- Function parameters and public return types are explicit.
- There is no implicit null.
- Pattern matching is exhaustive.
- Numeric widening is permitted only when lossless and unambiguous.
- Narrowing is explicit and checked unless inside an unsafe boundary.
- Dynamic typing is absent from the initial language.

Generics use a hybrid compilation model:

- entity, link, and interface representations compile uniformly when layout is
  known independently of the type argument;
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
not required. Every exit edge receives compiler-generated lifecycle and resource
cleanup.

Fatal invariant failures abort the process. Fatal abort does not promise user
cleanup. Destructors cannot fail.

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
- retain a direct entity view in foreign state.

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
  resolve it with `when target as value { ... }`
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

Diagnostics must include the operation that created the restriction, the use
that violates it, and one concrete repair when one is mechanically known.

## 12. Compiler Architecture

The compiler pipeline is defined by Keld semantics:

```text
Source
  -> Lossless Syntax Tree
  -> Module Semantics
       names
       types
       interfaces
       error effects
  -> Typed Flow IR
       explicit evaluation order
       branches, loops, and calls
       direct-reference provenance
  -> Lifecycle Planner
       lifecycle tree
       entity states
       link classification
       keep and retire legality
       hidden access windows
       cleanup on every exit
  -> Executable IR
       explicit store and lifecycle operations
       no unresolved memory decisions
  -> Interpreter or LLVM Lowering
  -> Native object or WebAssembly object
```

Ownership of decisions is strict:

- Module semantics owns names, types, interfaces, and visible effects.
- Typed Flow IR owns source evaluation order and control-flow identity.
- The lifecycle planner proves memory operations and creates cleanup paths.
- Executable IR owns the exact runtime operation sequence.
- The interpreter defines executable-IR behavior for tests.
- LLVM lowering emits already-proven operations and makes no lifecycle policy.

LLVM artifacts are backend details, not Keld's package or module format.
Compiled libraries store Keld module semantics, generic layout information,
provenance summaries, and lifecycle/effect summaries.

## 13. Repository Boundaries

The planned Rust workspace uses small crates with one owner each:

```text
crates/keld-cli/           command-line interface and diagnostics output
crates/keld-syntax/        source text, lexer, lossless parser, syntax tree
crates/keld-semantics/     names, types, interfaces, visible effects
crates/keld-flow/          typed control-flow representation
crates/keld-lifecycle/     Custody Ledger verification and cleanup planning
crates/keld-ir/            executable IR and validation
crates/keld-interpreter/   semantic oracle
crates/keld-runtime/       slots, generations, lifecycle membership, cleanup
crates/keld-backend-llvm/  LLVM-only lowering and object emission
tests/                     cross-stage and end-to-end programs
docs/                      normative design and implementation records
```

The first implementation plan may combine crates temporarily only when the
boundary remains explicit and splitting immediately would add no independent
test surface.

## 14. Backend Contract

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
- Preserve source locations through executable IR and backend lowering.
- Differential-test interpreter and compiled output.
- Do not serialize LLVM bitcode as Keld's stable package representation.

Direct machine-code generation, source transpilation, JVM bytecode, and CIL are
excluded from the initial implementation.

## 15. First Buildable Milestone

The first milestone proves Custody Ledger semantics before native code
generation.

Included source features:

- modules with one source file;
- `Int`, `Bool`, structs, and entities;
- local `let` bindings;
- functions with explicit parameter and return types;
- `lifecycle`, entity construction, `link`, `when`, `keep`, and `retire`;
- field read and mutation;
- `if` and block control flow; and
- deterministic cleanup on normal return.

Included tooling:

- `keld check <file>`;
- `keld run --engine interpreter <file>`;
- stable diagnostic codes for lifecycle failures; and
- an executable-IR textual dump for debugging tests.

Excluded from this milestone:

- LLVM lowering;
- WebAssembly;
- concurrency and async;
- interfaces and generics;
- typed error effects;
- user-defined cleanup;
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

## 16. Verification Strategy

Verification proceeds in layers:

- state-transition unit tests for every slot transition;
- property tests over randomized allocation, keep, retire, resolve, and
  lifecycle-end sequences;
- compile-pass tests for valid lifecycle and link programs;
- compile-fail tests for every diagnostic family;
- executable-IR validation before interpretation or backend lowering;
- interpreter/runtime differential tests;
- native/interpreter differential tests once LLVM lowering exists;
- sanitizer-backed runtime stress tests for native builds; and
- model-level proofs of progress and preservation after the executable core is
  stable enough to formalize without churn.

No milestone is complete solely because the compiler builds. Its language-level
done criteria and negative safety tests must pass.

## 17. Non-Goals for Version 0.1

- Transparent reclamation of arbitrary unstructured object graphs.
- Making every reference permanently valid.
- Hiding the logical possibility that a deliberately retired target is absent.
- Supporting raw-pointer programming in ordinary modules.
- Matching low-level ownership-tree performance for every workload.
- Providing a tracing collector or reference-counted fallback.
- Claiming worldwide novelty or formal correctness before evidence exists.

## 18. Design Summary

Keld separates three concerns:

- identity is represented by copyable, non-owning links;
- lifetime is determined by deterministic, acyclic lifecycles; and
- direct access is bounded by compiler-generated windows.

Programmers see entities, links, lifecycles, `keep`, `retire`, and explicit
handling of missing dynamic targets. They do not manage slots, generations,
guards, custody tokens, or allocators.

This separation is the language's defining memory-model decision. All compiler,
runtime, backend, and diagnostic work must preserve it.
