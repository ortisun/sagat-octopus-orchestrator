# Implementation Plan: Flow control and step policies

**Branch**: `feature/022-flow-control-step-policies` | **Date**: 2026-10-26 | **Spec**: [spec.md](./spec.md)

**Input**: `specs/022-flow-control-step-policies/spec.md`

Status: Proposed — not started.

## Summary

Add four additive IR capabilities (`stepCache`, `earlyReturn`, `failureHandler`, `concurrencyKeys`) and
two API surfaces over existing admission (standalone steps, draft test runs). Every new decision is a
journal event committed with the transition it causes; shared cross-run state (cache entries, semaphores,
handler dispatches) is a set of tenant-keyed PostgreSQL rows changed in documented transactions, after
each path's own locks and before deferred content usage (per-path orders below and in ADR-0056). Every run
creator admits through one permit port (Run creators). No worker protocol change, no new `Error.code`, no new run
status. Concurrency permits follow the proposed [ADR-0056](../../docs/adr/ADR-0056-concurrency-keys-are-durable-permits-granted-by-the-waiters-shard-owner.md);
the other choices are D1160–D1195 in [research.md](./research.md).

## Technical Context

**Language/Version**: Rust 1.98.0 from `rust-toolchain.toml`; the locked TypeScript, Go, Python, .NET/C#
and Java SDK toolchains; the Studio's locked Node toolchain. No toolchain change.

**Primary Dependencies**: existing Tokio, Axum/tonic adapters, SQLx PostgreSQL store, NATS hints, the
bounded expression evaluator (`crates/infra-expr`), the IR codec (`crates/infra-ir`), OTel facade
(`crates/telemetry`). HMAC-SHA-256 for keyed digests uses `ring::hmac`, already a dependency of
`crates/application` (`crates/application/src/managed_secrets.rs` keys the 017 intent binding with it);
no new dependency.

**Storage**: PostgreSQL only: new tables in [data-model.md](./data-model.md); additive columns on `runs`;
retained cache values and pins charged to `tenant_content_bytes`. No object-store change (blob outputs are
not cached, G1161).

**Testing**: domain unit/property tests; real-PostgreSQL integration and fault suites under
`tests/{integration,fault,contract,e2e,performance}/flow-control-step-policies/`; five-SDK conformance in
`tests/contract/sdk-conformance`; Studio Vitest and browser journeys; the tenant battery
`tests/contract/security-multitenancy/rest/battery.rs`; runtime-role sweep (`SAGAT_TEST_AS_RUNTIME_ROLE=1`).

**Target Platform**: existing single-region Linux containers, Docker Compose for development, Helm.

**Project Type**: modular monolith with shared contracts, five SDKs and the Studio.

**Performance Goals**: benchmark criteria only (NFR-022-004): lookup overhead on step readiness, hit
versus dispatch latency, permit acquisition with 1/16/256 contenders on one key, grant delay after
release, handler dispatch delay, store-writer throughput. Report environment and failures.

**Constraints**: domain types stay free of SQLx/Axum/NATS and `serde_json` as business representation;
the evaluator caps are unchanged; no user code in the API or scheduler; every new resource finite.

**Scale/Scope**: six user stories, 38 functional requirements, one ADR proposal, four capability gates.

## Architecture and crate placement

