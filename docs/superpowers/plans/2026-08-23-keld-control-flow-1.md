# Keld Control Flow-1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add source-level `while`, `break`, and `continue` by extending Keld's existing flow-sensitive storage, lifecycle, provenance, cleanup, interpreter, and native parity machinery from acyclic CFGs to cyclic CFGs.

**Architecture:** Semantics gains explicit loop HIR, flow lowering builds ordinary cyclic CFG with structured `ExitScopes` edges, lifecycle verification moves from predecessor-completion scheduling to a monotone fixed-point worklist, and storage reuses its existing worklist/lattice on loop backedges. Executable IR, interpreter, and LLVM remain loop-agnostic and consume only validated branch/goto/cleanup operations.

**Tech Stack:** Rust 1.97.0, Cargo workspace, Keld validated executable IR, LLVM 22.1.8 through the existing Windows GNU Native-1 toolchain, MinGW-w64 GCC, PowerShell CI/bootstrap.

**Spec:** `docs/superpowers/specs/2026-08-22-keld-control-flow-1-design.md`

## Global Constraints

- `break` and `continue` target only the innermost enclosing `while`; labels and values remain deferred.
- A `while` condition is re-evaluated before every iteration, must be `Bool`, and is an ordinary Keld expression.
- A loop body is a lexical storage scope per dynamic iteration but does not create an implicit lifecycle.
- `break`, `continue`, normal fallthrough, and `return` execute all cleanup for scopes/lifecycles they leave.
- Storage joins keep the existing `Empty(reason) | Live | MaybeLive` lattice; no loop-only home state is introduced.
- Cleanup ordering keeps the existing `Known(order) | Divergent` model; runtime metadata remains bounded by static homes.
- Entity/lifecycle analysis is conservative on loop backedges; uncertain loop-carried provenance becomes may-alias rather than an incorrect must-alias/must-distinct fact.
- No executable-IR loop instruction, runtime loop object, GC, RC, implicit lifecycle, or iteration allocation is introduced.
- Interpreter, native LLVM O0, and native LLVM O2 must observe identical loop behavior and runtime faults.
- The same static allocation site executed on multiple iterations keeps one site ID and increasing attempt ordinals.
- Rust toolchain stays pinned at 1.97.0 and Native-1 remains `x86_64-w64-windows-gnu` with LLVM 22.1.8.

---

## File Structure

Control Flow-1 should modify existing owners rather than create parallel loop subsystems.

- `crates/keld-semantics/src/features.rs` — stop gating `while`, `break`, `continue`.
- `crates/keld-semantics/src/hir.rs` — add `HirWhile`, `HirStmtKind::{While,Break,Continue}`.
- `crates/keld-semantics/src/check.rs` — type-check loops, track loop nesting for KLD0112, recurse call collection, classify `while true` fallthrough conservatively.
- `crates/keld-semantics/tests/{feature_gate,types,entrypoint}.rs` — source acceptance, Bool condition, outside-loop diagnostics, return checking.
- `crates/keld-flow/src/lower.rs` — lower loops to cyclic CFG and maintain innermost-loop targets.
- `crates/keld-flow/src/{cfg,op,dump}.rs` — only adjust helpers/dumps if cyclic lowering exposes assumptions; do not add a loop terminator.
- `crates/keld-flow/tests/{control_flow,storage_scopes,evaluation_order}.rs` — CFG shape, nested targeting, condition reevaluation, lexical exit scopes.
- `crates/keld-lifecycle/src/verify.rs` — replace DAG predecessor scheduling with fixed-point propagation.
- `crates/keld-lifecycle/src/provenance.rs` — make joins convergence-safe and conservative for loop-carried entity identity.
- `crates/keld-lifecycle/tests/{retirement,provenance_facts,alias_refinement,lifecycle_order}.rs` — cyclic liveness/provenance/lifecycle cases.
- `crates/keld-storage/src/{verify,cleanup,state}.rs` — prove existing worklist converges on cycles, clean exited scopes on backedges, preserve bounded cleanup-order tracking.
- `crates/keld-storage/tests/{home_verifier,cleanup_planner}.rs` — loop-head `MaybeLive`, repair by assignment, continue/break cleanup, divergent initialization order.
- `crates/keld-ir/src/*` and `crates/keld-ir/tests/*` — only change if validation currently assumes acyclic CFG; keep the public instruction/terminator surface unchanged.
- `crates/keld-interpreter/tests/*` — executable cyclic CFG behavior and loop source fixtures.
- `crates/keld-native-backend/tests/{differential,native_int}.rs` — source loop O0/O2 parity and repeated allocation-attempt parity.
- `crates/keld-cli/tests/fixtures/*.keld` and `crates/keld-cli/tests/*` — representative accepted/rejected source fixtures and end-to-end observations.
- `docs/spec/control-flow.md`, `README.md`, `docs/compiler-architecture.md`, `docs/milestone-acceptance.md`, `docs/superpowers/specs/2026-08-11-keld-language-design.md` — normative and acceptance documentation after implementation is green.

