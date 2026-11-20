# Data model — feature 018

Proposed; nothing here exists. Every table is keyed by `tenant_id` first, carries `namespace_id`,
and references other tenant rows through composite foreign keys `(tenant_id, …)` as the existing
schema does (`crates/infra-postgres/migrations/20261009000100_execution.up.sql`). Times are
`timestamptz` written from the database clock. "Runtime role" is `sagat_app`. Variable-size columns
are classified in `crates/infra-postgres/storage-classification.json` (ADR-0027): `content`
(customer-controlled, charged to `tenant_content_bytes`), `protected_system` (bounded, closed shapes).

## 1. States and transitions

### Schedule

```text
            create                 pause(note)                  delete
   (none) ─────────▶ ACTIVE ◀──────────────────────▶ PAUSED ──────────────▶ DELETED (tombstone)
                       │  resume(note)                 ▲
                       │  authority revoked / failure  │
                       └───────────────────────────────┘  (system note, audit schedule.auto_paused)
   remainingActions = 0 ⇒ ACTIVE but no next action ("exhausted"); an update raising it re-arms.
```

`TRIGGER`-origin schedules are created and deleted by publication, never by the schedule API.

### Schedule action (one per tenant, schedule, nominal time)

| Outcome | Terminal | Run | Meaning |
| --- | --- | --- | --- |
| `STARTED` | yes | yes | Run admitted in the action's transaction. |
| `BUFFERED` | no → `STARTED`, `MISSED_CATCHUP_WINDOW`, `DROPPED_DELETED` | later | Waiting for tracked runs (BUFFER_ONE/ALL, CANCEL_OTHER). |
| `SKIPPED_OVERLAP` | yes | no | `SKIP` with an open run, or tracking limit. |
| `DROPPED_BUFFER_FULL`, `DROPPED_DELETED` | yes | no | Buffer full; schedule deleted while buffered. |
| `COALESCED` | yes | no | Catch-up mode `ONE` acted on an earlier missed time. |
| `MISSED_CATCHUP_WINDOW` | yes | no | Recorded once per recovery as a summary (count, first, last). |
| `ALREADY_ACTED` | yes | existing | A backfill or a replayed key found an existing action/run. |
| `AUTHORITY_REVOKED` | yes | no | Authorizing subject lost `run:start`; schedule paused. |
| `BUSINESS_ID_CONFLICT` | yes | no | Templated business id held by an open execution under `FAIL`. |

An owed time whose admission was refused has **no** action row: the schedule keeps `next_action_at`
and sets `retry_at` (plan transaction 4).

### Backfill

`PENDING → RUNNING → COMPLETED | CANCELLED`; cursor = last nominal time processed.

### Business-id binding

```text
(absent) ──start──▶ bound(open holder) ──holder terminal (observed, not written)──▶ bound(closed)
bound(closed) ──start allowed by reuse policy──▶ bound(new holder)
bound(closed, finished_at + window < now) ──purge──▶ (absent)
```

A chain holder is open until its chain has a final outcome; the binding stores the chain's initial
run id (the chain id, `execution_chains_initial_run_fk`).

### Delayed run

`WAITING (startDelayedUntil set, no step released) ──release──▶ RUNNING`, or, with 022 run-level
concurrency, `──release──▶ WAITING/CONCURRENCY` when the release enqueues it (022 FR-022-018); `cancel` ⇒
`CANCELLED`, deadline ⇒ `TIMED_OUT`. Journal: `run.started`, `run.start_delayed{until}`,
`run.start_released`. Instants follow 028's time classes (`workflow_due` for `start_delayed_until`).

### Batch job and item

```text
job:  SELECTING ──▶ RUNNING ──▶ COMPLETED
         │             │──cancel──▶ CANCELLING ──▶ CANCELLED
         └────────────┴──authority revoked / invariant──▶ FAILED
item: PENDING ──▶ SUCCEEDED | SKIPPED(reason) | FAILED(code) | CANCELLED
      PENDING ⇄ DEFERRED (quota, rate, lock wait; bounded attempts, then FAILED)
```

### Reset

```text
REQUIRE_TERMINAL:  (request) ──one transaction──▶ CREATED(new run)
CANCEL_SOURCE:     (request) ──cancel source + row──▶ PENDING_SOURCE ──source terminal, still eligible──▶ CREATED
                                                                     └──not eligible / no free 022 permit / purged──▶ FAILED(reason)
```

