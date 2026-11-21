# Implementation Plan: Cross-namespace service operations

**Branch**: `feature/030-cross-namespace-services` | **Date**: 2026-10-26 | **Spec**: [spec.md](spec.md)

**Input**: `specs/030-cross-namespace-services/spec.md`

Status: Proposed — not started.

## Summary

Add service contracts (immutable revisions owned by a handler namespace), a tenant-wide registry of
service endpoints with handler bindings and caller allow-lists, and an additive `serviceCall` node.
A call is carried across the namespace boundary by durable operation messages between a caller-side
record and a handler-side record; each side changes only under its own run and shard fence
([ADR-0065](../../docs/adr/ADR-0065-cross-namespace-operations-cross-only-through-durable-operation-messages.md)).
The handler is an ordinary workflow run in both modes. Results cross only under the declared
disclosure and classification of the operation. No worker protocol change is needed.

## Technical Context

**Language/Version**: Rust from `rust-toolchain.toml`; the locked TypeScript, Go, Python, .NET/C# and
Java SDK toolchains; the Studio's locked Node toolchain. No toolchain change.

**Primary Dependencies**: existing Tokio, Axum, tonic, SQLx, NATS JetStream and OTel crates; the
existing JSON Schema adapter (`crates/infra-schema`) and expression evaluator (`crates/infra-expr`).
No new dependency.

**Storage**: PostgreSQL authority: new registry, pin, record and message tables; additive columns on
`runs`; additive quota resources. Payloads stay in `payloads` (tenant, digest). NATS carries hints.

**Testing**: Rust unit and property tests; integration and fault tests on real PostgreSQL under the
runtime role; contract tests (OpenAPI, AsyncAPI baseline, IR and contract schemas); security battery;
five-SDK conformance; Studio Vitest and real-browser e2e; a performance harness on the feature 010
reference environment.

**Target Platform**: the existing single-region Linux deployment (Compose for development, Helm).

**Project Type**: Rust modular monolith, five SDKs, Studio SPA.

**Performance Goals**: none promised. Measure NFR-030-005 (SC-030-014).

**Constraints**: no transaction locks runs of two namespaces; payload-free projections; closed metric
labels; begin every transaction with `BeginSafely::begin_safely` (ADR-0014); database clock for every
deadline; root-path calls only in the first increment.

**Scale/Scope**: four permissions, two document kinds (one new), one node kind, nine tables, five
message kinds, about 20 REST operations, five SDK builders, one Studio area.

## Constitution Check

These are design checks, not approval of an implementation.

| Principle | Before research | After design | Evidence / future gate |
|---|---|---|---|
| I — durable, atomic transitions; terminal never reopens | Pass | Pass | Transaction table below; a terminal caller step is never reopened by a late message (discarded, audited); T030-022..025, T030-029..031 |
| II — pure domain, ports and adapters, no empty mappers | Pass | Pass | Domain types in `crates/domain/src/services.rs`; schema checks behind an application port implemented in `crates/infra-schema`; no DTO mapper without semantics; `scripts/check-architecture.py`, T030-004, T030-009, T030-014 |
| III — at-least-once effects, compensation as steps | Pass | Pass | One handler run per operation is a platform guarantee only; every settlement carries an effect class; an effect-possible failure is compensated or reviewed, never "not done"; a deadline looks the outcome up first; compensation is a derived call (US09); T030-055..057, T030-065 |
| IV — pinned versions, additive contracts, five SDKs | Pass | Pass | Contract pinned at publication; additive IR/OpenAPI/AsyncAPI; shared conformance; T030-005..008, T030-051..054 |
| V — isolation, tenant boundary, secrets | Pass | Pass | Composite FKs, allow-list, fail-closed revocation, no secret in results, cross-tenant opacity; T030-032..037 |
| VI — observability and audit | Pass | Pass | Journal-derived spans with links, closed labels, audit in decision transactions; reviews counted by reason; T030-046..050 |
| VII — inbox/outbox, redeliverable messages, backpressure, dead letters | Pass | Pass | Messages are outbox and inbox with unique keys; polling recovery; deferral; `DEAD` with redrive; T030-026..030 |
| VIII — failure tests, verified contracts, honesty | Pass | Pass | Crash matrix CP1–CP8, duplicates, lost hints, races, failed compensation, restart; contract linting; benchmark report; T030-029..031, T030-034..035, T030-057, T030-061..063, T030-065, T030-067, T030-068 |

