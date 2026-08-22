# Keld Control Flow-1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement source-level `while`, `break`, and `continue` as ordinary cyclic Keld control flow while preserving the existing single-home storage lattice, deterministic scope/lifecycle cleanup, conservative entity provenance, validated executable IR, and interpreter/native parity.

**Architecture:** Keep the existing grammar and backend-neutral executable IR. Add loop forms to semantic HIR, lower them into existing `Branch`/`Goto`/`ExitScopes`, replace the lifecycle verifier's DAG scheduling with monotone fixed-point analysis, introduce bounded merge provenance at cyclic entity joins, reset exited lexical homes before static loop scopes are re-entered, then prove the same validated cyclic IR through the interpreter and LLVM O0/O2 paths. No loop-specific runtime object, memory manager, or backend opcode is introduced.

**Tech Stack:** Rust 1.97.0 workspace, existing Keld syntax/HIR/flow/lifecycle/storage/IR/interpreter crates, LLVM 22.1.8 Native-1 Windows GNU backend, MinGW-w64 `x86_64-pc-windows-gnu`, and the existing differential allocation-observation harness.

**Spec:** `docs/superpowers/specs/2026-08-22-keld-control-flow-1-design.md`

## Global Constraints

- Work only on branch `feature/control-flow-1`, based on design commit `407f6d85a5b9322f6a8496b9d9cdb9525577e665` or a descendant.
- The parser grammar already contains `while`, `break`, and `continue`; do not invent new syntax.
- `break` and `continue` target only the innermost enclosing `while`. Labels and control-flow values remain deferred.
- A plain `while` creates a lexical storage scope for its body, not a Custody Ledger lifecycle.
- Every normal scope exit caused by body fallthrough, `continue`, `break`, or `return` must use existing deterministic cleanup machinery. Non-catchable runtime faults keep their existing cleanup contract.
- Keep `Home = Empty(reason) | Live | MaybeLive` unchanged. Loops extend the current join semantics to cyclic CFGs rather than adding a loop-only state rule.
- Preserve `Known`/`Divergent` cleanup-order tracking. Hidden metadata must be bounded by static function blocks/locals/scopes, never dynamic iteration count.
- Entity analysis must lose precision rather than make an incorrect must-alias, must-distinct, or live-proof claim.
- Do not add an executable-IR loop instruction. The interpreter and LLVM backend must consume the same existing control-flow and cleanup forms.
- Do not enable `for`, iterators, `loop`, labels, `while let`, loop `else`, pattern matching, typed errors, recursion, async, concurrency, or another deferred feature.
- Use TDD for every task: focused RED, minimum implementation, focused GREEN, then the broader gate.
- Preserve historical bootstrap wording. Add Control Flow-1 as a later milestone rather than rewriting the historical first-milestone record.

---

### Task 1: Admit loops in semantic HIR and define diagnostics/fallthrough

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
- Add source diagnostic `KLD0112` for loop control outside a loop.
- Keep `KLD0107` as the existing non-`Bool` condition diagnostic.
- Make all HIR visitors, especially `called_functions`, recurse through loop conditions and bodies.

- [ ] **Step 1: Write semantic RED tests**

  In `crates/keld-semantics/tests/loops.rs`, add a typed loop:

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

  Require a `HirStmtKind::While` whose nested bodies retain the `Break` and `Continue` statements.

  Add outside-loop cases:

  ```keld
  fn main() -> Int { break }
  fn main() -> Int { continue }
  ```

  Require `KLD0112`, no semantic module, and a message naming the offending keyword.

  Add a condition-type case:

  ```keld
  fn main() -> Int {
      while 1 { break }
      return 0
  }
  ```

  Require `KLD0107`.

  In `entrypoint.rs`, add literal-true fallthrough tests:

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

  Also prove that a `break` owned by a nested loop does not make the outer literal-true loop fall through:

  ```keld
  fn spin(flag: Bool) -> Int {
      while true {
          while flag { break }
      }
  }
  fn main() -> Int { return 0 }
  ```

  Finally put the recursion-under-loop regression in `loops.rs` because this repository has no dedicated recursion test file: a call cycle hidden only inside a loop must still be detected by the current recursion gate.

