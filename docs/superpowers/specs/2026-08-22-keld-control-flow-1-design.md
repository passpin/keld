# Keld Control Flow-1 Design

Status: approved design for implementation

Date: 2026-08-22

## 1. Goal

Control Flow-1 adds source-level `while`, `break`, and `continue` without adding a new memory model or backend-specific loop operation. The milestone extends Keld's existing flow-sensitive storage and Custody Ledger analyses from acyclic CFGs to cyclic CFGs while preserving deterministic cleanup, single-home storage, lifecycle safety, and interpreter/native parity.

The governing rule is that loops are ordinary structured control flow. Keld's existing storage, lifecycle, provenance, cleanup, and fault semantics continue to apply on every loop edge.

## 2. Source surface

The accepted source forms are the existing grammar productions:

```keld
while condition {
    ...
}

break
continue
```

Rules:

- `break` targets the innermost enclosing `while`.
- `continue` targets the innermost enclosing `while`.
- Loop labels are not part of Control Flow-1.
- `break` and `continue` do not carry values.
- `loop`, `for`, `while let`, loop `else`, and labeled control flow remain deferred.
- `break` or `continue` outside a `while` is a static error.

The parser grammar does not change; the semantic feature gate stops rejecting these three existing syntax forms.

## 3. Condition semantics

A `while` condition:

- is evaluated before the first iteration;
- is re-evaluated before every subsequent iteration;
- must have type `Bool`;
- is an ordinary Keld expression and may contain function calls, checked arithmetic, storage operations, and other already-accepted expression effects.

Condition evaluation is a full-expression boundary. Loans, temporary managed values, and hidden entity access views created solely for evaluating the condition must not remain live when control enters the body or exits the loop.

Existing condition diagnostics remain authoritative for non-`Bool` conditions.

## 4. CFG lowering

A `while` lowers to ordinary cyclic flow:

```text
          +----------------------+
          |                      |
          v                      |
      condition ---- true ----> body
          |                      |
        false                 backedge
          |                      |
          v                      |
         exit <------------------+
```

No executable-IR loop instruction is introduced. Flow lowering uses the existing branch, goto, scope-exit, and phi machinery.

`break` and `continue` are represented as structured exits, not raw jumps that skip cleanup:

```text
continue = ExitScopes(required scopes) -> Goto(condition)
break    = ExitScopes(required scopes) -> Goto(loop exit)
```

A loop-target stack in flow lowering resolves nested-loop control to the innermost loop.

## 5. Lexical storage and cleanup

Each dynamic execution of the loop body has the semantics of entering the body's lexical storage scope anew.

Body-local managed homes are therefore initialized and cleaned independently on every iteration. Outer locals persist across iterations.

Normal body fallthrough, `continue`, `break`, and `return` must all execute the cleanup required by the lexical scopes they leave. Cleanup follows the existing Keld rule: live managed homes are destroyed in reverse successful-initialization order, `MaybeLive` homes use hidden flags, and moved or uninitialized homes are skipped.

`continue` cleans the current iteration's exited scopes before re-evaluating the condition. `break` cleans the current iteration's exited scopes before entering the loop's continuation block.

Control Flow-1 does not alter Keld's runtime-fault contract. Non-catchable Keld faults remain termination paths for which user cleanup is not promised.

## 6. Storage-state fixed point

Loops use the existing home lattice rather than a loop-specific invariant:

```text
Empty(reason)
Live
MaybeLive
```

Joins remain:

```text
Live + Live       = Live
Empty + Empty     = Empty(joined reason)
Live + Empty      = MaybeLive
MaybeLive + other = MaybeLive unless a later operation refines the state
```

The storage verifier computes a monotone fixed point across loop backedges. A loop-carried place may therefore become `MaybeLive` when some iterations leave it live and others leave it moved or uninitialized.

Reads, loans, copies, and `take` still require `Live`. Assignment to a `var` may repair `Empty` or `MaybeLive` by installing a new value and producing `Live`.

The zero-iteration path participates in the loop-exit join. A value initialized only inside a `while` is not definitely initialized after the loop unless another rule proves all exits initialize it.

## 7. Cleanup-order fixed point

Loop-carried reinitialization can change successful-initialization order across iterations. Control Flow-1 reuses the existing cleanup-order model:

```text
Known(order)
Divergent
```

If all incoming loop states imply one compatible order, cleanup remains statically ordered. If different iterations can produce incompatible orders, the existing hidden per-scope tracker is used.

No metadata may grow with the number of dynamic iterations. Hidden cleanup state is bounded by the static homes in the function.

## 8. Lifecycle semantics

A `while` does not create an implicit lifecycle.

Entities created in a plain loop body join the currently active lifecycle, exactly as entities created in any other lexical block or ordinary function call. Leaving the body only ends lexical local references and managed value scopes; it does not retire such entities unless an explicit lifecycle ends or `retire` executes.

An explicit lifecycle nested in a loop keeps its ordinary lexical extent:

```keld
while cond {
    lifecycle frame {
        ...
        continue
    }
}
```

A `continue`, `break`, or `return` that exits `frame` must end that lifecycle before transferring control. Remaining members are retired by the ordinary deterministic lifecycle algorithm. Entities previously kept in a strict ancestor survive as usual.

## 9. Lifecycle and provenance fixed point

Entity analysis also becomes cyclic.

At a loop join, the verifier keeps only facts valid on every incoming edge. Existing rules continue to apply:

- live references with incompatible lifecycle facts degrade conservatively;
- a path that retires or invalidates a reference prevents later direct use unless all incoming paths re-establish a valid live proof;
- equality and distinctness facts survive a join only when proven on all incoming paths.