---

### Task 1: Accept and Type-Check Loop Syntax in Semantics

**Files:**
- Modify: `crates/keld-semantics/src/features.rs`
- Modify: `crates/keld-semantics/src/hir.rs`
- Modify: `crates/keld-semantics/src/check.rs`
- Modify: `crates/keld-semantics/tests/feature_gate.rs`
- Modify: `crates/keld-semantics/tests/types.rs`
- Modify: `crates/keld-semantics/tests/entrypoint.rs`

**Interfaces:**
- Consumes: existing `SyntaxKind::{WhileStmt,BreakStmt,ContinueStmt}`, `TypeStore::BOOL`, `HirBlock`, `HirExpr`.
- Produces:
  - `pub struct HirWhile { pub condition: HirExpr, pub body: HirBlock }`
  - `HirStmtKind::While(HirWhile)`
  - `HirStmtKind::Break`
  - `HirStmtKind::Continue`
  - diagnostic `KLD0112` for loop control outside an enclosing loop.

- [ ] **Step 1: Write failing semantic acceptance tests**

Add tests equivalent to:

```rust
#[test]
fn while_break_and_continue_are_not_feature_gated() {
    let result = analyze_text_for_test(r#"
fn main() -> Int {
    var i = 0
    while i < 4 {
        i += 1
        if i == 2 { continue }
        if i == 3 { break }
    }
    return i
}
"#);
    assert!(result.diagnostics.iter().all(|d| d.code.0 != "KLD0004"));
}

#[test]
fn while_condition_must_be_bool() {
    let result = analyze_text_for_test("fn main() -> Int { while 1 { break } return 0 }");
    assert!(result.diagnostics.iter().any(|d| d.code.0 == "KLD0107"));
}

#[test]
fn loop_control_outside_loop_is_rejected() {
    for keyword in ["break", "continue"] {
        let result = analyze_text_for_test(&format!("fn main() -> Int {{ {keyword} return 0 }}"));
        assert!(result.diagnostics.iter().any(|d| d.code.0 == "KLD0112"));
    }
}
```

Also add return-completeness cases:

```rust
#[test]
fn literal_true_without_direct_break_is_non_fallthrough() { /* fn main()->Int { while true {} } */ }

#[test]
fn break_in_nested_loop_does_not_make_outer_true_loop_fallthrough() { /* outer while true, inner break */ }

#[test]
fn direct_break_makes_true_loop_may_fallthrough() { /* fn f()->Int { while true { break } } -> KLD0111 */ }
```

- [ ] **Step 2: Run focused tests and verify RED**

Run:

```powershell
cargo test -p keld-semantics --test feature_gate
cargo test -p keld-semantics --test types
cargo test -p keld-semantics --test entrypoint
```

Expected: loop programs fail because the feature gate rejects them and HIR has no loop statements.

- [ ] **Step 3: Add HIR forms and semantic loop context**

Implement in `hir.rs`:

```rust
#[derive(Clone, Debug)]
pub struct HirWhile {
    pub condition: HirExpr,
    pub body: HirBlock,
}

pub enum HirStmtKind {
    // existing variants...
    While(HirWhile),
    Break,
    Continue,
}
```

In `BodyChecker`, add an integer loop-depth field initialized to zero. For `WhileStmt`, check the condition with expected `Bool`, increment depth only while checking the body, then emit `HirStmtKind::While`. For `BreakStmt` and `ContinueStmt`, emit KLD0112 when depth is zero.

