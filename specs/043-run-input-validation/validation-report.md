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

## 2026-11-16, final gates after round 2 of the approval review (`feature/043-creators`, head `01db0b91`)

Same machine and settings. Every suite was run on the committed tree, once as the owner and once with
`SAGAT_TEST_AS_RUNTIME_ROLE=1`; no test was retried.

| Target | Owner | `SAGAT_TEST_AS_RUNTIME_ROLE=1` |
|---|---|---|
| `integration_durable_execution` | 192 passed, 0 failed | 192 passed, 0 failed |
| `integration_schedules_bulk_operations` | 756 passed, 0 failed, 2 ignored | 756 passed, 0 failed, 2 ignored |
| `integration_events_webhooks_messaging` | 120 passed, 0 failed | 120 passed, 0 failed |
| `integration_workflow_primitives` | 805 passed, 0 failed, 23 ignored | 805 passed, 0 failed, 23 ignored |
| `integration_payload_classification` | 43 passed, 0 failed, 2 ignored | 43 passed, 0 failed, 2 ignored |
| `integration_platform_foundation` | 106 passed, 0 failed | 106 passed, 0 failed |
| `fault_events_webhooks_messaging` | 12 passed, 0 failed | 12 passed, 0 failed |
| `fault_workflow_primitives` | 93 passed, 0 failed | 93 passed, 0 failed |
| `fault_schedules_bulk_operations` | 131 passed, 0 failed | 131 passed, 0 failed |
| `fault_payload_classification` | 8 passed, 0 failed, 1 ignored | 8 passed, 0 failed, 1 ignored |
| `fault_platform_foundation` | 38 passed, 0 failed | 38 passed, 0 failed |