**Governance gate**: ADR-0065 must be accepted by the owner, and the owner questions O1480–O1482
answered or their defaults confirmed, before T030-004. Feature 016's owner must accept the carrier
extension (G1493) before T030-037 enables classified disclosure of results.

## Project Structure

### Documentation (this feature)

```text
specs/030-cross-namespace-services/
├── spec.md
├── research.md
├── plan.md
├── data-model.md
├── quickstart.md
├── contracts/
│   ├── README.md
│   ├── service.schema.json
│   ├── workflow-service-call.schema.json
│   ├── openapi-service-operations.yaml
│   ├── asyncapi-service-operations.yaml
│   ├── operation-messages.md
│   ├── errors.md
│   └── conformance.md
├── checklists/requirements.md
└── tasks.md
docs/adr/ADR-0065-cross-namespace-operations-cross-only-through-durable-operation-messages.md
```

### Source code (planned additions to existing boundaries)

```text
crates/domain/src/services.rs                     # names, contract/endpoint rules, allow-list, ids, outcome mapping, limits
crates/domain/src/workflow.rs                     # NodeKind::ServiceCall (label "serviceCall")
crates/domain/src/execution/{event,state,command}.rs  # service_call.* and service_operation.* events, wait kind, reducer
crates/domain/src/{identity,audit,quota}.rs       # 4 permissions, audit actions/resource types, quota resources
crates/application/src/services.rs                # contract and endpoint use cases, ports
crates/application/src/service_operations.rs      # schedule, admit, refuse/defer, complete, apply, cancel, redrive
crates/application/src/telemetry.rs               # derive service-call spans and links from the journal
crates/infra-ir/src/                              # codec of serviceCall and of the Service document
crates/infra-schema/src/                          # closed/bounded checks, canonical equality, classification walk reuse
crates/infra-postgres/src/{services,service_operations}.rs + migrations/  # tables, grants, triggers
crates/infra-postgres/storage-classification.json # new variable-size columns
crates/scheduler/src/passes.rs                    # admission, notification, cancellation, deadline passes
crates/scheduler/src/monitor.rs                   # two namespace alert conditions
crates/api-http/src/routes/services.rs, crates/api-http/src/dto_services.rs
crates/telemetry/src/{metrics,span}.rs
apps/cli/src/services.rs, apps/cli/src/args.rs
frontend/apps/studio/src/features/services/       # registry, contracts, endpoints, operations
frontend/apps/studio/src/features/runs/           # serviceCall runtime node and links
frontend/apps/studio/src/features/workflows/      # serviceCall inspector
sdks/{typescript,go,python,dotnet,java}/          # builders and clients
specs/001-platform-foundation/contracts/          # canonical merge targets (T030-005..008)
tests/{contract,integration,fault,e2e,security,performance}/service-operations/
tests/contract/sdk-conformance/{scenarios,expected}/service-*.json
docs/runbooks/service-operations.md, docs/architecture/execution-semantics.md (section)
```

Migration timestamps are allocated at implementation time after the latest merged migration
(today `20261220000000`), to avoid collisions with the parallel drafts.

## Architecture

1. **Domain** (`services.rs`): `ServiceName`, `OperationName`, `EndpointName` (slug rules),
   `OperationMode`, `ServiceCallCancelPolicy`, `HandlerBinding {workflow, VersionPolicy}`, `AllowList`
   (bounded set of `NamespaceId`), `ServiceCallLimits`, `OperationId::derive(tenant, run, instance)` and
   `RunId::for_operation(op)` (domain-separated, length-delimited, like ADR-0030 §4 scoped ids),
   `OperationOutcome` and the mapping of FR-030-015, reserved-code checks. Schemas are not interpreted
   in the domain: it holds digests and validated metadata; the application asks a `ContractSchemas` port
   (implemented in `infra-schema`) to check closed/bounded output, canonical equality and
   classification.
