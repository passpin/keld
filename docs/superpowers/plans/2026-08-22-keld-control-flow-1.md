# Keld Control Flow-1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement source-level `while`, `break`, and `continue` as ordinary cyclic Keld control flow while preserving the existing single-home storage lattice, deterministic scope/lifecycle cleanup, conservative entity provenance, validated executable IR, and interpreter/native parity.

**Architecture:** Keep the existing grammar and backend-neutral executable IR. Add loop forms to semantic HIR, lower them into `Branch`/`Goto`/`ExitScopes`, replace the lifecycle verifier's DAG scheduling with monotone fixed-point analysis, make loop-carried entity provenance finite and conservative, reset exited lexical homes when a static loop body scope is re-entered, then prove the same cyclic IR through the interpreter and LLVM O0/O2 paths. No loop-specific runtime object, memory manager, or backend instruction is introduced.

**Tech Stack:** Rust 1.97.0 workspace, existing Keld syntax/HIR/flow/lifecycle/storage/IR/interpreter crates, LLVM 22.1.8 Native-1 Windows GNU backend, MinGW-w64 `x86_64-pc-windows-gnu`, existing differential allocation-observation harness.

**Spec:** `docs/superpowers/specs/2026-08-22-keld-control-flow-1-design.md`

## Global Constraints

- Work only on branch `feature/control-flow-1`, based on `407f6d85a5b9322f6a8496b9d9cdb9525577e665` or a descendant.
- The parser grammar already contains `while`, `break`, and `continue`; do not invent new syntax.
- `break` and `continue` target only the innermost enclosing `while`. Labels and values remain deferred.
- A plain `while` creates a lexical storage scope for its body, not a Custody Ledger lifecycle.
- Every normal scope exit caused by body fallthrough, `continue`, `break`, or `return` must use the existing deterministic cleanup machinery. Non-catchable runtime faults keep their existing cleanup contract.
- Keep `Home = Empty(reason) | Live | MaybeLive` unchanged. Loops extend the current join semantics to cyclic CFGs rather than imposing a new loop-only state rule.
- Preserve `Known`/`Divergent` cleanup-order tracking and keep all hidden metadata bounded by static function homes/scopes, never dynamic iteration count.
- Entity facts must lose precision rather than make an incorrect must-alias, must-distinct, or live-proof claim.
- Do not add an executable-IR loop instruction. The interpreter and LLVM backend must consume the same existing `Branch`, `Goto`, cleanup, Phi, and storage/lifecycle operations.
- Do not add `for`, iterators, `loop`, labels, `while let`, loop `else`, pattern matching, typed errors, recursion, async, or concurrency.
- Use TDD for each task: focused RED first, minimum implementation, focused GREEN, then broader gate.
- Preserve the historical first-milestone documentation; add Control Flow-1 as a later accepted milestone instead of rewriting history.

---

### Task 1: Admit loops in semantic HIR and define source diagnostics/fallthrough

**Files:**
- Modify: `crates/keld-semantics/src/features.rs`
- Modify: `crates/keld-semantics/src/hir.rs`
- Modify: `crates/keld-semantics/src/check.rs`
- Modify: `crates/keld-semantics/tests/feature_gate.rs`
- Modify: `crates/keld-semantics/tests/entrypoint.rs`
- Create: `crates/keld-semantics/tests/loops.rs`

**Interfaces:**
- Add `HirWhile { condition: HirExpr, body: HirBlock }`.
- Add `HirStmtKind::While(HirWhile)`, `HirStmtKind::Break`, and `HirStmtKind::Continue`.
- Add semantic diagnostic `KLD0112` for loop control outside a loop.
- Keep `KLD0107` as the existing non-`Bool` condition diagnostic.
- Make HIR visitors, including `called_functions`, traverse loop conditions and bodies.

- [ ] **Step 1: Write semantic RED tests**

  In `crates/keld-semantics/tests/loops.rs`, add focused tests using these source shapes:

  ```keld
  fn main() -> Int {
      var i = 0
      var total = 0
      while i < 5 {
          i += 1
          if i == 2 { continue }
          if i == 4 { break }
          total += i
      }
      return total
  }
  ```

  Require analysis to produce a module whose main body contains `HirStmtKind::While`, and whose loop body contains `Break` and `Continue` under the nested `if` statements.

  Add two outside-loop cases:

  ```keld
  fn main() -> Int { break }
  fn main() -> Int { continue }
  ```

  Require `KLD0112`, no semantic module, and messages naming the offending keyword.

  Add a condition type case:

  ```keld
  fn main() -> Int {
      while 1 { break }
      return 0
  }
  ```

  Require `KLD0107`.

  Add fallthrough cases to `entrypoint.rs`:

  ```keld
  fn spin() -> Int {
      while true { continue }
  }
  fn main() -> Int { return 0 }
  ```

  must not produce `KLD0111`, while:

  ```keld
  fn maybe(flag: Bool) -> Int {
      while true {
          if flag { break }
      }
  }
  fn main() -> Int { return 0 }
  ```

  must produce `KLD0111`.

  Also prove a `break` inside a nested `while` does not make the outer literal-true loop fall through:

  ```keld
  fn spin(flag: Bool) -> Int {
      while true {
          while flag { break }
      }
  }
  fn main() -> Int { return 0 }
  ```

  Finally add a recursive-call-under-loop case to whichever current semantic test covers the recursion gate, or to `loops.rs` if no dedicated file exists, so `called_functions` cannot accidentally skip loop bodies.

