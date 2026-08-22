# Keld Control Flow Semantics

Status: normative Keld 0.1 control-flow model

This document defines the accepted semantics of `while`, `break`, and `continue`.
It supplements [`grammar.md`](grammar.md), while storage behavior remains governed
by [`storage-values.md`](storage-values.md) and numeric faults by
[`numeric-safety.md`](numeric-safety.md).

## 1. Accepted forms

Keld 0.1 accepts these structured control-flow forms:

```keld
while condition {
    statements
}

break
continue
```

`break` exits the innermost enclosing `while`. `continue` begins the next
condition evaluation of the innermost enclosing `while`. Neither form carries a
value, and both are static errors outside a `while` body (`KLD0112`).

Loop labels, `loop`, `for`, `while let`, loop `else`, and break values are not
part of Control Flow-1.

## 2. Condition evaluation

A `while` condition is evaluated before the first iteration and again after each
backedge. It must have type `Bool`; Keld has no loop-specific truthiness.

The condition is an ordinary expression. Calls, checked arithmetic, storage
operations, allocations, and other effects already accepted for expressions
execute in normal left-to-right order. A fault raised while evaluating the
condition occurs before body entry for that evaluation.

Condition evaluation is a full-expression boundary. Loans, managed temporaries,
hidden entity views, and other temporary state created only for the condition
must be closed or cleaned before control enters the body or takes the false exit.
A condition that allocates repeatedly executes the same static allocation site
on each evaluation; dynamic allocation attempt numbers advance normally.

## 3. Zero iterations and re-evaluation

If the first condition evaluates to `false`, the body executes zero times. The
zero-iteration path participates in every post-loop flow join. In particular, a
value initialized only in the body is not thereby definitely initialized after
the loop.

After body fallthrough or `continue`, the condition is evaluated from the
current program state. The compiler must not reuse a previous condition value.

## 4. Body lexical scope

Each dynamic body execution has the semantics of entering the body's lexical
storage scope anew. A body-local home from one iteration is not a still-live
home in the next iteration.

Outer locals remain loop-carried state. Body-local managed homes are cleaned on
every edge that leaves their lexical extent before that edge reaches its target.
This uses the ordinary Keld storage-cleanup rules rather than a loop-specific
cleanup mechanism.

## 5. Structured exits and cleanup

Normal body fallthrough, `continue`, `break`, and `return` perform the structured
cleanup required by every lexical scope they leave.

- fallthrough cleans the current body scope before the next condition;
- `continue` cleans exited body scopes before the next condition;
- `break` cleans exited body scopes before the loop continuation;
- `return` cleans every exited storage scope and explicit lifecycle before the
  function return.

Live managed homes are destroyed in reverse successful-initialization order.
`MaybeLive` homes use their existing conditional state, while moved or
uninitialized homes are skipped. Re-entering the same static body scope starts
its body-local homes in their entry state again.

Non-catchable runtime faults keep the existing Keld fault contract. Control
Flow-1 does not add a new promise of user cleanup on such termination paths.

## 6. Storage state at backedges

Loop backedges use the same Home lattice and joins as other CFG merges:

```text
Empty(reason)
Live
MaybeLive
```

A loop-carried home is usable only when the converged state permits the requested
operation. Reads, loans, copies, and `take` require `Live`. A `var` assignment may
repair `Empty` or `MaybeLive` by installing a new value and producing `Live`.

Analysis is a monotone fixed point over the cyclic CFG. Hidden cleanup metadata
is bounded by static homes and scopes; it must not grow with the number of
runtime iterations. Existing divergent cleanup-order tracking is used when
incoming iteration histories imply incompatible successful-initialization order.

## 7. Lifecycles

A `while` does **not** create an implicit lifecycle.

An entity allocated in a plain loop body belongs to the lifecycle that was
already active at the allocation point. Leaving the lexical body scope does not
retire that entity merely because an iteration ended.

An explicit lifecycle nested inside a loop keeps its ordinary lexical extent.
If `continue`, `break`, or `return` exits that lifecycle, the lifecycle ends
before control reaches the transfer target. Remaining members retire through the
ordinary deterministic lifecycle algorithm. An entity explicitly kept into a
strict active ancestor survives the inner lifecycle exit as usual.

## 8. Lifecycle and provenance joins

Lifecycle liveness, entity provenance, alias facts, and identity refinements are
also solved to a fixed point over loop backedges. A fact survives a join only
when it is valid on every incoming path required by the existing lifecycle
rules.

Repeated execution of one static entity-allocation instruction does not imply
one dynamic entity identity. Values carried across iterations therefore merge
provenance conservatively. When identity cannot be proved equal or distinct,
analysis prefers may-alias rather than an unsound must-alias or must-distinct
claim. A later identity comparison may refine those conservative facts inside
its branch.

A retirement or retirement effect reached while evaluating the condition takes
effect before either the body or the false loop exit. Post-loop direct uses must
therefore satisfy the converged lifecycle proof just like uses after any other
control-flow join.

## 9. Literal-true fallthrough rule

A general `while condition` is considered able to fall through because its first
condition may be false.

A literal `while true` is structurally non-fallthrough only when its body has no
syntactic `break` targeting that same loop. A `break` inside a nested loop does
not make the outer loop fall through. Keld does not perform a general termination
proof for this classification.

Dead bodies are still parsed and type-checked, including the body of
`while false`.

## 10. Executable IR and engines

Control Flow-1 introduces no executable-IR loop opcode and no runtime loop
object. Source loops lower to ordinary cyclic executable CFGs composed from the
existing blocks, branches, gotos, structured cleanup operations, and
terminators.

Executable IR must pass the same validator before interpretation or native
lowering. A CFG backedge does not weaken register, storage, lifecycle, view, or
cleanup validation. Mutable source locals may use backend storage appropriate to
the target, but ordinary IR temporaries retain their single-definition contract.

The interpreter and native LLVM O0/O2 engines execute the same validated IR and
must produce equivalent return values, faults, source spans, and deterministic
allocation observations. Repeated dynamic executions of one static allocation
site retain one site ID while advancing its attempt number.

## 11. Diagnostics

The existing diagnostic families remain authoritative inside loops. Examples
include:

- `KLD0107` for a non-`Bool` condition;
- `KLD0112` for `break` or `continue` outside a loop;
- lifecycle diagnostics when a loop join invalidates a direct entity proof; and
- storage diagnostics such as `KLD2008` when a loop-carried Home is
  `MaybeLive` at a required-live use.

Loops do not downgrade these failures into warnings and do not introduce an
escape hatch around existing storage or lifecycle checks.

## 12. Deferred forms

Control Flow-1 does not define:

- `for` or iterator protocols;
- labeled loops or labeled `break`/`continue`;
- `break` values;
- `loop` expressions;
- `while let`;
- loop `else`;
- pattern matching or `match` semantics;
- typed-error handling;
- recursion, concurrency, or async behavior; or
- any new memory-management mechanism.

Later control-flow features should reuse the cyclic CFG and structured-exit
contracts specified here rather than create parallel storage or lifecycle rules.