- [ ] **Step 2: Update feature-gate tests and verify RED**

  Remove `while`, `break`, and `continue` from `every_deferred_construct_is_rejected_before_typing` in `feature_gate.rs`.

  Replace the current unsupported-feature cascade example that depends on `while` with an unsupported outer enum containing a generic child:

  ```keld
  enum Box[T] { Value(T) }
  fn main() -> Int { return 0 }
  ```

  Require one outer `enum` `KLD0004`, not a second generic diagnostic from inside the rejected subtree.

  Run:

  ```powershell
  $env:CARGO_BUILD_TARGET='x86_64-pc-windows-gnu'
  cargo test -p keld-semantics --test loops --test feature_gate --test entrypoint
  ```

  Expected: RED because loops are still feature-gated and HIR has no loop variants.

- [ ] **Step 3: Implement semantic loop forms**

  In `hir.rs`, add:

  ```rust
  #[derive(Clone, Debug)]
  pub struct HirWhile {
      pub condition: HirExpr,
      pub body: HirBlock,
  }
  ```

  plus `While`, `Break`, and `Continue` statement variants.

  In `features.rs`, stop classifying `SyntaxKind::{WhileStmt,BreakStmt,ContinueStmt}` as unsupported and leave every other deferred gate unchanged.

  In `BodyChecker`, add `loop_depth: usize`, initialized to zero. Implement:

  - `check_while`: type-check its condition with the same Bool requirement as `if`, clone the lexical environment for the body, increment `loop_depth` only while checking that body, then emit `HirStmtKind::While`.
  - `check_break` / `check_continue`: emit `KLD0112` when `loop_depth == 0`; otherwise emit the HIR marker.

  Extend `collect_block_calls`: `While` visits condition and body; `Break` and `Continue` contain no calls.

  Replace the current return-only structural helper with fallthrough classification. General `while condition` may fall through. Literal `while true` is non-fallthrough only when a recursive syntax walk of its body finds no `BreakStmt` targeting that same loop. Recurse through `if`, `when`, lifecycle, and ordinary nested blocks, but do not count breaks beneath a nested `WhileStmt`.

- [ ] **Step 4: Run semantic GREEN and commit**

  ```powershell
  cargo test -p keld-semantics
  cargo test -p keld-syntax
  ```

  Expected: PASS; parser/grammar goldens remain unchanged.

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
- Reuse `Terminator::{Goto,Branch,ExitScopes}`; add no loop terminator.
- Reuse existing `storage_scopes_until`, `active_lifecycles`, `new_storage_scope`, and block sealing.

- [ ] **Step 1: Write Flow RED tests**

  Add a basic loop:

  ```keld
  fn main() -> Int {
      var i = 0
      while i < 3 { i += 1 }
      return i
  }
  ```

  Require a condition branch and a reachable body edge back to condition evaluation.

  Add nested-loop targeting:

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

  Prove inner `continue` returns to the inner condition and inner `break` reaches the inner exit.

  Add a lifecycle cleanup edge:

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

  Require `continue` to use `ExitScopes` with the lexical scopes being left and the explicit `iteration` lifecycle before jumping to condition evaluation.

- [ ] **Step 2: Verify Flow RED**

  ```powershell
  cargo test -p keld-flow --test control_flow --test storage_scopes
  ```

  Expected: RED because `lower_statement` has no loop forms.