- [ ] **Step 2: Update feature-gate regressions and verify RED**

  Remove `while`, `break`, and `continue` from `every_deferred_construct_is_rejected_before_typing` in `feature_gate.rs`.

  Replace the current outer-unsupported cascade example that depends on `while` with an unsupported outer enum containing a generic child, for example:

  ```keld
  enum Box[T] { Value(T) }
  fn main() -> Int { return 0 }
  ```

  Require exactly the outer `enum` unsupported diagnostic rather than a second generic diagnostic.

  Run:

  ```powershell
  $env:CARGO_BUILD_TARGET='x86_64-pc-windows-gnu'
  cargo test -p keld-semantics --test loops --test feature_gate --test entrypoint
  ```

  Expected: new loop tests fail because the feature gate still rejects loops and HIR has no loop variants.

- [ ] **Step 3: Implement the semantic loop surface**

  In `hir.rs`, add:

  ```rust
  #[derive(Clone, Debug)]
  pub struct HirWhile {
      pub condition: HirExpr,
      pub body: HirBlock,
  }
  ```

  and the three statement variants.

  In `features.rs`, stop returning unsupported features for `SyntaxKind::{WhileStmt,BreakStmt,ContinueStmt}` and leave all other deferred gates unchanged.

  In `BodyChecker`, add `loop_depth: usize`, initialized to zero. Implement:

  - `check_while`: check its condition exactly like `if`, require `Bool`, clone the lexical environment for the body, increment `loop_depth` only while checking that body, and emit `HirStmtKind::While`.
  - `check_break` / `check_continue`: emit `KLD0112` when `loop_depth == 0`; otherwise emit the HIR marker statement.

  Extend `collect_block_calls` so `While` visits the condition and body and `Break`/`Continue` have no calls.

  Replace the return-only structural helper with a fallthrough-oriented helper. A general `while` can fall through. A `while true` is non-fallthrough only when a recursive syntax scan of its body finds no `BreakStmt` targeting that loop; stop that scan when it encounters a nested `WhileStmt`, but recurse through `if`, `when`, lifecycle, and ordinary blocks.

- [ ] **Step 4: Run semantic GREEN and commit**

  Run:

  ```powershell
  cargo test -p keld-semantics
  cargo test -p keld-syntax
  ```

  Expected: PASS; grammar goldens remain unchanged because no syntax production changed.

  Commit:

  ```powershell
  git add crates/keld-semantics/src/features.rs crates/keld-semantics/src/hir.rs crates/keld-semantics/src/check.rs crates/keld-semantics/tests/feature_gate.rs crates/keld-semantics/tests/entrypoint.rs crates/keld-semantics/tests/loops.rs
  git commit -m "feat(semantics): type while break and continue"
  ```

---

### Task 2: Lower structured loops into cyclic Flow CFG with explicit exits

**Files:**
- Modify: `crates/keld-flow/src/lower.rs`
- Modify: `crates/keld-flow/tests/control_flow.rs`
- Modify: `crates/keld-flow/tests/storage_scopes.rs`

**Interfaces:**
- Add an internal loop-target stack to `FunctionBuilder`.
- Each target stores the condition-entry block, loop-exit block, outer storage scope, and active-lifecycle depth at loop entry.
- Reuse `Terminator::{Goto,Branch,ExitScopes}`; do not add a new terminator.

- [ ] **Step 1: Write Flow RED tests for one loop, nested targets, and cleanup edges**

  Add a basic source test:

  ```keld
  fn main() -> Int {
      var i = 0
      while i < 3 { i += 1 }
      return i
  }
  ```

  Require the lowered function to contain a condition branch and a reachable edge from the body region back to the condition region.

  Add a nested case:

  ```keld
  fn main() -> Int {
      var outer = 0
      while outer < 2 {
          outer += 1
          var inner = 0
          while inner < 3 {
              inner += 1
              if inner == 1 { continue }
              break
          }
      }
      return outer
  }
  ```

  Prove the inner `continue` returns to the inner condition and the inner `break` reaches the inner exit, not the outer targets.

  Add a lifecycle case:

  ```keld
  entity E { value: Int }
  fn main() -> Int {
      var i = 0
      while i < 2 {
          lifecycle iteration {
              let e = E(value: i)
              i += 1
              continue
          }
      }
      return i
  }
  ```

  Require the `continue` edge to be an `ExitScopes` that names both the nested storage scopes being left and the `iteration` lifecycle before its `Goto` target.