| Layer | Placement | Content |
|---|---|---|
| Domain | `crates/domain/src/workflow.rs` (+ new `crates/domain/src/workflow/flow_control.rs`) | `CachePolicy`, `ConcurrencyPolicy`, `FailureHandler`, `NodeKind::EarlyReturn`; placement and region rules; fingerprints as typed values |
| Domain | `crates/domain/src/execution/{event.rs,state.rs,command.rs,plan.rs,sagas.rs}` (+ new `execution/flow_control.rs`) | events `step.cache_hit`, `step.mocked`, `result.returned`, `failure_handler.started/finished`, `concurrency.acquired/queued/released/wait_timed_out`; reducer rules; mocked completions excluded from compensation plans |
| Application | new `crates/application/src/{step_cache.rs,concurrency.rs,failure_handlers.rs}`; new `crates/application/src/runs/{standalone.rs,test_runs.rs}`; `runs/ad_hoc.rs` shared retention; `quotas.rs`, `authz.rs` | ports and use cases: lookup/store/invalidate/erase-by-source, the single permit admission port used by every run creator plus grant/timeout, handler configuration and dispatch, standalone and test admission |
| IR adapter | `crates/infra-ir/src/convert.rs` (+ new `crates/infra-ir/src/flow_control.rs`) | parse/validate new fields and kind; capability gates; `DefinitionProblem` codes; canonical key-expression text for fingerprints |
| PostgreSQL | new `crates/infra-postgres/src/{step_cache.rs,concurrency.rs,failure_handlers.rs}`; `execution.rs`, `dispatch.rs`, `quotas.rs`, `privilege.rs`; migrations; `storage-classification.json` | rows, locks, grants for `sagat_app`, byte accounting triggers |
| Scheduler | new `crates/scheduler/src/flow_control.rs` | shard-fenced permit grant and queue timeout; cache store writer and purge; handler dispatcher |
| HTTP API | new `crates/api-http/src/routes/{step_cache.rs,concurrency.rs,failure_handlers.rs,step_executions.rs,test_runs.rs}`, `dto_flow_control.rs` | routes in [contracts/openapi-flow-control.yaml](./contracts/openapi-flow-control.yaml) |
| Telemetry | `crates/telemetry` | closed metric vocabulary below |
| Apps | `apps/control-plane`, `apps/execution-engine` | flow-control digest key configuration (D1168); no new process |
| Studio | `frontend/apps/studio/src/features/{workflows,runs,operations}` | authoring fields, test panel, run badges, operational views |
| SDKs | `sdks/typescript/src/{workflow.ts,client.ts}`, `sdks/go/{workflow,client}`, `sdks/python/sagat_octopus_orchestrator/{workflow.py,client.py}`, `sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/{Workflows,SagatClient.cs}`, `sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/{workflow,SagatClient.java}` | builders and client calls |

`scripts/check-architecture.py` must stay green: keyed digests are computed in the application layer
(new `crates/application/src/keyed_digest.rs`, same pattern as the 017 binding key); the domain sees only
digest value objects and never the key.

## Transaction boundaries

Each row is one transaction; "commits" lists everything that commits together. Lock order inside each:
the existing order of that path, then its `runs` write (whose trigger updates `run_active_counts`), then
the new shared rows (cache scope → entry; semaphores sorted by pool, scope, group, key), then, on the
terminal path only, the namespace failure-handler policy row (`FOR SHARE`), and quota and usage rows last
(ADR-0027, ADR-0050 slot 8b). After a semaphore a transaction locks only those listed transition rows and
usage rows, never an existing run, count or activity-quota row; the cache and handler-policy rows are the
transition rows ADR-0050 decision 7 lets this feature list (per-path orders in ADR-0056 decision 2).