2. **IR**: `NodeKind::ServiceCall {endpoint, service, operation, input, start_timeout, cancel_policy,
   contract_revision, compensate_with, irreversible}`; the node's `timeout` and `next`/`on_error` use the
   existing node fields. The capability gate rejects the node when disabled and inside bodies.
3. **Reducer**: entering the node yields a `ServiceCallScheduled` command; the step waits with kind
   `SERVICE_CALL` and two deadlines; `ServiceCallStarted`, `ServiceCallFinished` and
   `ServiceCallCancelRequested` events settle it. Replay of a run reproduces the same state from the
   journal; a recorded outcome is never re-evaluated.
4. **Application**: `ServiceOperations` use cases are invoked by the scheduler passes and the
   terminal-transaction hook of the run store; management use cases by the HTTP routes. Every use case
   receives the trusted tenant from the principal or the row, never from a client field.
5. **Scheduler passes** (ADR-0020): `service_admissions` over pending `START` rows of owned shards (by
   handler run id), `service_notifications` over pending `STARTED`/`START_REFUSED`/`COMPLETED` rows
   (by caller run shard), `service_cancellations` over pending `CANCEL` rows (by handler run shard);
   caller deadlines join the existing wait-deadline pass. Every pass fences its action in the action's
   transaction and claims rows with `FOR UPDATE SKIP LOCKED`; hints only shorten the polling delay.
   The purge of settled messages is a singleton loop under an `engine_leases` row (as the telemetry
   projector, ADR-0017), in bounded batches on the database clock.
6. **No new process.** Nothing runs user code; the handler run is executed by its own workers.
7. **Effect classes** (D1512). The reducer maps every settlement to `none`, `failed`, `applied`,
   `partial` or `unknown` and routes an effect-possible failure to saga compensation or to the existing
   manual review (`step.needs_review`), never to `onError`. A caller deadline after admission moves the
   record to `LOOKING_UP` with a lookup deadline (the outcome lookup window) before it settles.
8. **Feature 031** (D1661). Nothing encoded crosses in v1. Contract publication refuses
   `x-sagat-encoding` in the operation schemas; endpoint save refuses a handler version that declares
   encoding; caller publication calls 031's shared check (`EncodingCheck`, FR-031-006) and refuses a
   `serviceCall.input` or `compensateWith.input` from an encoded source; admission (CP2) and the
   handler's terminal hook (CP5) run 031's runtime envelope check and refuse the start or withhold the
   result. No server component decodes or re-encodes.
9. **Reset** (D1516). The domain declares the reset rule of each new event type for 018's registry;
   a reset run's restored `service_call.*` events are provenance and create no rows.

## Transaction boundaries

Runtime commit points CP1–CP8 are the crash-injection matrix of SC-030-003.