- [ ] **Step 3: Implement `LoopTarget` and `lower_while`**

  Add an internal target equivalent to:

  ```rust
  #[derive(Clone, Copy)]
  struct LoopTarget {
      condition: BlockId,
      exit: BlockId,
      outer_storage_scope: StorageScopeId,
      lifecycle_depth: usize,
  }
  ```

  and `loop_targets: Vec<LoopTarget>` to `FunctionBuilder`.

  Lower each `while` into four logical regions:

  1. condition evaluation in a child storage scope;
  2. a condition-branch block in the loop's outer storage scope;
  3. body entry in a separate child storage scope;
  4. loop exit in the outer storage scope.

  The pre-loop block goes to condition evaluation. After producing the Bool result, close the condition full-expression with:

  ```text
  ExitScopes([condition_scope]) -> Goto(condition_branch)
  ```

  and branch there to body or exit. Scalar Bool survives this cleanup; condition-only managed temporaries do not.

  Push the loop target only while lowering the body. Normal body fallthrough closes body-local storage scopes and jumps to condition evaluation.

  For `break`/`continue`, call `storage_scopes_until(target.outer_storage_scope)` and collect `active_lifecycles[target.lifecycle_depth..]` in reverse order. Emit one `ExitScopes` to loop exit or condition evaluation. Keep the current source path sealed after the terminator.

- [ ] **Step 4: Run Flow GREEN and commit**

  ```powershell
  cargo test -p keld-flow
  ```

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
- Create: `crates/keld-lifecycle/tests/control_flow.rs`
- Modify: `crates/keld-lifecycle/tests/retirement.rs`

**Interfaces:**
- `AbstractState` remains the lifecycle abstract state.
- `VerifiedFlowModule::entity_facts_at` must still expose one final fact snapshot per reachable operation.
- Diagnostics/proof IDs must be emitted once from converged states, not once per solver iteration.

- [ ] **Step 1: Write lifecycle RED tests**

  Add an accepted loop carrying a stable entity parameter/reference:

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

  Require lifecycle verification to succeed and final facts to exist for every reachable entity operation in the loop.

  Add a retirement-join rejection:

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

  Require the post-loop use to produce existing join-liveness diagnostic `KLD1003`.

- [ ] **Step 2: Verify lifecycle RED**

  ```powershell
  cargo test -p keld-lifecycle --test control_flow --test retirement
  ```

  Expected: RED or missing reachable facts because current `Analyzer::run` uses predecessor indegrees and assumes a DAG.

- [ ] **Step 3: Implement solve/replay fixed-point analysis**

  Remove the indegree/`complete_predecessor` scheduler from `Analyzer::run`.

  Use two phases:

  **Solve**
  - `incoming: Vec<Option<AbstractState>>`;
  - seed entry with `initial_state()`;
  - process `VecDeque<BlockId>`;
  - transfer the block from its current joined entry state;
  - join each outgoing state into the successor;
  - enqueue a successor only when its incoming state changes;
  - suppress diagnostics, proof numbering, and `entity_facts` insertion while solving;
  - keep effect/return-origin inference active because summary inference itself is a monotone fixed point.

  **Replay**
  - after convergence, walk reachable blocks once in stable `BlockId` order;
  - begin each block from its converged entry state;
  - record `EntityOperationFacts`, diagnostics, and proof annotations exactly once;
  - do not feed replay output back into the solver.

  Preserve the solve-phase `Inference` result across replay so function summaries do not depend on replay order.

  Keep joins idempotent and finite: origins union finite source sets; equality/distinctness are finite sets; reference states only lose permission at joins.

- [ ] **Step 4: Run lifecycle GREEN and commit**

  ```powershell
  cargo test -p keld-lifecycle
  cargo test -p keld-flow
  ```

  Commit:

  ```powershell
  git add crates/keld-lifecycle/src/verify.rs crates/keld-lifecycle/src/provenance.rs crates/keld-lifecycle/tests/control_flow.rs crates/keld-lifecycle/tests/retirement.rs
  git commit -m "refactor(lifecycle): solve cyclic flow to fixed point"
  ```

---

### Task 4: Add bounded merge provenance for loop-carried entity references