The source run never changes state because of a reset other than the ordinary cancellation that
`CANCEL_SOURCE` requests.

## 2. Tables

### `schedules`

| Column | Type | Notes |
| --- | --- | --- |
| `tenant_id`, `namespace_id`, `id` | uuid | PK `(tenant_id, id)`; `id` UUIDv7; FK namespace. |
| `name` | text | 1..128 `[a-z0-9][a-z0-9-]*`; unique `(tenant_id, namespace_id, name)` where `state <> 'DELETED'`. Content (customer text). |
| `origin` | text | `API` or `TRIGGER`. |
| `trigger_id` | uuid null | `TRIGGER` only; unique; FK `workflow_triggers(id)` with `ON DELETE` handled by publication in the same transaction. |
| `state` | text | `ACTIVE`, `PAUSED`, `DELETED`. |
| `head_revision` | int | FK `(tenant_id, id, head_revision)` → `schedule_revisions`, deferred. |
| `state_version` | bigint | CAS for every mutation. |
| `note` | text null | ≤ 512 chars; content. `note_by`, `note_at`. |
| `authorized_by` | text | Subject (≤ 255); protected (bounded identity). |
| `remaining_actions` | int null | 0..1,000,000. |
| `next_action_at`, `next_actual_at`, `retry_at`, `next_wake_at` | timestamptz null | `next_wake_at = least(next_actual_at, retry_at, recheck_at)`. |
| `tracked_runs` | uuid[] | ≤ 1,000 entries; protected (UUIDs). |
| `buffered` | int | Count of `BUFFERED` actions. |
| `counters` | jsonb | Closed object of non-negative integers per outcome; protected (generated schema). |
| `shard` | smallint | Generated `run_shard(id)` (ADR-0020 function). |
| `created_at`, `updated_at`, `deleted_at` | timestamptz | |

Indexes: `(shard, next_wake_at) WHERE state = 'ACTIVE'` (the pass's due scan, bounded by `now()` per
ADR-0020 decision 4); `(tenant_id, namespace_id, name)`.

### `schedule_revisions` (immutable)

`(tenant_id, schedule_id, revision)` PK; `spec jsonb` (closed schema of
`contracts/schedule-spec.schema.json`, content), `policies jsonb` (closed, protected), `action jsonb`
(workflow id, version policy, business-id prefix and policies, `nominalTimeField`, run timeout;
content because of the prefix), `input_digest bytea` → `payloads (tenant_id, digest)` (the input is a
content-addressed payload charged once per stored row), `actor`, `request_id`, `created_at`. ALWAYS
trigger refuses `UPDATE`/`DELETE` except retention purge of revisions no action references.

### `schedule_actions`

`(tenant_id, schedule_id, origin, nominal_at)` PK; `origin` (`SPEC` for scheduled and backfill
actions, which share the spec's nominal times; `MANUAL` for trigger-now, nominal time from
`clock_timestamp()` in microseconds, D1035); `kind` (`SCHEDULED`, `TRIGGERED`, `BACKFILL`),
`revision`, `outcome`, `reason` (closed code), `actual_at`, `decided_at`, `run_id uuid null` (FK runs),
`backfill_id uuid null`, `summary_count int null`, `summary_last_at timestamptz null` (for
`MISSED_CATCHUP_WINDOW`/`COALESCED` summaries). Protected (closed codes, ids, times). Index
`(tenant_id, schedule_id, decided_at DESC)` for recent actions; `(tenant_id, schedule_id) WHERE outcome
= 'BUFFERED'` ordered by `nominal_at`. Retention: keep the last 1,000 per schedule or 30 days (hard
10,000); `BUFFERED` rows are never purged.

### `schedule_backfills`

`(tenant_id, schedule_id, id)` PK; `range_start`, `range_end`, `cursor_at`, `overlap`, `state`,
`requested_by`, `request_id`, counters (closed jsonb), `created_at`, `finished_at`. Protected.
Unique open backfill count enforced by the namespace and per-schedule ceilings.

### `schedule_obligations`