| Id | Transaction | Commits together | Locks, in order |
|---|---|---|---|
| T1 | Publish contract revision | service row (created on first use), immutable revision, quota counts, audit, request receipt | service row FOR UPDATE (or insert) → quota fence → usage |
| T2 | Create or change endpoint | endpoint head, immutable endpoint revision, allow-list rows added/removed, audit per change, receipt | endpoint head FOR UPDATE (CAS on expected revision) → allow-list rows → quota fence |
| T3 | Publish caller version | existing publication records, caller pins, audit | existing publication order → publisher membership `FOR SHARE` (slot 4a) → endpoint head FOR SHARE → allow-list row FOR SHARE (slot 4b) |
| CP1 (T4) | Schedule a call | step `WAITING` with deadlines, caller record `SCHEDULED`, `START` message, journal `service_call.scheduled`, quota counts, audit `service.operation_scheduled`, outbox hint | shard fence → chain → caller run FOR UPDATE (slot 3R) → publisher membership `FOR SHARE` (4a) → inserts of the record and the new `START` row (no wait) → quota fences and usage (8b) |
| CP2 (T5) | Admit | handler run (existing admission: tree lock, active-run quota, input pin, journal `run.started` with cause, first tasks, outbox), execution record, `START` → `APPLIED`, `STARTED` message, rate tokens, audit `service.operation_started` | shard fence (handler run id) → `START` FOR UPDATE SKIP LOCKED (slot 3) → the handler version's namespace fences and publisher memberships as the existing admission takes them (4a) → the handler version's pins in 4b order (017 connection heads …) → service-endpoint head FOR SHARE → allow-list row FOR SHARE (4b, end) → secrets (5) → tree and active-run admission, run insert, execution record and `STARTED` inserts (6) → rate bucket, quota, usage (8b) |
| CP3 (T5r) | Refuse | `START` → `REFUSED` with code, `START_REFUSED` message, audit `service.operation_refused` | shard fence → `START` FOR UPDATE SKIP LOCKED (3) → endpoint head FOR SHARE → allow-list row FOR SHARE (4b) |
| — (T5d) | Defer | `START` attempts unchanged, `next_attempt_at`, `deferral_reason` | `START` FOR UPDATE SKIP LOCKED |
| CP4 (T6) | Apply `STARTED` / `START_REFUSED` | from `SCHEDULED`: caller record `STARTED` with handler run link and journal `service_call.started`; or, for `START_REFUSED`, `service_call.finished` with the refusal code and effect `none`, step `FAILED`, routing, outbox; message `APPLIED`. A `STARTED` for a record already past `SCHEDULED` is a no-op `APPLIED`; a `START_REFUSED` for a settled record is `DISCARDED` and audited | chain → caller run FOR UPDATE (shard fence) → caller record → message |
| CP5 (T7) | Handler terminal | existing terminal transaction (`run.finished`, outbox) plus execution record `COMPLETED`, journal `service_operation.completed`, `COMPLETED` message (outcome, result digest or code), audit `service.operation_completed` | existing terminal order (chain → handler run, slot 3R) → execution record → endpoint head FOR SHARE → allow-list row FOR SHARE (rows this ADR lists for the transition) → usage (8b) |
| CP6 (T8) | Apply `COMPLETED` | from `SCHEDULED` (handler run id derived), `STARTED`, `CANCELLING` or `LOOKING_UP`: caller record settled with outcome and effect class, journal `service_call.finished`, step outcome and output (retained-data charge), compensation planning or `step.needs_review` for an effect-possible failure, next tasks/timers, outbox, message `APPLIED`; for a settled record: message `DISCARDED`, late outcome and class written once to the record, audit `service.operation_completion_discarded` | chain → caller run FOR UPDATE (shard fence) → caller record → message → usage |
| CP7 (T9) | Caller deadline, lookup-window end or cancellation | `START` still pending: `WITHDRAWN`, record `START_TIMED_OUT`/`CANCELLED` with effect `none`. `START` applied: deadline → record `LOOKING_UP` with lookup deadline and a `CANCEL`; lookup-window end → record `TIMED_OUT` with effect `unknown` and routing to compensation or review; cancellation → policy (`abandon`/`request_cancel` settle with `unknown`, `wait_cancelled` → `CANCELLING`). Step outcome, journal, audit together | chain → caller run FOR UPDATE (shard fence) → caller record → `START` FOR UPDATE (waits for an admission in progress) |
| CP8 (T10) | Apply `CANCEL` | handler run stop requested (existing `run.stop_requested`), journal `service_operation.cancel_requested`, execution record, message `APPLIED`, audit; a terminal or purged (missing) handler run makes it a no-op `APPLIED` | chain → handler run FOR UPDATE (shard fence) → execution record → message |
| T11 | Dead / redrive | message `DEAD` (attempt ceiling) with the alert found by the monitor; redrive resets attempts with audit `service.message_redriven` | message FOR UPDATE |

Rules shared by all: no transaction takes a mutation lock on a run of the other side; a pass that
finds its candidate changed (state, revision, shard generation) skips it and the next poll retries;
no response is acknowledged before commit; external I/O never happens inside these transactions
(there is none). CP7 waiting on the `START` row cannot deadlock: CP2 holds `START` and then locks only
endpoint, allow-list and admission resources, never a caller-side row.