| # | Transaction | Locks (in order) | Commits together |
|---|---|---|---|
| T1 | Step becomes ready, cache hit (inside the readiness transaction: a worker completion, a scheduler pass or an admission) | the path's existing order — chain (if any) → run, plus the shard fence only on scheduler paths that already hold it; no new fence is taken (worker completions hold none) → cache scope row (share) → entry (share) → hit provenance insert | copied output payload, hit provenance row, step `SUCCEEDED`, `step.cache_hit` + `step.completed`, next tasks/timers/subscriptions, outbox, telemetry note, usage delta |
| T2 | Miss completed (CompleteStep) | existing completion order | existing completion records + store obligation row |
| T3 | Store writer | obligation (`SKIP LOCKED`) → scope row (share) → entry insert-if-absent → policy fence | entry, obligation deleted, content charge; on quota refusal a second transaction deletes the obligation only |
| T4 | Invalidate by scope/node | scope row (update) | `invalidated_at`/revision bump, audit |
| T5 | Invalidate by key / purge batch | scope row (share) → entries (update, `SKIP LOCKED` for purge) | deletions, byte release, audit (invalidation only) |
| T6 | `earlyReturn` reached | existing reducer order | result payload, `runs.result_digest`/`result_source`/`result_available_at`, `result.returned`, `run.result_returned` deliveries and NATS hint, next tasks |
| T7 | Would-finish with handler | existing stop order | `failure_handler.started` (pending status), handler entry tasks or derived child creation (existing child path), outbox; no `run.finished` |
| T8 | Handler ends / run finishes | existing terminal order → `runs` update → run semaphore → namespace handler policy (share) | `failure_handler.finished`, `run.finished`, run-finished deliveries, permit release (`concurrency.released`), dispatch row when due, outbox |
| T9 | Handler dispatch | shard fence (share) → dispatch row (`SKIP LOCKED`) → source run identity `KEY SHARE` (ADR-0039 pattern) → ordinary admission locks | handler run (run, journal, tasks, outbox, audit), dispatch `ADMITTED` + `handler_run_id`; or attempts/`next_attempt_at`/`DEAD` in a separate transaction after a refused admission |
| T10 | Admission with run permit (every run creator, see Run creators) | ADR-0050 merged admission order (C4): slots 1–5 (authority, receipt, aggregates, catalog heads, secrets) → slot 6 `runs` insert (with its `run_active_counts` trigger rows) → **slot 7 semaphore** (insert or update) → slot 8 quota and usage | run, holder or waiter row, `concurrency.acquired` or `concurrency.queued` + queue timer, counters, usage |
| T11 | Grant to the oldest unpaused waiter whose own limit admits it (skip-ahead, no head-of-line blocking); with 019, by (019 priority 1 first, enqueue time asc) among those (D1602) | shard fence (share) → waiter run (`SKIP LOCKED`) → semaphore | holder row, waiter row removed, timer cancelled, `concurrency.acquired`, first ready tasks, outbox |
| T12 | Queue timeout | timer path (`apply_if`) → run → `runs` update → semaphore | waiter removed, `concurrency.wait_timed_out`, run `FAILED` (`CONCURRENCY_WAIT_TIMEOUT`) directly as in T8 without a workflow handler phase (it never held a permit); namespace dispatch row when due |
| T13 | Step claim with step permit | run → drain fence → session → sorted activity quota rows → sorted semaphores | lease, holder row, journal, outbox |
| T14 | Attempt ends | existing completion/failure/expiry order (run → step) → semaphore | existing records + holder row removed, `concurrency.released` |
| T15 | Standalone admission | existing ad hoc order (017 `RunStore::create`) | private definition (reuse or insert), run, receipt, audit, journal, tasks, outbox |
| T16 | Draft test admission | workflow row (share, draft revision check) → existing ad hoc order | private definition, run (`DRAFT_TEST`), pin payloads, `step.mocked` for upstream pins, tasks for `startAt`, receipt, audit, NATS hint |
| T17 | Namespace handler configuration | namespace policy fence (update) | handler row with revision CAS, audit |
| T18 | Erasure of a source run (called by 020) | cache entries by `source_run_id` (update) → obligations of the run | entries and obligations deleted, bytes released; hit provenance and pins left for 020 to act on |

No transaction locks two existing runs. No provider, worker or HTTP I/O happens inside any of them.

### Run creators

ADR-0056's invariant holds only if every writer that creates a run uses the permit admission port (T10).
Each row needs a test that fails when the creator bypasses it (T022-064).

