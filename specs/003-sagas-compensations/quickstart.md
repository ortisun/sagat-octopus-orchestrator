# Validation quickstart: Native sagas and compensations

Everything below was run against this repository. The pieces that are **not** built are listed at the end.

## Prerequisites

- The Rust toolchain pinned by `rust-toolchain.toml` and Docker (the suites start throw-away PostgreSQL 16 containers through Testcontainers; no port is published on the host).
- Node 22 (`npx`) only for the contract linters in CI.

## Validation flow

1. **The user stories, the contracts and the failures**, against real PostgreSQL, a fake OIDC provider, real sockets and a real HTTP provider:

   ```sh
   cargo test -p sagat-tests --test e2e_sagas_compensations            # US01–US04 over REST and gRPC, with the engine as a process
   cargo test -p sagat-tests --test integration_sagas_compensations    # plans, compensations, lookups, decisions, stopping, children, the breaker
   cargo test -p sagat-tests --test contract_rest_sagas_compensations  # the saga operations and statuses against the OpenAPI and AsyncAPI documents
   cargo test -p sagat-tests --test contract_grpc_sagas_compensations  # the worker protocol: task kinds, targets, a protocol-1.0 worker
   cargo test -p sagat-tests --test fault_sagas_compensations          # crashes, duplicates, races, a provider that accepts before it times out, connections killed
   cargo test -p sagat-domain --test sagas             # the coordinator as a pure function, 60 scenarios
   cargo test -p sagat-domain -p sagat-application -p sagat-infra-ir -p sagat-infra-postgres -p sagat-api-grpc -p sagat-execution-engine
   ```

2. **The example.** `examples/payment-saga.workflow.json` is the reference saga (reserve the limit, submit the payment with a lookup, wait for the confirmation) and is what the end-to-end stories publish.

3. **Failure injection** (`tests/fault/sagas-compensations/`). Each test states an invariant that must hold whatever was cut off:

   | Class | Test | What it does |
   |---|---|---|
   | Crash at a critical point | `crashes::a_failure_that_is_never_committed_does_not_start_a_compensation` | The failure, the abort and the plan are applied inside a transaction that is abandoned (the process dies before COMMIT): nothing is written; the worker's repeated report then starts the compensation. |
   | Restart | `crashes::a_restart_between_the_plan_and_the_lease_loses_nothing`, `a_run_waiting_for_a_person_is_still_waiting_after_every_process_restarted` | A second process with nothing in memory leases the compensation, and decides for a run that waits for a person. |
   | Effect before checkpoint | `crashes::a_crash_after_the_provider_released_and_before_the_report_releases_once` | The provider released the limit and the worker died: the retry carries the same key and the provider performs it once. |
   | Duplicates | `crashes::duplicate_reports_of_a_lookup_and_of_a_compensation_commit_once` | Each is acknowledged the second time as already applied; one event. |
   | Race, lease against answer | `races::a_lease_that_runs_out_while_the_answer_arrives_is_decided_once` | Eight rounds: an answer and an expiry never both take effect, and one always does. |
   | Race, schedulers | `races::schedulers_that_notice_the_same_lost_lease_at_once_request_one_reconciliation` | Eight schedulers over six lost leases: one lookup each. |
   | Race, timeout against signal | `races::a_signal_and_a_timeout_arriving_together_decide_the_saga_exactly_one_way` | Twelve rounds: exactly one of `wait.satisfied` / `wait.timed_out`; the saga goes on or compensates, and replays. |
   | Provider accepts before it times out | `provider::a_provider_that_accepts_before_it_times_out_is_asked_instead_of_paid_again` | A real HTTP provider records the payment and answers too late: the platform looks it up; one payment, one request. |
   | Provider failed before accepting | `provider::a_provider_that_failed_before_accepting_is_asked_and_then_paid_with_the_same_key` | The lookup says not found; the retry uses the same key; one payment, two requests. |
   | Duplicate return | `provider::a_release_delivered_twice_is_performed_once_and_the_duplicate_is_said_so` | The provider answers `duplicate: true` to the second release with the same key. |
   | Lost broker delivery | `integration_sagas_compensations`'s hints are the same outbox as feature 002 (`outbox.rs`); a held-back compensation is the only new case (G63) | Polling is the claim; hints only wake. |
   | Connections killed | `chaos::connection_kills_while_sagas_succeed_and_compensate_lose_nothing_and_duplicate_nothing` | Twenty runs, half declined, four workers retrying every call while every other backend is terminated for three seconds: every run ends in the state its history demands, every journal replays, every effect has a key and happened once. |

