# Keld Managed Cleanup and List Verification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every managed Keld value receive one explicit, deterministic cleanup on normal exits, then complete the normative `List[T]` indexing, replacement, removal, clearing, reservation, capacity, and nested-storage verification surface.

**Architecture:** `keld-flow` will preserve lexical storage scopes and address-free projected places. `keld-storage` will remain the sole owner of Home/loan analysis and will add an executable cleanup plan: direct reverse-order drops for uniform paths, conditional flags for `MaybeLive`, and a compact per-scope order tracker only when incoming paths disagree about successful-initialization order. `keld-ir` will encode moves, loans, replacements, drops, bounds, and capacity operations explicitly; the interpreter will execute that contract without simulating loans or transfers through structural copies.

**Tech Stack:** Rust 2024 workspace, Cargo stable 1.97 GNU Windows toolchain, existing lossless syntax/HIR/Flow/Storage/IR/interpreter pipeline, deterministic unit and integration tests.

## Global Constraints

- Do not change the approved source rules in `docs/spec/storage-values.md` or `docs/spec/numeric-safety.md` unless a new contradiction is demonstrated with a focused failing test and recorded before any semantic edit.
- Preserve left-to-right evaluation and address-free pending reservations.
- Managed cleanup occurs on normal scope exit and return. Future typed-error edges must use the same cleanup plan. Non-catchable runtime faults remain memory-safe but do not promise user cleanup.
- `List[T]` and `Text` stay single-home. Managed aggregate fields compose recursively. `Map`, `Set`, `Slice`, iterators, `for`, substrings, and user cleanup remain unavailable.
- A named single-home value never moves or copies implicitly. Owned temporaries transfer automatically into consuming contexts.
- A normal single-home parameter is a call-bounded loan. A `take` parameter is a callee-owned home.
- No managed loan operation may allocate merely to simulate borrowing.
- Cleanup order is reverse successful-initialization order; fields and List elements are cleaned in reverse declaration/index order.
- `List()` allocates no element buffer. Bounds and capacity checks execute in every build.
- `reserve` faults with `CapacityFault` for impossible sizes and `AllocationFault` only after the preferred-growth attempt and minimum-capacity retry both fail. `try_reserve` returns `false` and leaves the List unchanged for either failure.
- Use `cargo +stable-x86_64-pc-windows-gnu` because the current host lacks the MSVC linker.
- Every task follows red-green-refactor, passes focused tests, runs `cargo +stable-x86_64-pc-windows-gnu fmt --all -- --check`, and commits only its files.

---

## File Ownership Map

### New files

- `crates/keld-storage/src/cleanup.rs` — cleanup-order dataflow, `MaybeLive` flags, uniform versus tracked scopes, and exit action construction.
- `crates/keld-storage/src/plan.rs` — public, immutable storage-plan types consumed by IR lowering.
- `crates/keld-storage/tests/cleanup_planner.rs` — Home-to-cleanup planning and CFG-order tests.
- `crates/keld-storage/tests/list_indexing.rs` — compile-pass/fail indexed-place and element-origin cases.
- `crates/keld-storage/tests/list_reservations.rs` — indexed replacement and projected-place overlap tests.
- `crates/keld-flow/tests/storage_scopes.rs` — lexical storage-scope and exit-edge lowering tests.
- `crates/keld-ir/tests/storage_validation.rs` — malformed move, loan, home, cleanup, Optional, and List IR rejection tests.
- `crates/keld-interpreter/src/cleanup.rs` — iterative managed-value destruction and test-only cleanup tracing.
- `crates/keld-interpreter/src/list.rs` — `RuntimeList`, checked capacity arithmetic, growth retry, and deterministic allocation injection.
- `crates/keld-interpreter/src/place.rs` — address-free runtime loan/place resolution across frames, fields, and List indices.
- `crates/keld-interpreter/tests/cleanup.rs` — end-to-end local, temporary, field, nested aggregate, and entity cleanup tests.
- `crates/keld-interpreter/tests/list_surface.rs` — end-to-end bounds, indexing, replacement, removal, clear, reserve, and nested List tests.

### Modified files

- `crates/keld-semantics/src/hir.rs` — typed List expressions and projected assignment places.
- `crates/keld-semantics/src/check.rs` — dispatch into focused List checking helpers and accept indexed places.
- `crates/keld-semantics/src/check/list.rs` — create as the focused checker for List built-ins and indexing.
- `crates/keld-semantics/tests/types.rs` — typed-HIR and diagnostic coverage for the complete List surface.
- `crates/keld-flow/src/cfg.rs` — `StorageScopeId`, scope ownership, projected places, and per-block scope identity.
- `crates/keld-flow/src/op.rs` — generalized List receiver operations and indexed-replacement reservation operations.
- `crates/keld-flow/src/lower.rs` — lexical scope exits, projected-place evaluation, and exact List evaluation order.
- `crates/keld-flow/src/dump.rs` — stable dumps for scope exits, projections, and List operations.
- `crates/keld-flow/tests/evaluation_order.rs` — receiver/index/RHS ordering and whole-receiver nested structural calls.
- `crates/keld-storage/src/lib.rs` — export cleanup-plan interfaces.
- `crates/keld-storage/src/state.rs` — cleanup-order state joined with the existing Home lattice.
- `crates/keld-storage/src/verify.rs` — connect existing transfer/loan checks to plan construction and indexed reservations; move cleanup-specific logic into `cleanup.rs`.
- `crates/keld-storage/tests/home_verifier.rs` — moved/reinitialized home cleanup facts.
- `crates/keld-storage/tests/loan_reservations.rs` — generalized projection overlap behavior.
- `crates/keld-ir/src/module.rs` — Optional types, register storage roles, storage scopes, and projected argument sources.
- `crates/keld-ir/src/instruction.rs` — explicit home install/replacement/drop and complete List instructions.
- `crates/keld-ir/src/lower.rs` — consume `FunctionStoragePlan`; never infer a memory decision locally.
- `crates/keld-ir/src/validate.rs` — ownership-state, cleanup, projected-place, Optional, bounds, and capacity validation.
- `crates/keld-ir/src/dump.rs` — stable text for every new executable operation.
- `crates/keld-ir/tests/dump_golden.rs` — one canonical full-storage IR golden.
- `crates/keld-interpreter/src/frame.rs` — owned, loan, and drop-slot register storage plus tracked cleanup order.
- `crates/keld-interpreter/src/value.rs` — `RuntimeList`, Optional values, and structural copy support without recursive host calls.
- `crates/keld-interpreter/src/machine.rs` — explicit move/loan/drop execution and List instruction dispatch.
- `crates/keld-interpreter/src/fault.rs` — `Capacity` runtime fault.
- `crates/keld-interpreter/src/lib.rs` — test-only cleanup/allocation controls without changing CLI behavior.
- `crates/keld-interpreter/tests/storage.rs` — retain existing storage behavior as regression coverage.
- `crates/keld-cli/src/render.rs` — render `CapacityFault` consistently with other runtime faults.
- `crates/keld-cli/tests/cli.rs` — bounds and capacity exit-code/rendering checks.
- `docs/compiler-architecture.md` — record the cleanup-plan and executable List boundaries after implementation.

---

### Task 1: Preserve Lexical Storage Scopes in Flow IR

**Files:**
- Modify: `crates/keld-flow/src/cfg.rs`
- Modify: `crates/keld-flow/src/op.rs`
- Modify: `crates/keld-flow/src/lower.rs`
- Modify: `crates/keld-flow/src/dump.rs`
- Create: `crates/keld-flow/tests/storage_scopes.rs`

**Interfaces:**
- Produces: `StorageScopeId(pub u32)`, `FlowFunction::storage_scope_parents`, `FlowFunction::local_scopes`, `FlowBlock::storage_scope`, and `Terminator::ExitScopes { storage_scopes, lifecycles, next }`.
- Preserves: existing lifecycle IDs and lifecycle exit ordering; storage scopes are a separate lexical tree.

- [ ] **Step 1: Write failing Flow tests for nested and early exits**

Add tests that lower the following source and assert exact scope ownership:

```rust
#[test]
fn branch_locals_exit_before_the_merge() {
    let flow = lower_text_for_test(
        "fn main() -> Int { if true { let text: Text = \"inner\" } else { let n = 0 } return 0 }\n",
    )
    .expect("source reaches Flow");
    let main = flow.function_named("main").expect("main exists");

    let text_local = LocalId(0);
    assert_ne!(main.local_scopes[text_local.0 as usize], StorageScopeId(0));
    assert!(main.blocks.iter().any(|block| matches!(
        &block.terminator,
        Terminator::ExitScopes { storage_scopes, next: ExitTarget::Goto(_), .. }
            if storage_scopes == &[main.local_scopes[text_local.0 as usize]]
    )));
}

#[test]
fn return_exits_storage_scopes_inside_out() {
    let flow = lower_text_for_test(
        "fn main() -> Int { lifecycle level { let outer: Text = \"a\" if true { let inner: Text = \"b\" return 1 } } return 0 }\n",
    )
    .expect("source reaches Flow");
    let main = flow.function_named("main").expect("main exists");
    let exit = main.blocks.iter().find_map(|block| match &block.terminator {
        Terminator::ExitScopes { storage_scopes, next: ExitTarget::Return(Some(_)), .. } => {
            Some(storage_scopes)
        }
        _ => None,
    }).expect("return exit exists");
    assert!(exit.windows(2).all(|pair| {
        main.storage_scope_parents[pair[0].0 as usize] == Some(pair[1])
    }));
}
```

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-flow --test storage_scopes
```

Expected: compile failure because `StorageScopeId`, local scope metadata, and storage exit lists do not exist.

- [ ] **Step 3: Add lexical scope metadata without cleanup behavior**

Add `StorageScopeId`, then add the listed fields to the existing Flow structs:

```rust
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StorageScopeId(pub u32);

pub storage_scope_parents: Vec<Option<StorageScopeId>>,
pub local_scopes: Vec<StorageScopeId>,
pub storage_scope: StorageScopeId,
```

Extend `ExitScopes` exactly as follows:

```rust
ExitScopes {
    storage_scopes: Vec<StorageScopeId>,
    lifecycles: Vec<LifecycleId>,
    next: ExitTarget,
}
```

In `FunctionBuilder`, allocate root scope `StorageScopeId(0)`, assign parameters to it, allocate one child for every nested `HirBlock`, assign each `let`/`var` local to the current scope, and emit inside-out storage scope IDs on branch merge, lifecycle exit, `when` exit, and return. Do not emit cleanup operations in this task.

- [ ] **Step 4: Update stable Flow dumps and run GREEN**

Run:

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-flow --all-targets
cargo +stable-x86_64-pc-windows-gnu test -p keld-lifecycle --all-targets
```

Expected: all Flow and lifecycle tests pass; dumps show concrete sequences such as `storage_exit [s2, s1]` independently of lifecycle exits.

- [ ] **Step 5: Commit the scope-preservation slice**

```powershell
git add crates/keld-flow
git commit -m "feat: preserve lexical storage scopes in flow"
```

---

### Task 2: Define the Immutable Storage Cleanup Plan

**Files:**
- Create: `crates/keld-storage/src/plan.rs`
- Create: `crates/keld-storage/src/cleanup.rs`
- Modify: `crates/keld-storage/src/lib.rs`
- Modify: `crates/keld-storage/src/verify.rs`
- Create: `crates/keld-storage/tests/cleanup_planner.rs`

**Interfaces:**
- Consumes: `VerifiedFlowModule`, Flow storage scopes, the existing `Home` lattice, and existing `ValueOrigin` results.
- Produces: `FunctionStoragePlan`, aligned one-to-one with Flow functions, blocks, and operations.

- [ ] **Step 1: Write failing ownership-classification tests**

Add assertions for borrowed parameters, consuming parameters, locals, and temporaries:

```rust
#[test]
fn plan_distinguishes_owned_temporary_local_home_and_loan() {
    let verified = verify_source(
        "fn inspect(items: List[Int]) -> Int { return items.length }\nfn forward(take items: List[Int]) -> List[Int] { let copied = items.copy(); return take copied }\nfn main() -> Int { return 0 }\n",
    );
    let module = verified.module.expect("storage verifies");
    let inspect = &module.annotations.functions[0];
    let forward = &module.annotations.functions[1];

    assert!(matches!(inspect.locals[0], LocalStorage::Loan));
    assert!(matches!(forward.locals[0], LocalStorage::Home { .. }));
    assert!(forward.values.iter().any(|value| matches!(
        value,
        ValueStorage::OwnedTemporary { .. }
    )));
}
```

- [ ] **Step 2: Run the test and verify RED**