**Files:**
- Modify: `crates/keld-lifecycle/src/verify.rs`
- Modify: `crates/keld-lifecycle/src/provenance.rs`
- Modify: `crates/keld-lifecycle/tests/provenance_facts.rs`
- Modify: `crates/keld-lifecycle/tests/control_flow.rs`

**Interfaces:**
- Extend `Catalog` with preallocated merge provenances keyed by `(BlockId, LocalId)` for entity-flow locals at join blocks.
- Add an analyzer-owned `join_at(block, incoming_states)` that can create a conservative local merge using only preallocated IDs.
- `AliasRelation` remains `MustAlias | MustDistinct | MayAlias`; `facts.rs` needs no new public relation.

- [ ] **Step 1: Write repeated-site RED regressions**

  Add a same-static-allocation-site loop carrying an older dynamic entity into the next iteration:

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

  At the second iteration, require `previous` and the new `current` not to be `MustAlias`. In the `previous != current` true branch require `MustDistinct`, so `previous` remains usable after `retire current`.

  Add a loop-carried case whose incoming identities differ but are not freshly proven distinct and require `MayAlias` before an identity comparison rather than an incorrect must relation.

  Add repeated `when` resolution inside a loop. The same static resolve site may resolve different identities on different iterations; a carried previous result and the new resolution must be conservative until comparison/refinement.

- [ ] **Step 2: Verify provenance RED**

  ```powershell
  cargo test -p keld-lifecycle --test provenance_facts --test control_flow
  ```

  Expected: current state joins either lose the carried local entirely or reuse one static entity-producing provenance too precisely.

- [ ] **Step 3: Implement fixed merge provenance at CFG joins**

  During catalog construction, compute predecessor counts once and preallocate one merge provenance for every entity-flow local at every block that has more than one predecessor. This is bounded by `blocks × entity locals`; it does not depend on runtime iteration count.

  Replace plain `AbstractState::join(states, cause)` at block entry with analyzer `join_at(block, states)`:

  1. use the existing abstract-state join for refs/origins/failure/equality/distinct base state;
  2. for each entity local, inspect its incoming `RefValue`s;
  3. if all incoming references are the same, keep that exact reference;
  4. if all incoming paths have a reference of the same entity type but provenances differ, point the joined local at the block/local merge provenance;
  5. compute the merge provenance's `RefState` as the least-permissive join of the incoming reference states, using `Dynamic` lifecycle when live lifecycle facts disagree;
  6. union incoming origins into the merge origin;
  7. do not make the merge provenance equivalent to any one constituent and do not manufacture distinct facts;
  8. if an incoming path lacks the local reference, preserve the existing conservative unavailable behavior rather than pretending it is live.

  This makes a loop header stable: preheader identity and backedge identity merge to one fixed provenance; later iterations do not allocate more compiler metadata.

  Fresh `AllocateEntity` already calls `distinguish_fresh`. Once the carried old reference has a distinct merge provenance, the new allocation can soundly become `MustDistinct` from that live carried reference. `ResolveLink` does not receive fresh-distinct treatment and therefore remains `MayAlias` until identity comparison. Entity-returning calls continue to follow their existing summary rules.

  Ensure stale static `ValueId` entries from a previous traversal cannot make a newly executed entity-producing operation look like its previous dynamic result before that operation is transferred. If the solve state reaches a defining block with such stale SSA values, clear only values defined by that block before transfer; do not clear locals or parameter values.

- [ ] **Step 4: Run provenance GREEN and commit**

  ```powershell
  cargo test -p keld-lifecycle
  ```

  Existing acyclic guarantees must remain unchanged: copied aliases are `MustAlias`, separately fresh simultaneous allocations are `MustDistinct`, and unrelated parameter/link identities remain `MayAlias` unless refined.

  Commit:

  ```powershell
  git add crates/keld-lifecycle/src/verify.rs crates/keld-lifecycle/src/provenance.rs crates/keld-lifecycle/tests/provenance_facts.rs crates/keld-lifecycle/tests/control_flow.rs
  git commit -m "fix(lifecycle): merge loop-carried entity provenance"
  ```