Remove only these three cases from `unsupported_feature`:

```rust
SyntaxKind::WhileStmt => Some("while"),
SyntaxKind::BreakStmt => Some("break"),
SyntaxKind::ContinueStmt => Some("continue"),
```

Do not change any other deferred feature gate.

- [ ] **Step 4: Extend semantic visitors and fallthrough analysis**

Update `collect_block_calls` so `While` collects calls from both condition and body; `Break`/`Continue` contribute none.

Implement structural non-fallthrough classification with this rule:

```text
while true is non-fallthrough iff the body contains no syntactic break targeting this exact loop;
breaks inside nested while bodies are ignored for the outer loop.
```

Do not perform reachability or general termination proofs.

- [ ] **Step 5: Run semantic GREEN tests**

Run the same three focused test binaries, then:

```powershell
cargo test -p keld-semantics
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/keld-semantics
git commit -m "feat(semantics): type check while control flow"
```

---

### Task 2: Lower `while`, `break`, and `continue` to Structured Cyclic Flow

**Files:**
- Modify: `crates/keld-flow/src/lower.rs`
- Modify if needed for dump assertions only: `crates/keld-flow/src/dump.rs`
- Modify: `crates/keld-flow/tests/control_flow.rs`
- Modify: `crates/keld-flow/tests/storage_scopes.rs`
- Modify: `crates/keld-flow/tests/evaluation_order.rs`

**Interfaces:**
- Consumes: `HirStmtKind::{While,Break,Continue}`, existing `Terminator::{Goto,Branch,ExitScopes}` and `ExitTarget::Goto`.
- Produces: cyclic `FlowFunction.blocks`; no new `Terminator` variant.

- [ ] **Step 1: Write failing CFG-shape tests**

Add a test for:

```keld
fn main() -> Int {
    var i = 0
    while i < 3 {
        i += 1
    }
    return i
}
```

Assert the lowered CFG contains:

```text
entry -> condition
condition Branch(body, exit)
body -> condition
exit -> return
```

Add nested-loop tests proving inner `break` reaches only inner exit and inner `continue` reaches only inner condition.

Add a storage-scope test where `continue` inside nested blocks/lifecycles emits `ExitScopes` for exactly the scopes crossed, and `break` exits the loop body scope but not an enclosing outer function/block scope.

- [ ] **Step 2: Run flow tests and verify RED**

```powershell
cargo test -p keld-flow --test control_flow
cargo test -p keld-flow --test storage_scopes
cargo test -p keld-flow --test evaluation_order
```

Expected: failure because loop HIR is not lowered.

- [ ] **Step 3: Add an explicit loop-target stack to the lowerer**

Use a private lowering record equivalent to:

```rust
#[derive(Clone, Copy)]
struct LoopTarget {
    condition: BlockId,
    exit: BlockId,
    body_scope: StorageScopeId,
    lifecycle_depth: usize,
}
```

If the lowerer already has generalized scope-exit helpers, store only the target block plus the lexical scope/lifecycle boundary needed by those helpers rather than duplicating cleanup policy.

For `While`:

1. allocate condition/body/exit blocks;
2. terminate the current block with `Goto(condition)`;
3. lower condition in the condition block and terminate with `Branch { then_block: body, else_block: exit }`;
4. push the loop target while lowering body;
5. normal body fallthrough uses a structured scope exit back to condition;
6. pop the target and continue lowering in exit.

For `Continue`, emit the same structured-exit machinery used by return/block exits, with `ExitTarget::Goto(condition)`.

For `Break`, use `ExitTarget::Goto(exit)`.

Never create a raw jump that skips storage/lifecycle exit metadata.

- [ ] **Step 4: Preserve condition evaluation order and full-expression boundary**

Add/extend evaluation-order assertions so all condition operations are in the condition block and execute again on every backedge. Owned temporaries/loans produced only by condition evaluation must be closed/dropped before the `Branch` leaves the condition expression boundary, using existing temporary cleanup lowering.

- [ ] **Step 5: Run flow GREEN tests**

Run the focused tests plus:

```powershell
cargo test -p keld-flow
```