**Slots of ADR-0050 decision 7 (C4, D1517).** Handler admission (CP2) reads its shard fence and locks
the `START` row in slot 3; takes the handler version's namespace fences and its publisher's memberships
in 4a exactly as the existing admission does; then the 4b catalog heads in 4b's order — the handler
version's own pins (017 connection heads, 021, 024, 025 heads) before the service-endpoint head and the
allow-list entries, which are last in 4b; secrets in 5; the new handler run, the execution record and
the `STARTED` row in 6; rate, quota and usage in 8b. Because 4b puts the version's pins before the
endpoint head, CP2 resolves the binding from an unlocked read of the endpoint head, locks in slot order,
then rechecks the endpoint revision and allow-list version under their share locks; a change rolls the
transaction back and the pass retries. Refusal (CP3) uses slots 3 and 4b only. Caller publication (T3)
takes memberships (4a) before the endpoint head and allow-list (4b). Runtime transitions (CP1, CP4–CP8)
start at slot 3R with their run and then take the rows this plan lists, ending with 8b.

**Exception for ADR-0050 decision 7 to list (D1660).** CP7 (caller deadline or cancellation) locks an
existing `START` row, a slot-3 row, after the caller run (3R), which decision 7 forbids for transitions
in general. It is safe here and needed: the only order-(a) transaction on a `START` row is handler
admission, which creates its run and never locks an existing run, the caller record or any caller-side
row, so no transaction can hold a `START` row while waiting for a caller run; and the lock is what lets
the caller decide, by first commit, whether a start was admitted (effect `none` versus a started
handler run). The exception is limited to `START` rows locked by the transaction of the run that wrote
them.

## Authorization

| Permission | admin | developer | operator | viewer | worker | runner | database-runner |
|---|:-:|:-:|:-:|:-:|:-:|:-:|:-:|
| `service:read` | yes | yes | yes | yes | – | – | – |
| `service:publish` | yes | yes | – | – | – | – | – |
| `service:endpoint:manage` | yes | – | – | – | – | – | – |
| `service:call` | yes | yes | – | – | – | – | – |

| Decision point | Checks (all inside the tenant of the principal or of the row) |
|---|---|
| Publish contract revision | `service:publish`; handler namespace exists in the tenant |
| Create/change/pause/resume/delete endpoint, edit allow-list | `service:endpoint:manage`; endpoint belongs to the addressed namespace; every allow-list namespace is in the tenant (else `NOT_FOUND`, audited) |
| Read registry, contracts, endpoints | `service:read` |
| Publish a caller version | existing `definition:publish`; `service:call`; endpoint not deleted; caller namespace allow-listed (advisory) |
| Schedule (CP1) | caller version publisher's current role grants `service:call` |
| Admit (CP2) | endpoint `ACTIVE` (paused defers); caller namespace allow-listed; pinned revision served; binding compatible |
| Disclose (CP5) | endpoint not deleted; caller namespace still allow-listed (O1481 default) |
| List/read operations of a namespace | `run:read` in that namespace; cross-namespace link fields are ids; following them needs `run:read` there |
| Redrive a dead message | `run:operate` in the receiving namespace, with a reason, audited |

A missing or foreign endpoint, service, namespace, run or operation is `NOT_FOUND` with the existing
restricted denial audit. Policy and membership changes take effect on the next admitted decision. The
four permissions are additive grants to the existing built-in roles; when feature 029 is enabled its
evaluator decides effective permissions and the visibility of services, endpoints and operation
listings (C10, D1519). Resolving a manual review on a call uses the existing `RESOLVE_STEP`
(`saga:override`) and `RETRY_STEP` (`run:operate`) of `operateRun`.

## Finite ceilings