| Creator | Owner | Permit behaviour |
|---|---|---|
| `startRun`, event/webhook/broker triggers, cron triggers | 002, 004 | acquire or queue; queue-full refusal (synchronous) or pending retry (event, cron) |
| Child nodes, including derived `onFailure~h` children | 002, 022 | acquire or queue in the child-creation transaction |
| `RERUN_AS_NEW`, 017 private reruns, ad hoc and probe admission | 008, 017 | acquire or queue |
| Standalone steps, draft test runs (draft-test pool), namespace handler dispatch | 022 | acquire or queue |
| Schedule firing, batch reruns, Signal-with-Start, Update-with-Start | 018 | acquire or queue at admission; with a start delay, at the delay's release |
| Reset to a point | 018 | acquire a fresh permit in the reset admission transaction or refuse (`CONCURRENCY_PERMIT_UNAVAILABLE`); never queue; copied `concurrency.*` rows are 018-`restored` history; reset rules per event in `contracts/journal-events.md` (D1600) |
| Continue-As-New successors | 014 | not applicable: `concurrency` with `continueAsNew` is refused |
| Any future creator (for example 030 handler runs) | owner feature | MUST call the same port; enforced by T022-064 |

## Authorization

| Operation | Permission(s) | admin | developer | operator | viewer | machine roles |
|---|---|---|---|---|---|---|
| Publish definitions with new fields | `definition:publish` (existing) | ✓ | ✓ | – | – | – |
| Read run flow-control state, cache-hit provenance | `run:read` | ✓ | ✓ | ✓ | ✓ | – |
| List cache entries, concurrency groups (metadata, labels only) | `ops:read` | ✓ | ✓ | ✓ | ✓ | – |
| Invalidate cache entries | `cache:invalidate` (new) | ✓ | ✓ | – | – | – |
| Set/clear namespace failure handler | `failure-handler:manage` (new) | ✓ | – | – | – | – |
| Read namespace failure handler, dispatches | `ops:read` | ✓ | ✓ | ✓ | ✓ | – |
| Redrive a dead dispatch | `run:operate` + `run:start` | ✓ | – | ✓ | – | – |
| Cancel a queued run | `run:operate` (existing cancel) | ✓ | – | ✓ | – | – |
| Start a standalone step | `run:start` + `step:standalone` (new); `script:read` for a script node; 017 database permissions for a database node | ✓ | ✓ | – | – | – |
| Start a draft test run | `definition:write` + `run:start` | ✓ | ✓ | – | – | – |
| Pin from a previous run | test-run permissions + `run:read` + disclosure rights of the pinned value | per grant | per grant | – | – | – |
| Disclose an early or final result | existing `payload-disclosures` rules (FR-016-007) | per level | per level | per level | public only | – |

The ✓ marks are the proposed built-in role grants; `payload:reveal-pii` stays grant-only (ADR-0033
addendum). Machine roles (worker, runner, database-runner) receive none of the new permissions. Every
route resolves tenant and namespace from the authenticated context, returns an opaque `NOT_FOUND` for a
foreign resource, reauthorizes on idempotent replay, and audits denials through the existing restricted
denial path.

## Finite ceilings

| Resource | Default | Hard ceiling |
|---|---:|---:|
| Cache TTL (`ttlSeconds`, required) | — | 2,592,000 s (30 days) |
| Cache key canonical value | 1 KiB | 4 KiB |
| Cache entry value (inline only) | 256 KiB | 256 KiB |
| Cache entries per namespace | 10,000 | 100,000 |
| Pending store obligations per namespace | 1,000 | 10,000 |
| Cached nodes per definition | 32 | 256 |
| Concurrency `limit` per key | declared (≥ 1) | 10,000 |
| Concurrency key canonical value | 256 B | 256 B |
| Queue timeout (`queueTimeoutMs`) | 600,000 ms | 86,400,000 ms (and ≤ run deadline) |
| Waiters per key | 1,000 | 10,000 |
| Queued runs per namespace (`queued_runs`, also counted in `active_runs`) | 1,000 | 10,000 |
| Live semaphore keys per namespace | 10,000 | 100,000 |
| Concurrency declarations per definition | 1 run + 16 node | 1 run + 64 node |
| `earlyReturn` nodes per definition | 16 | 64 |
| Failure handler `timeoutMs` | 300,000 ms | 3,600,000 ms |
| Handler dispatch attempts before `DEAD` | 12 | 12 |
| Pending handler dispatches per namespace | 10,000 | 10,000 |
| Standalone timeout | 300,000 ms | 3,600,000 ms |
| Private definitions (standalone + test) | existing workflow quota (1,000) | existing ceiling |
| Active draft test runs per namespace / per actor | 16 / 4 | 64 / 16 |
| Pins per test run | 64 | 256 |
| Bytes per pin / per test run | 256 KiB / 1 MiB | 256 KiB / 4 MiB |
| Test run deadline | 900,000 ms | 86,400,000 ms |