- [ ] **Step 2: Verify Flow RED**

  Run:

  ```powershell
  cargo test -p keld-flow --test control_flow --test storage_scopes
  ```

  Expected: loop sources now reach flow lowering but fail because `lower_statement` has no loop variants.

- [ ] **Step 3: Implement `LoopTarget` and `lower_while`**

  Add an internal shape equivalent to:

  ```rust
  #[derive(Clone, Copy)]
  struct LoopTarget {
      condition: BlockId,
      exit: BlockId,
      outer_storage_scope: StorageScopeId,
      lifecycle_depth: usize,
  }
  ```

  Add `loop_targets: Vec<LoopTarget>` to `FunctionBuilder`.

  Lower each `while` with four logical regions:

  1. a condition-evaluation block in a child storage scope;
  2. a condition-branch block in the loop's outer storage scope;
  3. a body entry block in a separate child storage scope;
  4. an exit block in the outer storage scope.

  The pre-loop block goes to condition evaluation. After lowering the Bool expression, terminate the condition-evaluation scope with:

  ```text
  ExitScopes([condition_scope]) -> Goto(condition_branch)
  ```

  Then branch to body or exit. This makes the condition a full-expression boundary before either successor.

  Push the loop target only while lowering the body. Normal body fallthrough exits the body storage scope and goes to condition evaluation.

  For `break`/`continue`, collect storage scopes from the current scope back to but excluding `outer_storage_scope`; collect active lifecycles above `lifecycle_depth` in innermost-first cleanup order; terminate with `ExitScopes` to loop exit or condition respectively. A terminated block remains sealed so subsequent source statements do not get attached to that path.

- [ ] **Step 4: Run Flow GREEN and commit**

  Run:

  ```powershell
  cargo test -p keld-flow
  ```

  Expected: PASS, including existing `if`/`when`/lifecycle CFG tests.

  Commit:

  ```powershell
  git add crates/keld-flow/src/lower.rs crates/keld-flow/tests/control_flow.rs crates/keld-flow/tests/storage_scopes.rs
  git commit -m "feat(flow): lower structured while loops"
  ```

---

### Task 3: Replace lifecycle DAG scheduling with a convergent cyclic worklist

**Files:**
- Modify: `crates/keld-lifecycle/src/verify.rs`
- Modify: `crates/keld-lifecycle/src/provenance.rs`
- Create or modify: `crates/keld-lifecycle/tests/control_flow.rs`
- Modify: `crates/keld-lifecycle/tests/retirement.rs` if the existing retirement diagnostics are asserted there

**Interfaces:**
- `AbstractState` remains the lifecycle abstract state.
- The analyzer must expose converged per-block entry states to a one-time diagnostic/fact replay.
- `VerifiedFlowModule::entity_facts_at` must still return exactly one final fact snapshot per operation.

- [ ] **Step 1: Write lifecycle RED tests for cyclic state propagation**

  Add an accepted loop whose outer entity remains live:

  ```keld
  entity E { value: Int }
  fn read_many(e: E) -> Int {
      var i = 0
      var total = 0
      while i < 3 {
          total += e.value
          i += 1
      }
      return total
  }
  fn main() -> Int {
      lifecycle level {
          let e = E(value: 2)
          return read_many(e)
      }
  }
  ```

  Require lifecycle verification to succeed and every reachable loop operation to have final facts.

  Add a retirement join:

  ```keld
  entity E { value: Int }
  fn maybe_retire(e: E, retire_now: Bool) -> Int retires e {
      var i = 0
      while i < 1 {
          if retire_now { retire e; break }
          i += 1
      }
      return e.value
  }
  fn main() -> Int { return 0 }
  ```

  Require the post-loop direct use to be rejected with the existing control-flow-join liveness diagnostic (`KLD1003`), not accepted because the retirement backedge/path was skipped.

- [ ] **Step 2: Verify lifecycle RED**

  Run:

  ```powershell
  cargo test -p keld-lifecycle --test control_flow --test retirement
  ```

  Expected: at least the cyclic verification case fails or fails to record final facts because `Analyzer::run` waits for DAG indegrees to reach zero.

- [ ] **Step 3: Implement a two-phase fixed-point analyzer**

  Remove the indegree/`complete_predecessor` scheduler from `Analyzer::run`.

  Introduce an analysis phase distinction equivalent to `Solve` versus `Replay`:

  - **Solve:** `incoming: Vec<Option<AbstractState>>`; seed the entry with `initial_state`; process a `VecDeque<BlockId>`; transfer a block; join each outgoing state into the successor; enqueue only when the successor state changes. Diagnostic emission, proof numbering, and `entity_facts` insertion are disabled in this phase. Function-effect inference remains monotone and active.
  - **Replay:** after convergence, walk reachable blocks in stable `BlockId` order once using the converged entry state. Record `EntityOperationFacts`, diagnostics, and proof annotations exactly once. Do not feed replay outputs back into the solver.

  Save the solve-phase `Inference` result before replay and restore it afterward so summary inference is independent of diagnostic replay order.

  Keep `AbstractState::join` as the lattice join. Joining equal states must be idempotent so the queue terminates.

