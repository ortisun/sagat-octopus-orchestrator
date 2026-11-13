# Implementation Plan: Flow control and step policies

**Branch**: `feature/022-flow-control-step-policies` | **Date**: 2026-10-26 | **Spec**: [spec.md](./spec.md)

**Input**: `specs/022-flow-control-step-policies/spec.md`

Status: Proposed — not started. The release 0.3.0 slice (US02 and the new US07, OD-2026-11-13-11) is planned in
[Release 0.3.0 slice](#release-030-slice) at the end of this file.

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
With the 0.3.0 slice: seven user stories, 50 functional requirements, two ADR proposals, five gates.

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

## Release 0.3.0 slice

Added on 2026-11-13 for OD-2026-11-13-11 (research.md, Owner decision of 2026-11-13). Proposed; nothing is
built. The slice is US02 (concurrency keys, above, unchanged) and US07 (rate limits with buffering,
FR-022-039..050). Everything above this section stays the plan of the whole feature; this section says
what the slice takes from it and what it adds. Rate limits follow the proposed
[ADR-0088](../../docs/adr/ADR-0088-rate-limits-are-a-durable-window-of-grants-on-the-database-clock-and-the-scheduler-grants-never-the-claim.md);
the choices are D022-001..017 in [research.md](./research.md).

### What the slice takes from the plan above

| Area | In the slice | Left for the other stories |
|---|---|---|
| Capabilities | `concurrencyKeys`, new `rateLimits` | `stepCache`, `earlyReturn`, `failureHandler` |
| Domain and IR | `ConcurrencyPolicy`, new `RateLimitPolicy`, their validation and events | cache, handler, `earlyReturn`, `startAt` rules |
| Transactions | T10–T14 (permits); T19–T25 below (rate) | T1–T9, T15–T18 |
| Run creators | every creator that exists on the tree when the slice is built (start API, event and cron triggers, child nodes, reruns, 017 ad hoc and probes; 018's when its branch is merged) | 022 standalone, draft-test and handler dispatch (not built) |
| Tables | `concurrency_semaphores`, `concurrency_holders`, `concurrency_waiters`, `step_concurrency_requirements`; the three rate tables of data-model.md | cache, handler, pin tables; the new `runs` columns |
| Permissions | none new (`ops:read` lists keys, existing cancel cancels a waiter) | `cache:invalidate`, `failure-handler:manage`, `step:standalone` |
| REST | `Run.concurrency`, `Run.rateLimit`, `GET .../concurrency-keys`, `GET .../rate-limit-keys` | every other path of the fragment |
| Worker protocol | unchanged | unchanged |
| SDKs | `concurrency` and `rateLimit` builders in six languages (the Rust SDK is new to this feature, D022-016) | the other builders and client calls |
| Studio | the two declarations in the workflow settings and the node inspector, the waiting state on runs and steps, the two key lists | the rest of T022-054..056 |

### Architecture and crate placement (rate limits)

| Layer | Placement | Content |
|---|---|---|
| Domain | `crates/domain/src/workflow/flow_control.rs`, `crates/domain/src/execution/{flow_control.rs,event.rs,state.rs}` | `RateLimitPolicy {group, key, limit, per, queue_timeout}`; the pure window decision (grants that count, admit or wait, position, estimate) as a function of instants passed in; events `rate_limit.queued/granted/wait_timed_out/dequeued`; wait kind `RATE_LIMIT` for runs and steps |
| Application | new `crates/application/src/rate_limits.rs`; the permit admission port of `crates/application/src/concurrency.rs` grows into one admission-gate port (permit and slot) | grant-or-buffer at admission and at step readiness, grant from the buffer, timeout, dequeue, lease recording, listing |
| IR adapter | `crates/infra-ir/src/flow_control.rs` | parse and validate `rateLimit`; gate `rateLimits`; problem codes |
| PostgreSQL | new `crates/infra-postgres/src/rate_limits.rs`; `execution.rs` (readiness and admission call sites); migration; `storage-classification.json` | rows and locks of T19–T25; grants for `sagat_app` |
| Scheduler | `crates/scheduler/src/flow_control.rs` | the grant pass (T20), lease recording (T23), purge (T25); queue deadlines stay in the existing timer pass (T21) |
| HTTP API | `crates/api-http/src/routes/rate_limits.rs`, `dto_flow_control.rs` | `Run.rateLimit`, `GET /namespaces/{namespaceId}/rate-limit-keys` |
| Telemetry, monitor | `crates/telemetry`, `crates/scheduler/src/monitor.rs` | metrics and alerts below |

The domain decides from values it is given (instants, counts); the database clock is read by the adapter
inside the transaction and passed in (`scripts/check-architecture.py` stays green). The claim code
(`crates/infra-postgres/src/workers.rs`) is not edited for rate limits.

### Transaction boundaries (rate limits)

Same conventions as the table above. "Window" is the `rate_limit_windows` row of the key; grant and
waiter rows of a key are touched only under it. Rate-limit rows are rate rows of ADR-0050 slot 8b: they
come after every entity lock and after a semaphore, sorted by (pool, scope, group, key), and before the
quota and usage rows. No transaction locks a run after a window row.

| # | Transaction | Locks (in order) | Commits together |
|---|---|---|---|
| T19 | Admission of a run whose definition declares `rateLimit` (extends T10; every run creator) | ADR-0050 slots 1–6 → slot 7 semaphore when `concurrency` is also declared → window (insert or update) → quota and usage | run; either the grant row, `rate_limit.granted` (and `concurrency.acquired`) and the first tasks, or the waiter row(s), `rate_limit.queued` (and `concurrency.queued`), the queue timer and no task; counters |
| T20 | Grant from the buffer, by the pass of the waiter's shard owner: the oldest unpaused waiter of the key that its own limit and its other gate admit | shard fence (share) → waiter run (`SKIP LOCKED`) → `runs` update → semaphore when declared → window | grant row, waiter row(s) removed, timer cancelled, `rate_limit.granted` (and `concurrency.acquired`); for a run its first ready tasks and outbox; for a step the step `READY` |
| T21 | Queue deadline of a waiting run or step (existing timer pass, `apply_if`) | timer path → run → `runs` update → semaphore when declared → window | waiter removed, `rate_limit.wait_timed_out` (or `concurrency.wait_timed_out` when the permit is what is missing at the deadline); run `FAILED` directly as T12, or the step's failure route |
| T22 | A step with node-level `rateLimit` would become ready (inside the readiness transaction: a worker completion, a scheduler pass, a retry timer or an admission) | the path's existing order → window | either grant row, `rate_limit.granted` and the step `READY`; or waiter row, `rate_limit.queued`, queue timer and the step `WAITING` (`RATE_LIMIT`); next to whatever the path already commits |
| T23 | Lease recording, by the pass: a grant row of the shard with no lease instant whose step has been leased, or has ended without a lease | shard fence (share) → window → grant row | lease instant copied from the `step.leased` fact, or the grant row deleted (slot returned); no run row is locked or written |
| T24 | Cancellation, run deadline or other terminal transition of a waiter, or of a granted step never leased | the path's order (run → `runs` update → semaphore when declared) → window | waiter removed and `rate_limit.dequeued`, or the unleased grant row removed; with the path's own records |
| T25 | Purge, in bounded batches | window → grant rows that no acquirer can count any more → the window row itself when it has no waiter and no grant | deletions only |

A release of a permit (T8, T14) is unchanged and never touches a window. The claim (T13) is unchanged: a
step with both node-level gates is granted its slot by T20 or T22 and takes its permit at the claim.

### Run creators (rate limits)

The table "Run creators" above applies as written, with "acquire or queue" read as "grant or buffer, for
each gate the definition declares, through the one port". Two rows differ: a **reset** (018) takes a slot
or is refused with `RATE_LIMIT_SLOT_UNAVAILABLE` and never waits (D022-010, O1168); **Continue-As-New
successors** are not applicable because `rateLimit` with `continueAsNew` is refused (G1177). A schedule
firing (018) creates a run that may be buffered here; 018's overlap policy sees it as an open run.

### Finite ceilings (rate limits)

| Resource | Default | Hard ceiling |
|---|---:|---:|
| `limit` per `second` | declared (≥ 1) | 100 |
| `limit` per `minute` | declared (≥ 1) | 6,000 |
| `limit` per `hour` | declared (≥ 1) | 10,000 |
| Rate-limit key canonical value | 256 B | 256 B |
| Queue timeout (`queueTimeoutMs`) | 600,000 ms | 86,400,000 ms (and ≤ run deadline) |
| Waiters per key | 1,000 | 10,000 |
| Queued runs per namespace (`queued_runs`, shared with concurrency waiters) | 1,000 | 10,000 |
| Live rate-limit keys per namespace | 10,000 | 100,000 |
| Rate-limit declarations per definition | 1 run + 16 node | 1 run + 64 node |
| Grants to one key in one pass | 256 | 256 |

The hard ceilings on `limit` bound the grant rows a key can hold (at most its `limit` plus its unleased
grants) and are design bounds to verify by T022-089, not measured capacity. The per-pass bound caps one
key at about 512 grants a second at the default interval, above the per-second ceiling.

### Failure and recovery matrix (rate limits)

| Failure | Expected state | Handling and test |
|---|---|---|
| Crash before T19 or T22 commits | No run or no transition; no grant row | The retried request or report repeats; one grant at most |
| Crash between a grant and its journal event | Cannot exist: one transaction | Crash-point test around T20 asserts grant row, event and tasks are all present or all absent |
| Duplicate grant (two replicas across a shard takeover, or a repeated pass) | The stale fence or the missing waiter row makes the second a no-op | Takeover barrier test; the window's grant counter equals the journals' grants |
| Grant races queue timeout or cancellation | First commit wins | 20 rounds each (SC-022-018) |
| Lost hint | Rows durable | The next pass finds waiters by index |
| Engine down for several windows | Waiters buffered; the window aged | First pass grants at most `limit` per key; restart e2e |
| Replica clocks disagree | No effect: only the database clock is read | Two replicas with offset process clocks (SC-022-023) |
| Database clock moves backwards | Grants pause until it catches up; none added | Test with a clock port set back one window |
| Flood of starts on one key | Buffer fills, further starts refused; other keys granted | SC-022-017 |
| Limit lowered while runs wait | No revocation; the policy binds waiters at their next evaluation | SC-022-022 |
| Workers down while steps are granted | At most `limit` granted and unleased; no further grant | SC-022-021 |
| Lease recorded late or the pass crashes before T23 | The grant keeps counting as unleased | Under-delivery only; fault test asserts no early grant |
| Granted step cancelled before lease | Slot returned in T24 | SC-022-021 |
| Reset of a rate-limited source | Slot or refusal | With 018 (G1181), T022-086 |
| Digest key version change with live waiters or grants | Activation refused | As ADR-0056 decision 7, extended to rate rows |

### Observability (rate limits)

Spans: `workflow.rate_limit.admit`, `workflow.rate_limit.grant`. Attributes as above, plus
`sagat.rate_limit.group` (a name); never a key value or label.

| Metric | Labels |
|---|---|
| `rate_limit_decisions_total` | `scope` ∈ run, step; `outcome` ∈ granted, queued, granted_from_queue, refused_full, refused_reset, skipped_ahead, timed_out, dequeued, slot_returned |
| `rate_limit_wait_seconds` (histogram) | `scope` |
| `rate_limit_waiters` | `scope` |
| `rate_limit_grant_lag_seconds` (histogram: from the instant a slot was free with a waiter present to the grant) | — |
| `rate_limit_unleased_grants` | — |

Alerts (engine monitor, per namespace): `rate_limit_grant_lagging` (a key has had a waiter and a free
slot for longer than a threshold: the pass is not keeping up or the clock went back),
`rate_limit_queue_saturated` (a key's waiters above 80% of its ceiling). Both get a section in
`docs/runbooks/flow-control.md` next to `concurrency_queue_stuck`. No audit action is added: admission and
grants are journal facts, and policy changes use the existing quota-policy audit.

### Migration and compatibility (the slice)

- **Database**: two additive migrations, each with a refusing down migration, grants for `sagat_app` and
  manifest entries. The first holds the concurrency tables (the US02 part of T022-009); the second the
  rate tables (T022-070). The latest migration on `develop` today is `20270112000000`; the numbers are
  taken at implementation after listing the other branches (D1191). No existing table is altered.
- **IR**: additive `concurrency` and `rateLimit` (top level and on root `activity`/`script`), gated by
  `concurrencyKeys` and `rateLimits`; old documents unchanged; a disabled gate refuses before a version
  exists.
- **REST**: additive `Run.concurrency`, `Run.rateLimit`, the step's wait kind value, and two list routes.
  Older clients ignore them.
- **Worker protocol (`worker.proto`)**: no change and no minor. Older workers keep working.
- **AsyncAPI**: no new message.
- **Journal**: nine additive event types (the five `concurrency.*` and the four `rate_limit.*`); old
  readers show them as unknown events.
- **Rollout and rollback**: each gate alone. Disabling `rateLimits` stops new publications and starts that
  use it; the grant pass, lease recording and timers keep running until waiters and grants drain; tables
  are dropped only when empty.

### Constitution Check (the slice)

| Principle | Result | Evidence / future gate |
|---|---|---|
| I — durable atomic transitions | Pass | T19–T25; a grant and its journal event are one transaction; fault suite T022-080 |
| II — pure domain | Pass | The window decision is a domain function of values; the clock is read in the adapter |
| III — at-least-once effects | Pass | A slot bounds dispatch; it promises nothing about the effect; retries take a new slot (D022-008) |
| IV — pinned versions, six SDKs | Pass | Additive IR; the acquirer's pinned limit; six-language scenarios T022-084 |
| V — tenant boundary | Pass | Rows keyed by tenant and namespace; keyed digests and labels; battery T022-090 |
| VI — OTel, audit, typed errors | Pass | Metrics and alerts above; `contracts/errors.md` |
| VII — inbox/outbox | Pass | A buffered event or cron start is a run; a refused one stays pending in its inbox |
| VIII — failure tested, honest | Pass | Matrix above; G1174..G1183 state what is not covered or not measured |

### Complexity Tracking (the slice)

| Addition | Why needed | Simpler alternative rejected because |
|---|---|---|
| A row per counted grant | A strict window needs the instants | A token bucket is one row but cannot be both strict and fast under a 500 ms pass (D022-003) |
| A grant pass and lease recording in the scheduler | A slot is freed by time; a step's slot counts from its lease | Granting in the claim adds work to the hot path and makes pollers probe steps they cannot lease (D022-004) |
| One port for permit and slot | Both or neither, in one lock order | Two independent queues hold one resource while waiting for the other (D022-006) |

### Delivery sequence (the slice)

1. Owner gate (T022-066): ADR-0056, ADR-0088, O1163–O1168.
2. Foundations cut to the slice (T022-002..010, 012..014 as amended; T022-068..070).
3. US02: T022-021..029, with ADR-0056's evidence.
4. US07: T022-071..082, with ADR-0088's evidence; the hot-path budget (T022-076) before any call site in
   a completion is merged.
5. Surfaces and gates: SDKs (T022-048..052, 067, 083), conformance (T022-053, 084), Studio (T022-054..056
   cut, 085), creators (T022-064, 086), contracts (T022-060, 087), runbook (T022-061, 088), benchmark
   (T022-062, 089), battery (T022-058, 090), restart e2e (T022-059), final gates (T022-065).

### Size

63 tasks: 38 existing ones (some narrowed to the slice) and 25 new. By the project's own pace (one
reviewed pull request per slice of a trust boundary, a CI run of about two hours per push), this is an
estimate of 20 to 26 pull requests: about 9 for US02 (the admission port touches every run creator), about
10 for US07, the rest for SDKs, Studio and gates. It is the size of feature 018's foundations, not of a
patch release item; O1167 (body nodes) would add to it. No date is implied.