`(tenant_id, schedule_id, nominal_at, run_id)` PK; `kind` (`CANCEL_RUN`), `state` (`PENDING`, `DONE`),
`idempotency_key`, `created_at`, `done_at`. Protected. Index `(tenant_id) WHERE state = 'PENDING'`.

### `business_ids`

| Column | Type | Notes |
| --- | --- | --- |
| `tenant_id`, `namespace_id`, `business_id` | uuid, uuid, text | PK `(tenant_id, namespace_id, business_id)`; `business_id` 1..200 visible ASCII (`CHECK`); **content** (caller text). |
| `execution_id` | uuid | Holder run id, or chain id (= initial run id); FK runs. |
| `workflow_id` | uuid | For display; FK workflows. |
| `bound_at` | timestamptz | |
| `reuse_window_s` | integer | Effective window frozen at binding (≤ 31,622,400). |

The holder's status and `finished_at` are read from `runs` / `execution_chains`, never copied, so no
run-driven transition writes this table. Purge (bounded batches, maintenance): rows whose holder is
terminal and `finished_at + reuse_window_s < now()`. Erasure (feature 020) of a holder deletes its
binding rows in the erasure transaction; the id then becomes free early, and the erasure report says so.

### Additive `runs` columns (all nullable)

`business_id text` (copy for listing and filter; index `(tenant_id, namespace_id, business_id
text_pattern_ops)`), `start_delayed_until timestamptz` (partial index `(run_shard(id),
start_delayed_until) WHERE start_delayed_until IS NOT NULL AND status = 'WAITING'`), `schedule_id uuid`,
`schedule_nominal_at timestamptz`, `schedule_action_kind text`, `schedule_revision int` (index
`(tenant_id, schedule_id, created_at)`), `reset_sequence bigint` (non-null only with `rerun_of`;
`CHECK (reset_sequence IS NULL OR rerun_of IS NOT NULL)`). `business_id` is content (customer text,
subject to 020 erasure); the others are protected. Existing rows keep nulls; no backfill. The
immutability trigger refuses every change of these columns after insert except `business_id → NULL`
in the statement that sets `archive_state = 'ERASED'` (feature 020's erasure path).

### `schedule_trigger_handover` (singleton)

One row: `state` (`LEGACY`, `HANDED_OVER`), `changed_by`, `changed_at`; protected; changed only by the
audited operator command, which is an activation of the instance-capability registry below: it requires
`schedules-v1` from every live `EXECUTION_ENGINE` instance and records that requirement in the same
transaction.

### Instance-capability registry (owned by 018; used by 029 and 031)

One deployment-level registry (not tenant-scoped) through which every process that holds platform
database credentials — each control-plane and each execution-engine replica — announces what its binary
can do, and through which a feature is activated only when every live process can honour it. Runners and
SDK workers hold no database credentials and keep the worker-protocol minor negotiation (ADR-0015).

| Table | Columns | Notes |
| --- | --- | --- |
| `serving_instances` | `instance_id uuid` PK (random per process start), `kind` (`CONTROL_PLANE`, `EXECUTION_ENGINE`; grows only), `scheduler_owner text null` (an engine's `scheduler_members.owner`, for correlation only), `binary_version text` (≤ 64), `capabilities text[]` (closed values, ≤ 32), `started_at`, `last_seen` | at most 1,000 rows; protected |
| `platform_requirements` | `capability text` PK, `kinds text[]` (the instance kinds that must hold it), `required_since`, `recorded_by` | protected |
| `platform_requirement_epoch` | singleton `epoch bigint` | incremented in every transaction that inserts or deletes a requirement |

Rules (D1034, G1607):

1. **Register before serving.** A process starts serving (first API request, first scheduler pass) only
   after one transaction that locks `platform_requirement_epoch` `FOR SHARE`, upserts its own row and
   reads the requirements; if a requirement for its kind names a capability it lacks, it does not serve
   (readiness false, `BINARY_CAPABILITY_MISSING` in its log). Taking the epoch row closes the race with a
   concurrent activation, which locks it `FOR UPDATE`.
2. **Heartbeat and liveness.** Every 10 s each process sets `last_seen = clock_timestamp()` (database
   clock); an instance is **live** when `last_seen` is within the last 30 s; a clean stop deletes its row;
   rows silent for more than one day are deleted by any heartbeat; a full table refuses registration.
3. **Re-check while serving.** Each API admission and stream re-evaluation (control plane) and each
   scheduler round (engine) compares a cached epoch; when it moved, the process re-reads the
   requirements and, lacking one, answers `503 UNAVAILABLE`/`BINARY_CAPABILITY_MISSING` (API) or stops its
   passes and fails readiness (engine).
4. **Activation (operator only).** A requirement is recorded **only** by an audited operator activation
   command, never inside a tenant or user transaction. A feature that needs every process is activated in
   one transaction: lock `platform_requirement_epoch` `FOR UPDATE` → verify that every live instance of the
   required kinds lists the capability → insert the requirement and bump the epoch → write the feature's
   own activation row → audit. Removing a requirement is the same command in reverse and only after the
   feature verifies that nothing depends on it. Until its operator activation, a feature refuses its own
   feature use with its own reason (018 `SCHEDULE_TRIGGER_HANDOVER_PENDING`, 029
   `ACCESS_CAPABILITY_DISABLED`, 031's equivalent).
5. **Capability values** are owned by the feature that introduces them: `schedules-v1` (018,
   `EXECUTION_ENGINE`), `access-narrowing-v1` (029, `CONTROL_PLANE`; activated by the operator before the
   first restricted folder or operation list can be created), `payload-encoding-v1` (031, `CONTROL_PLANE`
   and `EXECUTION_ENGINE`).

`scheduler_members` is unchanged (shard membership and the replica count for shard shares); adding
control-plane rows or capabilities to it would distort the shard-share computation of ADR-0020.
Grants: the runtime role reads all three tables and upserts/deletes only its own `serving_instances` row
(row ownership by `instance_id` through a `SECURITY DEFINER` function); requirement and epoch rows change
only through the operator command's definer function. The registry ships in a release before any feature
that activates through it (029's prerequisite release, T029-003); whichever of 018, 029, 031 is
implemented first ships it.

### `batch_jobs`

| Column | Type | Notes |
| --- | --- | --- |
| `tenant_id`, `namespace_id`, `id` | uuid | PK `(tenant_id, id)`; `id` UUIDv7. |
| `action` | text | Closed set (D1021). |
| `params` | jsonb | Closed per action (step id, instance path, operation name, update key); protected. |
| `payload_digest` | bytea null | Signal payload or update arguments → `payloads`; content via payload row. |
| `selector` | jsonb | Closed structured filter or run-id list (≤ 1,000 UUIDs); content (`businessIdPrefix`). |
| `reason` | text | 8..2000 chars; content. |
| `created_by` | text | Subject; protected. |
| `state` | text | `SELECTING`, `RUNNING`, `CANCELLING`, `COMPLETED`, `CANCELLED`, `FAILED`. |
| `failure_reason` | text null | Closed code. |
| `max_items`, `rate_milli` | int | Items per second × 1,000. |
| `selected`, `succeeded`, `skipped`, `failed`, `cancelled`, `deferred` | int | Counters updated in item transactions. |
| `select_cursor` | uuid null | Keyset cursor. |
| `selection_bound_at` | timestamptz | = creation instant (database clock). |
| `selection_horizon` | xid8 | `pg_snapshot_xmax(pg_current_snapshot())` at creation; selection starts when `pg_snapshot_xmin(pg_current_snapshot()) >= selection_horizon` (D1033). |
| `next_slot_at` | timestamptz | Durable rate slot. |
| `shard` | smallint | Generated `run_shard(id)`. |
| `created_at`, `started_at`, `finished_at`, `expires_at` | timestamptz | |

Index `(shard, next_slot_at) WHERE state IN ('SELECTING','RUNNING','CANCELLING')`.

### `batch_items`

`(tenant_id, job_id, ordinal)` PK; `run_id` (unique `(tenant_id, job_id, run_id)`), `state`,
`reason` (closed code), `error_code` (closed `Error.code`), `attempts smallint`, `next_attempt_at`,
`new_run_id uuid null` (rerun), `updated_at`. Protected. Index `(tenant_id, job_id, state, ordinal)`.
Items do not reference `runs` by foreign key, so a run purged by a future retention policy does not
block item retention; the item keeps the identifier only.

### `run_resets`

`(tenant_id, id)` PK; `namespace_id`, `source_run_id` (FK runs), `source_sequence bigint`,
`prefix_digest bytea(32)`, `reapply` (closed), `business_id_policy` (`KEEP`, `DROP`), `source_policy`,
`state` (`PENDING_SOURCE`, `CREATED`, `FAILED`), `failure_reason`, `new_run_id uuid null` (FK runs),
`actor`, `reason text` (8..2000, content), `reapply_report jsonb` (closed list ≤ 256 of
`{sequence, kind, outcome, reason}`; protected), `shard` generated, `created_at`, `completed_at`.
Index `(shard, created_at) WHERE state = 'PENDING_SOURCE'`.

### Journal

New event types in `run_events` (closed `detail` shapes in `contracts/journal-and-events.md`):
`run.start_delayed`, `run.start_released`, `run.reset`. The re-emitted prefix of a reset run uses the
existing event types; `runs.reset_sequence = N` marks sequences `1..N` as restored, every re-emitted
row carries `detail.restored = true` and the re-emitted `run.started` carries
`reset_of {source_run_id, source_sequence}`, so a journal reader (telemetry, 020 analytics, ADR-0056's
holder count) can skip restored history without reading another table. `wait.satisfied` gains an
optional `via` (`signal`, `event`, `signal_with_start`) in `detail` for new writes; old rows decode
unchanged. A `state.updated` row of an Update declared `reapplyOnReset: true` sets `payload_digest` to
the retained canonical Update arguments (a `payloads` row, content class); other Updates retain
nothing (`operation_update_receipts` keeps only a request hash).

### Payload references (entries of 020's content-store registry)

`schedule_revisions.input_digest`, `batch_jobs.payload_digest` and the retained-argument
`run_events.payload_digest` of `state.updated` reference `payloads (tenant_id, digest)` by foreign key
and are registered as payload references in `storage-classification.json` and as entries of 020's
content-store registry (`crates/infra-postgres/run-content-references.json`, owned by 020, see
`specs/020-visibility-analytics-archival/data-model.md`): retained Update arguments go with the
journal; schedule input and batch payloads keep their payload (not run content).
`run.started` detail gains optional schedule provenance and `businessId` is **not** journaled (it is
on the run row only).

## 3. Grants for the runtime role

| Table | Grants | Guard |
| --- | --- | --- |
| `schedules`, `schedule_backfills`, `schedule_obligations`, `batch_jobs`, `batch_items`, `run_resets` | `SELECT, INSERT, UPDATE` | `DELETE` only through bounded purge functions (`SECURITY DEFINER`, fixed `search_path`, tenant-filtered) |
| `schedule_revisions` | `SELECT, INSERT` | ALWAYS trigger refuses `UPDATE`/`DELETE` outside the purge function |
| `schedule_actions` | `SELECT, INSERT, UPDATE` | `UPDATE` only `BUFFERED → terminal`; trigger enforces |
| `business_ids` | `SELECT, INSERT, UPDATE, DELETE` | `DELETE` only rows past their window or of an erased holder (trigger check) |
| `runs` new columns | existing grants | immutable after insert (trigger) except `business_id → NULL` on erasure; `start_delayed_until` cleared only by release |
| `schedule_trigger_handover` | `SELECT` | changed only through a `SECURITY DEFINER` function used by the operator command |
| `serving_instances`, `platform_requirements`, `platform_requirement_epoch` | `SELECT`; own-row upsert/delete through a definer function | requirement and epoch rows only through the operator command's definer function |

`T018-009` adds every new statement to the runtime-privilege test
(`tests/integration/platform-foundation/runtime_privileges`).

## 4. Retention summary

| Data | Kept | Purged by |
| --- | --- | --- |
| Schedule revisions | while referenced by the head or any retained action/run | maintenance, bounded |
| Schedule actions | last 1,000 or 30 days per schedule; `BUFFERED` always | maintenance, bounded |
| Deleted schedule tombstone | 30 days after the last action purge | maintenance |
| Business-id bindings | until `finished_at + window` of a terminal holder | maintenance, bounded |
| Batch jobs and items | `expires_at` (30 d default, 90 d max) after terminal | maintenance, one job per transaction |
| Run resets | with the new run (no separate purge in this feature) | — (G42/G69 decide run retention) |

Purges release charged content bytes in the same transaction (ADR-0027).