Effective values are the minimum of the declaration or request, namespace policy and deployment ceiling.
Zero is not unlimited: a zero count refuses new admission. Admission fails before work is created. Byte
values also fall under `tenant_content_bytes`; counts are new `QuotaResource` values governed by the
existing tenant/namespace policies and operator ceilings (feature 009). These are design bounds to verify
by test, not measured capacity.

## Failure and recovery matrix

| Failure | Expected state | Handling and test |
|---|---|---|
| Crash before T1 commits | Predecessor transition rolled back with it, nothing copied | The retried report or pass repeats readiness; one hit at most (fault test) |
| Crash after T2, before T3 | Step done, obligation pending | Writer leases it later; insert-if-absent makes repeats harmless |
| T3 refused by content quota | Step and run unaffected | Obligation removed, counted `skipped_quota` (SC-022-002) |
| Entry invalidated during T1 | Hit commits before invalidation (share lock) or misses after | Barrier test both orders |
| Two concurrent misses | Both execute; one entry | Second store `duplicate`; documented, not deduplicated |
| Crash between T6 and continuation | Result persisted with the step | Next step scheduled in T6 itself; replay check |
| Crash after T7 | Handler phase durable | Handler tasks or child already persisted; restart continues |
| Handler fails / times out / cancelled | T8 with pending status | Status assertion per `on` value (SC-022-005) |
| Crash after T8, before T9 | Dispatch `PENDING` | Dispatcher picks it up; exactly one run (SC-022-006) |
| Crash inside T9 | Rolled back | Retry admits once; same idempotent admission key |
| Handler workflow missing / quota full | Dispatch retried, then `DEAD` | Namespace alert `failure_handler_dead`; redrive |
| Lost NATS hint for grant/handler | Rows durable | Scheduler pass finds eligible waiters/dispatches by index |
| Grant races queue timeout | First commit wins | 20-round race test (SC-022-003) |
| Two replicas grant across a shard takeover | Fence rejects the stale one | Takeover barrier test |
| Crash between release (T8/T14) and grant (T11) | Free capacity, queued waiter | Next pass grants; no permit lost |
| Holder stuck in `MANUAL_REVIEW` | Permit held | Operational view shows holder age; waiters time out |
| Duplicate standalone/test request | Same run | Receipt replay with reauthorization |
| Draft changed during test admission | `CONFLICT` | Revision check under share lock |
| Worker crash during a mocked node | Not applicable | Mocked nodes never lease (assert zero leases) |
| Reset of a source that held a run permit | Fresh permit or refusal | 018 reset admission through T10; restored events ignored by the invariant; points inside a handler phase refused (SC-022-015) |
| Erasure (020) of a source run | Entries and obligations gone; copies listed | T18 then next lookup misses (SC-022-016) |
| Digest key version changed with live holders/waiters | Activation refused | Startup check per ADR-0056 decision 7 |
| Restart of all engines | Waiting runs, pending obligations, dispatches resume | Restart e2e with queued runs and pending dispatches |
| Failed compensation then handler | Handler waits for terminal status (`MANUAL_REVIEW` first) | Saga fault test with failed compensation, then operator resolution |

## Observability