Run:

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-storage --test cleanup_planner
```

Expected: compile failure because cleanup-plan types do not exist.

- [ ] **Step 3: Add exact public plan types**

Define these interfaces in `plan.rs`:

```rust
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum HomeId {
    Local(LocalId),
    Temporary(ValueId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LocalStorage {
    Trivial,
    Loan,
    Home { scope: StorageScopeId },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValueStorage {
    Trivial,
    EntityFlow,
    Loan(Place),
    OwnedTemporary { scope: StorageScopeId },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoreKind {
    Initialize,
    ReplaceLive,
    ReplaceMaybeLive,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CleanupAction {
    Drop(HomeId),
    DropIfLive(HomeId),
    CleanupTrackedScope(StorageScopeId),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OperationStoragePlan {
    pub store: Option<StoreKind>,
    pub post_success: Vec<CleanupAction>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BlockStoragePlan {
    pub operations: Vec<OperationStoragePlan>,
    pub exit: Vec<CleanupAction>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FunctionStoragePlan {
    pub locals: Vec<LocalStorage>,
    pub values: Vec<ValueStorage>,
    pub drop_flags: BTreeSet<HomeId>,
    pub tracked_scopes: BTreeSet<StorageScopeId>,
    pub blocks: Vec<BlockStoragePlan>,
}
```

Replace `StorageAnnotations::verified_functions` with `StorageAnnotations::functions: Vec<FunctionStoragePlan>`. Keep `FunctionStorageSummary` separate because it is public call-effect metadata.

- [ ] **Step 4: Classify every managed Flow value at its unique definition**

Move origin-to-storage classification into `cleanup.rs`. A successful verification must classify every managed `ValueId` as exactly `Loan(place)` or `OwnedTemporary`; `Unknown` is a compiler diagnostic boundary, not an executable fallback. Keep implicit-copy and entity-flow values out of cleanup homes.

- [ ] **Step 5: Run focused and existing verifier tests**

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-storage --all-targets
```

Expected: the new classification test and all existing Home/loan tests pass.

- [ ] **Step 6: Commit the plan data model**

```powershell
git add crates/keld-storage
git commit -m "feat: describe executable storage cleanup plans"
```

---

### Task 3: Plan Reverse-Initialization Cleanup Across CFG Joins

**Files:**
- Modify: `crates/keld-storage/src/state.rs`
- Modify: `crates/keld-storage/src/cleanup.rs`
- Modify: `crates/keld-storage/src/verify.rs`
- Modify: `crates/keld-storage/tests/cleanup_planner.rs`
- Modify: `crates/keld-storage/tests/home_verifier.rs`

**Interfaces:**
- Consumes: per-operation Home transitions and Flow `storage_scopes` exit lists.
- Produces: direct `Drop`, `DropIfLive`, or `CleanupTrackedScope` actions with no unresolved cleanup choice.

- [ ] **Step 1: Add failing tests for uniform, conditional, and order-divergent paths**

Cover all three planner modes:

```rust
#[test]
fn uniform_scope_uses_direct_reverse_drops() {
    let plan = plan_for_main(
        "fn main() -> Int { let first: Text = \"a\"; let second: Text = \"b\"; return 0 }\n",
    );
    assert_eq!(
        return_actions(&plan),
        &[CleanupAction::Drop(HomeId::Local(LocalId(1))),
          CleanupAction::Drop(HomeId::Local(LocalId(0)))]
    );
}

#[test]
fn maybe_live_home_uses_one_conditional_flag() {
    let plan = plan_for_main(
        "fn main() -> Int { var text: Text; if true { text = \"a\" } return 0 }\n",
    );
    assert_eq!(plan.drop_flags, [HomeId::Local(LocalId(0))].into_iter().collect());
    assert!(return_actions(&plan).contains(&CleanupAction::DropIfLive(HomeId::Local(LocalId(0)))));
}

#[test]
fn divergent_successful_initialization_order_tracks_only_that_scope() {
    let plan = plan_for_main(
        "fn main() -> Int { var a: Text; var b: Text; if true { a = \"a\"; b = \"b\" } else { b = \"b\"; a = \"a\" } return 0 }\n",
    );
    assert_eq!(plan.tracked_scopes, [StorageScopeId(0)].into_iter().collect());
    assert_eq!(return_actions(&plan), &[CleanupAction::CleanupTrackedScope(StorageScopeId(0))]);
}
```

- [ ] **Step 2: Run the tests and verify RED**

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-storage --test cleanup_planner
```

Expected: ownership classification exists, but no exit actions or tracked-scope detection exists.

- [ ] **Step 3: Add cleanup-order state and joins**

Use this state shape internally:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
enum CleanupOrder {
    Known(Vec<HomeId>),
    Divergent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ScopeCleanupState {
    scope: StorageScopeId,
    order: CleanupOrder,
}
```

On successful initialization, remove an already-active home from the order and append it. On `take` or immediate drop, remove it. At a join, retain `Known` only when all reachable incoming active orders are identical after excluding homes outside that scope; otherwise use `Divergent`. Mark only divergent scopes as tracked.

Run a second annotation pass after the fixed point: emit direct reverse drops for `Known`, `DropIfLive` only for homes whose exit state is `MaybeLive`, and one `CleanupTrackedScope` for a divergent scope. Nested scopes exit inside-out.

- [ ] **Step 4: Plan temporary cleanup and replacement**

For each owned temporary:

- activate it only after its producing operation succeeds;
- transfer it into the next consuming home without copying;
- retain it through a normal loan call and drop it in `post_success` after the loan closes;
- remove it when returned or transferred into a consuming parameter; and
- leave it in the enclosing scope plan if a future typed-error edge can bypass its next use.

Annotate each managed `StoreLocal` as `Initialize`, `ReplaceLive`, or `ReplaceMaybeLive`. Reinitializing a moved home appends that home as the newest successful initialization.

- [ ] **Step 5: Run storage tests and property-check joins**

Add a small generated acyclic-CFG test that enumerates initialize/take/reinitialize actions on three homes and compares planner exits against a reference stack model. Then run:

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-storage --all-targets
```

Expected: all direct, conditional, divergent-order, temporary, and reference-model cases pass.

- [ ] **Step 6: Commit CFG cleanup planning**

```powershell
git add crates/keld-storage
git commit -m "feat: plan deterministic managed cleanup"
```

---

### Task 4: Encode and Validate Explicit Home Operations in Executable IR

**Files:**
- Modify: `crates/keld-ir/src/module.rs`
- Modify: `crates/keld-ir/src/instruction.rs`
- Modify: `crates/keld-ir/src/lower.rs`
- Modify: `crates/keld-ir/src/validate.rs`
- Modify: `crates/keld-ir/src/dump.rs`
- Create: `crates/keld-ir/tests/storage_validation.rs`
- Modify: `crates/keld-ir/tests/dump_golden.rs`

**Interfaces:**
- Consumes: `FunctionStoragePlan` only; IR lowering does not recompute Home or loan facts.
- Produces: typed register roles and explicit non-failing install/move/drop sequences.

- [ ] **Step 1: Write malformed-IR tests**

Construct modules that contain one error each and assert `KLD9006`:

```rust
#[test]
fn managed_home_cannot_return_live_without_cleanup() {
    let mut module = lower_ok(
        "fn main() -> Int { let value: Text = \"Keld\"; return 0 }\n",
    );
    remove_last_cleanup_of_main(&mut module);
    assert_ir_error(&module, "KLD9006", "live managed home at return");
}

#[test]
fn moved_home_cannot_be_dropped_twice() {
    let mut module = lower_ok(
        "fn consume(take value: Text) { return }\nfn main() -> Int { let value: Text = \"Keld\"; consume(take value); return 0 }\n",
    );
    insert_drop_after_move(&mut module);
    assert_ir_error(&module, "KLD9006", "drop of empty home");
}

#[test]
fn a_loan_register_cannot_be_moved_or_dropped() {
    let mut module = lower_ok(
        "fn inspect(value: Text) -> Int { return value.byte_length }\nfn main() -> Int { return inspect(\"Keld\") }\n",
    );
    replace_first_loan_read_with_move(&mut module);
    assert_ir_error(&module, "KLD9006", "loan register used as an owned source");
}

#[test]
fn tracked_scope_cleanup_must_name_the_register_scope() {
    let mut module = lower_order_divergent_scope();
    rewrite_cleanup_scope(&mut module, StorageScopeId(u32::MAX));
    assert_ir_error(&module, "KLD9006", "unknown cleanup scope");
}

#[test]
fn displaced_values_must_be_dropped_after_replacement() {
    let mut module = lower_ok(
        "fn main() -> Int { var value: Text = \"old\"; value = \"new\"; return 0 }\n",
    );
    remove_first_drop_slot(&mut module);
    assert_ir_error(&module, "KLD9006", "live displaced value at return");
}
```

Define all five mutation helpers in the test file by flattening `module.functions[*].blocks[*].instructions`, locating the named instruction with `find_map`, and changing exactly one instruction. Call `expect("compiler-produced instruction exists")` before mutation, so a lowering regression cannot turn the negative test into a false pass.

- [ ] **Step 2: Run and verify RED**

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-ir --test storage_validation
```

Expected: compile failure because register storage roles and cleanup instructions do not exist.

- [ ] **Step 3: Add IR storage roles and scope metadata**

Add:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegisterStorage {
    Trivial,
    EntityFlow,
    Loan,
    Home { scope: StorageScopeId, conditional: bool },
    DropSlot,
}

pub register_storage: Vec<RegisterStorage>,
pub storage_scope_parents: Vec<Option<StorageScopeId>>,
```

Add executable operations:

```rust
InstallHome {
    destination: Register,
    source: Register,
    displaced: Register,
    span: Span,
},
MoveHome { destination: Register, source: Register, span: Span },
DropHome { home: Register, span: Span },
DropIfLive { home: Register, span: Span },
DropSlot { slot: Register, span: Span },
CleanupTrackedScope { scope: StorageScopeId, span: Span },
ReplacePlace {
    destination: ArgumentSource,
    source: Register,
    displaced: Register,
    span: Span,
},
ReplaceField {
    view: ViewId,
    field: FieldId,
    source: Register,
    displaced: Register,
    span: Span,
},
```

`InstallHome` is one non-failing commit: move the new value into `destination`, place the previous value or absence in `displaced`, then allow a later `DropSlot`. `ReplacePlace` provides the same contract for a local aggregate field. An entity field replacement follows `OpenView(Edit) -> ReplaceField -> CloseView -> DropSlot`, preserving replacement-before-destruction without holding a view during cleanup.

- [ ] **Step 4: Lower storage annotations mechanically**

Map managed producer `ValueId`s and managed locals to `RegisterStorage::Home`, place-derived values to `Loan`, and replacement scratch registers to `DropSlot`. Use `MoveHome` for `take`, constructor fields, List elements, consuming arguments, owned returns, and Phi inputs. Use structural copy only for source `.copy()`.

Emit planner `post_success` actions immediately after the operation. Emit terminator exit actions before lifecycle `EndLifecycle` and before the final branch/return. For entity retirement, lifecycle cleanup continues to own entity-field destruction.

- [ ] **Step 5: Validate the ownership automaton**

Extend IR validation with the lattice `Empty | Live | MaybeLive` for `Home` registers and `Empty | Live` for `DropSlot`. Check all predecessor joins, exact operation preconditions, scope ancestry, no open entity view across structural/drop instructions, and no live owned register on normal return except the selected owned return value.

- [ ] **Step 6: Run IR tests and inspect the golden**

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-ir --all-targets
```

Expected: malformed modules fail with `KLD9006`; compiler-produced IR validates; the golden contains explicit `install_home`, `move_home`, `drop`, `drop_if_live`, and `cleanup_scope` operations.

- [ ] **Step 7: Commit executable cleanup IR**

```powershell
git add crates/keld-ir
git commit -m "feat: encode explicit managed cleanup in IR"
```

---

### Task 5: Execute Loans Without Structural Copies

**Files:**
- Create: `crates/keld-interpreter/src/place.rs`
- Modify: `crates/keld-interpreter/src/frame.rs`
- Modify: `crates/keld-interpreter/src/machine.rs`
- Modify: `crates/keld-interpreter/src/lib.rs`
- Modify: `crates/keld-interpreter/tests/storage.rs`

**Interfaces:**
- Consumes: `RegisterStorage::Loan` and projected `ArgumentSource` values.
- Produces: address-free runtime loan handles that resolve through frame/register or entity-identity roots.

- [ ] **Step 1: Write failing no-copy loan tests**

Add deterministic allocation injection that fails the next structural-copy allocation, then prove read loans still work:

```rust
#[test]
fn read_loan_of_heap_text_does_not_allocate_or_copy() {
    let result = run_text_with_controls_for_test(
        "fn size(value: Text) -> Int { return value.byte_length }\nfn main() -> Int { let value: Text = \"abcdefghijklmnopqrstuvwxyz\"; return size(value) }\n",
        TestControls::fail_structural_copy(1),
    ).expect("loan does not allocate");
    assert_eq!(result.value, Value::Int(26));
}

#[test]
fn two_read_parameters_can_share_one_source_place() {
    let result = run_text_for_test(
        "fn sum(a: List[Int], b: List[Int]) -> Int { return a.length + b.length }\nfn main() -> Int { let items: List[Int] = List(); items.push(1); return sum(items, items) }\n",
    ).expect("overlapping reads execute");
    assert_eq!(result.value, Value::Int(2));
}
```

- [ ] **Step 2: Run and verify RED**

Run the two tests by exact name. Expected: the read loan attempts `try_copy_value` and injected allocation fails.

- [ ] **Step 3: Replace `Option<Value>` registers with explicit slots**

Use:

```rust
pub(crate) enum RegisterSlot {
    Empty,
    Owned(Value),
    Loan(RuntimePlace),
    DropSlot(Option<Value>),
}

pub(crate) enum RuntimePlaceRoot {
    Frame { frame: FrameId, register: Register },
    Entity { entity: EntityId },
}

pub(crate) enum RuntimeProjection {
    Field(FieldId),
    Index(usize),
}
```

Normalize a loan through an existing loan parameter to its original root. Entity-field loans retain only entity identity and projections, never a payload address. Snapshot evaluated List indices into `usize` only after their checked `Int` conversion.

- [ ] **Step 4: Resolve reads and edits through the loan handle**

Replace `frame_value` with typed helpers:

```rust
fn read_register(
    &self,
    frame: FrameId,
    register: Register,
) -> Result<&Value, InterpreterFailure>;
fn with_place_mut<R>(
    &mut self,
    place: &RuntimePlace,
    f: impl FnOnce(&mut Value) -> R,
)
    -> Result<R, InterpreterFailure>;
fn take_owned_register(
    &mut self,
    frame: FrameId,
    register: Register,
) -> Result<Value, InterpreterFailure>;
```

Normal loan calls install `RegisterSlot::Loan` in the callee; they do not take, copy, or write back. Structural operations mutate through `with_place_mut`. Consuming calls move an owned register. Delete `LoanReturn` after all existing structural-loan tests pass through direct place mutation.

- [ ] **Step 5: Make aggregate construction and Phi consume managed inputs**

For a managed field/input, call `take_owned_register`; for an implicit-copy input, copy the scalar/link value. A selected managed Phi input moves into its destination. No constructor, return, or field loan may invoke `try_copy_value` unless the source program contains `.copy()`.

- [ ] **Step 6: Run interpreter storage regression tests**

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-interpreter --test storage
```

Expected: existing 21 storage tests plus no-copy read-loan cases pass.

- [ ] **Step 7: Commit exact runtime loans**

```powershell
git add crates/keld-interpreter
git commit -m "fix: execute storage loans without hidden copies"
```

---

### Task 6: Execute and Trace Deterministic Managed Cleanup

**Files:**
- Create: `crates/keld-interpreter/src/cleanup.rs`
- Modify: `crates/keld-interpreter/src/frame.rs`
- Modify: `crates/keld-interpreter/src/machine.rs`
- Modify: `crates/keld-interpreter/src/value.rs`
- Modify: `crates/keld-interpreter/src/lib.rs`
- Create: `crates/keld-interpreter/tests/cleanup.rs`

**Interfaces:**
- Consumes: explicit cleanup IR and entity payloads returned by `keld-runtime` retirement.
- Produces: iterative destruction, one hidden active-home order tracker per marked scope, and a test-only `CleanupEvent` stream.

- [ ] **Step 1: Write failing cleanup-order tests**

Add cases for:

```rust
#[test]
fn locals_drop_in_reverse_successful_initialization_order() {
    let trace = trace_text_for_test(
        "fn main() -> Int { let first: Text = \"a\"; let second: Text = \"b\"; return 0 }\n",
    ).expect("program executes");
    assert_eq!(trace.text_markers(), vec![(1, b'b'), (1, b'a')]);
}

#[test]
fn moved_and_uninitialized_homes_are_skipped() {
    let trace = trace_text_for_test(
        "fn consume(take value: Text) { return }\nfn main() -> Int { let a: Text = \"a\"; var b: Text; consume(take a); return 0 }\n",
    ).expect("program executes");
    assert_eq!(trace.text_markers(), vec![(1, b'a')]);
}

#[test]
fn maybe_live_home_drops_only_on_the_initialized_path() {
    let live = trace_text_for_test(
        "fn main() -> Int { var value: Text; if true { value = \"x\" } return 0 }\n",
    ).expect("live path executes");
    let empty = trace_text_for_test(
        "fn main() -> Int { var value: Text; if false { value = \"x\" } return 0 }\n",
    ).expect("empty path executes");
    assert_eq!(live.text_markers(), vec![(1, b'x')]);
    assert!(empty.text_markers().is_empty());
}

#[test]
fn reinitialized_home_becomes_the_newest_cleanup() {
    let trace = trace_text_for_test(
        "fn consume(take value: Text) { return }\nfn main() -> Int { var a: Text = \"a\"; let b: Text = \"b\"; consume(take a); a = \"c\"; return 0 }\n",
    ).expect("program executes");
    assert_eq!(trace.text_markers(), vec![(1, b'a'), (1, b'c'), (1, b'b')]);
}

#[test]
fn owned_temporary_loan_drops_after_the_call_closes() {
    let trace = trace_text_for_test(
        "fn inspect(value: Text) -> Int { return value.byte_length }\nfn main() -> Int { return inspect(\"abcdefghijklmnopqrstuvwxyz\") }\n",
    ).expect("program executes");
    assert_eq!(trace.result.value, Value::Int(26));
    assert_eq!(trace.text_markers(), vec![(26, b'a')]);
}

#[test]
fn fields_and_list_elements_drop_in_reverse_order_iteratively() {
    let trace = trace_text_for_test(
        "struct Pair {\nleft: Text\nright: Text\n}\nfn main() -> Int { let values: List[Pair] = List(); values.push(Pair(left: \"a\", right: \"b\")); values.push(Pair(left: \"c\", right: \"d\")); return 0 }\n",
    ).expect("program executes");
    assert_eq!(trace.list_indices(), vec![1, 0]);
    assert_eq!(trace.field_ids(), vec![FieldId(1), FieldId(0), FieldId(1), FieldId(0)]);
}
```

Use a hidden test API returning `ExecutionTrace { result, cleanup: Vec<CleanupEvent> }`. Events identify `Home(Register)`, `StructField { definition, field }`, `EntityField { definition, field }`, `ListElement { index }`, and the non-allocating Text marker `{ byte_length, first_byte }`. The marker exists only in the test observer and is not part of Keld execution semantics.

- [ ] **Step 2: Run and verify RED**

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-interpreter --test cleanup
```

Expected: compile failure because explicit cleanup execution and tracing do not exist.

- [ ] **Step 3: Add iterative managed destruction**

Implement a non-recursive work stack:

```rust
enum CleanupTask {
    Value { value: Value, path: CleanupPath },
}

fn cleanup_value(value: Value, root: CleanupPath, trace: &mut CleanupTrace) {
    let mut work = vec![CleanupTask::Value { value, path: root }];
    while let Some(CleanupTask::Value { value, path }) = work.pop() {
        match value {
            Value::Struct { definition, fields } => {
                for (index, field) in fields.into_iter().enumerate() {
                    work.push(CleanupTask::Value {
                        value: field,
                        path: path.struct_field(definition, FieldId(index as u32)),
                    });
                }
            }
            Value::List(elements) => {
                for (index, element) in elements.into_iter().enumerate() {
                    work.push(CleanupTask::Value {
                        value: element,
                        path: path.list_element(index),
                    });
                }
            }
            Value::Unit
            | Value::Int(_)
            | Value::Bool(_)
            | Value::Text(_)
            | Value::Entity(_)
            | Value::Link(_)
            | Value::Lifecycle(_) => {}
        }
    }
}
```

Use definition layout metadata to translate vector positions to declared `FieldId`s. Empty each container before its Rust value falls out of scope so host recursive drop is never the cleanup algorithm. Task 9 extends this match with `Value::Optional(Some(value))` by pushing its boxed payload and treats `Value::Optional(None)` as empty.

- [ ] **Step 4: Execute home and replacement instructions**

`MoveHome` deactivates the source and activates the destination. `InstallHome` moves the source into the destination before exposing the displaced value in `DropSlot`; `DropSlot` then destroys it. `DropIfLive` is a no-op only for an empty conditional home. `CleanupTrackedScope` pops the scope's fixed-capacity home-ID tracker in reverse activation order.

Allocate tracker storage once when building the frame, sized from `Function::register_storage`; no cleanup operation allocates.

- [ ] **Step 5: Route entity payload cleanup through the same engine**

Replace `end_lifecycle_with(lifecycle, drop)` and `finish_with(drop)` callbacks with cleanup closures that destroy entity fields in reverse declaration order. The store still changes the entity to `Dying` before invoking that closure.

- [ ] **Step 6: Run cleanup, storage, runtime, and IR tests**

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-interpreter --all-targets
cargo +stable-x86_64-pc-windows-gnu test -p keld-runtime --all-targets
cargo +stable-x86_64-pc-windows-gnu test -p keld-ir --all-targets
```

Expected: cleanup trace order is exact; no moved/uninitialized value is destroyed; existing lifecycle retirement order remains unchanged.

- [ ] **Step 7: Commit managed cleanup execution**

```powershell
git add crates/keld-interpreter
git commit -m "feat: execute deterministic managed cleanup"
```

---

### Task 7: Type the Complete List and Optional Surface

**Files:**
- Modify: `crates/keld-semantics/src/hir.rs`
- Modify: `crates/keld-semantics/src/check.rs`
- Create: `crates/keld-semantics/src/check/list.rs`
- Modify: `crates/keld-semantics/src/types.rs`
- Modify: `crates/keld-semantics/tests/types.rs`

**Interfaces:**
- Produces: typed HIR for index, indexed assignment, `get`, `try_remove`, `clear`, `reserve`, and `try_reserve`.
- Preserves: `Option[T]` as the nominal executable form of `T?`; no match or iterator support is enabled.

- [ ] **Step 1: Write failing semantic tests**

Add exact type assertions:

```rust
#[test]
fn types_complete_list_operations() {
    let module = analyze_ok(
        "fn probe(items: List[Int], index: Int) { let value: Int = items[index]; let maybe: Int? = items.get(index); let removed: Int? = items.try_remove(index); items[index] = 1; items.clear(); items.reserve(4); let ok: Bool = items.try_reserve(4); return }\nfn main() -> Int { return 0 }\n",
    );
    let probe = &module.functions[0];
    let expressions = probe.body.statements.iter().filter_map(|statement| match &statement.kind {
        HirStmtKind::Let { initializer, .. }
        | HirStmtKind::Expr(initializer)
        | HirStmtKind::Assign { value: initializer, .. } => Some(initializer),
        HirStmtKind::Var { initializer: Some(initializer), .. } => Some(initializer),
        _ => None,
    }).collect::<Vec<_>>();
    assert!(matches!(&expressions[0].kind, HirExprKind::ListIndex { .. }));
    assert_eq!(expressions[0].ty, TypeStore::INT);
    assert!(matches!(&expressions[1].kind, HirExprKind::ListGet { .. }));
    assert!(matches!(module.types.kind(expressions[1].ty), TypeKind::Optional(inner) if *inner == TypeStore::INT));
    assert!(matches!(&expressions[2].kind, HirExprKind::ListTryRemove { .. }));
    assert!(matches!(
        &probe.body.statements[3].kind,
        HirStmtKind::Assign { target, .. }
            if matches!(target.projections.as_slice(), [HirProjection::Index(_)])
    ));
    assert!(matches!(&expressions[4].kind, HirExprKind::ListClear(_)));
    assert!(matches!(&expressions[5].kind, HirExprKind::ListReserve { .. }));
    assert!(matches!(&expressions[6].kind, HirExprKind::ListTryReserve { .. }));
    assert_eq!(expressions[6].ty, TypeStore::BOOL);
}

#[test]
fn get_rejects_single_home_elements() {
    let diagnostics = analyze_errors(
        "fn invalid(items: List[Text]) { let value = items.get(0); return }\nfn main() -> Int { return 0 }\n",
    );
    assert_eq!(diagnostics[0].code.0, "KLD0106");
}

#[test]
fn managed_struct_field_replacement_requires_a_var_base() {
    let accepted = analyze_ok(
        "struct Holder {\nvalue: Text\n}\nfn main() -> Int { var holder = Holder(value: \"old\"); holder.value = \"new\"; return holder.value.byte_length }\n",
    );
    assert_eq!(accepted.main, FunctionId(0));
    let rejected = analyze_errors(
        "struct Holder {\nvalue: Text\n}\nfn main() -> Int { let holder = Holder(value: \"old\"); holder.value = \"new\"; return 0 }\n",
    );
    assert_eq!(rejected[0].code.0, "KLD2010");
}
```

Also assert negative diagnostics for a non-`Int` index, wrong argument counts, non-List receivers, `take list[index]`, and indexed compound assignment.

- [ ] **Step 2: Run and verify RED**

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-semantics --test types
```

Expected: indexing still reports `KLD0004` and the new methods are unknown.

- [ ] **Step 3: Add projected HIR places and List expressions**

Replace field-only places with:

```rust
pub enum HirProjection {
    Field(FieldId),
    Index(HirExpr),
}

pub struct HirPlace {
    pub base: LocalId,
    pub projections: Vec<HirProjection>,
    pub span: Span,
}
```

Add:

```rust
ListIndex { list: Box<HirExpr>, index: Box<HirExpr> },
ListGet { list: Box<HirExpr>, index: Box<HirExpr> },
ListTryRemove { list: Box<HirExpr>, index: Box<HirExpr> },
ListClear(Box<HirExpr>),
ListReserve { list: Box<HirExpr>, additional: Box<HirExpr> },
ListTryReserve { list: Box<HirExpr>, additional: Box<HirExpr> },
```

Keep `ListRemove` and `ListPush`, but route all List checking through `check/list.rs`. `List.get` is available only when `storage_class(T) == ImplicitCopy`; it returns `TypeKind::Optional(T)`. `try_remove` returns `TypeKind::Optional(T)` for every executable `T`.

- [ ] **Step 4: Accept indexed assignment without weakening `take`**

Parse the existing `Place` node left to right, resolve fields and List element types projection by projection, require a `var` base only when replacing a struct field or whole local, and allow content replacement through a `let` List. Continue to reject every non-empty projection in `take` with `KLD2004`.

- [ ] **Step 5: Run semantic and feature-gate tests**

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-semantics --all-targets
```

Expected: all complete List typing cases pass; `Map`, `Set`, `Slice`, iterator, `for`, and match behavior remains unchanged.

- [ ] **Step 6: Commit typed List surface**

```powershell
git add crates/keld-semantics
git commit -m "feat: type complete List operations"
```

---

### Task 8: Lower Projected Places and Verify Indexed Reservations

**Files:**
- Modify: `crates/keld-flow/src/cfg.rs`
- Modify: `crates/keld-flow/src/op.rs`
- Modify: `crates/keld-flow/src/lower.rs`
- Modify: `crates/keld-flow/src/dump.rs`
- Modify: `crates/keld-flow/tests/evaluation_order.rs`
- Create: `crates/keld-storage/tests/list_indexing.rs`
- Create: `crates/keld-storage/tests/list_reservations.rs`
- Modify: `crates/keld-storage/src/verify.rs`
- Modify: `crates/keld-storage/tests/loan_reservations.rs`

**Interfaces:**
- Produces: one generalized address-free `Place` projection model and indexed replacement reservations.
- Consumes: typed `HirProjection` expressions in exact source order.

- [ ] **Step 1: Write failing Flow evaluation-order tests**

Assert these operation orders:

```text
items[index] = replacement()
  CopyLocal(items)
  evaluate index
  BeginIndexedReplacement(place, saved_index)
  evaluate replacement()
  ListReplace

items.push(items.remove(0))
  evaluate remove argument completely
  open whole receiver structurally
  push removed temporary
```

Add a test that `matrix[row][column]` evaluates `row` before `column` and records both index projections without a raw address. Add a managed struct-field assignment test that emits `ReplacePlace` only after its replacement is fully evaluated; entity fields continue to emit a view-bounded entity replacement.

- [ ] **Step 2: Run Flow tests and verify RED**

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-flow --test evaluation_order
```

Expected: the new HIR variants are not lowered.

- [ ] **Step 3: Generalize place projections and List receivers**

Use:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlaceProjection {
    Field(FieldId),
    Index(IndexIdentity),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IndexIdentity {
    Constant(i64),
    Value(ValueId),
}

pub struct Place {
    pub base: LocalId,
    pub projections: Vec<PlaceProjection>,
}

pub struct StorageReceiver {
    pub value: ValueId,
    pub place: Option<Place>,
}
```

Replace local/place/value triplicates for List operations with one operation carrying `StorageReceiver`. Lower constant integer indices as `Constant`; all other evaluated indices use their `ValueId`.

Add `FlowOp::ReplacePlace { place, value, span }` for local aggregate fields. Keep entity replacement distinct as `WriteEntityField`, because lifecycle lowering must still create an edit view around the eventual `ReplaceField` IR instruction. The storage verifier requires an owned replacement for either form and never treats the displaced field as a source-level value.

- [ ] **Step 4: Add indexed replacement reservation operations**

Add:

```rust
BeginIndexedReplacement {
    reservation: u32,
    list: Place,
    index: ValueId,
    span: Span,
},
EndIndexedReplacement { reservation: u32, span: Span },
```

The verifier records `PendingReservation::IndexedDestination { list }`. During RHS evaluation it rejects `Structural` or `Take` access to that List or any prefix containing it with `KLD2007`. Reads and non-structural edits do not change the saved index and remain allowed.

- [ ] **Step 5: Implement projection overlap rules**

Walk equal-base projections until divergence:

- different fixed fields are disjoint;
- different constant List indices are disjoint;
- identical constants or identical `ValueId`s overlap;
- two other indices may overlap;
- a prefix overlaps its descendant; and
- structural access to a List place overlaps every element descendant.

Add compile tests for `items[0]` versus `items[1]`, unknown indices, nested elements, field-separated Lists, and entity bases that must-alias, must-differ, or may-alias.

- [ ] **Step 6: Run Flow and storage verification suites**

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-flow --all-targets
cargo +stable-x86_64-pc-windows-gnu test -p keld-storage --all-targets
```

Expected: indexed loan uses pass/fail according to overlap; RHS structural mutation and base transfer report `KLD2007`.

- [ ] **Step 7: Commit projected-place verification**

```powershell
git add crates/keld-flow crates/keld-storage
git commit -m "feat: verify indexed List places and reservations"
```

---

### Task 9: Execute Checked Indexing, `get`, and Indexed Replacement

**Files:**
- Modify: `crates/keld-ir/src/module.rs`
- Modify: `crates/keld-ir/src/instruction.rs`
- Modify: `crates/keld-ir/src/lower.rs`
- Modify: `crates/keld-ir/src/validate.rs`
- Modify: `crates/keld-ir/src/dump.rs`
- Modify: `crates/keld-ir/tests/storage_validation.rs`
- Create: `crates/keld-interpreter/src/list.rs`
- Modify: `crates/keld-interpreter/src/cleanup.rs`
- Modify: `crates/keld-interpreter/src/value.rs`
- Modify: `crates/keld-interpreter/src/place.rs`
- Modify: `crates/keld-interpreter/src/machine.rs`
- Modify: `crates/keld-interpreter/src/fault.rs`
- Create: `crates/keld-interpreter/tests/list_surface.rs`

**Interfaces:**
- Produces: executable Optional values and checked element operations.
- Preserves: borrowed single-home elements never become owned without `.copy()` or removal.

- [ ] **Step 1: Write failing end-to-end index tests**

Cover:

```rust
#[test]
fn copyable_index_read_returns_the_element() {
    let result = run_text_for_test(
        "fn main() -> Int { let items: List[Int] = List(); items.push(7); return items[0] }\n",
    ).expect("valid index executes");
    assert_eq!(result.value, Value::Int(7));
}

#[test]
fn negative_and_equal_length_indices_fault_with_bounds() {
    for index in [-1, 1] {
        let source = format!(
            "fn main() -> Int {{ let items: List[Int] = List(); items.push(7); return items[{index}] }}\n"
        );
        let fault = run_text_for_test(&source).expect_err("invalid index faults");
        assert_eq!(fault.kind, RuntimeFaultKind::Bounds);
    }
}

#[test]
fn text_element_can_be_loaned_but_not_bound_as_owned() {
    let result = run_text_for_test(
        "fn size(value: Text) -> Int { return value.byte_length }\nfn main() -> Int { let items: List[Text] = List(); items.push(\"Keld\"); return size(items[0]) }\n",
    ).expect("indexed Text loan executes");
    assert_eq!(result.value, Value::Int(4));
    assert_static_error(
        "fn invalid(items: List[Text]) { let owned = items[0]; return }\nfn main() -> Int { return 0 }\n",
        "KLD2004",
    );
}

#[test]
fn indexed_replacement_installs_before_cleaning_the_old_value() {
    let trace = trace_text_for_test(
        "fn main() -> Int { let items: List[Text] = List(); items.push(\"old\"); items[0] = \"new\"; return items[0].byte_length }\n",
    ).expect("replacement executes");
    assert_eq!(trace.result.value, Value::Int(3));
    assert_eq!(trace.text_markers(), vec![(3, b'o'), (3, b'n')]);
}

#[test]
fn get_returns_none_without_fault_and_does_not_change_the_list() {
    let list = RuntimeList::from_values(vec![Value::Int(7)]);
    assert_eq!(list.get_copy(1), Ok(None));
    assert_eq!(list.length(), 1);
    assert_eq!(list.get_copy(0), Ok(Some(Value::Int(7))));
}
```

- [ ] **Step 2: Run and verify RED**

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-interpreter --test list_surface
```

Expected: IR lowering lacks Optional and List index instructions.

- [ ] **Step 3: Add executable Optional, the List wrapper, and projected argument sources**

Add `IrType::Optional(Box<IrType>)`, `Value::Optional(Option<Box<Value>>)`, and recursive storage-class/validation handling. Extend `cleanup_value` so a present Optional pushes its boxed payload and an absent Optional performs no cleanup. Replace `ArgumentSource::fields` with `ArgumentSource::projections: Vec<ArgumentProjection>`, where an index projection stores the already-evaluated index register.

Create the unobservable runtime wrapper now because indexing must not depend on a raw `Vec<Value>`:

```rust
pub struct RuntimeList {
    elements: Vec<Value>,
}

impl RuntimeList {
    fn new() -> Self;
    fn from_values(values: Vec<Value>) -> Self;
    fn length(&self) -> usize;
    fn get_copy(&self, index: usize) -> Result<Option<Value>, CopyAllocation>;
    fn into_elements(self) -> std::vec::IntoIter<Value>;
}
```

Derive `Debug`, `Eq`, and `PartialEq`; expose the type and its test constructors with `#[doc(hidden)]` because `Value::List` is public. Expose `CopyAllocation` the same way for the test-only `get_copy` result. Change `Value::List(Vec<Value>)` to `Value::List(RuntimeList)` and update structural copy plus cleanup iteration without changing capacity behavior yet.

- [ ] **Step 4: Add checked List instructions**

Add:

```rust
ListIndex { dst: Register, receiver: Receiver, index: Register, span: Span },
ListGet { dst: Register, receiver: Receiver, index: Register, span: Span },
ListReplace {
    receiver: Receiver,
    index: Register,
    value: Register,
    displaced: Register,
    span: Span,
},
```

`ListIndex` copies implicit-copy elements or creates a loan register for single-home elements. `ListGet` is validated only for implicit-copy elements and never faults for bounds. `ListReplace` checks bounds at final commit, moves the replacement in, returns the old element through `DropSlot`, writes a projected receiver back only when required by its root representation, and then executes `DropSlot`.

- [ ] **Step 5: Execute exact fault and commit behavior**

Convert source `Int` to `usize` only after checking nonnegative and `< length`. On failure, report `RuntimeFaultKind::Bounds` at the List operation span without mutating the List. Resolve projected loans for nested List elements through `place.rs`.

- [ ] **Step 6: Run List, cleanup, and IR validation tests**

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-interpreter --test list_surface
cargo +stable-x86_64-pc-windows-gnu test -p keld-interpreter --test cleanup
cargo +stable-x86_64-pc-windows-gnu test -p keld-ir --all-targets
```

Expected: bounds are deterministic, old elements clean after installation, and single-home indexed reads remain loans.

- [ ] **Step 7: Commit indexing execution**

```powershell
git add crates/keld-ir crates/keld-interpreter
git commit -m "feat: execute checked List indexing"
```

---

### Task 10: Complete Removal, `try_remove`, and `clear`

**Files:**
- Modify: `crates/keld-interpreter/src/list.rs`
- Modify: `crates/keld-interpreter/src/value.rs`
- Modify: `crates/keld-interpreter/src/machine.rs`
- Modify: `crates/keld-ir/src/instruction.rs`
- Modify: `crates/keld-ir/src/lower.rs`
- Modify: `crates/keld-ir/src/validate.rs`
- Modify: `crates/keld-ir/src/dump.rs`
- Modify: `crates/keld-interpreter/tests/list_surface.rs`
- Modify: `crates/keld-interpreter/tests/cleanup.rs`

**Interfaces:**
- Produces: `RuntimeList` with unobservable length/capacity representation and explicit ownership-preserving removal/clear operations.

- [ ] **Step 1: Write failing behavior and cleanup tests**

Add:

```rust
#[test]
fn try_remove_invalid_returns_none_and_preserves_length() {
    let mut list = RuntimeList::from_values(vec![Value::Int(7)]);
    assert_eq!(list.try_remove(1), None);
    assert_eq!(list.length(), 1);
}

#[test]
fn try_remove_success_returns_owned_element_and_closes_the_gap() {
    let result = run_text_for_test(
        "fn main() -> Int { let inner: List[Int] = List(); inner.push(7); let outer: List[List[Int]] = List(); outer.push(take inner); let removed = outer.try_remove(0); return outer.length }\n",
    ).expect("successful try_remove executes");
    assert_eq!(result.value, Value::Int(0));
}

#[test]
fn clear_destroys_elements_in_reverse_index_order() {
    let trace = trace_text_for_test(
        "fn main() -> Int { let items: List[Text] = List(); items.push(\"a\"); items.push(\"b\"); items.push(\"c\"); items.clear(); return 0 }\n",
    ).expect("clear executes");
    assert_eq!(trace.list_indices(), vec![2, 1, 0]);
    assert_eq!(trace.text_markers(), vec![(1, b'c'), (1, b'b'), (1, b'a')]);
}

#[test]
fn remove_and_clear_preserve_capacity() {
    let mut list = RuntimeList::with_capacity_for_test(8);
    list.push_for_test(Value::Int(1));
    list.push_for_test(Value::Int(2));
    let capacity = list.capacity_for_test();
    let _ = list.remove(0);
    let mut removed = Vec::new();
    list.clear_into(&mut removed);
    assert_eq!(list.capacity_for_test(), capacity);
}
```

- [ ] **Step 2: Run and verify RED**

Run the named tests. Expected: `try_remove`, `clear`, and capacity-observation test helpers do not exist.

- [ ] **Step 3: Extend `RuntimeList` with removal and capacity-preserving clear**

Use:

```rust
impl RuntimeList {
    fn capacity_for_test(&self) -> usize;
    fn remove(&mut self, index: usize) -> Value;
    fn try_remove(&mut self, index: usize) -> Option<Value>;
    fn clear_into(&mut self, cleanup: &mut Vec<Value>);
}
```

`remove` and successful `try_remove` move the element out and retain the allocation. `clear_into` pops elements in reverse order and sends them through `cleanup_value`; it does not call `Vec::clear` as the semantic cleanup algorithm. Update `RuntimeList::into_elements` so scope cleanup still consumes every element iteratively.

- [ ] **Step 4: Add and validate IR operations**

Add `ListTryRemove` returning `Optional(element)` and `ListClear` returning Unit. Both are structural and accept the generalized receiver/place form. Ensure removal results are owned temporaries when their element type is single-home.

- [ ] **Step 5: Run removal and cleanup tests**

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-interpreter --test list_surface
cargo +stable-x86_64-pc-windows-gnu test -p keld-interpreter --test cleanup
```

Expected: invalid `try_remove` is non-faulting and unchanged; clear order is exact; capacity is retained.

- [ ] **Step 6: Commit structural List completion**

```powershell
git add crates/keld-ir crates/keld-interpreter
git commit -m "feat: complete List removal and clear"
```

---

### Task 11: Implement Checked Capacity, Growth Retry, and Recoverable Reservation

**Files:**
- Modify: `crates/keld-interpreter/src/list.rs`
- Modify: `crates/keld-interpreter/src/machine.rs`
- Modify: `crates/keld-interpreter/src/fault.rs`
- Modify: `crates/keld-interpreter/src/lib.rs`
- Modify: `crates/keld-ir/src/instruction.rs`
- Modify: `crates/keld-ir/src/lower.rs`
- Modify: `crates/keld-ir/src/validate.rs`
- Modify: `crates/keld-ir/src/dump.rs`
- Modify: `crates/keld-interpreter/tests/list_surface.rs`
- Modify: `crates/keld-cli/src/render.rs`
- Modify: `crates/keld-cli/tests/cli.rs`

**Interfaces:**
- Produces: checked `reserve`, non-faulting `try_reserve`, and automatic `push` growth with deterministic test injection.

- [ ] **Step 1: Write failing capacity tests**

Add exact cases:

```rust
#[test]
fn reserve_negative_and_unaddressable_sizes_are_capacity_faults() {
    assert_eq!(required_capacity(0, -1), Err(CapacityError::Impossible));
    assert_eq!(required_capacity(0, i64::MAX), Err(CapacityError::Impossible));
}

#[test]
fn try_reserve_failure_returns_false_and_preserves_list() {
    let mut list = RuntimeList::from_values(vec![Value::Int(7)]);
    let mut allocations = AllocationController::fail_list_attempts([1, 2]);
    assert!(!list.try_reserve(8, &mut allocations));
    assert_eq!(list.values_for_test(), &[Value::Int(7)]);
}

#[test]
fn preferred_growth_failure_retries_minimum_capacity() {
    let mut list = RuntimeList::new();
    let mut allocations = AllocationController::fail_list_attempts([1]);
    list.reserve(1, &mut allocations).expect("minimum retry succeeds");
    assert_eq!(allocations.list_attempts(), 2);
    assert!(list.capacity_for_test() >= 1);
}

#[test]
fn successful_try_reserve_guarantees_the_next_n_pushes_do_not_grow() {
    let mut list = RuntimeList::new();
    let mut allocations = AllocationController::default();
    assert!(list.try_reserve(3, &mut allocations));
    let attempts = allocations.list_attempts();
    for value in [1, 2, 3] {
        list.push(Value::Int(value), &mut allocations).expect("reserved push succeeds");
    }
    assert_eq!(allocations.list_attempts(), attempts);
}

#[test]
fn push_reports_allocation_only_after_minimum_retry_fails() {
    let mut list = RuntimeList::new();
    let mut allocations = AllocationController::fail_list_attempts([1, 2]);
    assert_eq!(
        list.push(Value::Int(1), &mut allocations),
        Err(ReserveFailure::Allocation)
    );
    assert_eq!(allocations.list_attempts(), 2);
    assert_eq!(list.length(), 0);
}
```

- [ ] **Step 2: Run and verify RED**

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-interpreter --test list_surface
```

Expected: reserve operations and `Capacity` fault do not exist.

- [ ] **Step 3: Add checked capacity arithmetic**

Implement one shared calculation:

```rust
fn required_capacity(length: usize, additional: i64) -> Result<usize, CapacityError> {
    let additional = usize::try_from(additional).map_err(|_| CapacityError::Impossible)?;
    let required = length.checked_add(additional).ok_or(CapacityError::Impossible)?;
    let _source_length = i64::try_from(required).map_err(|_| CapacityError::Impossible)?;
    let bytes = required
        .checked_mul(std::mem::size_of::<Value>())
        .ok_or(CapacityError::Impossible)?;
    if bytes > isize::MAX as usize {
        return Err(CapacityError::Impossible);
    }
    Ok(required)
}
```

Keep the explicit `_source_length` conversion to enforce the source `Int.MAX` contract before target-address checks.

- [ ] **Step 4: Add preferred/minimum reservation with deterministic injection**

Use an interpreter-owned controller:

```rust
pub struct AllocationController {
    list_attempt: u64,
    fail_list_attempts: BTreeSet<u64>,
}

enum ReserveFailure {
    Capacity,
    Allocation,
}
```

If current capacity is insufficient, try an overflow-checked preferred target such as `max(required, max(4, capacity * 2))`. If that reservation fails, retry `required` exactly unless both targets are equal. `reserve` maps impossible sizes to `CapacityFault` and exhausted allocation attempts to `AllocationFault`; `try_reserve` maps both to `false` with no mutation. Automatic push uses the same routine for one additional element.

- [ ] **Step 5: Add IR and CLI fault support**

Add `ListReserve` and `ListTryReserve` instructions. Add `FaultKind::Capacity` and `RuntimeFaultKind::Capacity`, and render it through the existing runtime-fault path with exit status two.

- [ ] **Step 6: Run focused capacity and CLI tests**

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-interpreter --test list_surface
cargo +stable-x86_64-pc-windows-gnu test -p keld-cli --test cli
```

Expected: impossible sizes are deterministic capacity failures; injection proves minimum retry; recoverable reservation never mutates on failure.

- [ ] **Step 7: Commit capacity semantics**

```powershell
git add crates/keld-ir crates/keld-interpreter crates/keld-cli
git commit -m "feat: implement checked List capacity"
```

---

### Task 12: Pass the Nested Storage Matrix and Close the Milestone

**Files:**
- Modify: `crates/keld-storage/tests/home_verifier.rs`
- Modify: `crates/keld-storage/tests/list_indexing.rs`
- Modify: `crates/keld-storage/tests/list_reservations.rs`
- Modify: `crates/keld-interpreter/tests/storage.rs`
- Modify: `crates/keld-interpreter/tests/cleanup.rs`
- Modify: `crates/keld-interpreter/tests/list_surface.rs`
- Modify: `crates/keld-cli/tests/cli.rs`
- Modify: `docs/compiler-architecture.md`

**Interfaces:**
- Produces: the complete interpreter-era storage milestone evidence and an architecture document matching the implemented pipeline.

- [ ] **Step 1: Add the normative nested matrix**

Create compile-pass, compile-fail, and interpreter cases for:

- `List[List[Int]]` transfer, `.copy()`, index loan, indexed replacement, `remove`, `try_remove`, `clear`, bounds, reserve, and allocation retry;
- `List[List[Text]]` transfer, structural copy independence, borrowed indexed Text use, nested removal ownership, and iterative cleanup;
- whole-aggregate transfer and rejected partial moves from struct/entity/List elements;
- owned return into binding, consuming call, List push, field assignment, and another return;
- two read loans, incompatible edit/structural/take loans, projected pending reservations, and identity-aware entity-field overlap;
- failed field or indexed replacement evaluation preserving the old value;
- inline and heap Text cleanup under the same Home rules; and
- `Map`, `Set`, `Slice`, iterator, `for`, substring, and Text integer indexing remaining unavailable.

- [ ] **Step 2: Run the matrix and fix only demonstrated defects**

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p keld-storage --all-targets
cargo +stable-x86_64-pc-windows-gnu test -p keld-interpreter --all-targets
cargo +stable-x86_64-pc-windows-gnu test -p keld-cli --all-targets
```

Expected: every matrix case passes. If a test exposes a genuine semantic contradiction, stop before editing the normative rule and present the smallest counterexample.

- [ ] **Step 3: Update compiler architecture after behavior is green**

Document these exact ownership boundaries:

```text
VerifiedStorageModule
  = verified lifecycle flow
  + call effects
  + Home/value classifications
  + MaybeLive flags
  + per-operation transfer/drop actions
  + per-exit cleanup actions

keld-ir
  = explicit move/loan/install/drop
  + explicit checked List operations
  + no unresolved cleanup or alias decision
```

State that uniform cleanup paths use direct drops and only divergent successful-initialization orders use a hidden per-scope tracker.
Extend the diagnostic ownership table so `KLD9006` explicitly owns invalid executable storage/container IR; retain `KLD9001` through `KLD9005` for their existing view, register, CFG, lifecycle, and module-shape families.

- [ ] **Step 4: Run the full release gate**

```powershell
cargo +stable-x86_64-pc-windows-gnu test --workspace --all-targets --no-fail-fast
cargo +stable-x86_64-pc-windows-gnu clippy --workspace --all-targets -- -D warnings
cargo +stable-x86_64-pc-windows-gnu fmt --all -- --check
git diff --check
```

Expected: all commands exit zero. Record exact test counts and any LF-to-CRLF warnings separately from failures.

- [ ] **Step 5: Audit the milestone against both normative specifications**

Read every bullet in `docs/spec/storage-values.md` section 17 and `docs/spec/numeric-safety.md` section 13. For each cleanup/List requirement, name at least one passing test. Explicitly record that native and WebAssembly differential tests remain gated on the LLVM backend and therefore the full cross-engine clause is not claimed by this interpreter milestone.

- [ ] **Step 6: Commit the verified milestone evidence**

```powershell
git add crates/keld-storage/tests crates/keld-interpreter/tests crates/keld-cli/tests docs/compiler-architecture.md
git commit -m "test: verify managed cleanup and complete List semantics"
```

---

## Completion Criteria

The milestone is complete only when all conditions below are proven by fresh command output:

- every managed local, consuming parameter, owned temporary, aggregate field, Optional payload, and List element has exactly one cleanup home;
- moved, uninitialized, and absent values are never destroyed;
- normal scope exit and return clean live homes in reverse successful-initialization order;
- `MaybeLive` uses conditional metadata only where the CFG requires it;
- non-uniform initialization order uses a tracker only in affected scopes;
- replacement installs the new value before displaced cleanup, and entity views close before displaced cleanup;
- read loans do not copy or allocate, structural loans mutate the exact source place, and loans never become cleanup homes;
- indexing is checked, single-home elements remain borrowed, and indexed replacement enforces `KLD2007` stability;
- `remove`, `try_remove`, and `clear` preserve capacity and ownership rules;
- `reserve`, `try_reserve`, and automatic growth distinguish capacity from allocation failure and perform the minimum-capacity retry;
- nested `List[List[Int]]` and `List[List[Text]]` pass transfer, copy, loan, removal, replacement, fault, and cleanup cases;
- deferred containers and views remain unavailable; and
- workspace tests, strict Clippy, formatting, and diff checks pass.

LLVM/native/WebAssembly work starts only after these interpreter and static-verification criteria pass. Cross-engine differential completion remains a later backend gate, not a claim of this plan.