4. **The benchmark** (cost of planning, compensations per second, waiting costs nothing, the claim path with the breaker, a lookup):

   ```sh
   SAGAT_PERF_REPORT=tests/performance/reports/sagas-compensations.md \
     cargo test -p sagat-tests --test perf_sagas_compensations --release -- --ignored --nocapture
   ```

5. **Operating it:** `docs/runbooks/sagas.md`.

## Story evidence

| Story | Acceptance scenario | Evidence |
|---|---|---|
| US01 Define compensation on publish | A reserve action with compensate release; the author publishes a Saga; the plan records dependencies and durable compensation IDs | `us01::us01_publishing_a_saga_records_its_plan_with_durable_compensation_ids` (REST), `us01::us01_the_plan_the_workers_follow_is_the_plan_that_was_recorded` (the key a worker sees is built from the recorded id), `plan::the_stored_plan_is_exactly_what_the_definition_derives`, `plan::a_stored_plan_can_be_neither_changed_nor_removed_by_anyone`, `plan::a_plan_cannot_point_into_another_tenants_version`, `us01::us01_a_compensation_that_cannot_work_is_refused_when_the_author_publishes`, `us01::us01_another_tenant_cannot_read_a_plan...` |
| US02 Compensate after permanent failure | Reserve completed and submit failed permanently: release executes idempotently and the run indicates COMPENSATED | `us02::us02_a_declined_payment_releases_the_reserved_limit_and_the_run_is_compensated` (including the duplicate report acknowledged once), `us02_a_worker_that_dies_while_releasing_is_replaced...`, `us02_cancelling_a_payment_in_flight...`; domain: `reserve_completed_and_submit_failed_for_good_compensates_the_reservation`, `compensations_run_in_reverse_order...`, `independent_branches_are_compensated_at_the_same_time` |
| US03 Reconcile an uncertain effect | The provider accepted the payment but the response was lost; when the engine restarts a reconciliation step queries the outcome before retrying or compensating | `us03::us03_after_an_engine_restart_the_lost_payment_is_looked_up_before_it_is_retried` (the engine process is stopped with the lease held, restarted, the lookup is handed out, the payment is submitted exactly once and the run goes on), the other verdicts in `us03`, `reconcile::*`, `provider::*` |
| US04 Resolve a compensation failure | Compensation failed after retries; the operator reviews and intervenes; state and reason are audited without erasing previous events | `us04::us04_an_operator_asks_for_another_attempt_and_the_run_finishes_compensated` (the journal before is a prefix of the journal after), `us04_giving_up_on_a_release_is_an_administrators_decision...` (audit row with actor and reason), `review::*` (idempotent replay, racing operators, tenant isolation, permissions) |

## Requirement evidence

