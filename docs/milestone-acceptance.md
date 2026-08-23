# Keld milestone acceptance: bootstrap, storage, cleanup, List, and Native-1

This document records completed interpreter milestones and the completed
Native-1 Windows GNU gate. It is an acceptance map, not a promise of future
behavior: every claim below points to a normative specification and an
existing test suite. Deferred targets and language features remain outside
this acceptance.

## Historical bootstrap acceptance

The first bootstrap milestone was complete only when all 16 criteria below
passed in both debug and release workspace tests. A compiler build alone was
insufficient.

### Source-level acceptance cases

The black-box table is executed by
`crates/keld-cli/tests/milestone_acceptance.rs::criterion_10_library_and_cli_observations_match`.
Every accepted program runs first through the `run_source` library API and then
through the `keld` binary; both produce the same decimal `Int`.

| Case | Fixture | Expected result |
|---|---|---|
| cyclic graph | `cyclic_graph.keld` | `20` |
| ancestor keep | `keep_survives.keld` | `30` |
| stale link | `stale_link.keld` | `4` |
| retired use | `fail_retired_use.keld` | static `KLD1001` |
| may alias | `fail_may_alias.keld` | static `KLD1008` |
| unsupported feature | `fail_unsupported.keld` | static `KLD0004` |
| identity-refined aliases | `alias_distinct.keld` | `20` |
| broad retirement | `broad_retirement.keld` | `40` |
| numeric edge | `numeric_edges.keld` | `0` |
| numeric runtime fault | `runtime_div_zero.keld` | `DivisionByZeroFault`, exit 2 |

All paths are relative to `crates/keld-cli/tests/fixtures/`.

### Static proof cases

| Criterion | Existing proof |
|---:|---|
| 1 | `criterion_01_cyclic_graph_needs_no_ownership_syntax` and `cyclic_graph.keld` |
| 2 | `criterion_02_lifecycle_end_invalidates_member_links`; runtime `lifecycle_cleanup::lifecycle_end_stales_links_and_finish_runs_once` |
| 3 | `criterion_03_keep_to_ancestor_survives` and `keep_survives.keld` |
| 4 | `criterion_04_retired_direct_use_is_static_error` and `fail_retired_use.keld` |
| 5 | `criterion_05_stale_link_takes_absence_path` and `stale_link.keld` |
| 6 | `criterion_06_double_retirement_is_rejected`; lifecycle `retirement::retiring_a_must_alias_makes_both_names_retired` |
| 7 | `criterion_07_direct_reference_cannot_escape_to_persistent_storage`; lifecycle `links_and_escape::persistent_direct_reference_field_requires_a_link` |
| 8 | `criterion_08_cleanup_order_has_an_executable_proof`; runtime `lifecycle_cleanup::lifecycle_cleanup_is_reverse_adoption_order` |
| 9 | `criterion_09_runtime_model_and_generation_exhaustion_are_covered`; runtime model and generation tests below |
| 10 | `criterion_10_library_and_cli_observations_match` runs all ten source cases |
| 11 | `criterion_11_grammar_goldens_cover_the_milestone_syntax`; lexer/parser goldens |
| 12 | `criterion_12_may_alias_use_after_retirement_is_rejected` and `fail_may_alias.keld` |
| 13 | `criterion_13_identity_refined_distinct_parameters_are_accepted` and `alias_distinct.keld` |
| 14 | `criterion_14_broad_retirement_allows_link_reacquisition` and `broad_retirement.keld` |
| 15 | `criterion_15_views_close_before_structural_operations`; independently scanned validated fixtures |
| 16 | `criterion_16_numeric_stages_have_identical_boundaries`; numeric parity tests below |

### Runtime and numeric evidence

`keld-runtime/tests/model_sequences.rs::store_matches_reference_model_for_generated_sequences`
runs 256 seeds with 1,000 operations per seed and compares identities, slot
reuse, generations, exact runtime types, lifecycle membership, stale
resolution, cleanup order, and classified invalid-operation errors against a
source-independent model. The generation-exhaustion unit proves exhausted
slots are never reused.