Expected: PASS and flow dumps show ordinary cyclic `Goto`/`Branch`/`ExitScopes` only.

- [ ] **Step 6: Commit**

```bash
git add crates/keld-flow
git commit -m "feat(flow): lower structured while loops"
```

---

### Task 3: Make Lifecycle and Provenance Verification Converge on Cyclic CFGs

**Files:**
- Modify: `crates/keld-lifecycle/src/verify.rs`
- Modify: `crates/keld-lifecycle/src/provenance.rs`
- Modify: `crates/keld-lifecycle/tests/retirement.rs`
- Modify: `crates/keld-lifecycle/tests/provenance_facts.rs`
- Modify: `crates/keld-lifecycle/tests/alias_refinement.rs`
- Modify: `crates/keld-lifecycle/tests/lifecycle_order.rs`

**Interfaces:**
- Consumes: cyclic `FlowFunction`, existing `AbstractState::join`, `RefState`, `Origin`, equality/distinct sets.
- Produces: the same `VerifiedFlowModule` API, now valid for cyclic CFGs.

- [ ] **Step 1: Write RED tests for cyclic entity state**

Cover at least:

```keld
entity E { value: Int }
fn main() -> Int {
    let e = E(value: 1)
    var i = 0
    while i < 2 {
        if i == 0 { retire e }
        i += 1
    }
    return e.value
}
```

Expected: static lifecycle rejection because a loop path retires the proof.

Also add:

- a loop where an entity remains live on every backedge and remains usable;
- repeated allocation at one syntactic site across iterations, ensuring distinct dynamic iterations are not treated as a single must-alias identity;
- a conservative may-alias loop-carried pair refined by `a != b` inside the body;
- explicit lifecycle inside a loop with `continue`/`break`, verifying the lifecycle is ended on those exits;
- `keep` to an ancestor survives that inner lifecycle exit.

- [ ] **Step 2: Run lifecycle tests and verify RED/nontermination protection**

```powershell
cargo test -p keld-lifecycle --test retirement
cargo test -p keld-lifecycle --test provenance_facts
cargo test -p keld-lifecycle --test alias_refinement
cargo test -p keld-lifecycle --test lifecycle_order
```

Expected before implementation: cyclic functions are rejected, incompletely analyzed, or expose the DAG scheduling assumption. Tests must have a normal timeout in CI; do not accept a hang as RED evidence.

- [ ] **Step 3: Replace predecessor-completion scheduling with a changed-state worklist**

Refactor `Analyzer::run` to the shape already used by storage verification:

```rust
let mut incoming = vec![None::<AbstractState>; block_count];
incoming[entry] = Some(self.initial_state());
let mut queue = VecDeque::from([entry]);

while let Some(block_id) = queue.pop_front() {
    let Some(mut state) = incoming[block_id].clone() else { continue };
    transfer_operations(&mut state);
    for (successor, outgoing) in transfer_terminator(state) {
        let changed = join_into(&mut incoming[successor], outgoing, join_span);
        if changed {
            queue.push_back(successor);
        }
    }
}
```

The analysis must terminate because joins only lose precision toward the existing conservative states/fact intersections. Remove indegree/`complete_predecessor` logic that assumes DAG completion.

- [ ] **Step 4: Make facts/proofs recording idempotent across revisits**

Blocks may be analyzed multiple times. Do not assert an operation fact is inserted only once. Store the final conservative fact or update it when the incoming abstract state changes. Diagnostics/proof annotations must not duplicate on each worklist revisit; either defer diagnostic emission to a final stable pass or deduplicate by stable `(function, block, operation, diagnostic/proof kind)` identity.

- [ ] **Step 5: Preserve conservative loop-carried provenance**

`AbstractState::join` must continue intersecting equality/distinct facts. Ensure any fresh provenance associated with one static allocation instruction is not interpreted as one runtime entity across backedges. If proving dynamic-iteration distinctness would require new identity machinery, drop that fact to may-alias instead of inventing an unsound must-distinct relation.

- [ ] **Step 6: Run lifecycle GREEN and whole crate tests**

```powershell
cargo test -p keld-lifecycle
```

Expected: PASS, stable termination, no duplicate diagnostics/proofs.

- [ ] **Step 7: Commit**