| Requirement | Evidence |
|---|---|
| FR-003-001 saga boundary, forward, compensation, reconcile and cancellation policy in the IR | `crates/infra-ir/tests/codec.rs`, `crates/domain/tests/{workflow,sagas}.rs`, the workflow schema, `examples/payment-saga.workflow.json` |
| FR-003-002 the plan as a reverse DAG, respecting branch independence | `branches_of_a_parallel_node_are_independent_in_the_plan`, `what_ran_after_a_join_is_undone_before_the_branches_that_fed_it`, `the_plan_is_the_same_as_the_one_worked_out_the_long_way`; stored per version (`plan::*`) |
| FR-003-003 compensation with its own retries, idempotency, limits, timeout and circuit breaker | `RetryPolicy::COMPENSATION`, `compensationRetry` / `compensationTimeoutMs`, `compensation::a_compensation_that_fails_is_retried_with_its_own_budget`, the keys (`us02`, `crashes`), the breaker (`breaker::*`, 15 tests, plus 4 unit tests of its state machine, mutation-checked) |
| FR-003-004 COMPENSATED, PARTIAL, COMPENSATION_FAILED, MANUAL_REVIEW and irreversible | the statuses in the OpenAPI and a test that every status the engine can produce is documented; `an_irreversible_effect_makes_the_outcome_partial...`, `skipping_a_compensation_ends_the_saga_partial`, `with_the_fail_policy...`, `a_sibling_saga_whose_compensation_failed_is_not_hidden...` |
| FR-003-005 pause, retry compensation and skip, skip only with permission and an audited justification | `review::*`, `us04`, `contract_rest_sagas_compensations`; pause is feature 002's, and `cancelling_a_paused_run_still_undoes_what_it_did` |
| FR-003-006 cancellation and deadline reach sub-workflows under the configured policy | `children::*` (request_cancel, propagate, detach, a deadline, a child that compensates, a child that fails inside a parent saga) |
| FR-003-007 reconcile or mark manual for an ambiguous high-risk result | `reconcile::*`, `a_high_risk_step_without_a_lookup_waits_for_an_operator_instead_of_retrying`, `asking_again_for_an_effect_that_may_have_happened_is_an_administrators_decision` |
| NFR-003-SEC | tenant on every statement of every new table and the claim predicate; composite foreign keys; `review::another_tenant_cannot_decide...`, `us01::us01_another_tenant_cannot_read_a_plan...`, `breaker::the_breaker_belongs_to_one_tenant`; `saga:override` only for administrators; the security review (D71) |
| NFR-003-DUR | `fault_sagas_compensations` (13 tests) and the domain property tests, extended with sagas |
| NFR-003-OBS | the counters of D69 (`telemetry::...`), the audit records of D67, the journal events |
| NFR-003-COMP | `worker.baseline.json` gained 15 lines and lost none; a message declared as protocol 1.0 decodes every lease; the migration is additive and reversible |
| NFR-003-PERF | `tests/performance/reports/sagas-compensations.md` |

| Success criterion | State |
|---|---|
| SC-003-001 E2E reserve→submit fails→release; ambiguous provider requires reconciliation; release failure triggers manual review | Met: `us02`, `us03`, `us04` against the engine as a process and real PostgreSQL |
| SC-003-002 100% of US01..US04 pass in a reproducible automated suite | Met for the scenarios of the specification (`e2e_sagas_compensations`, 20 tests) |
| SC-003-003 no violation of the Constitution gates and no silent loss of persisted state | Partial: the tests above prove no confirmed loss in their scope; the v1.0.0 Constitution was ratified on 2026-10-11; only principle II has a dedicated automated architecture gate |
| SC-003-004 critical errors expose `code`, `trace_id` and remediation without leaking payload or secrets | Partial, as in 009: `code`, `traceId` and a stable message are proven (`us04_a_decision_that_makes_no_sense_is_refused_precisely`); no separate `remediation` field exists |

## The review (T003-034)

Two reviewers who had no part in writing it read the uncommitted change, one for security and tenant isolation and one for correctness and crash safety. They found sixteen defects (listed in D71 of `research.md`); each was reproduced by a test that failed before the fix, fixed, and the fix mutation-checked (about eighty mutations of the guards of this feature were tried, and every one that survived got a test or was shown equivalent: the only equivalent mutant is the operator-wait check in the reducer arm of `StepSkipped`, which a compensation cannot reach in any other state). Accepted risks are G62–G65.

## What is not built

The list and accepted tradeoffs remain in G51–G65 of [`research.md`](./research.md). Cross-feature gaps G58–G60 are now closed by features 006–008. Still open here: explicit proof of a script artifact used as a compensation target (G61), and the accepted circuit-breaker and migration limits G62–G65.