- [ ] **Step 4: Run lifecycle GREEN and broader lifecycle gate**

  Run:

  ```powershell
  cargo test -p keld-lifecycle
  cargo test -p keld-flow
  ```

  Expected: PASS with no duplicated diagnostics/facts and unchanged acyclic behavior.

- [ ] **Step 5: Commit the cyclic verifier infrastructure**

  ```powershell
  git add crates/keld-lifecycle/src/verify.rs crates/keld-lifecycle/src/provenance.rs crates/keld-lifecycle/tests/control_flow.rs crates/keld-lifecycle/tests/retirement.rs
  git commit -m "refactor(lifecycle): solve cyclic flow to fixed point"
  ```

  Omit unchanged paths from `git add` rather than creating empty edits.

---

### Task 4: Make repeated entity-producing sites provenance-safe across iterations

**Files:**
- Modify: `crates/keld-lifecycle/src/provenance.rs`
- Modify: `crates/keld-lifecycle/src/verify.rs`
- Modify: `crates/keld-lifecycle/src/facts.rs`
- Modify: `crates/keld-lifecycle/tests/provenance_facts.rs`
- Modify: `crates/keld-lifecycle/tests/control_flow.rs`

**Interfaces:**
- Extend lifecycle provenance with a finite `imprecise`/history concept; do not allocate abstract provenance dynamically per runtime iteration.
- Treat repeated definitions from cyclic `AllocateEntity`, entity-returning `Call`, and `ResolveLink` sites conservatively.
- `AliasRelation` remains `MustAlias | MustDistinct | MayAlias`.

- [ ] **Step 1: Write the repeated-site RED regressions**

  Add a same-static-allocation-site program that carries one iteration's entity into the next:

  ```keld
  entity Item { value: Int }
  fn inspect(seed: Item) -> Int {
      var previous = seed
      var i = 0
      while i < 2 {
          let current = Item(value: i)
          if i == 1 {
              if previous != current {
                  retire current
                  return previous.value
              } else {
                  return 99
              }
          }
          previous = current
          i += 1
      }
      return -1
  }
  fn main() -> Int {
      lifecycle level {
          let seed = Item(value: 7)
          return inspect(seed)
      }
  }
  ```

  At the second dynamic execution of the allocation, require the verifier not to classify `previous` and `current` as `MustAlias`. In the `previous != current` true branch require `MustDistinct`, allowing `previous` after `retire current`.

  Add a conservative carried-history case where two loop-carried locals may denote different historical instances and require `MayAlias` before an identity comparison rather than a false must relation.

  Add equivalent repeated-site coverage for a `when` link resolution inside a loop, because one static resolve site can produce different entity identities on different iterations.

- [ ] **Step 2: Verify provenance RED**

  Run:

  ```powershell
  cargo test -p keld-lifecycle --test provenance_facts --test control_flow
  ```

  Expected: the same static entity-producing provenance is currently reused and at least one new assertion exposes an incorrect must relation or over-precise retirement result.

- [ ] **Step 3: Add finite cyclic-definition history provenance**

  During catalog construction, compute the set of Flow blocks that belong to a CFG cycle. Use a deterministic strongly-connected-component traversal; a component is cyclic when it has more than one block or one block with a self-edge.

  For each entity-producing provenance defined in a cyclic block, preallocate bounded history provenances for compatible entity locals. Record them in the catalog keyed by `(definition_provenance, LocalId)` and include them in `catalog.entities`, so `AbstractState` sizes remain fixed before analysis begins.

  Extend `AbstractState` with a set marking history provenances as imprecise. Join this set by union. Project it into `EntityOperationFacts`.

  Immediately before a cyclic entity-producing site redefines its precise static provenance:

  - rewrite each local still carrying that previous precise provenance to its preallocated local-history provenance;
  - merge the old reference state and origin into the history state and mark the history provenance imprecise;
  - clear stale SSA `values` that still point at the previous dynamic instance rather than carrying them through another execution;
  - drop equality/distinctness relations whose old meaning depended on the redefined precise provenance;
  - then initialize the static provenance as the new precise dynamic result and apply ordinary fresh/distinct or resolve/call rules.

  Apply this before repeated `AllocateEntity`, entity-returning `Call`, and looped `ResolveLink` definitions, not only allocations.

  For an imprecise history provenance, same-ID occurrence does not by itself prove two different source references must-alias. Retiring through such a reference invalidates the conservative history class rather than claiming every possible historical identity was exactly retired. Identity `==`/`!=` branches between distinct history provenance IDs can still add equality/distinctness facts for that branch.

- [ ] **Step 4: Run provenance GREEN and all lifecycle tests**

  Run:

  ```powershell
  cargo test -p keld-lifecycle
  ```

  Expected: PASS; ordinary acyclic copied aliases remain `MustAlias`, separate fresh acyclic allocations remain `MustDistinct`, and unrefined parameters remain `MayAlias` exactly as before.