Criterion 16 compares constant evaluation and executable-IR interpretation for
overflow, division by zero, `Int.MIN / -1`, invalid shifts, and
`Int.MIN % -1 == 0`. The exhaustive operation table is
`keld-interpreter/tests/numeric.rs::runtime_numeric_edges_match_the_shared_checked_evaluator`;
the normative cases are in [`spec/numeric-safety.md`](spec/numeric-safety.md).

## Completed storage and value acceptance

The storage milestone implements the contracts in
[`spec/storage-values.md`](spec/storage-values.md), §§1–10 and §14. The
corresponding executable evidence is distributed across
`keld-semantics/tests/types.rs`, `keld-flow/tests/storage_scopes.rs`,
`keld-storage/tests/loan_reservations.rs`,
`keld-lifecycle/tests/{provenance_facts,links_and_escape,retirement}.rs`,
`keld-ir/tests/validation.rs`, and the interpreter integration suites.

Those tests cover `let`/`var` state, `take`, `.copy()`, normal and consuming
parameters, owned returns, Optional values, nested managed aggregates, loan
provenance, projected places, structural-view barriers, entity/link identity,
and validation of every explicit move/loan/install/drop operation. The
interpreter remains the semantic oracle and executes only validated IR.

## Completed cleanup and List acceptance

The List and cleanup milestone implements [`spec/storage-values.md`](spec/storage-values.md),
§§11–13 and §15–§17. Focused tests cover direct and projected construction,
length, push, remove, index loans, copy-get, replace, try-remove, clear,
reserve, try-reserve, nested Lists/Text, capacity preservation, preferred then
minimum growth, replacement-before-displaced-cleanup, and faulting bounds or
capacity paths. `keld-interpreter/tests/projected_allocation.rs` proves the
allocation-free projected `try_remove` path, while structural-view coverage
rejects both projected List mutation instructions.

Cleanup evidence covers reverse successful-initialization order, `MaybeLive`
conditional homes, divergent CFG joins, partial initialization, entity cleanup,
and final context teardown. Runtime model and allocator tests distinguish
language allocation faults from internal store invariant failures. The
historical full gate recorded 256 reference-model seeds and the GNU workspace,
strict Clippy, formatting, and diff checks as passing; Native-1 differential
execution is recorded below.

## Completed Control Flow-1 acceptance

Control Flow-1 implements the normative rules in
[`spec/control-flow.md`](spec/control-flow.md) without changing the historical
16 bootstrap criteria above. `while`, `break`, and `continue` lower through the
existing structured CFG and cleanup machinery; there is no implicit loop
lifecycle and no executable-IR or native loop opcode.

| Requirement | Executable evidence |
|---|---|
| ordinary structured loop result | `control_flow_loop.keld` returns `8` through the library, CLI interpreter, Native O0, and Native O2 paths |
| repeated managed allocation | `control_flow_allocations.keld` returns `6`; the repeated concat site keeps one site ID with attempts `1,2,3` |
| loop-control diagnostics | semantics/CLI acceptance rejects outside-loop `break` and `continue` with `KLD0112` |
| cyclic Home fixed point | `keld-storage/tests/control_flow.rs` covers `MaybeLive`, repair, zero iteration, and body-home re-entry |
| structured managed cleanup | `keld-interpreter/tests/cleanup.rs` covers body `Text` cleanup on `continue`, `List[Text]` cleanup on `break`, and condition-temporary cleanup |
| lifecycle exits | `keld-flow/tests/{control_flow,storage_scopes}.rs` proves `ExitScopes` for return and iteration lifecycle exits |
| no implicit lifecycle / ancestor keep | `keld-interpreter/tests/control_flow.rs` proves plain-loop entities remain in the enclosing lifecycle and kept entities survive an inner lifecycle `break` |
| lifecycle/provenance fixed point | `keld-lifecycle/tests/control_flow.rs` covers path retirement, loop-carried dynamic identity, and retirement call effects in conditions |
| condition boundary | interpreter/lifecycle regressions cover checked faults, call effects, and managed full-expression cleanup before body or loop exit |
| cyclic executable IR | `keld-ir` cyclic validation plus CLI milestone acceptance require a validated CFG backedge with no loop opcode |
| native parity and repeated condition allocation | `keld-native-backend/tests/differential.rs` compares interpreter with LLVM O0/O2, including one static condition-concat site with attempts `1,2,3` |