Spans (live, exported by the existing layer; run and attempt spans remain journal-derived, ADR-0017):
`workflow.cache.lookup`, `workflow.cache.store`, `workflow.cache.invalidate`,
`workflow.concurrency.acquire`, `workflow.concurrency.grant`, `workflow.failure_handler.dispatch`,
`workflow.early_return`, `workflow.standalone.admit`, `workflow.test_run.admit`. Attributes: the existing
`sagat.tenant.id`, `sagat.namespace.id`, `sagat.run.id`, `sagat.node`, `sagat.outcome`; plus
`sagat.concurrency.group` and `sagat.cache.scope` (names, not values); never a key value or label.

Metrics (closed label sets, no tenant/namespace/run/workflow/group/key):

| Metric | Labels |
|---|---|
| `step_cache_lookups_total` | `outcome` ∈ hit, miss, expired, invalidated, unauthorized, bypassed |
| `step_cache_stores_total` | `outcome` ∈ stored, duplicate, skipped_quota, skipped_blob, skipped_backlog |
| `step_cache_store_backlog` | — |
| `early_returns_total` | `outcome` ∈ returned, expression_error, data_limit |
| `failure_handler_phases_total` | `form` ∈ node, child; `outcome` ∈ succeeded, failed, timed_out, cancelled |
| `failure_handler_dispatches_total` | `outcome` ∈ admitted, retried, dead, redriven |
| `failure_handler_dispatch_backlog` | — |
| `concurrency_acquisitions_total` | `scope` ∈ run, step; `outcome` ∈ acquired, queued, refused_full, refused_reset, granted, skipped_ahead, timed_out, dequeued |
| `concurrency_wait_seconds` (histogram) | `scope` |
| `concurrency_waiters` | `scope` |
| `standalone_steps_total` | `kind` ∈ activity, script, database; `outcome` ∈ admitted, replayed, refused |
| `test_runs_total` | `outcome` ∈ admitted, replayed, refused_prod, refused_pin, refused_quota |