```bash
git add crates/keld-lifecycle
git commit -m "feat(lifecycle): verify cyclic control flow"
```

---

### Task 4: Prove Single-Home and Cleanup Behavior Across Loop Backedges

**Files:**
- Modify as needed: `crates/keld-storage/src/verify.rs`
- Modify as needed: `crates/keld-storage/src/cleanup.rs`
- Modify as needed: `crates/keld-storage/src/state.rs`
- Modify: `crates/keld-storage/tests/home_verifier.rs`
- Modify: `crates/keld-storage/tests/cleanup_planner.rs`

**Interfaces:**
- Consumes: cyclic verified flow, existing `Home::join`, `CleanupOrder`, structured `ExitScopes`.
- Produces: unchanged `VerifiedStorageModule`/`FunctionStoragePlan`, now correct for loop backedges.

- [ ] **Step 1: Write RED tests for loop-head home states**

Add source cases equivalent to:

```keld
fn maybe_live(cond: Bool) -> Int {
    var x = Text("a")
    var i = 0
    while i < 2 {
        if cond { let moved = take x }
        i += 1
    }
    return x.byte_length
}
```

Expected: KLD2008 at the read because loop-head/exit state becomes `MaybeLive`.

Add a repair case:

```keld
while cond {
    if other { let moved = take x }
    x = Text("reset")
}
return x.byte_length
```

Expected: accepted because assignment to `var` restores `Live` on every backedge.

Also test a `var` initialized only in the loop and read after it: zero iterations keep it not definitely live.

- [ ] **Step 2: Write cleanup tests for fallthrough, continue, break, and divergent order**

Use `List`/`Text` homes and inspect `FunctionStoragePlan` or interpreter-visible allocation/drop observations to prove:

- body-local homes are cleaned once per completed iteration;
- `continue` cleans before the next condition;
- `break` cleans before loop exit;
- outer homes remain owned across iterations;
- reinitializing different outer homes in different iteration histories can produce `CleanupOrder::Divergent` and activates the existing bounded tracker.

- [ ] **Step 3: Run storage tests and verify RED**

```powershell
cargo test -p keld-storage --test home_verifier
cargo test -p keld-storage --test cleanup_planner
```

Expected: failing assertions identify any backedge/scoped-cleanup gaps.

- [ ] **Step 4: Reuse, do not replace, the existing fixed-point home analysis**

Keep `Home::join` unchanged unless a failing test proves an implementation bug. The storage verifier already re-enqueues successors when joined state changes; adapt only assumptions that arose because prior CFGs were acyclic, especially:

- exit-state annotations must represent the stable final state rather than the last transient visit;
- `ExitScopes` must clear cleanup-order state before a continue/break successor join;
- condition/body owned temporaries must not leak across backedges;
- pending call/loan/indexed-reservation state must be empty at structured loop transfers.

Do not add a new loop-specific home or cleanup state.

- [ ] **Step 5: Run storage GREEN and whole crate tests**

```powershell
cargo test -p keld-storage
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/keld-storage
git commit -m "feat(storage): verify loop-carried homes and cleanup"
```

---

### Task 5: Validate and Execute Cyclic IR Through the Interpreter

**Files:**
- Inspect/modify only if needed: `crates/keld-ir/src/*`
- Modify/add focused tests under: `crates/keld-ir/tests/`
- Modify/add focused tests under: `crates/keld-interpreter/tests/`
- Add fixtures: `crates/keld-cli/tests/fixtures/control_flow_basic.keld`
- Add fixtures: `crates/keld-cli/tests/fixtures/control_flow_nested.keld`
- Add fixtures: `crates/keld-cli/tests/fixtures/control_flow_cleanup.keld`

**Interfaces:**
- Consumes: verified cyclic flow/storage lowering to existing executable `Instruction` and `Terminator` variants.
- Produces: validated cyclic `keld_ir::Module` accepted by the existing interpreter.

- [ ] **Step 1: Add IR validation regression tests for backedges**

Construct or lower a small legal cyclic module and assert `keld_ir::validate` accepts it. Add malformed cyclic variants only where they exercise existing invariants (bad Phi predecessor, invalid view across terminator, unresolved cleanup, invalid register role) and assert the existing KLD900x family rejects them.

