# Validation report: run input validation (design 043)

Dated runs of the checks that are regression checks rather than proofs (D043-060, D043-103): the oracle property test of
the platform's validator against `jsonschema` 0.58.6, and the measurements of the work counter (G043-017, G043-030).
A new run is a new dated section; an old one is never edited.

## 2026-11-15, phase 1 after the review (`feature/043-validator`)

Machine: Apple M3 Max, 16 cores, macOS 26.3, rustc 1.100.0.

### Oracle property test

`oracle::the_platform_validator_agrees_with_jsonschema_over_the_subset` (`crates/infra-schema/tests/run_input.rs`):
768 generated cases, each a schema of the subset with two definitions and one to seven inputs, checked under `$schema`
absent, draft-07 and 2020-12. The generators include the u64/f64 boundary pairs (`18446744073709551615` and
`1.8446744073709552e19`, `9223372036854775809` and `9.223372036854776e18`, `±2^63`) and nodes that hold both `properties`
and `additionalProperties` (D043-103).

| Run | Verdicts compared | Skipped (`workLimit`, `maxDepth`, backstop) | Skip ratio | Disagreements |
|---|---|---|---|---|
| unoptimised test build, `--nocapture` | 9,087 | 0 | 0.000% | 0 |

The test fails above a 5% skip ratio and on the first disagreement. The seed is proptest's own per run, so another
run compares a different sample.

### Work per unit

From `measure_k_and_the_cost_of_a_full_budget` (release, `--ignored`): the dearest keyword unit is `const` at 7.5 ns,
`multipleOf` on a huge float 8.7 ns, `required` 5.8 ns and `properties` 6.2 ns; a full budget of 1,000,000 units costs
at most 7.5 ms. A pattern at the state cap (466 states, the most a pattern reaches before its 16 KiB program) costs
0.0032 ns per (byte × state) in the raw automaton with the prefilter off, and 0.0144 at 225 states of classes. The
details and the k per keyword are in G043-017 and G043-030 of `research.md`.

## 2026-11-16, phase 3 (`feature/043-creators`)

Machine: Apple M3 Max, 16 cores, macOS 26.3, rustc 1.100.0, Docker Desktop. `RUST_TEST_THREADS=2`, a private
`CARGO_TARGET_DIR`. Run on the tree that adds the phase 3 tests and D043-156 (head `67ac0b05`).

| Target | Owner | `SAGAT_TEST_AS_RUNTIME_ROLE=1` |
|---|---|---|
| `integration_workflow_primitives` | 804 passed, 0 failed, 23 ignored | 804 passed, 0 failed, 23 ignored |
| `integration_schedules_bulk_operations` | 752 passed, 0 failed, 2 ignored | 752 passed, 0 failed, 2 ignored |
| `integration_events_webhooks_messaging` | 119 passed, 0 failed | 119 passed, 0 failed |
| `integration_platform_foundation` | 105 passed, 0 failed | 105 passed, 0 failed |
| `integration_durable_execution` | 192 passed, 0 failed | 192 passed, 0 failed |
| `fault_schedules_bulk_operations` | 131 passed, 0 failed | 131 passed, 0 failed |
| `fault_events_webhooks_messaging` | 11 passed, 0 failed | 11 passed, 0 failed |
| `fault_workflow_primitives` | 93 passed, 0 failed | 93 passed, 0 failed |
| `integration_claim_transition_efficiency` | 17 passed, 0 failed | not run |
| `integration_database_connections` | 7 passed, 0 failed | not run |
| `integration_payload_classification` | 42 passed, 1 failed, 2 ignored; the failure rerun alone: 1 passed | not run |

The one failure, `canary::event_and_dead_letter_values_follow_all_role_grant_and_reveal_cases`, never reached its
assertions: its disposable PostgreSQL did not answer, `unexpected response from SSLRequest: 0x48`. An `H` on the port
means an HTTP server answered the port the container was given, so the port was reused between the container's start
and the first connection. Rerun alone in a fresh process, it passed. Not a defect of design 043; recorded as an
environment failure, not retried in place.

The 23 contract targets passed, 0 failed (among them `contract_rest_durable_execution` 2, `contract_payload_classification`
18, `contract_iceberg_writes` 69, `contract_schedules_bulk_operations` 8, `contract_asyncapi_events_webhooks_messaging` 5,
`contract_developer_experience` 8). `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`
and the three python checks passed. The per-SDK checks of `sdks/AGENTS.md` passed in the six languages.

Conformance in the six languages (`SAGAT_RUST_SDK_TARGET_DIR` set to an owned external directory; the first run without it
failed at the Rust runner's `prepare.sh`, which refuses to start without one): `SAGAT_SDK_SCENARIO=run-input-validation`
1 passed (6 languages), `SAGAT_SDK_SCENARIO=collection-operators` 1 passed (6 languages). No golden was blessed.