---

### Task 5: Reset exited loop-body homes and prove cyclic storage cleanup

**Files:**
- Modify: `crates/keld-storage/src/verify.rs`
- Modify: `crates/keld-storage/tests/home_verifier.rs`
- Modify: `crates/keld-storage/tests/cleanup_planner.rs`
- Create: `crates/keld-storage/tests/control_flow.rs`

**Interfaces:**
- Keep `Home::join` unchanged.
- Keep `exit_states[block]` as the pre-exit state used to plan actual drops.
- Normalize only the outgoing successor state after `ExitScopes` to model completed lexical cleanup.

- [ ] **Step 1: Write storage RED tests**

  In `control_flow.rs`, add body-local repeated initialization:

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

  Require the body-local Text home to be a fresh `Initialize` on each dynamic entry and to have cleanup on the backedge, not a replacement of a previous iteration's home.

  Add loop-head `MaybeLive`:

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

  Require `KLD2008` at the post-loop read.

  Add repair by assignment:

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

  In `cleanup_planner.rs`, add a loop where two outer managed `var` homes are moved/reinitialized in different successful orders across paths/iterations. Require the outer scope to use `tracked_scopes`; tracker/drop-flag state must remain bounded by static homes.

- [ ] **Step 2: Verify storage RED**

  ```powershell
  cargo test -p keld-storage --test home_verifier --test cleanup_planner --test control_flow
  ```

  Expected: body-scope re-entry exposes that the current `ExitScopes` transfer resets cleanup-order metadata but leaves body-local `Home` states live across a backedge.

- [ ] **Step 3: Normalize homes after lexical scope exit**

  In `verify_function`, keep:

  ```text
  exit_states[block] = pre-exit state
  ```

  so `annotate_exit_plan` still sees exactly the homes that must be destroyed.

  Build the outgoing successor state separately. For `Terminator::ExitScopes`:

  - call existing `cleanup::exit_scopes` for cleanup-order metadata;
  - for each single-home local whose `function.local_scopes[local]` appears in `storage_scopes`, set `homes[local] = Home::Empty(EmptyReason::Uninitialized)`;
  - remove such locals from `borrowed` if present;
  - leave outer homes unchanged;
  - keep pending-call and indexed-reservation invariants unchanged, because those must already be closed before a terminator.

  Do not change `Home::join`. The existing worklist then naturally forms `MaybeLive` on loop heads/exits when outer paths disagree.

- [ ] **Step 4: Run storage GREEN and commit**

  ```powershell
  cargo test -p keld-storage
  cargo test -p keld-ir --test lowering
  ```

  Commit:

  ```powershell
  git add crates/keld-storage/src/verify.rs crates/keld-storage/tests/home_verifier.rs crates/keld-storage/tests/cleanup_planner.rs crates/keld-storage/tests/control_flow.rs
  git commit -m "fix(storage): reset lexical homes on loop exits"
  ```

---

### Task 6: Prove cyclic executable IR and interpreter behavior without a loop opcode

**Files:**
- Inspect/modify only on a demonstrated cycle bug: `crates/keld-ir/src/validate.rs`
- Modify: `crates/keld-ir/tests/lowering.rs`
- Modify: `crates/keld-ir/tests/validation.rs`
- Create: `crates/keld-interpreter/tests/control_flow.rs`
- Create: `crates/keld-cli/tests/fixtures/control_flow_loop.keld`
- Create: `crates/keld-cli/tests/fixtures/control_flow_allocations.keld`

**Interfaces:**
- `keld_ir::Module`, `Instruction`, and `Terminator` public enums remain unchanged.
- `keld_ir::validate` must accept valid cyclic CFG while continuing to reject invalid register/home/view/lifecycle state.
- `run_text_for_test` remains the interpreter semantic oracle.