Audit actions (in the decision's transaction): `cache.invalidated`, `failure_handler.set`,
`failure_handler.cleared`, `failure_handler.dispatch_redriven`, `standalone_step.started`,
`test_run.started`, and the existing PII reveal audit for a `fromRun` pin. System dispatch of a handler is
journaled in the handler run (`run.started` with provenance), not audited as a human action.

Alerts (engine monitor, per namespace, `operational_alerts`): `failure_handler_dead`,
`concurrency_queue_stuck` (oldest waiter older than its key's longest holder by a threshold while capacity
is zero), `cache_store_backlog`. Each gets a section in the new runbook
`docs/runbooks/flow-control.md`.

Logs: structured, redacted (ADR-0017 decision 5); key values, cached values, pins and failure messages are
never logged.

## Migration and compatibility plan

- **IR**: additive fields `cache`, `concurrency` (node and top level), `onFailure`, node kind
  `earlyReturn` in `workflow.schema.json` (fragment in [contracts/workflow-ir-fragment.schema.json](./contracts/workflow-ir-fragment.schema.json));
  old fixtures unchanged; disabled capabilities refuse before publication (ADR-0032 decision 3).
- **REST**: new paths and additive fields only (`Run.resultSource`, `Run.resultAvailableAt`,
  `Run.handlerFor`, `Run.concurrency`, `Run.failureHandler`; origin values `STANDALONE_STEP`,
  `DRAFT_TEST`; subscription event type `run.result_returned`). The existing origin enumeration keeps its
  name for compatibility.
- **AsyncAPI**: one new message for `run.result_returned` deliveries and the NATS hint; existing messages
  only grow (`docs/event-schema-evolution.md`); baseline regenerated deliberately.
- **Worker protocol**: unchanged. Older workers keep working; they never see mocked or cached steps.
- **Database**: one additive migration after `20261220000000` (D1191): new tables, `runs` columns, origin
  CHECK extension, grants, manifest entries, byte triggers. The down migration refuses while any new row
  exists rather than deleting customer data.
- **Rollout**: one capability at a time; rollback disables new publication and starts for that capability
  and keeps readers, the store writer, the grant loop and the dispatcher until affected runs and rows
  drain.
- **SDKs**: additive builder options and client calls; older SDKs ignore new fields in responses.

## Constitution Check

Design checks, not implementation approval or test results.

| Principle | Before research | After design | Evidence / future gate |
|---|---|---|---|
| I — durable atomic transitions | Pass | Pass | Transaction table T1–T17; ADR-0056; fault suites T022-020, T022-028, T022-034, T022-038, T022-047; restart e2e T022-059 |
| II — pure domain, inward dependencies | Pass | Pass | Typed policies in domain; keyed digests in application; `check-architecture.py` (T022-065) |
| III — at-least-once effects, compensation first | Pass | Pass | Cache refused on effectful nodes (D1163); handler after compensation (FR-022-013); mocks excluded from compensation (D1186) |
| IV — pinned versions, five SDKs | Pass | Pass | Additive IR; private definitions pinned; SDKs and conformance T022-048..053 |
| V — tenant boundary, secrets, isolation | Pass | Pass | Keyed digests (D1168); hit authorization (D1166); test runs refused in `prod`; battery and canaries T022-058 |
| VI — OTel, audit, typed errors | Pass | Pass | Observability section; `contracts/errors.md` |
| VII — inbox/outbox | Pass | Pass | Handler dispatch obligation (D1175); `run.result_returned` deliveries in T6 |
| VIII — failure tested, honest | Pass | Pass | Rule-9 matrix above; benchmark criteria only; ADR-0056 evidence pending |

## Complexity Tracking

| Addition | Why needed | Simpler alternative rejected because |
|---|---|---|
| Asynchronous cache store (obligation + writer) | Completion must never fail on cache quota | In-transaction store turns a full quota into a failed run (ADR-0027 deferred check) |
| Separate grant transaction for permits | One run lock per transaction | Granting in the release transaction locks two unrelated runs (ADR-0056 option 2) |
| Keyed digest key in control plane and engine | Business keys are personal data | Plain SHA-256 is an offline oracle for run readers |
| Handler dispatch table and loop | Exactly one handler run per failure, durable across crashes | Reusing outbound subscriptions would model an internal admission as an at-least-once external send |

## Delivery sequence

1. Foundations: IR fragment, capability gates, domain policies and events, migration, manifest, permissions,
   digest key port (Phase 2 of [tasks.md](./tasks.md)).
2. US02 concurrency (ADR-0056 evidence first), then US01 cache, then US03 handlers — each behind its gate.
3. US04 early return; US05 standalone; US06 test mode.
4. SDKs and conformance, Studio, runbook, canonical contract merge, benchmark, gates.

## Verification plan

- Unit/property: placement rules, fingerprints, key canonicalization, reducer rules for each event.
- Integration (real PostgreSQL, runtime role): every transaction of the table with barrier tests for its
  races; `verify_run`/`assert_consistent` extended with the semaphore and result checks.
- Fault: crash at each commit point T1–T17, duplicate delivery/request, lost NATS hint, grant/timeout race,
  failed compensation before a handler, restart with queued runs and pending dispatches.
- Contract: OpenAPI/AsyncAPI/IR schema lint and compatibility baselines; error catalogue; five-SDK
  conformance scenarios in [contracts/conformance.md](./contracts/conformance.md).
- Security: tenant battery for every new route; canaries for raw keys, cached values, pins, failure text.
- Cross-feature: one case per run creator (T022-064, with 018) and the erasure interplay against 020's
  manifest-driven reference check (T022-063); cases whose owner feature is absent are reported pending.
- E2E: [quickstart.md](./quickstart.md) happy and failure demos through the Studio and the CLI.
- Performance: `tests/performance/reports/flow-control-step-policies.md` with environment and failures.
- Results are recorded as run; a skipped or failing test is stated (constitution VIII).
