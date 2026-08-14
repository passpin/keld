# Keld Final Storage Correctness Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the three remaining medium-severity storage correctness findings at `af18dfa` with exhaustive View validation, provenance-aware entity liveness, and fallible executable allocation.

**Architecture:** `keld-ir` gains exhaustive instruction classification and a CFG identity-origin dataflow that is intentionally limited to executable Copy/Phi provenance. `keld-interpreter` removes avoidable allocation and makes Text/frame/runtime-place allocation fallible, while `keld-runtime` is audited rather than redesigned because its executable growth is already reserved fallibly.

**Tech Stack:** Rust workspace, Keld IR validator/interpreter/runtime, GNU stable Rust toolchain, Cargo integration tests, `stats_alloc` allocation assertions.

## Global Constraints

- Work only in `C:\dev\keld\.worktrees\custody-ledger-bootstrap`.
- Use the pre-existing untracked `crates/keld-ir/tests/review_adversarial.rs` as the first RED fixture, preserve its adversarial setup, then promote it into the tracked rejection regression required by this fix.
- Add regression tests before production changes and observe the intended RED result.
- Keep entity provenance limited to executable Copy/Phi identity flow; do not duplicate source lifecycle analysis.
- Map every normal interpreter/runtime allocation failure to Keld `AllocationFault` or eliminate the allocation.
- Do not begin another milestone.

---

### Task 1: Exhaustive structural-operation classification

**Files:**
- Modify and track: `crates/keld-ir/tests/review_adversarial.rs`
- Modify: `crates/keld-ir/src/validate.rs`

**Interfaces:**
- Consumes: `Instruction`, `validate`, and the existing open-View validation contract.
- Produces: exhaustive `is_structural(&Instruction) -> bool` classification including `ListPushPlace` and `ListRemovePlace`.

- [ ] **Step 1: Write failing projected mutation tests**

Run the existing `ListPushPlace` fixture first with its current acceptance assertion. Then preserve that setup, invert the assertion to require `KLD9001`, and add a matching `ListRemovePlace` case whose `ArgumentSource` is an entity List field while a read View is open.

- [ ] **Step 2: Verify RED**

Run:

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-ir --test review_adversarial -- --nocapture
```

Expected: both new assertions fail because validation currently returns no View diagnostic.

- [ ] **Step 3: Implement exhaustive classification**

Replace the `matches!` body with a wildcard-free `match`:

```rust
fn is_structural(instruction: &Instruction) -> bool {
    match instruction {
        Instruction::ListPushPlace { .. } | Instruction::ListRemovePlace { .. } => true,
        // Preserve every existing true classification explicitly.
        // Classify every remaining Instruction variant explicitly as false.
    }
}
```

Audit every mutating instruction against the View invariant while preserving the intentional `ReadField`/`WriteField`/`ReplaceField`/`CloseView` access window.

- [ ] **Step 4: Verify GREEN**

Run the focused test and all IR targets:

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-ir --test review_adversarial -- --nocapture
cargo +stable-x86_64-pc-windows-gnu test -p keld-ir --all-targets --no-fail-fast
```

Expected: all tests pass, including the promoted adversarial harness asserting rejection.

- [ ] **Step 5: Commit**

```powershell
git add -- crates/keld-ir/src/validate.rs crates/keld-ir/tests/review_adversarial.rs
git commit -m "fix(ir): classify projected list mutations as structural"
```

### Task 2: Provenance-aware IR entity liveness

**Files:**
- Create: `crates/keld-ir/tests/entity_liveness_validation.rs`
- Modify: `crates/keld-ir/src/validate.rs`

**Interfaces:**
- Consumes: CFG predecessor data, entity register types, `Copy`, entity `Phi`, `RetireEntity`, instruction uses, and terminator uses.
- Produces: per-block entity identity state carrying `BTreeMap<Register, BTreeSet<Register>>` origin sets and `BTreeSet<Register>` retired origins.

- [ ] **Step 1: Write failing direct and Copy-alias tests**

Add modules proving that validation rejects:

```rust
RetireEntity { entity: r0 }
RetireEntity { entity: r0 }
```

and:

```rust
Copy { dst: r1, src: r0 }
RetireEntity { entity: r0 }
OpenView { entity: r1, .. }
```

Assert a `KLD9002` diagnostic reports that the entity identity is retired.

- [ ] **Step 2: Verify the direct/Copy tests are RED**

Run:

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-ir --test entity_liveness_validation -- --nocapture
```

Expected: the malformed modules currently validate, so the assertions fail.

- [ ] **Step 3: Add identity-origin CFG dataflow**

Add incoming/outgoing entity states to `FunctionValidator`. Parameters and independent producers receive singleton origins. Entity `Copy` inherits the source origins. Entity `Phi` unions its input origins. `RetireEntity` adds every operand origin to the retired set. CFG joins union retired origins and merge register origin sets monotonically.

Before transferring an instruction, require every entity register returned by `instruction_uses` to have origins disjoint from the retired set. Apply the same rule to terminator uses. Avoid a second diagnostic when ordinary dominance/type validation already leaves the origin unknown.

- [ ] **Step 4: Write and verify failing Phi/maybe-retired tests**

Add a multi-block module where an entity Phi may name either of two origins, retire the Phi, then use one input alias. Add a branch where one predecessor retires an origin and the join uses its Copy alias. Run the test and observe RED before adding Phi/join transfer behavior.

- [ ] **Step 5: Complete Phi and join behavior, then verify GREEN**

Run:

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-ir --test entity_liveness_validation -- --nocapture
cargo +stable-x86_64-pc-windows-gnu test -p keld-ir --all-targets --no-fail-fast
cargo +stable-x86_64-pc-windows-gnu test -p keld-interpreter --test entities -- --nocapture
```