- [ ] **Step 5: Commit**

  ```powershell
  git add crates/keld-lifecycle/src/provenance.rs crates/keld-lifecycle/src/verify.rs crates/keld-lifecycle/src/facts.rs crates/keld-lifecycle/tests/provenance_facts.rs crates/keld-lifecycle/tests/control_flow.rs
  git commit -m "fix(lifecycle): widen loop-carried entity provenance"
  ```

---

### Task 5: Reset exited loop-body homes and prove cyclic storage cleanup

**Files:**
- Modify: `crates/keld-storage/src/verify.rs`
- Modify: `crates/keld-storage/src/cleanup.rs` only if a small helper belongs with existing scope-order helpers
- Modify: `crates/keld-storage/tests/home_verifier.rs`
- Modify: `crates/keld-storage/tests/cleanup_planner.rs`
- Modify: `crates/keld-storage/tests/storage_scopes.rs`

**Interfaces:**
- Keep `Home::join` unchanged.
- On `Terminator::ExitScopes`, normalize the outgoing abstract state to the state after those lexical scopes have been cleaned.
- Keep cleanup-plan annotations based on the pre-exit state, because that is the state whose homes must actually be destroyed.

- [ ] **Step 1: Write storage RED tests**

  Add a body-local repeated initialization case:

  ```keld
  fn main() -> Int {
      var i = 0
      while i < 3 {
          let text = "x"
          i += text.byte_length
      }
      return i
  }
  ```

  Require the body-local Text home to be treated as a fresh `Initialize` each iteration, with a cleanup action on the backedge, not as replacement of the previous iteration's home.

  Add a loop-head `MaybeLive` rejection:

  ```keld
  fn main() -> Int {
      var value = "x"
      var i = 0
      while i < 2 {
          if i == 0 { let moved = take value }
          i += 1
      }
      return value.byte_length
  }
  ```

  Require `KLD2008` at the read after the join.

  Add the repair form:

  ```keld
  fn main() -> Int {
      var value = "x"
      var i = 0
      while i < 2 {
          if i == 0 { let moved = take value }
          value = "reset"
          i += 1
      }
      return value.byte_length
  }
  ```

  Require success.

  Add zero-iteration definite initialization:

  ```keld
  fn main() -> Int {
      var value: Text
      while false { value = "ready" }
      return value.byte_length
  }
  ```

  Require the existing non-live-home diagnostic.

  Add a cleanup-order case where outer `var` homes are moved/reinitialized in different orders on different iterations and require the containing outer scope to enter `tracked_scopes`, with bounded drop flags/tracker metadata.

- [ ] **Step 2: Verify storage RED**

  Run:

  ```powershell
  cargo test -p keld-storage --test home_verifier --test cleanup_planner --test storage_scopes
  ```

  Expected: body-scope re-entry exposes that current `ExitScopes` handling clears cleanup-order metadata but leaves body-local `Home` state live across the backedge.

- [ ] **Step 3: Normalize abstract state after lexical scope exit**

  In `verify_function`, retain the current `exit_states[block] = pre_exit_state` so `annotate_exit_plan` sees exactly what must be cleaned.

  For the successor state of `ExitScopes`:

  - continue calling `cleanup::exit_scopes` for cleanup-order state;
  - for each single-home local whose `function.local_scopes[local]` is one of the exited storage scopes, set `state.homes[local] = Home::Empty(EmptyReason::Uninitialized)`;
  - remove those locals from `borrowed` if present;
  - do not reset an outer home merely because its value was mutated in the loop;
  - keep pending-call and indexed-replacement invariants unchanged; those structures must already be closed before a terminator.

  This state transformation models completed cleanup and makes the same static body scope safe to enter again on the next dynamic iteration.

  Keep `join_states` and `Home::join` unchanged so loop heads converge naturally to `MaybeLive` when outer paths disagree.

- [ ] **Step 4: Run storage GREEN plus IR lowering smoke**

  Run:

  ```powershell
  cargo test -p keld-storage
  cargo test -p keld-ir --test lowering
  ```

  Expected: PASS; loop-body cleanup is explicit and no existing List/Text storage behavior regresses.

- [ ] **Step 5: Commit**

  ```powershell
  git add crates/keld-storage/src/verify.rs crates/keld-storage/src/cleanup.rs crates/keld-storage/tests/home_verifier.rs crates/keld-storage/tests/cleanup_planner.rs crates/keld-storage/tests/storage_scopes.rs
  git commit -m "fix(storage): reset lexical homes on loop exits"
  ```

  Omit `cleanup.rs` if the implementation stays entirely in `verify.rs`.

---

### Task 6: Prove cyclic executable IR and interpreter behavior without a loop opcode

**Files:**
- Modify: `crates/keld-ir/tests/lowering.rs`
- Modify: `crates/keld-ir/tests/validation.rs` if that is the central CFG-validation test file; otherwise add the case to the existing appropriate validation test file
- Create: `crates/keld-interpreter/tests/control_flow.rs`
- Create: `crates/keld-cli/tests/fixtures/control_flow_loop.keld`
- Create: `crates/keld-cli/tests/fixtures/control_flow_allocations.keld`