Control Flow-1 must not confuse one static allocation site with one dynamic entity identity. The same source allocation executed in different iterations may create distinct runtime entities. Loop-carried entity provenance therefore uses conservative merged provenance unless identity equality is proved. When uncertain, the analysis prefers may-alias over an unsound must-alias or must-distinct conclusion.

Identity comparison may refine such conservative facts again inside a branch.

## 10. Verifier architecture

The lifecycle verifier currently assumes acyclic control flow for its predecessor scheduling. Control Flow-1 replaces that assumption with a monotone worklist fixed-point analysis compatible with CFG backedges.

The storage verifier already uses a changed-state work queue and state joins; that model is the reference shape for cyclic analysis.

The milestone must preserve stage ownership:

- semantics owns syntax acceptance, typing, and HIR;
- flow owns cyclic CFG construction and loop targets;
- lifecycle owns entity liveness, provenance, and lifecycle facts;
- storage owns single-home state, loans, and value cleanup;
- executable IR contains only already-proven control flow and cleanup;
- interpreter and LLVM lower the same validated IR without inventing loop policy.

## 11. HIR changes

Semantic HIR gains explicit statement forms equivalent to:

```text
While { condition, body }
Break
Continue
```

Loop target block IDs and cleanup details do not belong in HIR. They are flow-lowering concerns.

Call-graph traversal and other HIR visitors must recurse through loop conditions and bodies.

## 12. Definite fallthrough

A general `while condition` is assumed able to fall through because the condition may be false before any iteration.

Control Flow-1 may recognize the narrow syntactic case of a literal `while true` with no reachable `break` targeting that loop as non-fallthrough for function return checking. It does not attempt general termination proofs.

Dead loop bodies are still parsed and type-checked even when the condition is a literal `false`.

## 13. Diagnostics

`while`, `break`, and `continue` are removed from the bootstrap unsupported-feature diagnostic.

A new source-level diagnostic is reserved for loop control used outside a loop:

```text
KLD0112: loop control outside a loop
```

Representative messages:

```text
error[KLD0112]: `break` is only valid inside a `while` body
error[KLD0112]: `continue` is only valid inside a `while` body
```

Existing storage and lifecycle diagnostics remain authoritative when loop joins produce `MaybeLive`, invalidated entity proofs, conflicting loans, or other already-defined states.

## 14. Native and interpreter contract

Loops add no runtime object, reference counting, tracing, implicit lifecycle, or iteration allocation.

The executable form is ordinary validated cyclic CFG. Interpreter execution and LLVM lowering must agree at O0 and O2.

Native-1's deterministic allocation observation model remains valid for loops. Repeated execution of one static allocation site produces repeated attempts at the same site ID; interpreter and native observations must agree on attempt order and on failure behavior for any tested attempt.

## 15. Acceptance coverage

Control Flow-1 is complete only when tests cover all of the following.

### Basic control flow

- zero, one, and many iterations;
- condition re-evaluation;
- mutation of outer scalar and managed locals;
- nested loops;
- innermost `break` and `continue` targeting;
- `break` and `continue` outside loops rejected.

### Storage

- body-local `List` and `Text` cleanup on normal iteration end;
- cleanup on `continue`;
- cleanup on `break`;
- `take` and reinitialization of outer `var` homes;
- loop-head `MaybeLive` formation;
- repair of `MaybeLive` by assignment;
- zero-iteration definite-initialization behavior;
- dynamic cleanup-order tracking when iteration histories diverge.

### Lifecycle and provenance

- retirement on one loop path invalidates later direct use as required;
- explicit lifecycle cleanup on `continue`, `break`, and `return`;
- `keep` to an ancestor survives inner lifecycle exit;
- plain loop bodies do not create implicit lifecycles;
- repeated execution of one allocation site does not produce false must-alias or must-distinct facts;
- identity comparison can refine conservative loop-carried provenance.

### Conditions

- `Bool` requirement;
- ordinary call effects in conditions;
- checked-operation faults;
- managed temporaries and allocation in conditions do not leak loans or homes across the condition boundary.

### Engine parity

Representative loop fixtures must run identically through:

- the executable-IR interpreter;
- native LLVM O0;
- native LLVM O2.

Repeated allocations at one static site must also preserve the shared allocation-attempt observation contract.

## 16. Completion gate

The milestone is accepted only when:

1. `while`, `break`, and `continue` pass semantic analysis;
2. nested loop targets are correct;
3. lifecycle analysis reaches a sound fixed point on cyclic CFGs;
4. storage analysis reaches a sound fixed point on cyclic CFGs;
5. existing `Live`/`Empty`/`MaybeLive` semantics remain unchanged;
6. deterministic cleanup is correct on fallthrough, `continue`, `break`, and `return`;
7. explicit lifecycle semantics remain unchanged;
8. loop-carried entity provenance is conservative and sound;
9. validated executable IR contains no unresolved storage or lifecycle decision;
10. interpreter, native O0, and native O2 observations match;
11. repeated allocation-site attempt observations match across engines;
12. debug workspace tests pass;
13. release workspace tests pass;
14. strict Clippy passes;
15. formatting passes; and
16. repository diff review shows no unrelated feature expansion.

## 17. Deferred work

Control Flow-1 intentionally does not add:

- `for` or iterators;
- loop labels;
- `break` values;
- `loop` expressions;
- `while let`;
- loop `else`;
- pattern matching;
- typed errors;
- recursion;
- concurrency or async behavior;
- new runtime memory-management mechanisms; or
- new backend loop instructions.

Later control-flow features should reuse the cyclic CFG and structured-exit machinery established here rather than introduce parallel semantics.