The 23 contract targets: 366 passed, 0 failed. `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked --
-D warnings`, the three python checks and their unit tests (OK, 2 skipped) passed; the Studio types match the OpenAPI.
Per-SDK checks: TypeScript, Python, Go, Java and Rust (151 passed) passed. In C#, `dotnet test` failed once, 531 of 532,
on `ACallThatTakesLongerThanTheTimeoutIsATransportErrorAndAPostWithAKeyIsRepeated` ("timed out waiting for two
attempts"), while two integration suites ran beside it; the test gives its client 100 ms for each attempt, connection
included, and the fake server records a request only once it has read it, so on a loaded machine both attempts can end
before the server sees either. The test is not touched by design 043; rerun alone three times it passed, and the whole
project passed 532 of 532 with `dotnet format --verify-no-changes` clean. It is recorded as a load-sensitive test of the
C# SDK, not fixed here.

The canary failure of the earlier run (`unexpected response from SSLRequest: 0x48`) is explained by D043-171: Docker
Desktop publishes a container's port on every address with `SO_REUSEADDR`, so it can share the number of a
`127.0.0.1` listener a test already holds, and `127.0.0.1` then reaches the test's HTTP server, not PostgreSQL. The
fixture now tells that answer apart and starts another container (`a_port_an_http_server_answers_is_told_apart_and_not_waited_on`).

Conformance in the six languages, every scenario, no bless (`SAGAT_RUST_SDK_TARGET_DIR` set): the first run stopped at
`database-connections` because the root `CACHEDIR.TAG` of the private target directory was missing (the support code finds
the target by it; restored); the second stopped at the same scenario on `unexpected response from SSLRequest: 0x48` from
its target PostgreSQL, the D043-171 collision in another fixture, fixed by D043-176; the third, on the fixed tree, passed:
78 scenarios in six languages, 1 test passed in 1,752 s.

## 2026-11-16, the gates of the approval (`feature/043-creators`)

**Commits.** The suites below ran on `4b6eb1a649b281660c4622e20d0b348322a7d8a4`, the commit the four approval reports
read. One unit test failed there (below); its fix is `f48d5729eb534dc25c6cb04eeeabb0dc80202bad`, whose diff from
`4b6eb1a6` is one entry of the CLI's help catalog (`apps/cli/src/diagnostics.rs`), its section of
`docs/quickstarts/errors.md`, and records (review reports, the runbook, `research.md`); the architecture reviewer read
that diff (`reviews/2026-11-16/round-2/post-review-cli-architecture.md`, no Blocker or Major). The unit tests, the
developer-experience contract, the conformance run, the per-SDK checks and the static gates ran again on `f48d5729`.

**Environment.** Apple M3 Max, macOS 26.3, rustc 1.100.0, Docker Desktop (Testcontainers starts its own PostgreSQL),
`RUST_TEST_THREADS=2`, `CARGO_PROFILE_DEV_DEBUG=0`, a private `CARGO_TARGET_DIR` outside the repository; no test was
retried in place.

| Target (at `4b6eb1a6`) | Owner | `SAGAT_TEST_AS_RUNTIME_ROLE=1` |
|---|---|---|
| `integration_gitops_versioning` | 41 passed, 0 failed | 41 passed, 0 failed |
| `integration_database_connections` | 7 passed, 0 failed | 7 passed, 0 failed |
| `integration_run_input_cache` | 1 passed, 0 failed | 1 passed, 0 failed |
| `integration_claim_transition_efficiency` | 17 passed, 0 failed | 17 passed, 0 failed |
| `integration_durable_execution` | 192 passed, 0 failed | 192 passed, 0 failed |
| `integration_schedules_bulk_operations` | 757 passed, 0 failed, 2 ignored | 757 passed, 0 failed, 2 ignored |
| `integration_events_webhooks_messaging` | 120 passed, 0 failed | 120 passed, 0 failed |
| `integration_workflow_primitives` | 805 passed, 0 failed, 23 ignored | 805 passed, 0 failed, 23 ignored |
| `integration_payload_classification` | 43 passed, 0 failed, 2 ignored | 43 passed, 0 failed, 2 ignored |
| `integration_platform_foundation` | 107 passed, 0 failed | 107 passed, 0 failed |
| `fault_events_webhooks_messaging` | 12 passed, 0 failed | 12 passed, 0 failed |
| `fault_workflow_primitives` | 93 passed, 0 failed | 93 passed, 0 failed |
| `fault_schedules_bulk_operations` | 131 passed, 0 failed | 131 passed, 0 failed |
| `fault_payload_classification` | 8 passed, 0 failed, 1 ignored | 8 passed, 0 failed, 1 ignored |
| `fault_platform_foundation` | 38 passed, 0 failed | 38 passed, 0 failed |

Named in both roles: `the_inventory_runs_on_a_database_of_0_2_0_before_the_migration`,
`a_version_a_live_schedule_pins_is_never_superseded`,
`a_superseded_version_nothing_starts_does_not_fail_the_inventory_and_all_lists_it`,
`the_inventory_lists_the_versions_that_would_be_refused_and_exits_by_what_it_found` (each inventory run as the runtime
role in both), `a_buffered_action_released_and_started_is_counted_once`. The 23 contract targets: 366 passed, 0 failed.

The unit tests of `sagat-domain`, `sagat-application`, `sagat-infra-schema`, `sagat-infra-ir`, `sagat-infra-postgres`,
`sagat-api-http`, `sagat-scheduler`, `sagat-control-plane`, `sagat-execution-engine` and `sagat-cli`: at `4b6eb1a6`,
2,337 passed and 1 failed, `every_code_the_platform_returns_has_help_and_the_help_names_no_code_the_platform_does_not`
(`apps/cli/tests/diagnostics.rs`: "codes with no help: [\"VALIDATOR_BUSY\"]"), a defect of D043-183, whose dry run gives
that diagnostic code; fixed in `f48d5729`; at `f48d5729`, 2,338 passed, 0 failed, among them
`run_input::tests::a_refusing_recheck_is_counted`, `the_limits_and_the_charges_of_run_input_validation_v1_are_pinned`,
`run_input::tests::a_precheck_whose_compile_panics_is_interrupted`,
`run_input::tests::the_caller_whose_compile_panics_is_answered_interrupted` and
`inventory::tests::a_version_the_validator_cannot_judge_makes_the_inventory_exit_2_and_is_listed`.
`contract_developer_experience` at `f48d5729`: 8 passed.

Conformance at `f48d5729`, every scenario in the six languages, no bless: 78 scenarios, 1 test passed in 1,743 s; no
golden changed. Two earlier attempts stopped at `database-connections` for the environment, not the code: the private
target directory, made by hand before cargo, had no `CACHEDIR.TAG` (the support code finds the target by it; written),
and the `database-runner` binary was not yet built in that directory (`cargo build -p sagat-database-runner`). The
per-SDK checks of `sdks/AGENTS.md` at `f48d5729`: TypeScript, Python, Go, C# (532 of 532, `dotnet format` clean), Java
and Rust passed; the Studio types match the OpenAPI, and the Studio tests passed (99 files, 1,530 tests). `cargo fmt --check`, `cargo clippy --workspace --all-targets
--locked -- -D warnings`, the three python checks and their unit tests (`python3 -m unittest discover -s scripts/tests`:
OK, 2 skipped) passed at `f48d5729`.

**Seen failing first** (each red run in this session, before its fix): D043-177
`a_precheck_waits_in_the_starts_pool_and_is_busy_after_its_wait` (`Ok(())` where `Err(Busy)` was expected); D043-179 the
superseded inventory test (both old versions counted) and the `--all` CLI case (an unknown argument); D043-180
`a_panic_of_the_validator_is_counted_interrupted_and_no_room_deadline` (three `deadline`); D043-183
`a_dry_run_whose_pre_check_found_no_permit_is_unavailable_not_a_problem` (`Ok` with a problem) and
`a_promotion_whose_parse_finds_no_permit_is_unavailable_not_draft_has_changes` (`DraftHasChanges`); D043-184
`a_buffered_action_released_and_started_is_counted_once` (three `accepted` for two starts); D043-185
`a_version_a_live_schedule_pins_is_never_superseded` (the pinned version superseded); D043-186
`a_precheck_whose_compile_panics_is_interrupted` (the panic unwound); D043-187
`the_inventory_runs_on_a_database_of_0_2_0_before_the_migration` (`relation "schedules" does not exist`, exit 2). Tests
that guard an existing behaviour passed at once, as written: the pin test, the down-migration guard test, the sentinels
of rows 2 and 13, the rewritten `a_refusing_recheck_is_counted` (its expected counts are those of D043-184), and
`a_version_the_validator_cannot_judge_makes_the_inventory_exit_2_and_is_listed`, which tests the exit status and the
printed line built in D043-187.
