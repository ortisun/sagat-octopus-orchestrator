# Validation quickstart: Distributed observability and operations

Everything below was run against this repository. The pieces that are **not** built, and what could not be run where it was written, are listed at the end.

## Prerequisites

- The Rust toolchain pinned by `rust-toolchain.toml` and a PostgreSQL the suites can create databases in: Docker (Testcontainers starts PostgreSQL 16; no port is published on the host) or, with `SAGAT_TEST_DATABASE_URL=postgres://…@host:port/postgres`, a server you name (this is how the figures below were produced: PostgreSQL 18.6 from Homebrew, see "What was and was not run").
- Node 22 for the Studio (`npm ci` in `frontend/`); Playwright's Chromium for the journeys.
- To look at it: `docker compose -f infra/docker/compose.yaml -f infra/docker/compose.observability.yaml --env-file infra/docker/.env up -d --build` starts a Collector, Jaeger (`http://127.0.0.1:16686`), Prometheus (`:19091`) and Grafana (`:13000`, dashboard "Sagat overview"). Point the control plane and the engine at it with `OTEL_EXPORTER_OTLP_ENDPOINT=http://otel-collector:4318`.

## Validation flow

1. **The user stories**, against real PostgreSQL, a fake OIDC provider, a fake OTLP collector and real sockets:

   ```sh
   cargo test -p sagat-tests --test e2e_observability_operations            # US01-US04 and the trace links (26)
   cargo test -p sagat-tests --test fault_observability_operations          # outage, duplicates, races, restarts (18)
   cargo test -p sagat-tests --test integration_observability_operations    # migrations back and forth, tenant isolation, the edges of the projector and the monitor (20)
   cargo test -p sagat-tests --test contract_observability_operations --test contract_rest_observability_operations   # the monitoring files and the new REST operations against the OpenAPI (11)
   cargo test -p sagat-telemetry -p sagat-application -p sagat-domain      # the exporter, redaction, span ids, derive, the operator actions
   ```

2. **The Studio:** `npm test`, `npm run lint`, `npm run typecheck`, `npm run check:api` in `frontend/apps/studio` (the types are generated from the OpenAPI), and the journeys: `cargo test -p sagat-tests --test e2e_workflow_studio -- --ignored journeys`.

3. **The monitoring files:** `promtool check config`, `promtool check rules` and the Collector's `validate` on `infra/observability/` with the images the compose overlay pins (the `observability` job of CI; the cargo contract tests check that what the files say exists: every metric a dashboard draws or a rule evaluates is one the code emits, every alert points to a runbook section, no metric carries a label that grows with the data).

4. **The benchmark** (what observability costs the paths it watches):

   ```sh
   SAGAT_PERF_REPORT=tests/performance/reports/observability-operations.md \
     cargo test -p sagat-tests --test perf_observability_operations --release -- --ignored --nocapture
   ```

5. **Operating it:** `docs/runbooks/observability.md` (telemetry, operator actions, the audit read) and `docs/runbooks/alerts.md` (one section per alert).

## Story evidence

