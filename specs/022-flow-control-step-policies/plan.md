# Implementation Plan: Flow control and step policies

**Branch**: `feature/022-flow-control-step-policies` | **Date**: 2026-10-26 | **Spec**: [spec.md](./spec.md)

**Input**: `specs/022-flow-control-step-policies/spec.md`

Status: Proposed — not started. The release 0.3.0 slice (US02 and the new US07, OD-2026-11-13-11) is planned in
[Release 0.3.0 slice](#release-030-slice) at the end of this file.

*Amended 2026-11-24 (OD-2026-11-24-1, OD-2026-11-24-4):* the rest of the feature (US01, US03 to US06) is in release
0.3.0 too; its plan, as built on the slice, is [Release 0.3.0, the rest of 022](#release-030-the-rest-of-022) at the end
of this file. Where that section and the sections above disagree, it wins.

## Summary

Add four additive IR capabilities (`stepCache`, `earlyReturn`, `failureHandler`, `concurrencyKeys`) and
two API surfaces over existing admission (standalone steps, draft test runs). Every new decision is a
journal event committed with the transition it causes; shared cross-run state (cache entries, semaphores,
handler dispatches) is a set of tenant-keyed PostgreSQL rows changed in documented transactions, after
each path's own locks and before deferred content usage (per-path orders below and in ADR-0056). Every run
creator admits through one permit port (Run creators). No worker protocol change, no new `Error.code`, no new run
status. Concurrency permits follow the accepted [ADR-0056](../../docs/adr/ADR-0056-concurrency-keys-are-durable-permits-granted-by-the-waiters-shard-owner.md);
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
*Amended 2026-11-24 (decision D022-220):* six SDKs (Rust since D022-016), not five; with the revision of 2026-11-13,
60 functional requirements; the rest of the feature adds three proposed ADRs (ADR-0135 to ADR-0137).

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

*Amended 2026-11-24 (decision D022-220):* as built, the scheduler's flow-control passes are in
`crates/scheduler/src/passes.rs` (`gate_grants`, `gate_timeouts`; the store writer, the purge and the dispatcher join
them there), the application side is `crates/application/src/flow_control.rs` with `flow_control_keys.rs` and
`keyed_digest.rs` (there is no `concurrency.rs` or `rate_limits.rs`), and the adapter's gate files are
`crates/infra-postgres/src/execution/readiness_gate.rs` and `gate_queue.rs`. New files of the rest are named in the
plan's last section.

## Transaction boundaries

*Superseded 2026-11-25 (decision D022-259):* this table is the approved original, kept as history. The table that
governs is ["Transactions, as built"](#transactions-as-built-t1-to-t18-and-t31) in the section "Release 0.3.0, the rest of
022"; every row below has a row of the same number there (T11 to T14 are replaced by T26 to T29). The rows of this
table were briefly overwritten by the revision of `6f2cfd81` and are restored here from `d8d3cf79`.

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
| T11 | Grant to the oldest unpaused waiter whose own limit admits it (skip-ahead, no head-of-line blocking); with 019, by (019 priority 1 first, enqueue time asc) among those (D1602) | shard fence (share) → waiter run (`SKIP LOCKED`) → semaphore | holder row, waiter row removed, timer cancelled, `concurrency.acquired`, first ready tasks, outbox | *Revised 2026-11-13: replaced by T26 (ADR-0089, proposed).*
| T12 | Queue timeout | timer path (`apply_if`) → run → `runs` update → semaphore | waiter removed, `concurrency.wait_timed_out`, run `FAILED` (`CONCURRENCY_WAIT_TIMEOUT`) directly as in T8 without a workflow handler phase (it never held a permit); namespace dispatch row when due | *Revised 2026-11-13: replaced by T27.*
| T13 | Step claim with step permit | run → drain fence → session → sorted activity quota rows → sorted semaphores | lease, holder row, journal, outbox | *Revised 2026-11-13: removed by ADR-0089; a gated step is granted before it is `READY` (T26, T28) and the claim takes no semaphore.*
| T14 | Attempt ends | existing completion/failure/expiry order (run → step) → semaphore | existing records + holder row removed, `concurrency.released` | *Revised 2026-11-13: also when a granted step leaves `READY` other than by its lease (T29).*
| T15 | Standalone admission | existing ad hoc order (017 `RunStore::create`) | private definition (reuse or insert), run, receipt, audit, journal, tasks, outbox |
| T16 | Draft test admission | workflow row (share, draft revision check) → existing ad hoc order | private definition, run (`DRAFT_TEST`), pin payloads, `step.mocked` for upstream pins, tasks for `startAt`, receipt, audit, NATS hint |
| T17 | Namespace handler configuration | namespace policy fence (update) | handler row with revision CAS, audit |
| T18 | Erasure of a source run (called by 020) | cache entries by `source_run_id` (update) → obligations of the run | entries and obligations deleted, bytes released; hit provenance and pins left for 020 to act on |

No transaction locks two existing runs. No provider, worker or HTTP I/O happens inside any of them.

*Amended 2026-11-24 (decisions D022-203 to D022-216; ADR-0094 as built):* this table predates the evaluation rounds of
ADR-0094 and the one admission port `Engine::start`; the as-built table that replaces it is in
[Release 0.3.0, the rest of 022](#transactions-as-built-t1-to-t18-and-t31). Row T16's draft workflow row belongs at slot
4b of ADR-0050's order, not before slot 1 (D022-214); the namespace handler policy row of T8 is read without a lock
(D022-208).

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

*Amended 2026-11-24 (decision D022-220):* the catalog has 53 entries on the tree (`crates/domain/src/identity.rs`);
with `cache:invalidate`, `failure-handler:manage` and `step:standalone` it has 56 (task T022-011).

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
  *Amended 2026-11-24 (decision D022-220):* one additive migration per slice, numbered after the latest on the tree
  when it is written (`20270209000100` at `d8d3cf79`; the throughput branch adds `20270210000000` and `20270210000100`),
  after listing the other branches (D1191).
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
what the slice takes from it and what it adds. Rate limits follow the accepted
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
| T20 | Grant from the buffer, by the pass of the waiter's shard owner: the oldest unpaused waiter of the key that its own limit and its other gate admit | shard fence (share) → waiter run (`SKIP LOCKED`) → `runs` update → semaphore when declared → window | grant row, waiter row(s) removed, timer cancelled, `rate_limit.granted` (and `concurrency.acquired`); for a run its first ready tasks and outbox; for a step the step `READY` | *Revised 2026-11-13: replaced by T26.*
| T21 | Queue deadline of a waiting run or step (existing timer pass, `apply_if`) | timer path → run → `runs` update → semaphore when declared → window | waiter removed, `rate_limit.wait_timed_out` (or `concurrency.wait_timed_out` when the permit is what is missing at the deadline); run `FAILED` directly as T12, or the step's failure route | *Revised 2026-11-13: replaced by T27.*
| T22 | A step with node-level `rateLimit` would become ready (inside the readiness transaction: a worker completion, a scheduler pass, a retry timer or an admission) | the path's existing order → window | either grant row, `rate_limit.granted` and the step `READY`; or waiter row, `rate_limit.queued`, queue timer and the step `WAITING` (`RATE_LIMIT`); next to whatever the path already commits | *Revised 2026-11-13: generalized to every gate and readiness source by T28.*
| T23 | Lease recording, by the pass: a grant row of the shard with no lease instant whose step has been leased, or has ended without a lease | shard fence (share) → window → grant row | lease instant copied from the `step.leased` fact, or the grant row deleted (slot returned); no run row is locked or written | *Revised 2026-11-13: locks the grant row only, no window (D022-021); accepts only the granted attempt's `step.leased` committed after the grant (D022-029).*
| T24 | Cancellation, run deadline or other terminal transition of a waiter, or of a granted step never leased | the path's order (run → `runs` update → semaphore when declared) → window | waiter removed and `rate_limit.dequeued`, or the unleased grant row removed; with the path's own records |
| T25 | Purge, in bounded batches | window → grant rows that no acquirer can count any more → the window row itself when it has no waiter and no grant | deletions only |

A release of a permit (T8, T14) is unchanged and never touches a window. The claim (T13) is unchanged: a
step with both node-level gates is granted its slot by T20 or T22 and takes its permit at the claim.

### Run creators (rate limits)

The table "Run creators" above applies as written, with "acquire or queue" read as "grant or buffer, for
each gate the definition declares, through the one port". Two rows differ: a **reset** (018) takes a slot
or is refused with `RATE_LIMIT_SLOT_UNAVAILABLE` and never waits (D022-010; O1168, decided); **Continue-As-New
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

*Added 2026-11-20 (T022-077, D022-064).* The namespace sets them through four quota resources: the count
`rate_limit_live_keys` (window rows of both scopes; 10,000/100,000) and
the policies `rate_limit_per_second` (20/20 until SC-022-029's floor), `rate_limit_per_minute` (6,000/6,000) and
`rate_limit_per_hour` (10,000/10,000), each applied to the declarations of its window length. All four are of namespace
scope, as `concurrency_live_keys` and `concurrency_limit` are: a namespace's value, else the tenant's default, else the
deployment's; the tenant-wide view of the quotas lists them with no single usage (a count belongs to one namespace),
and a namespace's view gives `rate_limit_live_keys` its usage (the namespace's window rows) and the three policies none. The run waiters of a key,
`queued_runs` and the queue timeout are the concurrency gate's resources, shared. Lease recording (transaction T23) runs before the
grants of each tick; the purge (transaction T25) runs in the housekeeping, at most 64 keys and 500 grants a key a call, skipping a
key another transaction holds.

*Revised 2026-11-20 (D022-066 items 1 and 6).* A decision counts the namespace's window rows under the digest versions in
force before it creates one, as for the semaphores, without a namespace lock: decisions that create keys at the same
moment can together pass the ceiling by at most the new keys the other decisions of that moment create (their number
minus one when each creates one key; accepted by the owner, OD-2026-11-20-11). A run's start counts under the namespace's
admission lock; a readiness round under none. When the count leaves fewer rows of room than the new keys a decision
would create, it first reclaims, in its own transaction, windows of its own namespace that no waiter and only grants the
purge may delete keep (FR-022-056's boundary, the purge's own predicate): at most as many as it lacks, oldest update first,
each with at most 64 grants and none of the keys it files, locked `FOR UPDATE SKIP LOCKED` (it never waits, so the lock
order of the decision is unchanged), their grants deleted by a second statement; the windows go at the commit. The rows
of the versions in force are therefore bounded by the ceiling plus that overshoot, and no namespace's admission waits for
the deployment-wide purge, except for a window of more than 64 grants (G022-018).

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
| Reset of a rate-limited source | Slot or refusal (`RATE_LIMIT_SLOT_UNAVAILABLE`, rate first; D022-069) | T022-086 |
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

`refused_reset` of both families counts a refused reset in each family its start declares, not only in the gate that
refused it (D022-069 item 4): a reset refused by its slot while its permit was free counts in `rate_limit_decisions_total`
and in `concurrency_acquisitions_total`; the first refusing gate is the answer's `details.reason`. It counts attempts: a
pending reset whose failing transaction is retried counts again.

Alerts (engine monitor, per namespace): `rate_limit_grant_lagging` (a key has had a waiter and a free
slot for longer than a threshold: the pass is not keeping up or the clock went back),
`rate_limit_queue_saturated` (a key's waiters above 80% of its ceiling). Both get a section in
`docs/runbooks/flow-control.md` next to `concurrency_queue_stuck`. No audit action is added: admission and
grants are journal facts, and policy changes use the existing quota-policy audit.

*Built 2026-11-25 (task T022-082; decisions D022-077 items 4, 6 and 7, and D022-083).* The monitor raises four alerts,
`CONCURRENCY_QUEUE_STUCK`, `RATE_LIMIT_GRANT_LAGGING`, `RATE_LIMIT_QUEUE_SATURATED` and `FLOW_CONTROL_WAITER_ORPHANED`, one
per namespace and group, the group the subject, with the runbook's thresholds (`SAGAT_ALERT_UNCLAIMED_SECS`,
`SAGAT_ALERT_LEASE_GRACE_SECS`, 80% of the effective `flow_control_key_waiters`, none for an orphan). The wait histograms
observe each waiter granted from the queue, from its queueing to its grant; `rate_limit_grant_lag_seconds` observes each
slot the pass grants on its own key, from the key's hint as the pass selected it (or the waiter's queueing if later) to
the grant, an upper bound of the time a slot was free with the waiter present; the waiter gauges by scope and
`rate_limit_unleased_grants` are set by the monitor from the key rows' counters every look.

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

1. Owner gate: O1163–O1168 answered (OD-2026-11-13-16); ADR-0056 and ADR-0088 accepted (OD-2026-11-13-17);
   ADR-0089 and O1169 still open (T022-107). The instance-capability registry of feature 018 (T018-080,
   pull request 98) on `develop` (T022-094).
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
patch release item. Body nodes are out of 0.3.0 by the owner's answer to O1167. No date is implied.

## Revision after the reviews of 2026-11-13

The architecture, security and correctness reviews of the slice at `1a21cbdc` are recorded in
[reviews/2026-11-13/](reviews/2026-11-13/). This section answers them; where it and the sections above
disagree, it wins, and the rows above that it replaces say so. Decisions D022-019..034 in
[research.md](./research.md); ADR-0089 (proposed) for the change to accepted ADR-0056.

### Code facts the revision rests on (checked on `origin/develop` at `a4f6e9db`)

| Fact | Where |
|---|---|
| The claim reads per activity the oldest `READY` steps, at most `limit.min(64)` | `crates/infra-postgres/src/workers.rs` (`CLAIMABLE`, `CLAIMABLE_PATHS`, the bind of `$4`) |
| `step.released` is emitted by the planner in every `settle` | `crates/domain/src/execution/plan.rs`, `command.rs` (`settle`, `apply_then_settle`) |
| `step.retry_requested` sets a step `Ready` | `crates/domain/src/execution/state_effects.rs` |
| The retry pass emits `step.retry_due` for `RETRY_WAIT` steps | `crates/infra-postgres/src/maintenance.rs` (`due_retries`) |
| One function persists every transition and writes the task-ready hint for exactly those three events | `crates/infra-postgres/src/execution.rs` (`persist`) |
| Runs are inserted only by `PgRunStore::create` and the continue-as-new successors | `crates/infra-postgres/src/execution.rs`, `chain_progress.rs` |
| Schedule firing is on `develop` | `crates/application/src/schedule_firing.rs`, `runs.rs` |
| Fleet activation goes through 018's registry; use before activation is refused with the feature's own reason | ADR-0050 decision 9; `specs/018-schedules-bulk-operations/data-model.md` rules 1–5; ADR-0082 |
| Namespace counters are 16 slot rows maintained by a `SECURITY DEFINER` trigger, read-only to `sagat_app` | migration `20261017000100_active_run_counts` |
| `runs` has `UNIQUE (tenant_id, namespace_id, id)` | migration `20261102000000_execution_chains` |
| The budget selector covers `crates/infra-postgres/`, the worker and execution application files and `runs.rs` | `scripts/select-test-targets.py` (`HARNESSES`) |

### Transactions added or replacing earlier rows

| # | Transaction | Locks (in order) | Commits together |
|---|---|---|---|
| T26 | Grant routine (replaces T11 and T20): a key selected by the pass of its home shard; for one waiter, run or step, whatever its gates | waiter run (`SKIP LOCKED`) → key home-shard fence (share) → `runs` update → semaphore(s) → window(s) → `flow_control_counts` and usage (*corrected 2026-11-24, decision D022-077 item 2:* run, fence, keys, as decision D022-045 records and every pass takes them, ADR-0050 decision 7; this row named the fence first) | for every gate the waiter declares, the grant (`concurrency.acquired`, `rate_limit.granted` with the attempt for a step) or nothing; waiter rows removed; timer cancelled; a run's first tasks and outbox, or the step `READY`; the key's next-eligible hint |
| T27 | Timeout routine (replaces T12 and T21), at the waiter's single deadline | timer path (`apply_if`) → run → `runs` update → semaphore(s) → window(s) → counts | the refusing gate's `wait_timed_out` (rate first) and the other's `dequeued`; run `FAILED` as T12 or the step's failure route |
| T28 | Readiness gate at `persist` (generalizes T22 to every gate and every readiness source) | the path's own order → semaphore(s) → window(s) sorted → counts | for each gated step the reducer left `WAITING`: its grants (then `READY`) or its `*.queued` and timer; no task-ready hint for a waiting step |
| T29 | Return of a granted, unleased step's slot and permit (pause, review, cancellation, run end; extends T14 and T24) | the path's own order → semaphore(s) → window(s) | `concurrency.released`, `rate_limit.returned`, holder and grant rows removed, step `WAITING` (pause) or ended |
| T30 | Fleet activation (operator command of 018's registry) | 018's activation order | requirement rows for `flow-control-concurrency-v1` and `flow-control-rate-v1`, the feature's activation row, audit |

The claim (T13) no longer touches a semaphore: a gated step is granted before it is `READY`. Lease
recording (T23) locks only the grant row. The routine never locks a window before a semaphore and never
locks a run after either.

### The grant pass, by key

- Semaphores and windows carry `key_shard` (a stable hash of the key's identity modulo the 64 shards),
  `waiters` and `next_eligible_at`, all maintained under the key's lock; a partial index on
  `(key_shard, next_eligible_at) WHERE waiters > 0` selects due keys.
- `next_eligible_at` is a hint that may be early and is never late: a release (T8, T14, T29) sets it to now;
  a grant that leaves the key full sets it to the instant the oldest counted grant ages out, or to now plus
  the shortest period when an unleased grant holds a slot. *As built (D022-064, D022-065):* a rate grant to a window
  with waiters moves the hint no later than its instant plus the shortest window its waiters count over; a pass that
  grants nothing rests a window until the first instant a grant that counted when the pass began may stop counting for
  one of its waiters (each waiter's own window, D022-009; an unleased grant from the pass's start); the lease pass,
  after its commit, lowers each recorded window's hint to the lease plus the shortest waiting window, one window a
  statement. Every count of a decision is taken at the instant the decision journals. *Revised 2026-11-20 (D022-066
  items 2 to 4):* a window's rest is two statements in one transaction, the row locked if its hint is still the one
  the pass was selected by, then the hint computed by a new statement that sees every grant committed under that lock;
  an unleased grant rests its key from the instant of the statement that selected the key (`selected_at`, at or after
  its hint, at or before every count of the pass); a grant or a recorded lease moves the hint with `least(hint,
  instant)`, which keeps the hint when the instant is unknown.
- Keys are shared between tenants by `crates/scheduler/src/fairness.rs`. A pass visits at most 256 keys and
  makes at most 256 grants per key (the existing per-key bound), each grant its own transaction. *Amended 2026-11-24
  (decision D022-077 item 1):* a pass visits the scheduler's batch of keys (`batch`, 200 by default), never more than
  256 whatever an operator sets: one setting bounds the work of a tick for every pass, and the hard ceiling bounds the
  transactions one tick can start.
- Reservation rule (D022-021): on each gate, a waiter at eligible position `p` is granted only when counted
  holders or grants plus `p` are below its own limit.
- *Strict admission order (2026-11-21, decision D022-073).* Admission adds one condition to the reservation rule: a new
  arrival (`p` is every eligible waiter of the key, all ahead of it) is granted at once only when `p = 0` and the counted
  holders or grants are below its limit; otherwise it queues, and the pass grants in admission order. Eligible: not past
  its deadline, of a live run that is not paused, and below the count of its own window by the lower of its filed limit
  and the current ceiling (decisions D022-071, D022-074, D022-075 item 4). The eligibility is read only when the key has
  waiters and the acquirer's semaphore or window has room, so no budget line moves; a start queued with room lowers the
  key's hint to its statement's instant. *Amended 2026-11-24 (decision D022-076 items 1 to 3):* a start or step queued
  with room writes the hint to its statement's instant whatever the hint was, so a pass in progress on the key does not
  rest it; one queued at a full window lowers the hint to the first instant a counted grant ages out of its own window;
  a start that may not queue (a reset) is granted by the reservation rule (counted plus eligible waiters below its
  limit) and refused otherwise. *Amended 2026-11-24 (decision D022-078 item 2):* within one pass, a waiter the key
  refuses for lack of room holds back every younger waiter of the key whose limit and window are not looser, and a pass
  that held one back while the counts admitted it leaves the key due.
- *The delay strict order costs a new start* that the reservation rule alone would have granted at its admission
  instant `q` (decision D022-073 item 5). Let `I` be the idle pause of the scheduler loop (`SAGAT_SCHEDULER_INTERVAL_MS`,
  500 ms by default; `run_scheduler` sleeps `jittered(I)`, at most `1.2 I`, only after a tick that did no work) and `D`
  the duration of one tick.
  1. At `q` the key is due: its hint is at or before `q` (item 4 of the decision).
  2. The tick in progress at `q`, if any, may have selected its due keys before `q`; it ends by `q + D`.
  3. The next tick starts at once (the previous one did work) or after at most `1.2 I`, and its grant pass selects the
     key, provided the key is among those one pass visits (else one tick more: the two passes of SC-022-017).
  4. In that pass the key's waiters are tried in admission order and the new start is granted under the reservation
     rule: every grant to a waiter ahead moves one from `p` to the count, so `count + p` is what it was at `q` or lower
     (a counted grant aged out, a waiter ahead timed out or was cancelled). It is granted unless a waiter that was not
     eligible at `q` became eligible meanwhile (a resumed run), which is ahead of it by admission order.
  5. So the extra delay is at most `D + 1.2 I + D = 1.2 I + 2 D` (600 ms plus two ticks by default), `2 (1.2 I + D) + D`
     when more keys are due than one pass visits, and never a window. A pass that cannot hold the key's home shard
     (a takeover) adds the takeover, as for every buffered grant (gap G1174).
  The integration tests prove steps 1 and 4 (the key's hint at or before the start's commit, and one pass granting the
  waiter ahead and then the start, in order); steps 2, 3 and 5 rest on `apps/execution-engine/src/loops.rs`.
- *A pass counts as of its selection (decision D022-073 item 2).* Every decision of one pass counts a window's grants as
  of the instant the pass selected the key, so no grant ages out within a pass and a younger waiter is never granted
  before an older one the same rule refused in that pass. Never an over-grant (an earlier instant counts more); a grant
  that ages out during a pass is used by the next one. The queue deadline is checked at the decision's instant.

### Counters (D022-026)

`flow_control_counts (tenant_id, namespace_id, slot 0..15, queued_runs, live_keys)`, slot-8b usage rows, slot
from the run shard (waiters) or the key shard (keys), maintained by `SECURITY DEFINER` triggers on the waiter,
holder, grant and key rows; `sagat_app` has `SELECT` only. A dual-gated run counts once in `queued_runs`.
Ceilings compare the sum of the 16 slots read after the transaction's own rows, as for `active_runs`.

*Amended 2026-11-25 (task T022-105, decision D022-084):* the table holds `queued_runs` only (its slot from the run's
identifier, kept by triggers on the run-scope waiter rows); the live keys stay counted from the key rows under the digest
versions in force, which only a decision that would create a key reads: a counter of keys would be locked in the middle of
a sorted pass over the keys, or at the commit beside the deferred quota flush, where two transactions could wait on each
other. No line of the hot-path budget moves: the sum of the slots is one statement, as the scan it replaces was, and
the budget counts top-level statements and rows only.

### Ceilings, revised

| Resource | Default | Hard ceiling |
|---|---:|---:|
| Run waiters per key (concurrency and rate) | 100, and always below the effective `queued_runs` | 10,000 |
| Step waiters per key | not bounded per key: one per run and gated node | `active_runs` × gated nodes |
| `limit` per `second` until SC-022-029 meets its floor | 20 | 20 |
| `limit` per `second` after it | declared | 100 |
| Keys visited per grant pass | the scheduler's batch (200), at most 256 (2026-11-24, decision D022-077 item 1) | 256 |

### Stack and future sizes of a run creator

A debug build polls every future on the test thread's 2 MiB stack, and each frame holds the futures it creates. The
futures a gate added to every creator's path grew that stack past its end in a batch item's rerun (CI of 2026-11-21,
`batch_actions::the_effect_of_each_use_case_commits_with_the_record_of_its_item`; D022-070). Three futures are boxed
where they are created, and `tests/integration/schedules-bulk-operations/future_sizes.rs` holds each below a bound; a
bound changes here first, with its reason.

| Future | Measured (bytes) | Bound (bytes) | Reason of the last change |
|---|---|---|---|
| `BatchExecutor::step` (an item's use case boxed) | 2,040 | 3,072 | 2026-11-21: was 23,952 with the use case inline |
| `RunService::start` (the run's admission boxed, as its initial reduction already was) | 7,624 | 9,216 | 2026-11-21: was 11,136 |
| `Engine::start` (the gated part boxed; the ungated start is what remains) | 5,496 | 6,144 | 2026-11-21: was 6,256 |

The bounds are the measured sizes and about a fifth to a half more, and each lies below the size it had before its box,
so a change that inlines the future again fails the test, not a CI process. The test that overflowed needs 1,418 KiB of
stack after the boxes, against 2,080 KiB before (bisected with `RUST_MIN_STACK`, 32 KiB steps, on the arm64 debug build).

The run state's values a debug frame copies grow every frame that holds one (D022-102). The gates of a run's start and of
a step's attempts are boxed (`Gates` holds one pointer, an ungated run or step none), so is a node's declaration of its
gates (absent for an ungated node), and `crates/domain/tests/state_sizes.rs` holds the sizes below their bounds; a bound
changes here first, with its reason.

| Type | Measured (bytes) | Bound (bytes) | Reason of the last change |
|---|---|---|---|
| `StepState` | 136 | 160 | 2026-11-25: was 288 with its gates inline (128 on `develop`) |
| `RunState` | 368 | 384 | 2026-11-25: was 520 with its start's gates inline (336 on `develop`) |
| `Node` | 568 | 576 | 2026-11-25: was 704 with its gates' declaration inline (560 on `develop`) |

`scoped_event_storage::receiver::a_dispatcher_quota_refusal_with_held_inbox_keeps_lookback_pending` and its sibling
(integration_workflow_primitives) overflowed the 2 MiB test thread on CI (runs 45637128106 and 45649612535): they needed
2,048 KiB on this branch against 1,920 KiB on `develop` (same bisection), 1,984 KiB with the gates boxed (the node's box
moved nothing on this path), and 1,376 KiB once the test's own scenario future (about 80 KB, which a debug build kept in
several frames of the test thread) is boxed where it is created; that future is held below 4,096 bytes by
`scoped_event_storage::receiver::the_quota_refusal_future_kept_on_the_test_thread_stays_small` (D022-102).

### Hot-path budget of a gated definition

The lines of `tests/integration/claim-transition-efficiency/work_budget.rs` that a gate adds to (statements, transactions,
rows written). A line changes here first, with its reason; the test follows (OD-2026-11-20-8). The plain lines (an
admission 33/1/11, a completion with a next step 50/1/11, one that ends the run 47/1/9, a lease 38/2/5) do not move.

| Line | Budget | Reason of the last change |
|---|---|---|
| A start granted its permit at once | 48/1/15 | 2026-11-20: +1, the fleet activation read at the one admission point (`SELECT capability FROM platform_requirements`, `flow_control_admission`), added by commit 295b158b for decision D022-057 and found by the statement lists of commits 64b6f2d9 (47) and 34b8be41 (48) |
| A start that queues for its permit | 48/1/12 | 2026-11-20: +1, the same activation read |
| A completion that ends a run holding a start permit | 58/1/13 | unchanged since T022-022 |
| A completion whose next step declares `rateLimit` | 60/1/13 | 2026-11-20: +1, whether the step's window exists, so that a new window meets `rate_limit_live_keys` before it is created (T022-077, D022-064); the live count is read only for a new window |
| An admission whose entry step declares `rateLimit` | 44/1/13 | 2026-11-20: +1, the same activation read, and +1, the window's existence (task T022-077) |

*2026-11-20 (D022-066 item 1):* counting window rows instead of windows that still count leaves every line unchanged
(the same statement, without its grants probe). A decision that would be refused by `rate_limit_live_keys` adds two
statements, the reclaim's lock and its delete, and deletes at most 64 grants a window it reclaims; that is a path at the
ceiling, not one of these lines.

*2026-11-20 (task T022-078, architecture minor 6 of its review):* the rate reads move no line, and the budget cannot
see what they add, since it counts top-level statements and rows only: every rate waiter insert or delete runs, inside
its count trigger, a lock of its window row and a `min()` probe of the key's waiters (`oldest_wait_at`); a window update
writes two more index entries (`rate_limit_windows_listing_idx` and `rate_limit_windows_group_listing_idx`); and the
run read `RUN_SELECT`, which `check_database_replay` issues on the admission path, adds two primary-key probes (the
run's grant and waiter rows). Their cost is measured by the benchmark (task T022-089), not by these lines.

The activation read could ride on a statement the admission already issues (the digest key versions it reads later in
the same transaction); that folding is left to the benchmark's work (tasks T022-062 and T022-089), since it moves the
admission point's order of checks.

### Visibility of the rate gate (task T022-078; FR-022-048; decision D022-067)

What a reader sees is read from the rows the gate decides with, without a lock, and never names a key's value.

- **`Run.rateLimit`** (`getRun` and `listRuns`): one lateral probe of the run's grant row (run scope, its primary key)
  and of its waiter row, as `Run.concurrency` reads the permit. `GRANTED` while the run's slot counts at the statement's
  instant `t` (`leased_at > t - per`, the gate's half-open window: at `leased_at + per` exactly it counts no more and the
  field is absent; the journal keeps the grant); `WAITING` while its waiter row exists. `since` is the grant's or the
  queueing's database instant, `limit` the effective limit the row was decided with (for a waiting run read by `getRun`,
  the limit its next grant is decided with, the lower of its filed limit and the current ceiling, decision D022-075 item 3;
  a listing shows the filed limit), `per` its window, `queueDeadline`
  the waiter's deadline. The label follows FR-022-057 (the reveal grant, D022-060).
- **Position and estimate** (`getRun` only, a `WAITING` run; never in a listing): read at one instant `t` by one
  statement. With `L` the waiter's limit (the lower of its filed limit and the namespace's current ceiling of its window
  length, the limit the grant routine decides it with, decision D022-075 item 3), `p` its window, `n` the key's grants that count over `p` at `t` and `a` the
  waiters ahead of it that the pass visits (not past their deadline, their run running or waiting and not paused, in
  the queue's order `(enqueued_at, run_id, node)`). The grant routine also skips a waiter ahead whose own limit is
  below the count at the moment (`admits`); read at a full key that filter would rank every waiter first, so the
  position leaves it out: with the one limit a key has in practice (a second one is warned `RATE_LIMIT_MISMATCH` at
  publication) the waiters ahead are granted first, in that order:
  - `position = a + 1` (1 is next), exact at `t` (D022-014);
  - the routine grants the waiter when `n + a < L`; a grant to a waiter ahead moves one from `a` to `n`, so the sum
    falls only when a counted grant ages out. With `k = n + a - L + 1`: `k <= 0` gives `t` (there is room; the next pass
    grants it); `1 <= k <= n` gives `leased_(k) + p`, where `leased_(k)` is the `k`-th oldest lease instant of the `n`
    counted grants (at that instant the `k`-th one counts no more); `k > n` gives `null` (beyond one window). It is
    `null` too while the run also waits for a concurrency permit. A run-scope window holds only run grants, which count
    from their grant, so no slot ahead is held by an unleased step (D022-014's first case cannot arise here). An
    estimate, never a promise: the pass's lag and later waiters' deadlines move it. It is shown only to a caller with
    the namespace's reveal grant (D022-067 item 8): it is another run's grant instant plus the window. It is `null`
    too when the pass will not grant the run itself (D022-068 item 1): the run is paused, its deadline `d` is at or
    before `t`, or the instant is at or after `d` (the pass grants only while `d > now()`, so at `d` exactly the run
    times out first).
  - Cost: one statement, the `n` counted leases read in order up to the `k`-th through `rate_limit_grants_count_idx`
    (at most the key's limit, 10,000) and the waiters ahead through `rate_limit_waiters_order_idx` (at most the key's
    run waiters, `flow_control_key_waiters`, 10,000 at most). It runs in its own `READ ONLY` transaction with
    `statement_timeout = 2s`; an unavailable database (the timeout included) leaves both fields `null` and is logged
    as a warning, never an error of `getRun`; a corrupt row stays an error (D022-068 item 2). The store returns what the
    statement read and the run service derives the estimate (D022-068 item 7). A run granted between the read of its row and of its place is shown
    `WAITING` with no place.
- **A step's waiting state**: `RuntimeNode.waitingOn` is `rate_limit` for a step `WAITING` with `waiting_kind`
  `RATE_LIMIT` and `concurrency` for `CONCURRENCY` (rate first for a step that waits for both, as its row records).
- **`GET /namespaces/{namespaceId}/rate-limit-keys`** (`ops:read`): every window row of the namespace (the rows the
  quota usage of `rate_limit_live_keys` counts; the gate leaves out a forcibly retired version's rows, which drain,
  D022-068 item 4), in the order of a new index `rate_limit_windows_listing_idx (tenant_id, namespace_id,
  COALESCE(oldest_wait_at, 'infinity'), pool, limit_scope, group_name, key_digest)` and, for one group,
  `rate_limit_windows_group_listing_idx (tenant_id, namespace_id, group_name, COALESCE(oldest_wait_at, 'infinity'),
  pool, limit_scope, key_digest)`: oldest wait first, keys nobody waits for last. `oldest_wait_at` is kept on the window
  row by its count trigger, as `concurrency_semaphores.oldest_wait_at` is (D022-058). Per key, at the page's instant
  `t`: `waiters` (the counter); `grantsInWindow`, the grants that count over the row's longest period (`unleased`, or
  `leased_at > t - max_period_ms`); `unleasedGrants`; `nextSlotAt`, when somebody waits and no grant is unleased, the
  first instant a counted grant stops counting for one of the first 256 waiters in the queue's order
  (`GRANTS_PER_KEY_PER_PASS`, the most one pass grants a key; D022-068 item 3): for each window `p` among theirs, the
  oldest lease with `leased_at > t - p` (the same half-open window as `lease + p > t`, written so the index serves it),
  plus `p`, and the earliest of those; `null` when nobody waits, when a grant is unleased, or when no grant counts for
  any of those windows (there is room; the next pass grants). `queuedRuns` is the one count of `QUEUED_RUNS_SQL`. The same
  protections as the concurrency listing: labels only for a caller with the namespace's reveal grant; for any other
  caller `oldestWaitSince` and `nextSlotAt` truncated to their minute toward the past (they equal or derive from a run's
  own instants, D022-060, D022-061); a cursor sealed for the namespace under the digest key version in force, of its
  own kind (`r1`: a concurrency listing's cursor is a `400` here and the reverse); a `400` for an altered, foreign,
  other group's or retired cursor; a `422` for a group that is no group name; one `REPEATABLE READ READ ONLY` snapshot a
  page and `statement_timeout = 2s` (a `503`). Page cost: at most `limit + 1` (201) probes of the listing index, and for
  each key its counted grants through `rate_limit_grants_count_idx`, at most the hard ceiling of its longest window
  (10,000), plus for `nextSlotAt` at most 256 waiter rows through `rate_limit_waiters_order_idx` and one grant probe
  for each of at most three periods, so a page is bounded by its own keys and never by the namespace's, the purge's
  backlog or a key's whole queue (pinned by `rate_rest::a_page_of_rate_keys_reads_only_its_own_keys`, the `EXPLAIN`
  twin of the concurrency listing's).

### Admission port and checks

- The gate runs inside `PgRunStore::create`; the continue-as-new inserts of `chain_progress.rs` assert that
  their definition has no run-level gate. `scripts/check-architecture.py` gains a rule that `INSERT INTO runs`
  appears only in those two files (T022-093).
- The hot-path budget gains a line for a plain admission in Phase 2, before T022-022 (T022-091); T022-076
  moves to Phase 2; the selector also selects the budget suite for `crates/scheduler/`,
  `crates/domain/src/execution/` and `crates/application/src/{concurrency,rate_limits,flow_control}.rs`
  (T022-092).

### Security additions

- Keys: identity-only keys refused, `secret` keys refused, `pii` keys warned, labels `null` for callers who
  may not disclose the key's source (FR-022-056, FR-022-057); per-key digest rotation with the version in
  `flow_control_key_versions` (FR-022-053, FR-022-056).
- Every foreign key to `runs` from a flow-control table is `(tenant_id, namespace_id, run_id)` →
  `runs (tenant_id, namespace_id, id)`.
- Position and estimate are computed only by the single-run read; the list filter `group` accepts both
  explicit and derived group names.
- A grant does not re-authorize the initiator (FR-022-060).

### Failure and recovery, added

| Failure | Expected state | Handling and test |
|---|---|---|
| 64+ steps of key A wait; one step of key B ready | B leased by the next poll | SC-022-024, T022-099 |
| A readiness source not listed | The reducer still makes a gated step wait | One test per source; `verify_run` (T022-096) |
| Dual-gated head refused by its other gate | Younger waiters granted only while room remains for it | T022-097 |
| Dual-gated timeout | One `wait_timed_out`, one `dequeued` | T022-098 |
| Waiter of a terminal run | Alert `flow_control_waiter_orphaned`, skipped | T022-097 |
| An engine without the capability is live | Activation refused; use refused | SC-022-027, T022-094 |
| Pause of a run with a granted, unleased step | Slot and permit returned; asked again on resume | T022-100 |
| Database clock set back | No grant until it catches up | the counted grant instants moved one window ahead (T022-080, D022-072) |

### Constitution Check (the revision)

| Principle | Result | Evidence / future gate |
|---|---|---|
| I — durable atomic transitions | Pass | T26–T30; readiness, grant and journal in one commit; invariant 11 |
| II — pure domain | Pass | The readiness mapping is a reducer rule; locks and the clock stay in the adapter |
| IV — pinned versions | Pass | Registry activation (ADR-0050 decision 9); no mixed fleet runs a gate half |
| V — tenant boundary | Pass | FKs include the namespace; classified keys; labels gated by disclosure |
| VIII — failure tested, honest | Pass | The added failure rows; G1184..1186 |

### Size, revised

80 tasks: 38 existing, 25 added with the slice, 17 added by the review (T022-091..107). Estimate: 26 to 32
pull requests, and the registry of feature 018 must be on `develop` first. No date is implied.

## Release 0.3.0, the rest of 022

Added on 2026-11-24 for OD-2026-11-24-1 (the whole feature in release 0.3.0), with the design questions delegated
(OD-2026-11-24-4). Proposed; nothing of it is built. It plans US01, US03, US04, US05 and US06 and tasks T022-011,
T022-057 and T022-063 on the engine as the slice built it. Where it and the sections above disagree, it wins. The
decisions are D022-200 to D022-221 in [research.md](./research.md); the design is in three proposed ADRs:
[ADR-0135](../../docs/adr/ADR-0135-a-step-cache-hit-is-a-journaled-completion-decided-before-the-gates.md) (cache),
[ADR-0136](../../docs/adr/ADR-0136-failure-handlers-run-between-the-stop-and-the-terminal-commit.md) (failure handlers,
early return) and
[ADR-0137](../../docs/adr/ADR-0137-standalone-and-draft-test-runs-are-ordinary-runs-of-private-definitions.md)
(standalone steps, draft test runs), with dated amendments to ADR-0050 decision 7, ADR-0056 decision 2 and ADR-0094
decision 4.

*Revised 2026-11-25 after the reviews of `14b9e40b` (decisions D022-222 to D022-240; rulings in
[reviews/2026-11-25/](reviews/2026-11-25/)):* the rows and tables of this section are revised in place, each change dated;
a new proposed [ADR-0146](../../docs/adr/ADR-0146-every-end-of-a-run-goes-through-the-reducer.md) records the routing of
every end of a run through the reducer.

### Code facts the rest rests on (checked on this branch at `d8d3cf79`)

| Fact | Where |
|---|---|
| Every creator on the tree starts a run through `Engine::start`; a domain start call elsewhere is refused | `crates/application/src/execution.rs` (`start`), `scripts/check-architecture.py` (`run_start_errors`), D022-049 |
| The planner asks `Evaluation::Gates` once it has nothing else to emit, and `Evaluation::StartGates` for a released delay or a reset | `crates/domain/src/execution/plan.rs`, ADR-0094 |
| `persist` locks the active-run counter row of a commit that ends a run with a gate event, then gives back gates, then writes outputs to `payloads` `ON CONFLICT DO NOTHING` and charges logical output bytes per run (order corrected 2026-11-25, D022-258) | `crates/infra-postgres/src/execution.rs` (`persist`, `charge_logical_output_bytes`) |
| The task-ready hint is written for `step.released`, `step.retry_due` and `step.retry_requested` of a node that declares no gate, or of a step `READY` | `crates/infra-postgres/src/execution.rs`, the `StepReleased` arm near lines 5692 to 5703 |
| The collection finish and the loop finish write `run.finished` and the `runs` update themselves | `crates/infra-postgres/src/primitives.rs` near lines 3482 to 3510; `loop_progress.rs` near lines 764 to 785 (G1197). *Revised 2026-11-25 (decision D022-232), the whole list:* `primitives.rs` near 3495, 4026 to 4045 (a direct `task.ready` outbox and `run.finished`) and 4061; `loop_progress.rs` near 778 and 3409 to 3449; `chain_progress.rs` 2246 to 2248 (allow-listed, ADR-0146 decision 2) |
| A stopped run drains, a second stop is refused, a stopped run reads `CANCELLING`, `check_finish` refuses an end with an active step | `crates/domain/src/execution/plan.rs:306-308`, `:523`, `:602-606`; `state.rs:4636-4639`, `:433-434`, `:5133-5163`; `crates/application/src/runs.rs:3344` (added 2026-11-25) |
| While paused the planner is idle apart from the gates of steps made ready by the retry pass or an operator | `crates/domain/src/execution/plan.rs:101`, `:309-316` (added 2026-11-25) |
| Publication locks `workflows FOR UPDATE` then the namespace's secrets; activation locks the row first | `crates/infra-postgres/src/definitions.rs:618-642`, `quotas.rs:306-312`, `database/admission.rs:209`, `versioning.rs:534-537` (added 2026-11-25) |
| 017's `create` retains the private definition with its workflows quota lock before the slot-4a locks | `crates/infra-postgres/src/execution.rs:1531` against `1619-1640` (added 2026-11-25) |
| Secret-access and membership lock functions exist; keyed digests have the purposes `Concurrency` and `RateLimit`; namespaces may require publication approvals | migration `20261126000000_authority_read_locks`; `crates/application/src/keyed_digest.rs`; `publication_policies.required_approvals` (migration `20261016000000_versioning`) (added 2026-11-25) |
| `payloads (tenant_id, digest)` is content-addressed, append-only and referenced by foreign keys | migration `20261009000100_execution`, `20261029000000_collection_progress` |
| The gate pool `ORDINARY` is a literal at about thirty sites | `crates/infra-postgres/src/execution/{readiness_gate.rs,gate_queue.rs}`, `maintenance.rs`, `event_codec.rs`, `crates/application/src/flow_control_keys.rs`, two migrations' defaults |
| The permission catalog has 53 entries | `crates/domain/src/identity.rs` (`ALL: [Self; 53]`) |
| `RESET_SOURCE_TEST_RUN` is in 018's contract, not in code | `tests/contract/schedules-bulk-operations/errors.rs` |
| The run origins are `AD_HOC_STEP`, `AD_HOC_WORKFLOW`, `CONNECTION_PROBE` (and null) | migration `20261125000000_database_ad_hoc` |
| Namespace environments are `dev`, `staging`, `prod` | migration `20261008000000_identity` |
| Features 019, 020, 021, 024, 025, 026, 028, 030 and 031 are not on the tree; 016, 017 and 018 are | `specs/`, `crates/` |

### The order of a commit

*Rewritten 2026-11-25 (decisions D022-255, D022-259; ADR-0094 is the authority).* Every transition on an existing run, as
built:

1. **Entity locks**: chain, then run (the statement that locks the run also reads the namespace handler configuration
   by a left join, `FOR UPDATE OF runs`, so the configuration row is not locked); a shard fence only where a pass
   already holds one (a leaf lock).
2. **The rounds**, each asked once the planner has nothing else to emit:
   1. the lookup round (`Evaluation::CacheLookup`, ADR-0135): the secret-access and membership locks of the step's
      secret ids (shared, ascending), the activity's binding advisory lock (shared), then the cache scope, the
      tombstones and the entry, `FOR SHARE`; a hit journals `step.cache_hit` and `step.completed`, a miss
      `step.cache_missed`, an over-limit hit fails the step `RUN_DATA_LIMIT`;
   2. the gates round (`Evaluation::Gates`): semaphores, then windows, each sorted; a step a hit or a pin completed is
      not asked;
   3. rounds asked where the planner reaches a node: `Evaluation::EarlyResult`, `Evaluation::Child`,
      `Evaluation::Output`; a later lookup round for a cacheable step released after the gates round (ADR-0135
      decision 7).
3. **`persist`**: the active-run counter row when the commit ends the run, then the give-backs (sorted, under the same
   key order), then the rest: the `runs` row, journal, steps, tasks, timers, outbox, projections, hit provenance rows,
   the early result's columns, the handler phase's columns, then the namespace dispatch insert (a recorded deviation
   of ADR-0050 decision 7), followed only by the run's own `run.finished` row (corrected 2026-11-26, D022-280: the
   insert is not last); the task-ready hint only for a step `READY` when the commit ends.
4. **Usage**: quota, rate and usage rows and the deferred content flush (slot 8b).

The counter row after cache rows makes no cycle: deciders lock cache rows shared only, and no exclusive taker of a cache
row (T3, T4, T5, T18, the expiry update) takes the counter row. Admissions follow the slots of ADR-0050 decision 7 as
amended: slot 5 carries a cacheable admission's lookup locks, and slot 7 is run-scope gates, cache scope, cache entry,
step-scope gates. A domain property test checks that no progress emits a filed gate decision and `run.finished` in one
commit (D022-234).

### Transactions, as built (T1 to T18, and T31)

Replaces the table "Transaction boundaries" above for every row it lists (that table is marked superseded); T19 to T30
stay as the slice built them. *Rewritten 2026-11-25 (decision D022-259):* every row below is written whole with the
rulings of both reviews; this is the table that governs.

| # | Transaction | Locks (in order) | Commits together |
|---|---|---|---|
| T1 | Lookup round, in the commit that releases a cacheable step (a worker completion, a pass, a timer, a signal, an approval, a finished child, an admission) | the path's entity locks (chain → run; a fence only on a pass; no step or `worker_sessions` row locked before the round) → the secret-access and membership locks of the step's secret ids (shared, ascending; slot 5 at an admission) → an unlocked pre-read of the entry's `producer_subject` → that subject's membership lock and the activity's binding advisory lock (shared) → cache scope (`FOR SHARE`) → the scope's and key's tombstones (read) → cache entry (`FOR SHARE`; the producer re-checked, a change `unauthorized`) | `step.cache_hit` + `step.completed` (attempt 0; the payload read and passed through the outputs path, no new payload row; the hits of one round counted cumulatively against the run's retained bytes) and the hit provenance row; or `step.cache_missed {outcome}` with the key digest and version on `steps.cache_lookup`; or the step failed `RUN_DATA_LIMIT` through its route; then the successors' rounds; the hint only for a step `READY` when the commit ends |
| T2 | Miss completed | the completion's order (entity locks → rounds → `persist` → usage) | the completion's records + a store obligation for the outcomes `miss`, `expired`, `invalidated` (payload by reference, `ON DELETE RESTRICT`; the key digest and version copied from the step; the attempt's lease instant); a full pending-obligations ceiling skips it, counted |
| T3 | Store writer (a pass of `crates/scheduler/src/passes.rs`; no lease, no fence) | store obligation (`FOR UPDATE SKIP LOCKED`; its one column `UPDATE` grant guarded by an `ALWAYS` trigger) → scope row (inserted when absent, then `FOR SHARE`) → the scope's and key's tombstones (read) → a dead entry deleted under its lock → entry insert (`ON CONFLICT DO NOTHING` against a second writer) → the one usage row of entries and obligations | entry with `created_at` = the producing attempt's lease instant and its expiry from it, obligation deleted; skipped and counted when that instant is at or before the scope's or the key's invalidation, or when a count refuses |
| T4 | Invalidate by scope or node | scope row (inserted when absent, only for a scope a published version can derive, else `404`; `FOR UPDATE`) → the one usage row | `invalidated_at = clock_timestamp()` read after the lock, revision, audit |
| T5 | Invalidate by key, purge batch | the store obligations of the scope and key digest (invalidation by key; `FOR UPDATE`) → scope row (`FOR SHARE`) → the key's tombstone (upsert, `invalidated_at` after its lock; invalidation by key) → entries (`FOR UPDATE`, `SKIP LOCKED` for the purge) → the one usage row | the tombstone, deletions of entries and of those obligations, counts, audit (invalidation only); `entriesDeleted` null for a caller who may not disclose the key's locations |
| T6 | `earlyReturn` reached | entity locks → rounds (`Evaluation::EarlyResult`) → `persist` → usage | result payload, `runs.result_digest`, `result_source`, `result_available_at`, `result.returned`, `run.result_returned` outbox (none for a test tree), `next` scheduled |
| T7 | Would finish with a handler | the stop's entity locks → rounds → `persist` → usage | `failure_handler.started` (the stop moved into `pending_terminal_status` and cleared), `failure_handler_state = RUNNING`, the handler timer (`timeoutMs`, its ceiling), the region's first steps decided by the rounds (the runs of its `child` nodes are handler-tree runs) or the derived `onFailure~h` child's creation; the start's permit kept; no `run.finished` |
| T32 | Handler draining (added 2026-11-25) | the stop source's entity locks → `persist` → usage | `failure_handler.draining {cause}` (one cause; a further stop request while draining is refused `409`, D022-261), `failure_handler_state = CANCELLING` or `TIMING_OUT`, the region's leased attempts asked to stop, the `onFailure~h` child's cancellation requested; overrides a pause; the run deadline is not a source |
| T8 | Handler ends, or a run finishes (also a chain's finalization, for the dispatch) | the run's row locked by the statement that reads the handler configuration by a left join (`FOR UPDATE OF runs`) → rounds (none asked on an ending commit) → `persist`: the active-run counter row → the start's semaphore and the region steps' gates given back, sorted (a run's rate slot counts by time and is never given back) → the journal, the `runs` row, the deliveries → the dispatch insert (`lineage_first = true ... ON CONFLICT ... WHERE lineage_first DO NOTHING`, then, when it inserted nothing, a `SUPPRESSED_LINEAGE` row) → the `run.finished` journal row, appended after them with its `namespace_dispatch` detail (D022-260), last before usage → usage | `failure_handler.finished`, the handler timer cancelled, `concurrency.released`, `run.finished`, run-finished deliveries, the dispatch row of a qualifying run (always inserted: live, or suppressed and counted `skipped_lineage`), outbox; a recorded deviation of ADR-0050 decision 7, its waits leaf waits |
| T9 | Handler dispatch (a pass, shard-fenced) | first transaction: dispatch row (`FOR UPDATE SKIP LOCKED`) → the namespace dispatch budget checked (beyond it: deferred by `next_attempt_at` alone, alerted, no attempt counted, D022-264), else attempts and `next_attempt_at` counted, committed; then: lease row (leaf, `FOR SHARE`) → slot 3 dispatch row (locked inside `RunStore::create` through a `NewRun` field) → 3R source run identity (`KEY SHARE`) → the handler workflow's head read without a lock → 6 the run through `Engine::start` → 7 its gates → 8a the dispatch row's update → 8b | handler run (`handler_tree`, `handler_kind = DISPATCHED`, `handler_for_run_id`, its lineage), dispatch `ADMITTED`; a definition that continues as new fails the attempt `CONTINUE_AS_NEW_REFUSED` (D022-263); `DEAD` after 12 counted attempts; a redrive turns a `DEAD` or `SUPPRESSED_LINEAGE` row `PENDING` (a suppressed one keeps `lineage_first = false`) |
| T10 | Admission of a run with start gates (every creator) | as built (D022-049): slots 1 to 6 → 7 semaphores then windows → 8b | run, holder or waiter rows, `concurrency.*` and `rate_limit.*`, counters |
| T11 to T14 | — | replaced by T26 to T29 (ADR-0089, as built) | — |
| T15 | Standalone admission (`Engine::start`) | 2 receipt → 3 018 business-id row (when given) → the retained private definition with its workflows quota lock, where 017's `create` takes it (before 4a; a deviation inherited from 017) → 4a 017 namespace locks, and the namespace's draft-executions advisory key shared, then the publication policy read (`required_approvals`, `draft_executions_enabled`, `prod`; D022-269) → 4b connection heads (a database node) → 5 secrets, the lookup's secret-access locks, the unlocked pre-read of the entry's producer and its membership and binding locks of a cacheable node → 6 run → 7 run-scope gates → cache scope → cache entry (the producer re-checked) → step-scope gates (pool `ORDINARY`) → 8b | private definition, run (`STANDALONE_STEP`), receipt, audit, journal, tasks, outbox; a refusal before any row when review is required or in `prod` without the setting |
| T16 | Draft test admission (`Engine::start`) | 2 receipt → 3R `fromRun` sources (`KEY SHARE`, ascending id) → the draft row read without a lock (its `draft_revision` compared) → the retained private definition (an immutable copy of what was read) with its workflows quota lock → 4a 017 namespace locks, the namespace's draft-executions advisory key shared, and the publication policy read → 5 secrets, and a cacheable entry step's lookup locks (with the producer's pre-read) → 6 run (`DRAFT_TEST`, `test_tree`, `start_at`), pin payload rows, `test_run_pins` rows → 7 gates (pool `DRAFT_TEST`; cache rows of its own private version only) → 8b | the above, `step.mocked` and `step.routed` (no activation) for upstream pins, `run.started` with `start_at`, tasks for `startAt`, receipt, audit, NATS hint |
| T17 | Namespace handler configuration | `sagat_lock_namespace` (the namespace fence, never the `namespaces` row) → handler row (revision compare and set) | handler row, audit; a new or re-enabled handler refused while the pending dispatches are at their ceiling |
| T18 | Erasure of a source run (022-side port, called by 020) | the run's store obligations (`FOR UPDATE`) → cache entries by `(tenant_id, namespace_id, source_run_id)` (`FOR UPDATE`) → the one usage row | obligations and entries deleted, counts; hit provenance and pins kept and listed by source |
| T31 | Collection or loop finish, routed through the reducer (ADR-0146) | the path's entity locks (chain → run → the scope's rows) → rounds → `persist` (counter row, give-backs, the rest) → usage | `collection.finished` or the loop's events, the successor's release decided by the rounds, or `run.finished` with T8's records; every site of the code facts row routed |

No transaction locks two existing runs for update: T9's source run is a `KEY SHARE` identity guard (ADR-0039), as a
rerun's is. No provider, worker or HTTP I/O happens inside any of them.

*Revised 2026-11-25 after the third reads (decisions D022-260 to D022-270):* rows T1, T8, T9, T15, T16 and T32 above
carry the third rulings.

### Run creators added

| Creator | Port | Pool | Conformance case |
|---|---|---|---|
| Standalone step (`POST .../step-executions`; `database-executions` on the same path) | `Engine::start` | `ORDINARY` | task T022-221 |
| Draft test run (`POST .../test-runs`) and every child of a test tree | `Engine::start` | `DRAFT_TEST` (from `runs.test_tree`) | task T022-221 |
| Namespace handler dispatch | `Engine::start` | `ORDINARY` | task T022-221 |
| Derived `onFailure~h` child | `Engine::start` (the child path) | the parent's | task T022-221 |
| A run started by a `child` node of a handler region, or by a dispatched handler's own `child` node (added 2026-11-25, D022-243) | `Engine::start` (the child path) | the parent's; `handler_tree = true` | task T022-221 |

### Capabilities and activation (D022-206)

| IR or surface capability | Switch (per process, rollback) | Registry capability |
|---|---|---|
| `stepCache` | `SAGAT_FLOW_CONTROL_STEP_CACHE_ENABLED` | `step-cache-v1` |
| `earlyReturn` | `SAGAT_FLOW_CONTROL_EARLY_RETURN_ENABLED` | `early-return-v1` |
| `failureHandler` | `SAGAT_FLOW_CONTROL_FAILURE_HANDLER_ENABLED` | `failure-handler-v1` |
| standalone steps | `SAGAT_FLOW_CONTROL_STEP_EXECUTIONS_ENABLED` | `step-executions-v1` |
| draft test runs | `SAGAT_FLOW_CONTROL_DRAFT_TEST_RUNS_ENABLED` | `draft-test-runs-v1` |

Each registry capability is required of `CONTROL_PLANE` and `EXECUTION_ENGINE`. `failure-handler-v1` and
`early-return-v1` cannot be activated before the finish-path routing (T31) is in the binary.
*Manifest, added 2026-11-25 (decision D022-256; supersedes the build constant of the serving rule below):* each
capability has a manifest of the artefacts it needs (the event types the codec decodes, the `Evaluation` variants, the
migrations, the routes, the named acceptance test present and not ignored); a test fails when the capability's serving
constant is true while any artefact is missing (task T022-235). Rollback order: `platform_requirement_remove`, then the
switch; a switch gates new use only, never a phase or an obligation in flight.
*Serving rule, added 2026-11-25 (decision D022-236):* a binary lists a registry capability only once the last behaviour
slice of it has merged (for `step-cache-v1`, `early-return-v1` and `failure-handler-v1`, the routing of ADR-0146 too),
with a test per capability that a binary of an earlier slice does not list it (task T022-235). Rollback is switching a
capability off, never a binary downgrade, once a journal holds its events (D022-239). The switch names follow
`SAGAT_FLOW_CONTROL_CONCURRENCY_KEYS_ENABLED`; the task that adds them may rename them, and this table changes first.

### Authorization (T022-011)

*Reworded 2026-11-25 (decisions D022-244, D022-258); the first text said a handler run resets "under its publisher's
authority", which D022-228 and D022-244 withdraw.* The table "Authorization" above stands. The catalog grows from 53 to
56 permissions, and the built-in roles gain `RunRecovery` (`run:read`, `run:start`, `run:operate`; no publication,
approval, version or secret permission; a subject whose only role it is may not approve a publication or decide an
approval). Test runs need `definition:write` and `run:start`. A handler that recovers calls the public API with a
`RunRecovery` credential given to it as a secret, audited as that subject; no run holds its publisher's rights. Machine
roles receive none of the new permissions. Declaring a namespace-scope cache name and the workflows that may write it is
an administrator's setting of the namespace (D022-254).

*Added 2026-11-25 (decision D022-266): what the `RunRecovery` role may do, in a table of its own beside the
authorization table above (the ruling said "a column"; the rows differ, so it is a separate table: corrected
2026-11-26, D022-285). Revised 2026-11-26 (decisions D022-272, D022-274): the "bound namespaces only" of each row is
enforced by the principal (see "Revision of 2026-11-26 after the re-reads" below), and the binding's own routes are
added.*

| Operation | `RunRecovery` |
|---|---|
| Read runs and the definitions they run (`run:read`, `definition:read`) | ✓ in its bound namespaces only; elsewhere the opaque `404`, audited and counted |
| Start a run, a reset, a rerun (`run:start`, `run:operate`) | ✓ in its bound namespaces only; a reset or rerun of a definition that continues as new refused `422` (D022-279); *2026-11-27 (D022-310):* a start too, each of them a handler-tree run |
| Redrive a dispatch | – (`403`, `RECOVERY_ROLE_MAY_NOT_REDRIVE`, inside its binding; outside it the `404` first) |
| Approve a publication, decide an approval, publish, change a version, use a secret | – (`403`, `RECOVERY_ROLE_MAY_NOT_APPROVE` for the two approvals) |
| List, bind and unbind `RunRecovery` subjects of a namespace (`membership:manage`: the administrator only) | – |

The role's namespaces are an administrator's audited setting, as worker bindings are; the role cannot be assigned until
`failure-handler-v1` is activated; a migration widens `memberships_role_valid` (its `down` refuses while such a
membership exists); the OpenAPI role enum is widened in the same change.
~~*Revised 2026-11-25 (decisions D022-227, D022-228):* no run holds its publisher's operator rights: a handler that
recovers calls the public API with a machine identity given to it as a secret, audited as that identity.~~ *(Struck
2026-11-25, decision D022-270: superseded by D022-242 and D022-244; the paragraph above, reworded, governs.)* Enabling draft
executions in a namespace that requires approvals is a publication-policy change, under that route's permission and
audited.

### Revision of 2026-11-26 after the re-reads

The security and architecture reviewers re-read the third-read fixes (`1276d5b7`); decisions D022-272 to D022-285 in
[research.md](./research.md), and where each finding lives in
[reviews/2026-11-26/design-fixes-responses.md](reviews/2026-11-26/design-fixes-responses.md). The rows above that
changed say so.

**The namespace scope of a principal (D022-272).** Code facts, checked at `1b21a101`: `Principal` holds a tenant, a
subject and a role (`crates/domain/src/identity.rs:742-785`); `admit` checks permissions before it resolves the
namespace (`crates/application/src/api_admission.rs:274-324`); 44 sites of `crates/application/src` authorize
`run:read`, `run:start`, `run:operate` or `definition:read` with no namespace; the upload of a run input authorizes
`run:start` and the tenant only (`crates/application/src/runs.rs:973-984`, `crates/api-http/src/blobs.rs:118-122`).
The design:

| Part | Rule |
|---|---|
| `NamespaceScope` on `Principal` | `Tenant` for every role but a namespace-bound one; `Bound(set)` for `RunRecovery` (`Role::is_namespace_bound`); the set built only by `Membership::principal()` from the rows read in the membership statement; no row, or `failure-handler-v1` not active: an empty set |
| `Permission` scope class | namespace or tenant, by an exhaustive `match` without a wildcard arm |
| `Principal::authorize(permission)` | unchanged for a `Tenant` principal; refuses every namespace-scoped permission to a `Bound` one (`NamespaceScopeRequired`, `403`) |
| `Principal::authorize_in(namespace, permission)` | the scope first (outside: `OutsideNamespaceScope`, the opaque `404`, audited and counted), then the role (`403`) |
| The resolvers | `require_namespace`, a new `require_namespace_slug` (called by `RunService::namespace` and every slug lookup), `resolve_run`, `resolve_workflow`, `resolve_request`, `resolve_approval_request` and the slug target of `admit` take the operation's permissions: the role first (`403`, before any lookup, as today), the lookup (`404`), then `authorize_in` |
| A principal rebuilt from a stored member | through `Membership::principal()`, so it carries the scope (the reset completion, `crates/application/src/reset_completion.rs:361`) |
| A write by a bound subject | its binding row `FOR SHARE` in slot 4a after the memberships (ADR-0050 decision 7 as written 2026-11-26), so a revocation that commits first refuses it *(2026-11-27, D022-297, D022-314: through `public.sagat_lock_run_recovery_binding`, after the memberships where the path takes them)* |
| Evidence | domain tests of the scope (T022-256); the battery of every route as a `RunRecovery` subject (T022-257); the fitness rule (T022-258); the slot-4a lock and the replay after a revocation (T022-259) |
| The role-only check (added 2026-11-26, D022-287) | `Principal::check_role_before_lookup(permission)`, ~~returning `()`~~ returning `Result<(), IdentityError>`, `#[must_use]` *(corrected 2026-11-27, D022-304)*, called only in `authz.rs` and `api_admission.rs`; every other `.grants(` on an allow list by enclosing function; `Membership::restore` only in `crates/infra-postgres/src/identity.rs` |
| A pending reset (added 2026-11-26, D022-288) | completed with the re-read membership's principal and `authorize_in(entry.namespace_id, ..)`; ~~`AppError::OutsideNamespaceScope` maps to `AUTHORITY_REVOKED`~~ *(superseded 2026-11-27, D022-301: the scope refusal is `AppError::NotFound`; `complete_reset` checks the scope first and answers `StateConflict(AUTHORITY_REVOKED)`)*; `recheck_requester` reads the binding row `FOR SHARE` *(through the lock function, D022-297)* |
| A role change (added 2026-11-26, D022-286) | ~~removes the subject's bindings (an `AFTER UPDATE OF role` trigger)~~ *(superseded 2026-11-27, D022-298: refused by a restricting foreign key while the subject has a binding)*; a bind reads the role under the membership's lock |
| A miss and a denial (added 2026-11-26, D022-294) | for a bound principal, a namespace by id or slug is checked against the scope before any lookup; a run, workflow or request miss and a hit outside the scope give the same denial record, its work off the response path |
| From slice 2 (added 2026-11-26, D022-291) | the role is namespace-bound with an empty scope; the widening migration lands with T022-256; the manifest of `failure-handler-v1` names the scope's artefacts |

The binding is read on every request: the `Caller` extractor authenticates each request
(`crates/api-http/src/extract.rs:52`, `:88`) and the membership is read in one statement with no cache
(`crates/infra-postgres/src/identity.rs:362-377`). A replay is checked again: the start route admits before the use
case (`crates/api-http/src/routes/execution.rs:474-480`) and `start_with` resolves its namespace (`runs.rs:1391`)
before every receipt lookup (`:1424`, `:1699`, `:1768`). An open event stream authenticates its subject again every
30 seconds and reads the run with that principal on each poll (`crates/api-http/src/streams.rs:50`,
`crates/api-http/src/routes/execution.rs:1385-1400`); its maximum lifetime is 15 minutes (`streams.rs:49`), so a revoked
binding ends it within one re-check once `RunService::get` checks the scope (D022-296).

**The binding's table and routes (D022-273, D022-274, D022-276).** `run_recovery_namespaces` follows `worker_bindings`
*(not so, corrected 2026-11-27, D022-297: `worker_bindings` grants UPDATE; the row lock goes through a function)*
(grants SELECT, INSERT and DELETE; composite foreign keys to `namespaces` and to `memberships`, `ON DELETE CASCADE`; the
subject CHECK; the quota triggers). `listRunRecoveryBindings`, `bindRunRecovery` and `unbindRunRecovery` need
`membership:manage`. A trigger on `memberships` refuses the role `run-recovery` until `failure-handler-v1` is active.

**The overdue pass (D022-278).** The pass (`deadlines`, `crates/scheduler/src/passes.rs`, `stop_run(.., TimedOut, ..)`
at `:6684` today) calls `enforce_run_deadline`, a no-op for a run in its handler phase; `stop_run(TimedOut)` is
unchanged.

**Continue-as-new under a handler tree (D022-279).** A reset or rerun whose source is a handler-tree run or whose
principal holds `RunRecovery` is a handler-tree run; of a definition that continues as new it is refused `422`. The
defence in `refuse_gated_successor` (`crates/infra-postgres/src/chain_progress.rs:229`) ends a run that reaches the
boundary `FAILED` with `CONTINUE_AS_NEW_REFUSED`, as `CHAIN_LIMIT` is ended (`crates/scheduler/src/passes.rs:1505-1529`).

### Revision of 2026-11-27 after the third re-reads

The security reviewer re-read `5ef574f16` and the architecture reviewer `9960a3c1` and `5ef574f16`; decisions D022-297
to D022-314 in [research.md](./research.md) ("Decisions of the third re-reads of 2026-11-27"), and where each finding
lives in the third part of [reviews/2026-11-26/design-fixes-responses.md](reviews/2026-11-26/design-fixes-responses.md).
The rows above that changed say so. Three facts were checked by hand on `postgres:16-alpine` (research, facts 1 to 3).

| Part | Rule |
|---|---|
| The binding row lock (D022-297) | `public.sagat_lock_run_recovery_binding(p_tenant, p_subject, p_namespace)`, `SECURITY DEFINER`, pinned search path, `FOR SHARE OF` the row, executable by `sagat_app` alone, as `sagat_lock_memberships`; called in slot 4a, in `recheck_requester` and in the payload disclosure transaction; `sagat_app` keeps SELECT, INSERT and DELETE only |
| A role change (D022-298, supersedes D022-286) | refused while the subject has a binding, by `run_recovery_namespaces_membership_fk (tenant_id, subject, role)` → `memberships (tenant_id, subject, role)` `ON UPDATE RESTRICT ON DELETE CASCADE`; the administrator unbinds first (audited); provisioning reports `RUN_RECOVERY_BINDINGS_EXIST`; the runbook step in `docs/runbooks/flow-control.md` (T022-267) |
| A cascaded removal (D022-299) | an `AFTER DELETE` trigger on `run_recovery_namespaces` audits each row a referential cascade removes (`run_recovery.unbound`, actor `system:provisioning`); the route audits its own |
| The scope refusal (D022-301) | `IdentityError::OutsideNamespaceScope` converts to `AppError::NotFound`, the denial record queued; no new variant, so the event stream ends on it within its 30-second re-check; `complete_reset` checks the scope first and answers `StateConflict(AUTHORITY_REVOKED)`; `PendingAdmission` carries `namespace_id` |
| Payload disclosure (D022-302) | for a namespace-bound role, the binding row of the run's namespace through the lock function, after `sagat_lock_memberships`; the opaque `404` when it is gone |
| The fitness rule (D022-303) | also `.permissions()` on a `Role` (one entry, `auth_context`; the two other sites rewritten as `.grants(`) and `Principal::new(` with a non-literal role; every allow list keyed by `impl` and function |
| The lookup of a bound principal (D022-305) | `AND namespace_id = ANY($bound)` in the run, workflow and request lookups: a hit outside the scope returns no row, as a miss |
| The order of refusals (D022-306) | after the role check and before every refusal that depends on the addressed resource |
| Slice 2 (D022-307) | T022-263 builds the scope type, the scope class, the refusal in `Principal::authorize` and their domain tests; T022-256 keeps the bound set, the resolvers and the audit; the OpenAPI role enum lands with the migration in T022-256's pull request |
| The step-lock rule (D022-308) | stated in ADR-0056 decision 2(a) and ADR-0135 decision 2 in D022-282's words; T022-255 checks it by an allow list of enclosing functions, one entry (`prepare_delivery`, a `WAITING` step) |
| `RunRecovery` starts (D022-310) | handler-tree runs, never dispatched; a definition that continues as new refused `422` |

### Hot-path budget lines to record

The plain lines and the slice's gated lines do not move for a definition without the new fields (NFR-022-007). Each line
below is recorded, with its reason, by the change that adds it, before its call site merges (the rule of the slice).

| Line | Added by |
|---|---|
| A completion whose next step declares `cache`, hit | task T022-016 |
| A completion whose next step declares `cache`, miss | task T022-016 |
| A completion whose next step declares `cache` and `concurrency`, miss then granted | task T022-016 |
| A completion that reaches `earlyReturn` | task T022-035 |
| A completion that ends a run `FAILED` in a namespace with a default handler (the dispatch row; ~~the revision is read with the definition~~, the configuration read by the left join of the run's locking statement: struck 2026-11-26, D022-283, as D022-255 corrected) | task T022-031 |
| A completion that enters a handler phase | task T022-031 |
| A collection finish and a loop finish routed through the reducer, against today's lines | task T022-207 |
| A standalone admission, a draft test admission with two upstream pins | tasks T022-039, T022-042 |
| A cache hit's content read (the payload read for the outputs path), and the lookup round's secret locks and binding check | task T022-016 (added 2026-11-25, D022-231, D022-225) |
| A failing run in a namespace without a handler: unchanged; the handler configuration is a left join in the statement that locks the run, which adds no statement (corrected 2026-11-25, D022-255: no statement loads the definition) | task T022-031 |
| A child admission's `test_tree` and `handler_tree` check against its parent's | task T022-217 (added 2026-11-25, D022-258) |
| A request of a member: the membership statement with its bound set (an empty array for every role but `RunRecovery`) and its read of `platform_requirements` for `failure-handler-v1` (named 2026-11-27, D022-311), against today's | task T022-256 (added 2026-11-26, D022-272) |
| A qualifying failing run's dispatch insert, and its suppressed second statement | task T022-031 (added 2026-11-25, D022-242) |
| An update of a run that does not touch `test_tree`: unchanged (the guard is `BEFORE UPDATE OF test_tree`) | task T022-217 (added 2026-11-25, D022-237) |

### Observability, added

*Added 2026-11-25 (decision D022-254):* the attribute sets of the new spans and metrics are closed and carry no key, label
or source run: `workflow.cache.lookup` and `workflow.cache.store` carry `sagat.cache.scope` and `sagat.outcome`;
`workflow.failure_handler.dispatch` carries `sagat.outcome`; the metrics keep the label sets of the table above, with
`step_cache_lookups_total` gaining no label for `unauthorized` beyond its outcome. A run reader who may not see the
producer reads an `unauthorized` outcome as `bypassed`.

The metrics and spans of "Observability" above stand, with: `step_cache_lookups_total` counted from the journal's
`step.cache_hit` and `step.cache_missed` outcomes after commit; `workflow.cache.lookup` spans the lookup round;
the audit action `test_run.secret_resolved` (D022-202, the secret's name and version, never a value); alerts
`failure_handler_dead` and `cache_store_backlog` in `docs/runbooks/flow-control.md`.

*Added 2026-11-26 (decisions D022-274, D022-275, D022-278):*

| Metric | Labels |
|---|---|
| `run_deadline_outcomes_total` | `outcome` ∈ enforced, skipped_handler_phase |
| `run_recovery_namespace_denials_total` | `permission` ∈ `run:read`, `run:start`, `run:operate`, `definition:read` |

Audit actions: `run_recovery.bound` and `run_recovery.unbound` (resource type `run_recovery_binding`, in the transaction
of the change), and `run_recovery.namespace_denied` (resource type `namespace`, through the restricted denial path).

### Failure and recovery, added

| Failure | Expected state | Handling and test |
|---|---|---|
| A store cannot answer a lookup | The transaction fails; the step is not read as a miss | The driver retries; a fault test asserts no `READY` without a decision |
| A cacheable step released while paused | Cannot happen: the planner releases nothing while paused (revised 2026-11-25, D022-238) | A domain test that a pause releases no step |
| A result computed before an invalidation is stored after it | Skipped: the obligation is older than `invalidated_at` | Barrier test of T3 against T4 and T5 (D022-230) |
| Erasure races the store writer | One order: obligations, then entries | Barrier test of T18 against T3 (D022-230) |
| A dead entry where a store would insert | Replaced under its lock | Integration test (D022-230) |
| A hit over the run's retained bytes | The step fails `RUN_DATA_LIMIT` through its route | Integration test (D022-231) |
| ~~A handler that reruns its own failed run~~ | ~~At most one dispatch per lineage~~ | ~~Fault test with a machine identity (D022-228)~~ *(struck 2026-11-25, D022-270; see the lineage rows below)* |
| ~~A person reruns a failed run, which fails again~~ | ~~Dispatched again: a member's rerun begins a new lineage~~ | ~~Integration test (D022-241, O1176 (b))~~ *(struck 2026-11-25, D022-270: D022-242 reversed it; such a rerun extends the lineage and its dispatch is written `SUPPRESSED_LINEAGE`)* |
| A handler with a `RunRecovery` credential reruns its failed run, then tries to redrive (added 2026-11-25, D022-266) | Exactly one handler run; the redrive refused `403` | Fault test |
| A stop request while the phase drains (added 2026-11-25, D022-261) | Refused `409`; one cause | Domain and integration cases, both orders |
| The overdue pass picks a run that then opens its phase (added 2026-11-25, D022-262; restated 2026-11-26, D022-278) | The pass's `enforce_run_deadline` is a typed no-op, counted `skipped_handler_phase`; a stop with the reason `TimedOut` (the handler's timer) still drains the phase | Barrier test between pick and apply; a domain case of both in one phase |
| A dispatch deferred by the budget more than 12 times (added 2026-11-25, D022-264) | Still `PENDING`; admitted once | Integration test |
| A handler or test-tree run whose definition continues as new (added 2026-11-25, D022-263; resets and reruns added 2026-11-26, D022-279) | Refused at admission with `CONTINUE_AS_NEW_REFUSED` (a reset or rerun under a handler tree `422`); a run that reaches the boundary anyway ends `FAILED` with `CONTINUE_AS_NEW_REFUSED`; never non-terminal | One test per creator: dispatch, child, draft test, reset, rerun, batch rerun; a seeded successor ended by the pass |
| A `RunRecovery` subject calls a route of a namespace outside its binding (added 2026-11-26, D022-272, D022-275) | The opaque `404`, audited `run_recovery.namespace_denied`, counted; nothing changed | The battery of every route (T022-257) |
| A binding is revoked between a request's authentication and its write (added 2026-11-26, D022-277) | The write's slot-4a read finds no row: the opaque `404`, rolled back | `slot_order.rs` case, both orders (T022-259) |
| A start is replayed after its subject's binding was revoked (added 2026-11-26, D022-277) | The opaque `404`; nothing created | Integration test (T022-259) |
| `failure-handler-v1` switched off while `RunRecovery` subjects are bound (added 2026-11-26, D022-276) | Every such subject bound to no namespace; the rows kept and in force again on re-activation | Integration test (T022-253) |
| ~~A `RunRecovery` member's role is changed and changed back (added 2026-11-26, D022-286)~~ | ~~Its bindings removed by the first change; bound to no namespace until an administrator binds again~~ | ~~Integration test (T022-261)~~ *(superseded 2026-11-27, D022-298; see the rows below)* |
| A role change of a `RunRecovery` member that still has a binding (added 2026-11-27, D022-298) | Refused by `run_recovery_namespaces_membership_fk` (`RUN_RECOVERY_BINDINGS_EXIST` to provisioning); nothing changed; after the administrator unbinds (audited), admitted | Race at three isolation levels; the unbind-then-change path (T022-261) |
| A bind concurrent with a role change of its subject, at any isolation level (added 2026-11-27, D022-298) | One of them refused: the bind `422 NOT_A_RECOVERY_SUBJECT`, or the role change by the foreign key; never a binding for a member of another role | A held lock, at READ COMMITTED, REPEATABLE READ and SERIALIZABLE (T022-261) |
| A membership or a namespace deleted out of band while bound (added 2026-11-27, D022-299) | The bindings removed by the cascade, one `run_recovery.unbound` row each (actor `system:provisioning`) | Integration test (T022-266); a deletion against a concurrent bind at three isolation levels (T022-261) |
| A `RunRecovery` payload read whose binding is revoked before its disclosure transaction (added 2026-11-27, D022-302) | The opaque `404`; nothing disclosed | Held-lock case, both orders (T022-259) |
| A `RunRecovery` start that fails, or that continues as new (added 2026-11-27, D022-310) | A handler-tree run: no dispatch row; a definition that continues as new refused `422 CONTINUE_AS_NEW_REFUSED` | T022-244 case |
| A `RunRecovery` reset waits as a pending reset; the binding is revoked before the source ends (added 2026-11-26, D022-288) | The entry fails `AUTHORITY_REVOKED`; without the revocation it completes | Integration test (T022-262) |
| A `RunRecovery` event stream is open when its binding is revoked (added 2026-11-26, D022-296) | `reconnect` within one 30-second re-check | Integration test (T022-259) |
| Publication of a workflow with a database node against a test admission of its draft | No cycle (the draft row is not locked) | `slot_order.rs` case (D022-222) |
| The dispatcher crashes inside an admission | The attempt is already counted; `DEAD` after 12 | Fault test (D022-223) |
| Two failing runs of one lineage commit at once | Both reach their terminal status; one live dispatch row and one suppressed row | Fault test with a held commit (D022-242) |
| A handler starts fresh runs that fail | Dispatches beyond the namespace budget deferred and alerted, never dropped | Integration test (D022-243) |
| A stop source during the handler phase | `failure_handler.draining {cause}`, one cause: a further stop request is refused `409` (D022-261; "the first cause wins" corrected 2026-11-26, D022-285); never the run's stop | Domain cases (D022-246) |
| The run deadline passes during the phase | Nothing: only the handler's timer bounds the phase | Overdue pass test (D022-247) |
| A continue-as-new successor of a test-tree or handler-tree run, or of a definition with a new policy | Refused | Codec and `refuse_gated_successor` tests (D022-249) |
| A failing chain head | Dispatched through `persist`'s terminal path | Integration test (D022-250) |
| An older binary replays a journal the routed finish wrote | Replays | Mixed-version test (D022-251) |
| An attempt leased before an invalidation by key completes after it | Not stored as live (lease stamp, tombstone) | Barrier test with T2 open across T5 (D022-252) |
| A key failure's route releases a cacheable successor after the gates round | Looked up in a later round of the commit | A barrier test with an invalidation holding the scope row |
| A collection or a loop ends a run with a handler | Handler phase entered through the reducer | T31 tests; until then refused at publication |
| Configuration cleared while a terminal commit is in flight | Dispatched or not by commit order; revision recorded | Barrier test both orders (G022-024) |
| The dispatcher crashes at each commit point | Exactly one handler run | 20 rounds (SC-022-006) |
| A test tree's child runs a gated step | Filed under the `DRAFT_TEST` pool | Invariant 7 |
| An ordinary run's key full; a test run of the same key | The test run is granted | One test per gate |

### Constitution Check (the rest)

*Re-dated 2026-11-25 after the second rulings (decisions D022-242 to D022-259); the results stand.*

| Principle | Result | Evidence / future gate |
|---|---|---|
| I — durable atomic transitions | Pass | The rounds before `persist`; T1 to T18 and T31 as built; invariants 3 to 7, 11, 14 |
| II — pure domain | Pass | Lookup outcome, handler phase, pins and the result rules are reducer rules; locks and clocks in the adapter |
| III — at-least-once effects, compensation first | Pass | Cache refused on effectful nodes; the handler after compensation; mocks outside compensation planning |
| IV — pinned versions, six SDKs | Pass | Additive IR; registry activation; conformance in six languages |
| V — tenant boundary | Pass | Namespaced foreign keys (D022-213); test runs refused in `prod`; secrets per resolution; battery |
| VI — OTel, audit, typed errors | Pass | The metrics above; `contracts/errors.md` |
| VII — inbox/outbox | Pass | The dispatch obligation; `run.result_returned` in its commit |
| VIII — failure tested, honest | Pass | The matrix above; G022-021 to G022-027 state what is pending or accepted |

### Complexity Tracking (the rest; added 2026-11-25)

| Addition | Why needed | Simpler alternative rejected because |
|---|---|---|
| A recorded deviation of ADR-0050 decision 7: the terminal commit inserts a slot-3 dispatch row, by lineage | Exactly one obligation per failure, written with the failure; a suppressed dispatch visible and redrivable | A second transaction after the terminal commit loses the obligation in a crash (D022-223, D022-242) |
| A handler phase with a journaled draining state | The handler's work cannot run under the stop, and a draining sub-state must replay | A separate handler state machine, or relaxing the drain for handler nodes (D022-224, D022-246) |
| A per-capability manifest of artefacts | A capability served before its behaviour exists would run half of it | A constant set by a slice proves nothing (D022-256) |

### Delivery sequence (the rest)

The slices, their tasks and their evidence are in [tasks.md](./tasks.md#release-030-the-rest-of-022-od-2026-11-24-1):
documents; foundations; the finish-path routing; US04 in two slices; US01 in four; US03 in four; US05 in two; US06 in
four; the surfaces (decision D022-221).

### Size

*Revised 2026-11-27:* 68 new tasks (T022-200 to T022-267; the third re-reads added T022-263 to T022-267), riding in the
slices they serve; the estimate of pull requests stands.

*Revised 2026-11-26:* 63 new tasks (T022-200 to T022-262; the re-reads added T022-256 to T022-262), riding in the slices
they serve; the estimate of pull requests stands.

*Revised 2026-11-25:* 38 new tasks (T022-200 to T022-237; the rulings added T022-224 to T022-237) beside the 43 open
existing tasks the rest closes; the estimate of pull requests stands, the new tasks riding in the slices they serve.

24 new tasks (T022-200 to T022-223) beside the 43 open existing tasks the rest closes. By the slice's pace (one
reviewed pull request per slice of a trust boundary), about 23 pull requests for the slices and 8 to 10 for the
surfaces. No date is implied.