**Interfaces:**
- `keld_ir::Module` stays unchanged.
- `keld_ir::validate` must accept a valid cyclic CFG and continue rejecting invalid register/home/view/lifecycle states.
- `run_text_for_test` remains the interpreter semantic oracle.

- [ ] **Step 1: Add IR and interpreter RED/contract tests**

  Add `control_flow_loop.keld`:

  ```keld
  fn main() -> Int {
      var i = 0
      var total = 0
      while i < 6 {
          i += 1
          if i == 2 { continue }
          if i == 5 { break }
          total += i
      }
      return total
  }
  ```

  Expected result: `8`.

  Add `control_flow_allocations.keld`:

  ```keld
  fn main() -> Int {
      var i = 0
      var total = 0
      while i < 3 {
          let text = "x" + "y"
          total += text.byte_length
          i += 1
      }
      return total
  }
  ```

  Expected result: `6`.

  In `keld-ir/tests/lowering.rs`, compile the first fixture through storage verification and IR lowering. Require `validate(&module).is_empty()` and prove the executable CFG has at least one reachable edge returning to an earlier loop region; also assert there is no new instruction/terminator variant for loops.

  In `keld-interpreter/tests/control_flow.rs`, run the two fixtures or equivalent inline sources. Add nested `break`/`continue`, zero-iteration, and condition re-evaluation assertions.

  Add explicit lifecycle exit behavior:

  ```keld
  entity E { value: Int }
  fn main() -> Int {
      lifecycle outer {
          var i = 0
          var seen = 0
          while i < 3 {
              lifecycle iteration {
                  let e = E(value: i)
                  i += 1
                  if i == 1 { continue }
                  if i == 3 { break }
                  seen += e.value
              }
          }
          return seen
      }
  }
  ```

  Require successful execution and let existing runtime/lifecycle instrumentation tests prove the nested lifecycle ends on each structured exit.

- [ ] **Step 2: Run IR/interpreter tests**

  ```powershell
  cargo test -p keld-ir --test lowering
  cargo test -p keld-interpreter --test control_flow
  ```

  Expected after Tasks 1-5: these should either pass directly through the existing IR/interpreter machinery or expose a remaining forward-only CFG assumption.

- [ ] **Step 3: Keep executable IR generic**

  If the new cyclic validation case exposes a forward-only assumption in `crates/keld-ir/src/validate.rs`, change only the affected `compute_*_dataflow` routine to the same changed-state worklist pattern already used by its other dataflow passes: initialize entry facts, join predecessor facts, re-enqueue successors when an outgoing fact changes, and terminate at fixed point. Do not weaken any validation rule and do not add a loop IR form.

  The interpreter must continue selecting the next `IrBlockId` from the existing terminator semantics; no loop-specific interpreter branch is permitted.

- [ ] **Step 4: Run complete IR/interpreter GREEN and commit**

  ```powershell
  cargo test -p keld-ir
  cargo test -p keld-interpreter
  ```

  Commit all actual changes from this task:

  ```powershell
  git add crates/keld-ir/src/validate.rs crates/keld-ir/tests crates/keld-interpreter/tests/control_flow.rs crates/keld-cli/tests/fixtures/control_flow_loop.keld crates/keld-cli/tests/fixtures/control_flow_allocations.keld
  git commit -m "test(ir): prove cyclic control-flow execution"
  ```

  Omit `validate.rs` if the regression passes without a validator change.

---

### Task 7: Extend Native-1 differential parity to repeated loop allocation attempts

**Files:**
- Modify: `crates/keld-native-backend/tests/differential.rs`
- Modify: `crates/keld-native-backend/tests/native_int.rs` only if a focused native loop smoke belongs there
- Reuse: `crates/keld-cli/tests/fixtures/control_flow_loop.keld`
- Reuse: `crates/keld-cli/tests/fixtures/control_flow_allocations.keld`

**Interfaces:**
- Reuse `run_differential_case`, `discover_failure_schedules`, `interpreter_run`, and the existing test runtime observation protocol.
- Repeated executions of one static allocation site keep one frozen `site_id` and increment its `attempt` counter.

- [ ] **Step 1: Add differential loop coverage**

  Extend the existing source-surface fixture set used by:

  - `every_source_fixture_has_a_shared_allocation_failure_schedule_at_o0_and_o2`
  - `source_surface_fixtures_extend_the_same_differential_schedule`

  to include both Control Flow-1 fixtures.

  Add a focused regression for `control_flow_allocations.keld` that runs the interpreter with no failures, filters a repeated semantic allocation phase such as `Concat`, and asserts that one static `site_id` is observed at attempts `1`, `2`, and `3` in order. Then run the normal differential harness so each discovered attempt can be failed independently and compared with native O0/O2.