| Story | Acceptance scenario | Evidence |
|---|---|---|
| US01 Inspect timeline and attempt | A run with 3 attempts: each transition has a timestamp, a cause and a trace | `us01::every_transition_of_a_run_with_three_attempts_has_a_time_a_cause_and_a_trace`, `…the_attempts_endpoint_tells_each_attempt_with_its_outcome_its_code_and_its_span`, `…the_state_of_the_telemetry_says_how_far_the_spans_are_behind_the_journal_which_stays_whole` (CURRENT, CATCHING_UP, BEHIND), `…a_run_with_no_trace_and_a_run_that_predates_the_projection_say_so`; the cause is computed from the event (`event_cause.rs`, 6); derive and the attempts (`telemetry_derive.rs`, 17, among them the same spans in any split of the journal and a journal read in batches); the timeline's cause, span and telemetry note (`RunDetailPage.test.tsx`, `journal.test.ts`) |
| US02 Correlate external calls | An HTTP activity with trace propagation: the trace crosses ingress, scheduler, worker and provider | `us02::a_call_to_a_provider_is_one_trace_with_the_request_the_scheduler_the_attempt_and_the_provider` (the fake collector holds the request span, the run, the attempt and the provider's span, one parent chain), `trace_links.rs` (6: a `traceparent` in, the lease carrying the attempt's span, an event that starts a run in its trace, a delivery that continues it) |
| US03 Receive alerts for stuck runs | Outbox age or expired leases above the threshold: an alert with runbook and the right namespace | `us03` (5: an outbox that waits, a lease not taken back, a step nobody serves, who may see alerts and only their own tenant's, the webhook a subscriber is sent conforms to the AsyncAPI); `fault monitor.rs` (6: a failed look resolves nothing, two monitors raise one alert and tell once, a dead monitor is replaced, a namespace with many unserved activities, retention, the gauges carry no tenant); `integration monitor.rs` (5: the bounds, the fairness between tenants, a held step is not unserved) |
| US04 Execute an audited operational action | An operator with retry permission retries with a justification: new attempts appear and the previous ones stay | `us04` (9: retry now, hold and release, rerun, drain/undrain, the actions never cross a tenant, the audit trail, no existence oracle, the biggest page says there is more, the drains are bounded); `fault operators.rs` (4: the scheduler's retry and the operator's, two operators, a drain and a poll, a drain of one namespace); `operator_actions.rs` (10, among them the hold that keeps what is known about the last attempt); the Studio's `ops.test.ts` and `operations.test.tsx` |

## Edge cases of the specification

| Edge case | Where it is handled |
|---|---|
| Trace missing but journal present | The journal is the truth and the timeline reads it (transactional read models: `projectionLagMs` stays 0); the telemetry state says `BEHIND`, `NO_TRACE` or `UNTRACKED` (`us01`); a collector that is down delays the spans and loses none (`fault projection.rs`); a collector that wants other credentials is a wait, not a verdict (`integration projection.rs`) |
| Projection behind the journal | A lag budget (`SAGAT_TELEMETRY_LAG_BUDGET_SECS`, 30), `telemetry_projection_lag_seconds`, an alert, the run's state; the note a writer pays is 4% of a completion and a busy projector slows the median completion 1.09x (`tests/performance/reports/observability-operations.md`) |
| A note that claims more journal than there is | Settled with a warning instead of asked for for ever (`integration projection.rs`) |
| Two projectors at once, a slow collector | The lease is asked for again before each batch and only the call that moved the note counts the measurements (`integration projection.rs`) |
| Operator retry racing the scheduler's | The first commit wins and the second gets `409` (`operator_actions.rs`, `fault operators.rs`) |
| A drain racing a poll | The lease transaction checks again under a lock the drain takes exclusively (`integration isolation.rs`) |
| Secrets and payloads in telemetry | Redaction by field name and by the shape of a credential on logs and span attributes (`redact.rs` 9, `logs.rs` 10, `otlp.rs`), the audit read replaces credential-shaped strings, alert bodies carry counts and ages, no metric label is a tenant, a namespace, a run or an activity (`contract_observability_operations`) |
| Cross-tenant | Every new operation is in the cross-tenant battery of feature 009 (`contract_rest_security_multitenancy`); a denial is withheld from the audit read because its existence is an oracle (`us04`) |

## Reviews of the implementation (T008-034)

Two independent read-only reviews of the whole feature (a security one and a correctness one, by agents that had not written the code) found about twenty points. Each was checked against the code before anything was changed; all that stand were real, and each fixed one has a test that fails without the fix (several were also checked by weakening the guard).

An earlier finding came from the cross-tenant battery of feature 009 when it met the new `GET /audit`: the audit read listed `access.denied.cross_tenant`, which is written only when the id exists in another tenant, so it answered "does this exist elsewhere?" (D176, `us04::the_trail_of_a_tenant_does_not_tell_whether_an_identifier_it_probed_exists_elsewhere`). The integration suite found two more: a rollback of the alerts migration that failed with a subscription that asked only for alerts, and a projector note ahead of the journal that kept the projector busy for ever.

| # | Severity | Finding | Verdict and fix | Evidence |
|---|---|---|---|---|
| 1 | High | `MARK_MANUAL_REVIEW` of a step waiting out a retry after a lost lease overwrote its failure with `MARKED_FOR_REVIEW`: a saga stopped later did not undo an effect that may have happened (D180) | Real. The hold says why the step waits and leaves the failure alone | `operator_actions::holding_a_step_does_not_forget…`, `…a_held_step_whose_attempt_may_have_had_an_effect_is_still_undone…` |
| 2 | Medium | A look of the monitor is bounded and an alert not returned was resolved: an older condition pushed a live alert out (resolved webhook, fired again later); one tenant with enough groups hid everyone else's (D179) | Real. Ranked bounds per namespace, tenant and in all; the look says where it stopped short and reconcile resolves nothing there | `integration monitor.rs` (5; weakened the exclusion and the per-tenant rank to see them fail) |
| 3 | Medium | STEP_UNCLAIMED counted paused runs, forward steps of a run being stopped and compensations behind an open breaker | Real. The conditions of the claim | `monitor::a_step_held_back_on_purpose_is_not_called_unserved` |
| 4 | Medium | Timers and waits that start in one projection batch and end in a later one got no span | Real. Their starts are context events | `telemetry_derive.rs` (2) |
| 5 | Medium | The sampler applied to live spans only; `always_off` with an endpoint still exported every run and attempt | Real. The projector keeps the same share, decided from the trace id | `integration projection.rs::a_trace_the_sampler_does_not_keep…` |
| 6 | Medium | Any answer but a success except 429/502/503/504 was a verdict: a `401` from an expired token dropped the spans for good | Real. Only 400, 413 and 422 are verdicts; server errors are tried again; the rest is a wait | `otlp.rs` (2), `integration projection.rs::a_collector_that_wants_other_credentials…` |
| 7 | Medium | The routes ask for one row more than the page and the services clamped it back: `limit=200` never had a next cursor | Real | `us04::a_page_of_the_biggest_size_says_when_there_is_more` |
| 8 | Medium | The projector lease (30 s) was taken once per round and a slow collector can keep a round longer: two projectors, measurements counted twice | Real. Asked again before each batch; `mark_projected` says whether it moved the note | `integration projection.rs` (2) |
| 9 | Medium | `GET /runs/{id}/attempts` read one batch of 5 000 journal entries: attempts after it were missing or open | Real. Read in as many batches as it takes | `telemetry_derive.rs::the_attempts_of_a_journal_longer_than_a_batch…` |
| 10 | Low-medium | Reading the trail was recorded once for good (no window) | Real. Once per five minutes for the same reading | `integration isolation.rs::reading_the_trail_is_recorded_again…` |
| 11 | Low-medium | A poll that had looked at the session could lease after a drain returned (the old test drained, then polled, so it never met the race) | Real. A lock the drain takes exclusively and the lease transaction takes shared | `integration isolation.rs::a_drain_cannot_return_while_a_poll…` (weakened the lock to see it fail) |
| 12 | Low | A holder of `worker:manage` could grow `worker_drains` without bound | Real. 500 per namespace | `us04::a_namespace_has_a_bounded_number_of_drained_identities` |
| 13 | Low | The audit read ordered by the primary key and walked the table backwards across every tenant | Confirmed with `EXPLAIN` (10 144 rows removed for 51). Orders by the tenant's index | `integration isolation.rs::reading_the_trail_of_a_small_tenant…` holds the plan |
| 14 | Low | A run written to while projected kept the lag of the first time it fell behind | Real. A partial look is behind since its first entry not taken | `integration projection.rs` |
| 15 | Low | The sampler is decided from the trace id the caller sends; the monitor's cost grows with the failure; the migrations take locks on big tables | **Not fixed**, recorded: G187, G186+G188, G190 | `research.md` |
| 15b | Low | A probe of a foreign id is slower than a probe of a missing one because the denial is written before the `404` | **Not a defect**: the write is already off the response path (G189, withdrawn) | `crates/infra-postgres/src/identity.rs` |
| 16 | Low | Tests that proved less than they claimed (the "two monitors" test shares a lease holder; the drain race test was sequential) | Renamed to say what they prove; the race has its own test | `fault monitor.rs`, `fault operators.rs` |

## What was and was not run

- **Gate on the final code:** `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `scripts/check-architecture.py` and `cargo test --workspace --locked --no-fail-fast`. The suites that start containers other than PostgreSQL (NATS, Kafka, ElasticMQ and the script sandbox's containers: `e2e_durable_execution` (1), `integration_durable_execution` (3), `e2e_events_webhooks_messaging` (5), `e2e_inline_code_sandbox` (24), `fault_inline_code_sandbox` (7), `integration_inline_code_sandbox_containers` (23)) could not run where this was written, because Docker Desktop's filesystem had gone read-only after the disk filled; none of them touches code changed here. CI runs them.
- **The database** was PostgreSQL 18.6 on the same machine, not 16 in a container. The benchmark says which settings it ran with (`fsync`, `synchronous_commit`, `wal_sync_method`) and that on macOS commits are faster than on a server's storage.
- **The Playwright journeys:** 15 pass; the one that tries a script in the sandbox needs Docker (`SCRIPT_SANDBOX_UNAVAILABLE`).
- **The `observability` job of CI** (`promtool`, the Collector's `validate`, the dashboards as JSON) has not run on GitHub; the files were validated with the same images when they were written.

## Not built

OTLP over protobuf and gRPC (G169); exemplars (G173); alerts by pager or email (G175); thresholds per namespace (G174); a Studio screen for alerts and a control to drain a worker (G181); retention and export of the audit trail (G182); a sampler the caller cannot choose (G187); a bounded monitor for a very large outbox (G186, G188); the visual direction of the Studio (feature 007).
