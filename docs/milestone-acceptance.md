# Bootstrap Milestone Acceptance

The first milestone is complete only when all 16 criteria below pass in both
debug and release workspace tests. A compiler build alone is insufficient.

## Source-level acceptance cases

The black-box table is executed by
`crates/keld-cli/tests/milestone_acceptance.rs::criterion_10_library_and_cli_observations_match`.
Every accepted program runs first through the `run_source` library API and then
through the `keld` binary; both must produce the same decimal `Int`.

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

## Static proof cases

| Criterion | Proof |
|---:|---|
| 1 | `milestone_acceptance::criterion_01_cyclic_graph_needs_no_ownership_syntax` and `cyclic_graph.keld` |
| 2 | `milestone_acceptance::criterion_02_lifecycle_end_invalidates_member_links`; runtime unit `lifecycle_cleanup::lifecycle_end_stales_links_and_finish_runs_once` |
| 3 | `milestone_acceptance::criterion_03_keep_to_ancestor_survives` and `keep_survives.keld` |
| 4 | `milestone_acceptance::criterion_04_retired_direct_use_is_static_error` and `fail_retired_use.keld` |
| 5 | `milestone_acceptance::criterion_05_stale_link_takes_absence_path` and `stale_link.keld` |
| 6 | `milestone_acceptance::criterion_06_double_retirement_is_rejected`; lifecycle unit `retirement::retiring_a_must_alias_makes_both_names_retired` |
| 7 | `milestone_acceptance::criterion_07_direct_reference_cannot_escape_to_persistent_storage`; lifecycle unit `links_and_escape::persistent_direct_reference_field_requires_a_link` |
| 8 | `milestone_acceptance::criterion_08_cleanup_order_has_an_executable_proof`; runtime unit `lifecycle_cleanup::lifecycle_cleanup_is_reverse_adoption_order` |
| 9 | `milestone_acceptance::criterion_09_runtime_model_and_generation_exhaustion_are_covered`; runtime tests listed below |
| 10 | `milestone_acceptance::criterion_10_library_and_cli_observations_match` runs all ten source cases |
| 11 | `milestone_acceptance::criterion_11_grammar_goldens_cover_the_milestone_syntax`; `keld-syntax/tests/lexer.rs` and `parser_golden.rs` |
| 12 | `milestone_acceptance::criterion_12_may_alias_use_after_retirement_is_rejected` and `fail_may_alias.keld` |
| 13 | `milestone_acceptance::criterion_13_identity_refined_distinct_parameters_are_accepted` and `alias_distinct.keld` |
| 14 | `milestone_acceptance::criterion_14_broad_retirement_allows_link_reacquisition` and `broad_retirement.keld` |
| 15 | `milestone_acceptance::criterion_15_views_close_before_structural_operations`; every passing fixture is validated and scanned independently |
| 16 | `milestone_acceptance::criterion_16_numeric_stages_have_identical_boundaries`; numeric parity tests listed below |

## Runtime model cases

`keld-runtime/tests/model_sequences.rs::store_matches_reference_model_for_generated_sequences`
runs 256 seeds with 1,000 operations per seed. It compares a source-independent
reference model against the real store after allocate, link, resolve, retire,
begin, keep, and end operations. It verifies identities, slot reuse,
generations, exact runtime types, lifecycle membership, stale resolution,
cleanup order, and classified invalid-operation errors after every step.

`keld-runtime/src/store.rs::generation_exhaustion_permanently_retires_the_slot`
uses a bounded test generation counter to prove exhausted slots are never
reused. The integration sequence test exercises ordinary generation advances;
the focused unit reaches exhaustion without billions of iterations.

## Numeric parity cases

Criterion 16 checks the same boundary in constant evaluation and in executable
IR interpretation for overflow, division by zero, `Int.MIN / -1`, and invalid
shifts. It also checks that `Int.MIN % -1` returns zero. The exhaustive shared
operation table is in
`keld-interpreter/tests/numeric.rs::runtime_numeric_edges_match_the_shared_checked_evaluator`;
the normative primitive cases are in `keld-numeric/tests/int_boundaries.rs`.

## Full verification command

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

The run command must write exactly `20` followed by one newline. The dump must
be non-empty. Every command must exit zero.