| Resource | Scope | Default | Hard ceiling |
|---|---|---:|---:|
| Services | namespace | 100 | 1,000 |
| Revisions per service | service | 100 | 1,000 (referenced revisions are kept) |
| Operations per revision | revision | 32 | 32 |
| Declared error codes per operation | operation | 16 | 16 |
| Schema size (input or output) | schema | 64 KiB | 64 KiB |
| Contract document | revision | 256 KiB | 1 MiB |
| Service endpoints | tenant | 100 | 1,000 |
| Served revisions per endpoint | endpoint | 2 | 4 |
| Allow-list entries | endpoint | 64 | 256 |
| Endpoint revisions | endpoint | 1,000 | 1,000 plus one tombstone |
| Concurrent active operations | endpoint | 100 | 1,000 |
| Operation starts per second | endpoint | 50 | 500 |
| Active inbound operations | handler namespace | 1,000 | 10,000 |
| Pending outbound calls | caller namespace | 1,000 | 10,000 |
| Active calls per run | caller run | 32 | 64 |
| `serviceCall` nodes per definition | definition | 64 | 64 |
| Call depth across namespaces | call chain | 4 | 8 |
| Operation input / result | value | 256 KiB inline | 256 KiB inline; no blob descriptors |
| Async timeout | operation | 24 h | 60 days |
| Outcome lookup window after a caller deadline | call | 60 s | 15 min |
| Call attempts per step (operator retries) | step | — | 16 |
| Sync timeout | operation | 10 s | 60 s |
| Start timeout | operation | = timeout | ≤ timeout |
| Deferral backoff | `START` | 1 s, doubling | 60 s cap |
| Apply attempts before `DEAD` | notification, `CANCEL` | 12 | 12 |
| Settled message retention | message | 7 days | 30 days |
| Rows per listing page | API | 50 | 500 |

Effective value = min(request, namespace setting, tenant default, operator ceiling, hard ceiling).
Zero is not unlimited: a zero count refuses new admission. Admission fails before any work. Existing
run, content and retention quotas also apply; handler runs count against the handler namespace's
`active_runs` and the tenant's `tenant_active_runs`.

## Failure and recovery matrix

| Failure | Expected state | Handling and test |
|---|---|---|
| Crash before CP1 commit | No record, no message | The reduction that reached the node is retried; fault test per CP |
| Crash after CP1, before the hint | `START` pending | Admission poll finds it (lost-hint suite) |
| Crash inside CP2 | Rolled back; `START` pending | Next pass admits; no partial handler run |
| Two engines admit the same `START` | One commits | `SKIP LOCKED`, derived run id PK, unique execution record |
| Crash after CP2, before CP4 | Handler runs; caller unaware | `STARTED` applied later; caller deadline still runs |
| Crash inside CP5 | Existing terminal rollback | Existing scheduler retries the terminal transition |
| Crash after CP5, before CP6 | `COMPLETED` pending | Applied later; duplicate apply is a no-op |
| Completion vs caller deadline | First commit on the caller run wins | A completion first settles the call; a deadline first moves it to `LOOKING_UP` and the completion then settles it with its effect class; 100-round race |
| `COMPLETED` applied before `STARTED` | Call settled from `SCHEDULED` | Later `STARTED` is a no-op (fault case, D1511) |
| `STARTED` goes `DEAD` | Call stays `SCHEDULED` | `COMPLETED` still settles it; a start deadline finds the `START` applied and moves it to `STARTED`; redrive is a no-op |
| Caller deadline after admission, handler silent | `LOOKING_UP`, then `TIMED_OUT` with `unknown` | Compensation in a saga, manual review outside; late outcome written for lookup |
| Revocation or invalid result after a handler success | Effect `applied`, no value | Compensation or manual review; never `onError` |
| `CANCEL` for a purged handler run | No handler run | No-op `APPLIED` |
| Reset with an open call in the prefix | Refused | 018's registry rule `REFUSE` |
| Cancellation vs admission | First commit on `START` wins | Withdrawn and no run, or run plus policy; 100-round race |
| Endpoint paused, quotas full, rate empty | `START` deferred | No attempt consumed; start deadline withdraws it |
| Endpoint deleted / caller removed | Refused at CP2; `ACCESS_REVOKED` at CP5 | Isolation battery |
| Handler version incompatible (active policy) | Refused `OPERATION_HANDLER_UNAVAILABLE` | Binding-health test |
| Caller run purged before completion | `COMPLETED` discarded | Retention test |
| Notification keeps failing | `DEAD` after 12 attempts; alert | Redrive test; never deleted unprocessed |
| Shard takeover mid-action | Fence rejects stale owner | Shard fault test (SC-030-015) |
| Database unavailable | Nothing commits | Passes back off; no acknowledgement before commit |
| Engine restart | Durable rows drive progress | Restart suite with 1,000 calls in flight |
| Handler effect then crash before checkpoint | Unknown effect in the handler run | Existing reconciliation in the handler namespace; the caller waits |
| Compensation call fails (US09) | Existing `MANUAL_REVIEW`/`COMPENSATION_FAILED` | Saga fault test |
| Duplicate `CANCEL` | One stop request | Unique key, terminal check |