Do not create a `Loop` instruction or terminator.

- [ ] **Step 2: Run IR tests and verify whether RED exposes acyclic assumptions**

```powershell
cargo test -p keld-ir
```

If all legal cyclic CFG validation already passes, record that as GREEN baseline and make no production IR change. Do not modify validation merely to create work.

- [ ] **Step 3: Add interpreter source/integration cases**

Use fixtures with deterministic returned Int values, for example:

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

Expected result: `8` (`1 + 3 + 4`).

Nested fixture should distinguish inner and outer targets. Cleanup fixture should include per-iteration `Text`/`List` plus an explicit lifecycle whose exit is triggered by `continue` and `break`.

- [ ] **Step 4: Run interpreter tests**

```powershell
cargo test -p keld-interpreter
cargo test -p keld-cli --test milestone_acceptance
```

Expected: cyclic programs terminate with specified results and existing runtime fault formatting remains unchanged.

- [ ] **Step 5: Commit**

```bash
git add crates/keld-ir crates/keld-interpreter crates/keld-cli/tests/fixtures
git commit -m "test(ir): cover validated cyclic execution"
```

---

### Task 6: Extend Native Differential Coverage to Source Loops and Repeated Allocation Attempts

**Files:**
- Modify: `crates/keld-native-backend/tests/differential.rs`
- Modify: `crates/keld-native-backend/tests/native_int.rs`
- Modify only if a real lowering bug is exposed: `crates/keld-native-llvm/src/*` or `crates/keld-native-backend/src/*`

**Interfaces:**
- Consumes: unchanged validated executable IR surface and Native-1 observation schema.
- Produces: O0/O2 evidence that ordinary cyclic CFG and repeated site attempts match the interpreter.

- [ ] **Step 1: Add source-loop differential fixtures to the existing harness**

Include basic, nested, continue/break cleanup, and loop-condition-effect fixtures from Task 5 in the source-surface differential set. Require exact normalized interpreter/native observations at both O0 and O2.

- [ ] **Step 2: Add repeated allocation-site attempt parity**

Create a loop that executes one semantic allocation site multiple times, such as repeated `Text` concat/copy, List growth, struct/entity allocation, or whichever existing deterministic allocation-site fixture is easiest to repeat without introducing new language features.

Assert the observation stream keeps one frozen `site_id` and monotonically increasing `attempt` values across iterations. Inject a failure at a later attempt (not only attempt 1) and require interpreter/native to agree on status, fault kind, source span, and preceding allocation events.

- [ ] **Step 3: Run native differential tests on the pinned Windows GNU toolchain**

From a shell with Native-1 environment activated:

```powershell
$env:CARGO_BUILD_TARGET='x86_64-pc-windows-gnu'
. scripts/activate-llvm.ps1
cargo build -p keld-native-ffi --release --target x86_64-pc-windows-gnu
cargo test -p keld-native-backend --test differential
cargo test -p keld-native-backend --test native_int
```

Expected: RED only if LLVM/backend code has a real cyclic CFG/Phi/runtime parity defect.

- [ ] **Step 4: Fix only demonstrated backend defects**

If tests fail, fix the smallest lowering/runtime issue while preserving the rule that native code does not inspect source loop semantics. Typical valid fixes are predecessor/Phi handling, block emission order, or runtime observation sequencing. Do not add source-loop special cases to LLVM lowering.

- [ ] **Step 5: Re-run O0/O2 differential GREEN**