- [ ] **Step 1: Add source fixtures and IR/interpreter contracts**

  `control_flow_loop.keld`:

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

  `control_flow_allocations.keld`:

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

  In `keld-ir/tests/lowering.rs`, compile the first fixture through storage verification and IR lowering. Require `validate(&module).is_empty()` and prove at least one reachable CFG edge returns to a previously visited loop region. Assert the public executable IR surface gained no loop variant.

  In `keld-ir/tests/validation.rs`, add a hand-built minimal valid cyclic CFG and require validation success. This directly tests validator cycle support independently of source lowering.

  In `keld-interpreter/tests/control_flow.rs`, run both fixtures plus zero-iteration, nested-loop targeting, condition re-evaluation, and explicit lifecycle exit cases.

- [ ] **Step 2: Run IR/interpreter tests**

  ```powershell
  cargo test -p keld-ir --test lowering --test validation
  cargo test -p keld-interpreter --test control_flow
  ```

  Expected after Tasks 1-5: existing IR/interpreter machinery should already support backedges. If the hand-built valid cycle is RED due a forward-only validator assumption, fix only that demonstrated assumption.

- [ ] **Step 3: Fix IR validator only if RED proves a real cycle bug**

  `FunctionValidator` already owns predecessor/reachability plus definition, lifecycle, home, and entity-identity dataflow. If one `compute_*_dataflow` pass is forward-only, convert only that pass to a changed-state worklist: seed entry, recompute joined input, transfer, enqueue successors when output changes. Do not weaken validation rules and do not add any loop form.

  The interpreter must continue choosing the next block from existing terminators; no loop-specific execution branch is permitted.

- [ ] **Step 4: Run complete IR/interpreter GREEN and commit**

  ```powershell
  cargo test -p keld-ir
  cargo test -p keld-interpreter
  ```

  Commit actual changed paths only:

  ```powershell
  git add crates/keld-ir/tests/lowering.rs crates/keld-ir/tests/validation.rs crates/keld-interpreter/tests/control_flow.rs crates/keld-cli/tests/fixtures/control_flow_loop.keld crates/keld-cli/tests/fixtures/control_flow_allocations.keld
  # add crates/keld-ir/src/validate.rs only if Step 3 changed it
  git commit -m "test(ir): prove cyclic control-flow execution"
  ```

---

### Task 7: Extend Native-1 differential parity to repeated loop allocation attempts

**Files:**
- Modify: `crates/keld-native-backend/tests/differential.rs`
- Reuse: `crates/keld-cli/tests/fixtures/control_flow_loop.keld`
- Reuse: `crates/keld-cli/tests/fixtures/control_flow_allocations.keld`

**Interfaces:**
- Reuse `run_differential_case`, `discover_failure_schedules`, interpreter execution, and the existing test-runtime observation protocol.
- Repeated dynamic execution of one static allocation site keeps one site ID and increments `attempt`.

- [ ] **Step 1: Add differential loop coverage**

  Extend the source fixture sets used by:

  - `every_source_fixture_has_a_shared_allocation_failure_schedule_at_o0_and_o2`
  - `source_surface_fixtures_extend_the_same_differential_schedule`

  with both Control Flow-1 fixtures.

  Add a focused repeated-site assertion for `control_flow_allocations.keld`: run the no-failure interpreter schedule, filter the repeated Text-concat semantic allocation site, and require one `site_id` with attempts `1`, `2`, and `3` in order. Then run the existing differential harness so every discovered attempt can be failed independently and compared with native O0/O2.

- [ ] **Step 2: Build test runtime and run differential GREEN**

  ```powershell
  $env:CARGO_BUILD_TARGET='x86_64-pc-windows-gnu'
  . scripts/activate-llvm.ps1
  cargo build -p keld-native-ffi-test --release --target x86_64-pc-windows-gnu
  cargo test -p keld-native-backend --test differential -- --nocapture
  ```

  Require identical interpreter/native return or fault, source span, site ID, phase, attempt, and failure-observation sequence at O0 and O2.