Expected: malformed identity flows are rejected and compiler-produced IR still validates and executes.

- [ ] **Step 6: Commit**

```powershell
git add -- crates/keld-ir/src/validate.rs crates/keld-ir/tests/entity_liveness_validation.rs
git commit -m "fix(ir): validate entity identity liveness"
```

### Task 3: Fallible executable allocation

**Files:**
- Modify: `crates/keld-interpreter/tests/optional_allocation.rs`
- Create: `crates/keld-interpreter/tests/allocation_faults.rs`
- Modify: `crates/keld-interpreter/src/value.rs`
- Modify: `crates/keld-interpreter/src/machine.rs`
- Modify: `crates/keld-interpreter/src/frame.rs`
- Modify: `crates/keld-interpreter/src/place.rs`
- Audit: `crates/keld-interpreter/src/cleanup.rs`
- Audit: `crates/keld-interpreter/src/list.rs`
- Audit: `crates/keld-runtime/src/store.rs`

**Interfaces:**
- Consumes: `CopyAllocation`, `allocation_failure(Span)`, `RuntimeText`, `RuntimePlace`, `Frame::new`, and allocation-count instrumentation.
- Produces: allocation-free Optional wrapping, fallible heap Text construction/copy, fallible runtime-place metadata duplication/projection, and fallible frame setup.

- [ ] **Step 1: Write and verify the RED ListGet allocation test**

Instrument exactly one `ListIndex` and one equivalent `ListGet` instruction after interpreter setup. Assert both perform the same number of allocations, proving Optional wrapping adds none.

Run:

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-interpreter --test optional_allocation -- --nocapture
```

Expected: `ListGet` reports one additional allocation.

- [ ] **Step 2: Remove the temporary Optional box and verify GREEN**

Carry `Option<Value>` directly from `structural_copy` into `Value::into_optional_some`. Re-run `optional_allocation` and `list_surface`.

- [ ] **Step 3: Write Text allocation-fault regressions and verify RED**

Add tests for long `ConstText` execution and long heap Text structural copy. Use the existing test-control pattern to inject the relevant allocation attempt and assert `RuntimeFaultKind::Allocation`, while non-injected executions preserve exact UTF-8 bytes and lengths.

- [ ] **Step 4: Make heap Text construction and copy fallible**

Represent heap Text with an owned `String` so an already-reserved value can move without `into_boxed_str`. Add fallible construction from `&str` using `try_reserve_exact`, and make `copy_leaf` return `Result<Option<Value>, CopyAllocation>` so heap Text clone failure propagates through `structural_copy`. Map `ConstText` failure through `allocation_failure(span)`.

- [ ] **Step 5: Write RED audit tests for frame/place allocation**

Add focused unit/integration coverage that exercises heap Text through projected loans, nested indexed loans, calls, and Phi transfer under allocation instrumentation or injected reservation failure. The tests must distinguish normal Keld execution from cleanup-trace observation allocation.

- [ ] **Step 6: Remove or make remaining execution allocations fallible**

- Populate `Frame::home_scopes` only after `try_reserve_exact`.
- Remove infallible `RuntimePlace: Clone`; add fallible projection-vector clone and append operations.
- Borrow place metadata for immutable loan reads.
- Temporarily take and restore place metadata for mutable loan access.
- Map unavoidable call/Phi/indexed-loan metadata reservation failure to `AllocationFault`.
- Re-run the allocation API audit over non-test interpreter/runtime sources and document why every remaining `push`, `resize`, `extend`, clone, and collection is pre-reserved, fixed-capacity, moved, compiler-side, or test-only.

- [ ] **Step 7: Verify the focused allocation and execution suites**

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-interpreter --all-targets --no-fail-fast
cargo +stable-x86_64-pc-windows-gnu test -p keld-runtime --all-targets --no-fail-fast
```

Expected: all focused tests pass with no unexpected allocation or internal failure.

- [ ] **Step 8: Commit**

```powershell
git add -- crates/keld-interpreter crates/keld-runtime
git commit -m "fix(interpreter): make executable allocation fallible"
```

### Task 4: Final verification and clean handoff

**Files:**
- Verify only; no unrelated edits.

**Interfaces:**
- Consumes: all three committed fixes.
- Produces: completed workspace evidence and clean commit boundary.

- [ ] **Step 1: Run focused cross-subsystem suites**

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-lifecycle -p keld-storage -p keld-ir -p keld-interpreter -p keld-runtime -p keld-cli --all-targets --no-fail-fast
```

- [ ] **Step 2: Run the full workspace test gate**

```powershell
cargo +stable-x86_64-pc-windows-gnu test --workspace --all-targets --no-fail-fast
```

- [ ] **Step 3: Run static and formatting gates**

```powershell
cargo +stable-x86_64-pc-windows-gnu clippy --workspace --all-targets --all-features -- -D warnings
cargo +stable-x86_64-pc-windows-gnu fmt --all -- --check
git diff --check af18dfa..HEAD
```

- [ ] **Step 4: Review ownership and commit boundary**

Confirm `git status --short`, `git log --oneline af18dfa..HEAD`, and `git diff --stat af18dfa..HEAD`. The worktree must be clean, the promoted adversarial regression must be tracked, and no new milestone work may begin.