Run the two focused native test binaries again. Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/keld-native-backend crates/keld-native-llvm
git commit -m "test(native): prove loop differential parity"
```

---

### Task 7: Publish Normative Control-Flow Semantics and Run the Full Acceptance Gate

**Files:**
- Create: `docs/spec/control-flow.md`
- Modify: `README.md`
- Modify: `docs/compiler-architecture.md`
- Modify: `docs/milestone-acceptance.md`
- Modify: `docs/superpowers/specs/2026-08-11-keld-language-design.md`
- Modify as needed: `crates/keld-cli/tests/milestone_acceptance.rs`

**Interfaces:**
- Consumes: fully passing implementation and the approved Control Flow-1 design.
- Produces: normative public semantics and auditable acceptance evidence.

- [ ] **Step 1: Write `docs/spec/control-flow.md` from the implemented contract**

The document must normatively specify:

```text
1. while condition evaluation and Bool requirement
2. zero-iteration behavior
3. innermost break/continue targets
4. lexical body scopes per iteration
5. structured cleanup on fallthrough/continue/break/return
6. no implicit lifecycle for loops
7. explicit lifecycle exit interaction and keep-to-ancestor behavior
8. cyclic storage joins using Empty/Live/MaybeLive
9. cyclic lifecycle/provenance conservative joins
10. literal while true structural non-fallthrough rule
11. runtime faults retain the existing no-user-cleanup promise
12. executable representation is ordinary validated CFG
```

Do not duplicate List/numeric/lifecycle algorithms beyond linking to their normative owner documents.

- [ ] **Step 2: Update public status documents**

In `README.md`, move loops/`break`/`continue` from deferred to implemented while leaving `for`, iterators, match, typed errors, recursion, etc. deferred.

In `docs/compiler-architecture.md`, state that lifecycle and storage analyses use monotone fixed-point worklists and accept cyclic CFGs.

In the language-design normative-spec list, add `docs/spec/control-flow.md` without rewriting the historical approved baseline beyond necessary cross-reference/status corrections.

In `docs/milestone-acceptance.md`, add a Control Flow-1 section mapping each design completion gate to concrete tests.

- [ ] **Step 3: Add/extend CLI acceptance cases**

Acceptance must execute at least:

```text
basic loop result
nested innermost break/continue
managed cleanup/MaybeLive static rejection
explicit lifecycle cleanup on structured exit
```

For successful fixtures, compare library/CLI interpreter observations. Native parity evidence remains in the Native-1 differential suite rather than making every generic CLI test depend on LLVM.

- [ ] **Step 4: Run the full GNU workspace gate**

```powershell
$env:CARGO_BUILD_TARGET='x86_64-pc-windows-gnu'
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --workspace --release
```

Then activate the pinned native toolchain and run the focused native gate:

```powershell
. scripts/activate-llvm.ps1
cargo build -p keld-native-ffi --release --target x86_64-pc-windows-gnu
cargo test -p keld-native-backend --test differential
cargo test -p keld-native-backend --test native_int
```

Expected: every command exits zero.

- [ ] **Step 5: Review repository diff for scope creep**

Compare the branch against `feature/custody-ledger-bootstrap`. Expected language additions are only `while`, `break`, `continue` plus cyclic-analysis infrastructure and documentation. Reject unrelated enums, match, recursion, for-loops, labels, typed errors, new runtime memory mechanisms, or backend loop instructions.

- [ ] **Step 6: Commit final docs/acceptance evidence**

```bash
git add README.md docs crates/keld-cli/tests
git commit -m "docs: accept Keld Control Flow-1"
```

---

## Final Self-Review Checklist

Before declaring Control Flow-1 complete, verify every item below against `docs/superpowers/specs/2026-08-22-keld-control-flow-1-design.md`:

- [ ] Source forms are exactly `while`, value-less `break`, value-less `continue`; innermost target only.
- [ ] Conditions are ordinary `Bool` expressions and are re-evaluated on every iteration.
- [ ] Condition-only temporaries/loans/views do not survive the expression boundary.
- [ ] Loop body locals are new per iteration; outer homes persist.
- [ ] `continue`, `break`, and `return` use structured cleanup rather than raw jumps.
- [ ] Plain loops create no lifecycle; explicit lifecycle exit is honored.
- [ ] `Home` lattice semantics are unchanged and fixed-point propagation reaches stability.
- [ ] Cleanup-order tracking is bounded by static homes and remains deterministic.
- [ ] Lifecycle/provenance joins are conservative on backedges and do not invent dynamic-iteration identity equality/distinctness.
- [ ] Zero-iteration flow participates in post-loop state.
- [ ] Literal `while true` classification is structural and conservative only.
- [ ] Executable IR has no new loop variant and validates cyclic CFGs.
- [ ] Interpreter/native O0/O2 parity covers representative source loops.
- [ ] Repeated allocation-site attempt/failure observations match across engines.
- [ ] Full debug/release workspace, strict Clippy, formatting, native differential, and diff-scope gates pass.