## Observability

**Spans.** Journal-derived (ADR-0017): the caller step span of a `serviceCall` node has a computed id
from (caller run, node, operation id); the handler run span is its child in the same trace; both carry
an OTel link to the other. Live spans: `service.operation.admit`, `service.operation.notify`,
`service.operation.cancel`, `service.operation.redrive`, and the HTTP spans of the new routes.
Attributes: existing `sagat.tenant.id`, `sagat.namespace.id`, `sagat.run.id`, `sagat.node`,
`sagat.outcome`, `sagat.failure.code`, plus `sagat.service.endpoint`, `sagat.service.name`,
`sagat.service.operation`, `sagat.service.operation_id`, `sagat.service.mode`,
`sagat.service.handler_namespace.id`, `sagat.service.caller_namespace.id`. No payload, schema or
subject.

**Metrics** (labels only from the closed sets shown; no tenant, namespace, endpoint, run or operation):

| Metric | Labels |
|---|---|
| `service_operations_scheduled_total` | `mode` ∈ {async, sync} |
| `service_operations_started_total` | `mode` |
| `service_operations_refused_total` | `reason` ∈ the refusal codes of contracts/errors.md |
| `service_operations_deferred_total` | `reason` ∈ {endpoint_paused, namespace_active_runs, namespace_operations, endpoint_concurrency, endpoint_rate, transient} |
| `service_operations_finished_total` | `mode`, `outcome` ∈ {succeeded, failed, cancelled, timed_out, start_timed_out, refused, access_revoked, result_invalid}, `effect` ∈ {none, failed, applied, partial, unknown} |
| `service_operations_reviews_total` | `reason` ∈ {timed_out, access_revoked, result_invalid, partial, cancelled} (effect-possible failures sent to manual review) |
| `service_operation_start_latency_seconds` | `mode` |
| `service_operation_duration_seconds` | `mode`, `outcome` |
| `service_operation_messages_pending` (gauge) | `kind` ∈ {start, cancel, started, start_refused, completed} |
| `service_operation_message_oldest_age_seconds` (gauge) | `kind` |
| `service_operation_messages_dead_total` | `kind` |
| `service_operation_completions_discarded_total` | `reason` ∈ {caller_settled, caller_missing} |

**Alerts** (engine monitor, per namespace, outbox-announced as ADR-0017 §6): `service_messages_dead`
(dead messages in the receiving namespace) and `service_operations_deferred` (oldest deferral older
than its threshold, default 5 minutes). Runbook: `docs/runbooks/service-operations.md`.

**Audit actions** (resource types `Service`, `ServiceRevision`, `ServiceEndpoint`, `ServiceOperation`):
`service.revision_published`, `service.deleted`, `service.endpoint_created`, `service.endpoint_changed`,
`service.endpoint_paused`, `service.endpoint_resumed`, `service.endpoint_deleted`,
`service.endpoint_caller_allowed`, `service.endpoint_caller_removed`, `service.operation_scheduled`,
`service.operation_started`, `service.operation_refused`, `service.operation_cancel_requested`,
`service.operation_completed` (outcome, `resultDisclosed`), `service.operation_completion_discarded`,
`service.message_dead`, `service.message_redriven`, `service.denied`. Records carry ids, names, codes,
revisions, times and the trace id; never values or free text from callers.