The Native-1 executable surface remains **48 instruction variants and 6
terminators**. Control Flow-1 expands which CFG shapes are accepted, not the
instruction/terminator enum surface. The `surface_audit` suite remains the
exhaustive executable-surface guard.

## Verification command for this historical milestone

From the repository root in PowerShell:

```powershell
$env:CARGO_BUILD_TARGET='x86_64-pc-windows-gnu'
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --workspace --release
cargo run --release -p keld-cli -- run --engine interpreter crates/keld-cli/tests/fixtures/cyclic_graph.keld
cargo run --release -p keld-cli -- dump-ir crates/keld-cli/tests/fixtures/keep_survives.keld
```

The run command writes exactly `20` followed by one newline; the dump is
non-empty; every command exits zero.

## Native-1 status

The approved `x86_64-w64-windows-gnu` LLVM Native-1 design is recorded in
[`superpowers/specs/2026-08-14-keld-llvm-native-1-design.md`](superpowers/specs/2026-08-14-keld-llvm-native-1-design.md)
and its task plan in
[`superpowers/plans/2026-08-14-keld-llvm-native-1.md`](superpowers/plans/2026-08-14-keld-llvm-native-1.md).
The Native-1 implementation is executable on the frozen Windows GNU toolchain.
The evidence is split by gate:

| Gate | Evidence in this checkout | Status |
|---|---|---|
| LLVM/ABI foundation | `keld-native-toolchain` tests, ABI layout tests, and `scripts/bootstrap-llvm.ps1` | complete |
| Scalar CFG, calls, Phi, Text, direct List, structs, and entity smoke parity | `crates/keld-native-backend/tests/native_int.rs` at O0 and O2 | complete for the covered fixtures |
| CLI build/native run staging | `crates/keld-cli/tests/cli.rs::native_engine_matches_representative_source_fixtures`, `native_engine_forwards_runtime_faults_with_interpreter_format`, and `native_build_handles_unicode_paths_and_refuses_collisions`, plus the GNU smoke commands in README | complete for covered fixtures |
| Projected places and PE import audit | `crates/keld-native-backend/tests/native_int.rs::projected_list_receiver_runs_natively_at_both_optimization_levels` and the import assertions in `builds_and_runs_a_const_int_program` | complete for covered fixtures |
| Test-only allocation-control DLL and version-1 observation schema | `keld-native-ffi-test`, `crates/keld-native-abi/tests/allocation_sites.rs`, `crates/keld-ir/tests/allocation_schedule.rs`, and the `KELD_TEST_CONTROL`/`KELD_TEST_OBSERVATION` contract | complete; deterministic phase/ordinal IDs and context/store, Text, copy, struct/entity, and List-growth controls are covered |
| Full cross-engine allocation-failure schedules | `crates/keld-native-backend/tests/differential.rs::{every_source_fixture_has_a_shared_allocation_failure_schedule_at_o0_and_o2,source_surface_fixtures_extend_the_same_differential_schedule,ir_only_surface_fixtures_extend_the_same_differential_schedule,allocation_schedule_covers_preferred_and_exact_list_growth_failures}` | complete at O0 and O2; interpreter and native observations, faults, spans, and event sequences match |
| Exhaustive instruction/terminator source/IR differential surface | `crates/keld-native-backend/tests/differential.rs::every_executable_ir_variant_is_in_a_real_differential_fixture` and `surface_audit.rs` (48 instructions, 6 terminators) | complete |

Native-1 acceptance is complete for one-source `x86_64-w64-windows-gnu`
executables: both LLVM O0 and O2 builds use the same validated executable IR,
all current instruction and terminator variants have a real source or
validated-IR differential fixture, every reachable semantic allocation point
has deterministic frozen site IDs and failure coverage, and generated programs
are exercised with the sibling versioned runtime DLL only. Linux, MSVC,
WebAssembly, typed errors, and new language features remain deferred.