- [ ] **Step 2: Build the test runtime and verify Native RED/GREEN**

  On the pinned Windows GNU environment:

  ```powershell
  $env:CARGO_BUILD_TARGET='x86_64-pc-windows-gnu'
  . scripts/activate-llvm.ps1
  cargo build -p keld-native-ffi-test --release --target x86_64-pc-windows-gnu
  cargo test -p keld-native-backend --test differential control_flow -- --nocapture
  ```

  If Rust's exact test-name filter differs, run the complete differential test binary instead:

  ```powershell
  cargo test -p keld-native-backend --test differential -- --nocapture
  ```

  Expected: interpreter and native return/fault observations, source spans, allocation site IDs, phases, attempts, and allowed/failure flags match at O0 and O2.

- [ ] **Step 3: Run Native-1 focused regression suite**

  ```powershell
  cargo test -p keld-native-backend --test native_int -- --nocapture
  cargo test -p keld-native-backend --test differential -- --nocapture
  cargo test -p keld-native-backend --test surface_audit -- --nocapture
  ```

  Expected: PASS. The instruction/terminator surface count remains the current 48 instructions and 6 terminators unless an unrelated pre-existing count changed; Control Flow-1 itself adds zero variants.

- [ ] **Step 4: Commit**

  ```powershell
  git add crates/keld-native-backend/tests/differential.rs crates/keld-native-backend/tests/native_int.rs
  git commit -m "test(native): cover loop differential parity"
  ```

  Omit `native_int.rs` if no focused smoke was needed.

---

### Task 8: Promote Control Flow-1 through CLI acceptance and normative documentation

**Files:**
- Modify: `crates/keld-cli/tests/cli.rs`
- Modify: `crates/keld-cli/tests/milestone_acceptance.rs`
- Modify: `crates/keld-cli/tests/fixtures/fail_unsupported.keld`
- Modify: `README.md`
- Create: `docs/spec/control-flow.md`
- Modify: `docs/spec/grammar.md`
- Modify: `docs/spec/storage-values.md`
- Modify: `docs/compiler-architecture.md`
- Modify: `docs/milestone-acceptance.md`
- Modify: `docs/superpowers/specs/2026-08-11-keld-language-design.md`

**Interfaces:**
- `control-flow.md` becomes the normative semantic supplement for accepted loop behavior.
- Historical bootstrap criteria remain historical; Control Flow-1 gets a new acceptance section.

- [ ] **Step 1: Replace the now-obsolete unsupported-loop fixture**

  Change `fail_unsupported.keld` to a still-deferred parsed feature, using `match`:

  ```keld
  fn main() -> Int {
      match 0 {
          _ => 0
      }
  }
  ```

  Update `cli.rs` exact static-diagnostic expectation to line 2, column 5, `KLD0004`, and the message `` `match` is parsed but not supported by the bootstrap compiler `` with the existing help text.

  Add `control_flow_loop.keld` to both interpreter and native representative fixture tables with `8\n`.

- [ ] **Step 2: Add Control Flow-1 acceptance tests**

  Do not renumber or alter the historical 16 bootstrap criteria. Add a separate `CONTROL_FLOW_1` acceptance table/section in `milestone_acceptance.rs` covering:

  - basic while result `8`;
  - repeated managed allocation result `6`;
  - outside-loop `break`/`continue` static `KLD0112`;
  - loop-head `MaybeLive` static `KLD2008`;
  - explicit lifecycle cleanup on `continue` and `break`;
  - loop-carried entity liveness/provenance regression;
  - validated cyclic IR.

  Use the existing `compile_source`, `run_source`, CLI, and IR validation helpers rather than duplicating a compiler pipeline in tests.

- [ ] **Step 3: Write the normative control-flow document**

  Create `docs/spec/control-flow.md` with these normative sections:

  1. accepted source forms and innermost-loop target rule;
  2. Bool condition evaluation and full-expression boundary;
  3. zero-iteration behavior;
  4. lexical body storage scope per dynamic iteration;
  5. structured cleanup semantics of fallthrough/continue/break/return;
  6. no implicit lifecycle from `while`;
  7. fixed-point Home joins and `MaybeLive` use/reinitialization rules;
  8. lifecycle/provenance joins and conservative repeated-site identity handling;
  9. literal-true structural non-fallthrough rule;
  10. runtime-fault contract;
  11. executable-IR/backend neutrality;
  12. deferred loop forms.

  Keep it consistent with `storage-values.md` and the approved design; do not redefine storage or lifecycle rules in conflicting terms.

- [ ] **Step 4: Update cross-references and status docs**

  - In `grammar.md`, point loop semantic restrictions to `control-flow.md` and add it beside numeric/storage normative documents in the final grammar-scope paragraph.
  - In `storage-values.md`, add a short cyclic-control-flow paragraph stating that the existing Home join applies at loop backedges and that exited lexical homes are cleaned/reset before re-entry.
  - In `compiler-architecture.md`, replace acyclic-only wording with explicit cyclic Flow CFG fixed-point lifecycle/storage analysis while preserving crate ownership.
  - In the language-design document's normative-spec list, add `docs/spec/control-flow.md`; leave its historical first-buildable-milestone exclusion list intact.
  - In README, move `while`, `break`, and `continue` from deferred to implemented surface and link the new spec.
  - In `milestone-acceptance.md`, add a Control Flow-1 section mapping every design completion gate to concrete tests and record that executable IR still uses the existing 48/6 surface.