- [ ] **Step 3: Run Native-1 focused regression suite and commit**

  ```powershell
  cargo test -p keld-native-backend --test native_int -- --nocapture
  cargo test -p keld-native-backend --test differential -- --nocapture
  cargo test -p keld-native-backend --test surface_audit -- --nocapture
  ```

  Control Flow-1 must leave the executable surface at the existing 48 instruction variants and 6 terminators.

  Commit:

  ```powershell
  git add crates/keld-native-backend/tests/differential.rs
  git commit -m "test(native): cover loop differential parity"
  ```

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
- `control-flow.md` becomes the normative semantic supplement for accepted loops.
- Historical bootstrap acceptance remains historical; Control Flow-1 gets a separate acceptance section.

- [ ] **Step 1: Replace the obsolete unsupported-loop fixture**

  Change `fail_unsupported.keld` to a still-deferred parsed feature:

  ```keld
  fn main() -> Int {
      match 0 {
          _ => 0
      }
  }
  ```

  Update `cli.rs` to expect `KLD0004` on `match` at line 2, column 5, with the existing unsupported-feature help text.

  Add `control_flow_loop.keld` to representative interpreter/native fixture tables with `8\n`, and `control_flow_allocations.keld` with `6\n` where managed allocation parity belongs.

- [ ] **Step 2: Add Control Flow-1 acceptance tests**

  Do not renumber or rewrite the historical 16 bootstrap criteria. Add a separate Control Flow-1 acceptance section/table in `milestone_acceptance.rs` covering:

  - ordinary loop result `8`;
  - repeated managed allocation result `6`;
  - outside-loop `break`/`continue` -> `KLD0112`;
  - loop-head `MaybeLive` -> `KLD2008`;
  - cleanup on `continue`/`break` across an explicit lifecycle;
  - loop-carried entity provenance regression;
  - validated cyclic executable IR.

  Use existing compiler/library helpers rather than building another pipeline in the tests.

- [ ] **Step 3: Write `docs/spec/control-flow.md`**

  Normatively specify:

  1. accepted source forms and innermost-target rule;
  2. Bool condition evaluation and full-expression boundary;
  3. zero-iteration behavior;
  4. fresh lexical body scope per dynamic iteration;
  5. structured cleanup on fallthrough/continue/break/return;
  6. no implicit lifecycle from `while`;
  7. cyclic Home joins and `MaybeLive` repair/use rules;
  8. lifecycle/provenance joins and conservative repeated-site identity handling;
  9. literal-true structural non-fallthrough rule;
  10. runtime-fault cleanup contract;
  11. backend-neutral cyclic executable IR;
  12. deferred loop forms.

- [ ] **Step 4: Update cross-references/status docs**

  - `grammar.md`: point semantic loop restrictions to `control-flow.md` and include it in the normative-spec list.
  - `storage-values.md`: state that the existing Home join applies at backedges and exited lexical homes are cleaned/reset before re-entry.
  - `compiler-architecture.md`: describe cyclic Flow CFG fixed-point lifecycle/storage verification while preserving crate ownership.
  - language design normative-spec list: add `control-flow.md`; keep its historical first-buildable-milestone exclusion list intact.
  - README: move `while`/`break`/`continue` from deferred to implemented surface and link the new spec.
  - `milestone-acceptance.md`: add Control Flow-1 evidence and explicitly record that the executable IR still uses the existing 48/6 surface.