**Logs**: structured, redacted by the existing layer; operation id and codes only.

## Migration and compatibility

- Additive migrations: the nine tables of [data-model.md](data-model.md), `runs.service_call_depth
  smallint NOT NULL DEFAULT 0` with a check `0..8`, step wait kind `SERVICE_CALL`, quota resource values
  added to the existing checks, `quota_rate_buckets` source kind `SERVICE_ENDPOINT`, grants to
  `sagat_app`, ALWAYS triggers refusing updates of immutable rows, storage-classification manifest
  entries, foreign keys from message digests to `payloads`. Down migrations refuse while rows exist.
- Registrations in other features' registries (D1518, D1519): 020's content-store registry
  (`run-content-references.json`), 028's time-class catalog (`time-classification.json`), 018's
  reset-rule registry, 031's encoded-field registry, the consolidated FR-016-002 amendment, 029's list
  of permission-adding features. Each is added by the task that adds the matching code.
- IR: additive `serviceCall` in `workflow.schema.json` (v1, ADR-0032); a new `service.schema.json`.
  Older servers refuse the node; the capability is off by default until T030-064.
- OpenAPI `/v1` additive; AsyncAPI gains two hint channels (baseline regenerated deliberately);
  `sdks/error-catalog.json` unchanged (no new `Error.code`).
- Worker protocol unchanged; no SDK worker change.
- Rollout and rollback: enable per deployment; disabling stops new publications and schedules,
  keeps the passes applying messages until calls drain (FR-030-035).

## Delivery sequence

1. Governance and contracts: ADR acceptance, owner answers, draft contracts merged (T030-001..008).
2. Domain, IR and schema checks (T030-009..014).
3. Persistence and management API: contracts, endpoints, allow-lists, registry (US01, T030-015..019).
4. Caller publication and scheduling; admission; completion and apply (US02, T030-020..025); registry
   and time-class registrations (T030-070).
5. Delivery recovery and the fault suites (US04, T030-026..031); reset rules (T030-068).
6. Authorization battery and disclosure (US03, T030-032..037, T030-066, T030-067, T030-069).
7. Cancellation and deadlines (US05, T030-038..040, T030-065); quotas and depth (US06, T030-041..043).
8. Sync mode (US07, T030-044..045); observability and Studio (US08, T030-046..050).
9. SDKs, CLI and conformance (T030-051..054); compensation (US09, T030-055..057).
10. Docs, runbook, benchmark, release gates and capability enablement (T030-058..064).

## Verification plan

Each checkpoint records its commands and actual outcomes; a skipped or failing test is stated. The
release of the capability requires, with real PostgreSQL under the runtime role: the CP1–CP8 crash
matrix, including `COMPLETED` before `STARTED` and a dead `STARTED`; duplicate and lost-hint suites; the
two 100-round races; every effect-possible outcome routed to compensation or review; reset rules;
refusal of encoded values at publication, admission and result return; restart and shard takeover with 1,000
calls in flight; the isolation battery and disclosure canaries; quota and depth exhaustion on both
sides; the five-SDK conformance scenarios of [contracts/conformance.md](contracts/conformance.md);
contract checks (OpenAPI lint, AsyncAPI baseline, IR and contract schema compilation); Studio
accessibility checks and a real-browser journey; `scripts/check-architecture.py`; and the benchmark
report. The quickstart's happy and failure demos must run from a clean stack.

## Complexity Tracking

| Violation | Why needed | Simpler alternative rejected because |
|---|---|---|
| Two extra transactions per call (admission and apply) beyond the handler run itself | Keeps each namespace's run transactions independent (constitution I, ADR-0030 §18–19) | Completing the caller from the handler's terminal transaction locks two runs across namespaces (ADR-0065, option 2) |
| A message table instead of reusing `webhook_deliveries` | Point-to-point obligations with per-kind dedupe and shard placement | The outbound pipeline is subscription fan-out to external destinations with HTTP/broker semantics (ADR-0025) |
| A new document kind (`Service`) | The contract must be typed and pinned independently of handler workflows | Embedding contracts in endpoints would make every routing change a contract change and break pins |