- [ ] **Step 5: Run CLI/docs-facing GREEN and commit**

  ```powershell
  cargo test -p keld-cli --test cli --test milestone_acceptance
  cargo test -p keld-semantics --test feature_gate --test loops
  cargo test -p keld-ir --test lowering
  ```

  Then commit:

  ```powershell
  git add README.md docs/spec/control-flow.md docs/spec/grammar.md docs/spec/storage-values.md docs/compiler-architecture.md docs/milestone-acceptance.md docs/superpowers/specs/2026-08-11-keld-language-design.md crates/keld-cli/tests/cli.rs crates/keld-cli/tests/milestone_acceptance.rs crates/keld-cli/tests/fixtures/fail_unsupported.keld crates/keld-cli/tests/fixtures/control_flow_loop.keld crates/keld-cli/tests/fixtures/control_flow_allocations.keld
  git commit -m "docs: accept Keld Control Flow-1"
  ```

---

### Task 9: Run the complete Control Flow-1 acceptance gate and review scope

**Files:**
- Inspect: all changed files against `407f6d85a5b9322f6a8496b9d9cdb9525577e665`
- Modify: only regressions required by a failing acceptance gate; do not expand milestone scope

- [ ] **Step 1: Build pinned native runtime artifacts**

  ```powershell
  $env:CARGO_BUILD_TARGET='x86_64-pc-windows-gnu'
  . scripts/activate-llvm.ps1
  cargo build -p keld-native-ffi --release --target x86_64-pc-windows-gnu
  cargo build -p keld-native-ffi-test --release --target x86_64-pc-windows-gnu
  ```

  Confirm the expected sibling runtime artifacts exist under `target/x86_64-pc-windows-gnu/release/` before native CLI/differential tests.

- [ ] **Step 2: Run focused milestone suites**

  ```powershell
  cargo test -p keld-semantics
  cargo test -p keld-flow
  cargo test -p keld-lifecycle
  cargo test -p keld-storage
  cargo test -p keld-ir
  cargo test -p keld-interpreter
  cargo test -p keld-native-backend --test differential -- --nocapture
  cargo test -p keld-native-backend --test native_int -- --nocapture
  cargo test -p keld-native-backend --test surface_audit -- --nocapture
  cargo test -p keld-cli --test cli --test milestone_acceptance
  ```

  Expected: all PASS. In particular, there must be no infinite compile-time verifier loop and no dynamic metadata growth with loop iteration count.

- [ ] **Step 3: Run full debug/release workspace gates**

  ```powershell
  cargo test --workspace --all-targets --no-fail-fast
  cargo test --workspace --all-targets --release --no-fail-fast
  cargo clippy --workspace --all-targets -- -D warnings
  cargo fmt --all -- --check
  git diff --check
  ```

  Expected: all commands exit zero with the pinned Rust 1.97.0 toolchain and Windows GNU target.

- [ ] **Step 4: Run source-level interpreter/native smoke**

  ```powershell
  cargo run --release -p keld-cli -- run --engine interpreter crates/keld-cli/tests/fixtures/control_flow_loop.keld
  cargo run --release -p keld-cli -- run --engine native crates/keld-cli/tests/fixtures/control_flow_loop.keld
  cargo run --release -p keld-cli -- run --engine interpreter crates/keld-cli/tests/fixtures/control_flow_allocations.keld
  cargo run --release -p keld-cli -- run --engine native crates/keld-cli/tests/fixtures/control_flow_allocations.keld
  cargo run --release -p keld-cli -- dump-ir crates/keld-cli/tests/fixtures/control_flow_loop.keld
  ```

  Expected stdout is `8`, `8`, `6`, `6` respectively, each followed by one newline; `dump-ir` is non-empty and contains only existing executable instruction/terminator forms.

- [ ] **Step 5: Review the final diff against the approved scope**

  Inspect:

  ```powershell
  git diff 407f6d85a5b9322f6a8496b9d9cdb9525577e665...HEAD --stat
  git diff 407f6d85a5b9322f6a8496b9d9cdb9525577e665...HEAD -- docs crates README.md
  ```

  Confirm:

  - no new source syntax beyond the already-grammatical three forms;
  - no new memory-management mechanism;
  - no implicit loop lifecycle;
  - no new executable-IR/native loop opcode;
  - no weakening of existing storage/lifecycle diagnostics;
  - no accidental enablement of `match`, generics, imports, typed errors, recursion, or other deferred features;
  - no temporary CI/debug scaffolding remains;
  - all plan checkboxes reflect actual verified work.

- [ ] **Step 6: Commit any final acceptance-only adjustments**

  If Step 5 required documentation/test bookkeeping only, commit it separately:

  ```powershell
  git add -A
  git commit -m "test: close Control Flow-1 acceptance"
  ```

  Do not create the next milestone in this plan. Control Flow-1 is complete only after the full gate above is green.
