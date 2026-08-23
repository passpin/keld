# Keld Storage and Projected List Medium Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task with review checkpoints. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the three remaining medium-severity correctness and allocation findings at `9cd7bfe` without changing Keld source semantics.

**Architecture:** Storage verification will compare every storage access against the complete active pending-call reservation stack. Lifecycle verification will expose the CFG blocks it proved feasible, and storage verification will use that same feasibility boundary instead of asking for entity facts on omitted operations. Interpreter receiver resolution will borrow an existing runtime place for non-projecting List operations, allocating only when a new indexed loan projection must be retained.

**Tech Stack:** Rust 2024 workspace, `keld-lifecycle`, `keld-storage`, `keld-interpreter`, Cargo tests, `stats_alloc`, strict Clippy, rustfmt.

## Global Constraints

- Preserve single-home, loan, reservation, lifecycle-proven alias, cleanup, and List semantics.
- Add and run a failing regression before each production change.
- Keep ordinary source syntax unchanged; test fixtures may use existing syntax only.
- Treat lifecycle-infeasible operations as unreachable consistently; do not synthesize entity facts.
- Successful `List.try_remove` must not allocate because of receiver-place resolution.
- Use `cargo +stable-x86_64-pc-windows-gnu` for all Rust verification.

---

### Task 1: Check all active pending reservations

**Files:**
- Modify: `crates/keld-storage/tests/loan_reservations.rs`
- Modify: `crates/keld-storage/src/verify.rs`

**Interfaces:**
- Consumes: `HomeState.pending`, `Reservation`, and `reservation_conflicts`.
- Produces: one pending-access check that visits all active `PendingCall.reservations` entries.

- [x] **Step 1: Add the failing regressions**

Add tests that verify `KLD2005` for an outer read reservation followed by a nested structural removal, including:

```text
inspect(holder.items, identity(holder.items.remove(0)))
inspect(items, identity(identity(items.remove(0))))
```

where `identity` accepts and returns `Int`, and the receiver list contains one element.

- [x] **Step 2: Run the focused tests and observe the missing diagnostic**

Run:

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-storage --test loan_reservations -- --nocapture
```

Expected: the new tests fail because the current check sees only the innermost pending call.

- [x] **Step 3: Implement the invariant-level check**

Replace the single `state.pending.last()` reservation scan in `check_pending_access_inner` with a scan over `state.pending.iter().flat_map(|pending| &pending.reservations)`. Keep the existing effect-conflict predicate and indexed-reservation diagnostic selection unchanged.

- [x] **Step 4: Run the focused tests again**

Run the same command and require all reservation tests to pass.

### Task 2: Respect lifecycle-proven infeasible CFG blocks

**Files:**
- Modify: `crates/keld-lifecycle/src/verify.rs`
- Modify: `crates/keld-lifecycle/src/lib.rs` or the verified-module type definition if a reachability accessor is needed
- Modify: `crates/keld-storage/src/verify.rs`
- Create: `crates/keld-storage/tests/infeasible_cfg.rs`

**Interfaces:**
- Consumes: lifecycle analyzer reachability and `EntityOperationFacts` publication.
- Produces: a verified-flow reachability query and storage CFG propagation filtered by that query.

- [x] **Step 1: Add the failing identity-impossible branch regression**

Add a storage verification test for a function that copies an entity reference and branches on `enemy == alias`, placing a List operation in the impossible `else` branch. Assert that verification returns no diagnostics rather than panicking while requesting missing facts.

- [x] **Step 2: Run the focused regression and observe the panic**

Run:

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-storage --test infeasible_cfg -- --nocapture
```

Expected: the test exposes storage traversal of the lifecycle-omitted identity edge and the missing operation facts.

- [x] **Step 3: Export lifecycle-feasible block membership**

Record the analyzer’s reachable block set in `VerifiedFlowModule`, preserve it through `verify`, and expose a read-only `is_block_reachable(function, block)` query. The set must reflect identity-refined edges, not merely syntactic successors.

- [x] **Step 4: Filter storage analysis at the same boundary**

In `verify_function`, skip blocks not marked reachable and enqueue only reachable successors. Keep `operation_facts` strict for operations in a reachable block; no synthetic fallback facts are permitted.

- [x] **Step 5: Run focused storage and lifecycle tests**

Run:

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-storage --test infeasible_cfg -- --nocapture
cargo +stable-x86_64-pc-windows-gnu test -p keld-lifecycle --test alias_refinement -- --nocapture
```

### Task 3: Make projected `List.try_remove` receiver resolution allocation-free

**Files:**
- Modify: `crates/keld-interpreter/src/machine.rs`
- Modify: `crates/keld-interpreter/src/frame.rs`

**Interfaces:**
- Consumes: `Frame::loan`, `Receiver`, `RuntimePlace`, and existing `with_place_value`/`with_place_mut` accessors.
- Produces: receiver operations that borrow an already stored runtime place and only own a place when extending it for an indexed loan.

- [x] **Step 1: Add stats-based failing coverage**

Compile modules before opening a `stats_alloc::Region`, construct an interpreter before the region, and execute successful `try_remove` cases for a struct field and an entity field. Assert the operation path performs no additional allocations after the list has been prepared; the pre-fix projected-place clone must make the assertions fail.

- [x] **Step 2: Run the focused allocation tests and observe the extra allocation**

Run:

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-interpreter --test projected_allocation -- --nocapture
```

- [x] **Step 3: Implement borrowed receiver resolution**

Return a borrowed-or-owned receiver place (or an equivalent closure-based view). Borrow `Frame::loan(receiver.list)` directly for `try_remove`, `get`, `clear`, and reserve operations; preserve owned place construction for source projections and indexed loan creation.

- [x] **Step 4: Run focused interpreter tests**

Run:

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-interpreter --test optional_allocation -- --nocapture
cargo +stable-x86_64-pc-windows-gnu test -p keld-interpreter --test projected_allocation -- --nocapture
cargo +stable-x86_64-pc-windows-gnu test -p keld-interpreter --test list_surface -- --nocapture
cargo +stable-x86_64-pc-windows-gnu test -p keld-interpreter --lib -- --nocapture --test-threads=1
```

### Final verification and commit

- [x] Run the focused storage, lifecycle, and interpreter suites.
- [x] Run `cargo +stable-x86_64-pc-windows-gnu test --workspace --all-targets --no-fail-fast`.
- [x] Run `cargo +stable-x86_64-pc-windows-gnu clippy --workspace --all-targets --all-features -- -D warnings`.
- [x] Run `cargo +stable-x86_64-pc-windows-gnu fmt --all -- --check` and `git diff --check`.
- [x] Review the diff, preserve the pre-existing untracked `review_adversarial.rs`, and commit only the current fixes and their regression coverage.