- [ ] **Step 5: Run docs/CLI-facing GREEN and commit**

  ```powershell
  cargo test -p keld-cli --test cli --test milestone_acceptance
  cargo test -p keld-semantics --test feature_gate --test loops
  cargo test -p keld-ir --test lowering --test validation
  ```

  Commit:

  ```powershell
  git add README.md docs/spec/control-flow.md docs/spec/grammar.md docs/spec/storage-values.md docs/compiler-architecture.md docs/milestone-acceptance.md docs/superpowers/specs/2026-08-11-keld-language-design.md crates/keld-cli/tests/cli.rs crates/keld-cli/tests/milestone_acceptance.rs crates/keld-cli/tests/fixtures/fail_unsupported.keld crates/keld-cli/tests/fixtures/control_flow_loop.keld crates/keld-cli/tests/fixtures/control_flow_allocations.keld
  git commit -m "docs: accept Keld Control Flow-1"
  ```

---

### Task 9: Run the complete Control Flow-1 acceptance gate and review scope

**Files:**
- Inspect all changes against design commit `407f6d85a5b9322f6a8496b9d9cdb9525577e665`
- Modify only regressions required by a failing Control Flow-1 gate; do not expand scope

- [ ] **Step 1: Build pinned native runtime artifacts**

  ```powershell
  $env:CARGO_BUILD_TARGET='x86_64-pc-windows-gnu'
  . scripts/activate-llvm.ps1
  cargo build -p keld-native-ffi --release --target x86_64-pc-windows-gnu
  cargo build -p keld-native-ffi-test --release --target x86_64-pc-windows-gnu
  ```

  Confirm expected sibling runtime artifacts exist under `target/x86_64-pc-windows-gnu/release/` before native CLI/differential tests.

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

  Require no compile-time verifier nontermination and no metadata growth proportional to dynamic loop iterations.

- [ ] **Step 3: Run full debug/release workspace gates**

  ```powershell
  cargo test --workspace --all-targets --no-fail-fast
  cargo test --workspace --all-targets --release --no-fail-fast
  cargo clippy --workspace --all-targets -- -D warnings
  cargo fmt --all -- --check
  git diff --check
  ```

  All commands must exit zero on the pinned Rust 1.97.0 Windows GNU setup.

- [ ] **Step 4: Run source-level interpreter/native smoke**

  ```powershell
  cargo run --release -p keld-cli -- run --engine interpreter crates/keld-cli/tests/fixtures/control_flow_loop.keld
  cargo run --release -p keld-cli -- run --engine native crates/keld-cli/tests/fixtures/control_flow_loop.keld
  cargo run --release -p keld-cli -- run --engine interpreter crates/keld-cli/tests/fixtures/control_flow_allocations.keld
  cargo run --release -p keld-cli -- run --engine native crates/keld-cli/tests/fixtures/control_flow_allocations.keld
  cargo run --release -p keld-cli -- dump-ir crates/keld-cli/tests/fixtures/control_flow_loop.keld
  ```

  Expected stdout: `8`, `8`, `6`, `6`, each with one newline. `dump-ir` must be non-empty and contain only existing executable forms.

- [ ] **Step 5: Review the final diff against approved scope**

  ```powershell
  git diff 407f6d85a5b9322f6a8496b9d9cdb9525577e665...HEAD --stat
  git diff 407f6d85a5b9322f6a8496b9d9cdb9525577e665...HEAD -- docs crates README.md
  ```

  Confirm:

  - no new source syntax beyond the already-grammatical three forms;
  - no new memory-management mechanism;
  - no implicit loop lifecycle;
  - no new executable-IR/native loop opcode;
  - no weakening of storage/lifecycle diagnostics;
  - no accidental enablement of other deferred features;
  - no temporary CI/debug scaffolding remains;
  - plan checkboxes match actual verified work.

- [ ] **Step 6: Commit final acceptance-only adjustments if needed**

  If the final gate required only acceptance bookkeeping/tests, commit those exact changes:

  ```powershell
  git add -A
  git commit -m "test: close Control Flow-1 acceptance"
  ```

  Do not begin another milestone from this plan. Control Flow-1 is complete only after every gate above is green.
