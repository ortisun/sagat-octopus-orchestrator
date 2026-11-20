# Data model — feature 022 flow control and step policies

Proposed; nothing here exists yet. Every table is tenant-keyed, carries the namespace, is created by one
additive migration allocated after `20261220000000` (D1191), is granted to the runtime role `sagat_app`
with the minimum privileges listed, and has each variable-size column entered in
`crates/infra-postgres/storage-classification.json` (ADR-0027). Classes: **content** (charged to
`tenant_content_bytes`), **protected** (bounded by a closed schema, pattern or size check with a test).

## Domain entities and states

### Cache policy and entry

`CachePolicy {key: Expression, ttl: Duration, scope: Version | Workflow | Namespace(name), version:
Option<CacheVersion>}` on a root `activity`/`script` node. Validation: FR-022-001.

`NodeFingerprint` = digest of (activity name or artifact digest, canonical key-expression text, author
version, `outputSchema` carrier digest, classification rules version). `CacheScopeId` = digest of (scope
kind, pinned version id | workflow id | cache name, fingerprint). `KeyDigest` = HMAC-SHA-256 under the
namespace-separated flow-control key over the canonical key value (D1168).

Entry lifecycle: `obligation PENDING` → `READY` (stored) → deleted (expired and purged, or invalidated).
A scope can be invalidated as a whole: entries created at or before `invalidated_at` are dead for lookups
and purged later. Entries are immutable.

Step transitions added: `READY → SUCCEEDED` by `step.cache_hit` (no lease, attempt 0).

### Early result

`NodeKind::EarlyReturn {result: Expression}` with `next`. Run fields: `result_source ∈ {EARLY_RETURN,
END}`, `result_available_at`. The result is set once (D1169); `end.reached` after an early result records
`result_already_returned` in its detail.

### Failure handler

`FailureHandler {target: Region(NodeId) | Child {workflow, version, input}, on: StatusSet, timeout}`.

Run phase: `forward` → (stop finished with pending status P ∈ `on`) → `failure_handler.started{P}` →
handler region or derived child `onFailure~h` → `failure_handler.finished{outcome}` → `run.finished{P}`.
During the phase the run status is `RUNNING` or `WAITING` and `runs.failure_handler_state = RUNNING`;
the pending status is stored in `runs.pending_terminal_status`. `CANCELLED` never enters the phase, nor
does a run that never acquired its run permit (queue timeout, key error at admission): it finishes
directly and only the namespace handler may apply (D1197). A run holding a permit keeps it through the
phase and releases it in its terminal transaction.

`FailureContext {status, code, node, attempt, kind, occurredAt}`: closed, payload-free (D1195);
`kind ∈ {STEP_FAILED, RUN_TIMEOUT, COMPENSATED, PARTIAL, COMPENSATION_FAILED, CONCURRENCY_WAIT_TIMEOUT,
EXPRESSION_ERROR, STORAGE_QUOTA_EXCEEDED}`. `CONCURRENCY_WAIT_TIMEOUT` as a kind (run never admitted to
work) reaches only the namespace handler; a step-level timeout is `STEP_FAILED` with that code.

Dispatch lifecycle: `PENDING → ADMITTED` (handler run created) | `PENDING → DEAD` (after 12 attempts);
`DEAD → PENDING` by redrive (audited).

### Concurrency

`ConcurrencyPolicy {group: GroupName, key: Option<Expression>, limit: u32, queue_timeout: Duration}` at
workflow level (run permit) or on a root `activity`/`script` node (step permit). An absent key means the
single key of the group.

Run transitions added: admission → `WAITING(CONCURRENCY)` (queued) → `RUNNING` (granted) |
`FAILED(CONCURRENCY_WAIT_TIMEOUT)` | `CANCELLED`. Holder released at the run's terminal transaction.

Step: `READY` with a requirement row → `LEASED` (permit acquired at claim) | `FAILED`
(`CONCURRENCY_WAIT_TIMEOUT`). Permit released when the attempt ends.

### Standalone and test executions

Private retained definitions reuse the 017 index `(tenant_id, namespace_id, actor, digest)` with origin
`STANDALONE_STEP` or `DRAFT_TEST` on the run. `TestPin {node, source: Mock | RunPin{run, node}, payload
digest, floor}`; pins live only in run data, never in a definition (D1186).

## PostgreSQL tables

### `step_cache_scopes`

| Column | Type | Notes |
|---|---|---|
| `tenant_id`, `namespace_id` | uuid | FK `(tenant_id, namespace_id)` → `namespaces (tenant_id, id)` |
| `scope_digest` | bytea | 32 bytes; PK `(tenant_id, namespace_id, scope_digest)` |
| `scope_kind` | text | CHECK `IN ('VERSION','WORKFLOW','NAMESPACE')` |
| `workflow_id`, `version_id` | uuid null | FK to `workflows`/`workflow_versions` with tenant; null for `NAMESPACE` |
| `cache_name` | text null | pattern `^[a-z][a-z0-9.-]{0,63}$`; required for `NAMESPACE` |
| `node` | text | NodeId pattern; protected |
| `fingerprint` | bytea | 32 bytes |
| `invalidated_at` | timestamptz null | dead-before marker |
| `revision` | bigint | bumped by invalidation |

Grants: SELECT, INSERT, UPDATE (`invalidated_at`, `revision` only). Classification: all protected.

### `step_cache_entries`

| Column | Type | Notes |
|---|---|---|
| `tenant_id`, `namespace_id`, `scope_digest` | | FK to `step_cache_scopes` |
| `key_digest` | bytea | 32 bytes; PK `(tenant_id, namespace_id, scope_digest, key_digest)` |
| `entry_id` | uuid | UNIQUE `(tenant_id, entry_id)` |
| `output` | jsonb | **content**; CHECK `octet_length(output::text) <= 262144` |
| `output_digest`, `carrier_digest` | bytea | 32 bytes each |
| `classification_rules` | integer | copied from the producing version |
| `source_run_id`, `source_node`, `source_attempt`, `source_seq` | uuid, text, integer, bigint | provenance; no FK (a run's later archival must not delete or block entries) |
| `created_at`, `expires_at` | timestamptz | database clock; CHECK `expires_at > created_at` |

Indexes: `(tenant_id, expires_at)` for purge; `(tenant_id, source_run_id)` for erasure (T18). Grants:
SELECT, INSERT, DELETE; an `ALWAYS` trigger refuses UPDATE. Byte trigger charges `output` on insert and
releases on delete.

### `step_cache_hit_provenance`

PK `(tenant_id, consumer_run_id, node)`; `namespace_id`, `source_run_id`, `entry_id`, `payload_digest`,
`created_at`. Written in T1 next to the copied payload, so 020's erasure can find every run that received
a copy of a source run's output. Index `(tenant_id, source_run_id)`. FK `(tenant_id, consumer_run_id)` →
`runs`; no FK to the source (archival of the source must not cascade). Grants: SELECT, INSERT, DELETE.
Protected.

### `step_cache_store_obligations`

PK `(tenant_id, run_id, node, attempt)`; `namespace_id`, `scope_digest`, `key_digest`, `ttl_seconds`
(1..2,592,000), `created_at`, `lease_owner uuid null`, `lease_expires_at null`, `tries smallint`. FK
`(tenant_id, run_id)` → `runs`. Index `(lease_expires_at NULLS FIRST, created_at)`. Grants: SELECT,
INSERT, UPDATE (lease columns), DELETE. Protected. The obligation references the completed attempt's
payload row by FK. Erasure of the run deletes its obligations (T18); the
writer skips an obligation whose source payload no longer exists.

### `concurrency_semaphores`

PK `(tenant_id, namespace_id, pool, permit_scope, group_name, key_digest)`; `pool` CHECK `IN
('ORDINARY','DRAFT_TEST')`, so draft test trees never share a row with ordinary runs; `permit_scope` CHECK
`IN ('RUN','STEP')`, so run and step permits never share a row; `group_name` is an explicit group (pattern
`^[a-z][a-z0-9.-]{0,63}$`) or a derived default (`=<workflow>` for run permits, `=<workflow>/<node>` for
step permits; at most 130 characters); `holders integer` and `waiters integer` with CHECK `>= 0`;
`key_version smallint` (digest key version, G1163); `updated_at`. A row with `holders = 0 AND waiters = 0` is deleted by
the transaction that empties it. Grants: SELECT, INSERT, UPDATE, DELETE. Protected.

### `concurrency_holders`

PK `(tenant_id, run_id, holder_scope, node)`; `holder_scope` CHECK `IN ('RUN','STEP')`; `node` is `''`
for RUN; `generation bigint` (STEP: the attempt's lease generation); `namespace_id`, `pool`, `permit_scope`
(= `holder_scope`), `group_name`, `key_digest`, `declared_limit integer`, `acquired_at`. FK `(tenant_id, run_id)` → `runs`; FK to
`concurrency_semaphores` (restrict). Index `(tenant_id, namespace_id, group_name, key_digest)`. Grants:
SELECT, INSERT, DELETE. Protected.

### `concurrency_waiters`

PK `(tenant_id, run_id)`; `namespace_id`, `pool`, `permit_scope` (always `RUN`), `group_name`, `key_digest`,
`declared_limit`, `enqueued_at` (database clock in the admission transaction; ties broken by `run_id`),
`deadline_at`, `shard smallint` (= `run_shard(run_id)`, ADR-0020).
Indexes: `(tenant_id, namespace_id, group_name, key_digest, enqueued_at, run_id)` for oldest-first;
`(shard, enqueued_at)` for the grant pass. FK to `runs` and to the semaphore. Grants: SELECT, INSERT,
DELETE. Protected.

### `step_concurrency_requirements`

PK `(tenant_id, run_id, node)`; `namespace_id`, `pool`, `permit_scope` (always `STEP`), `group_name`,
`key_digest`, `declared_limit`, `ready_since`, `deadline_at`, `shard`. Written when the step becomes ready
(again on each retry), deleted at claim (a holder row replaces it) or at timeout. Indexes: `(tenant_id, namespace_id, group_name, key_digest, ready_since)`;
`(shard, deadline_at)`. Grants: SELECT, INSERT, DELETE. Protected.

### `namespace_failure_handlers`

PK `(tenant_id, namespace_id)`; `workflow_id` (FK with tenant), `version_number integer null`, `statuses
text[]` (CHECK non-empty subset of the five statuses), `enabled boolean`, `revision bigint`, `reason
text` (**content**, 8..2000 characters, user-entered justification), `updated_by text`, `updated_at`.
Grants: SELECT, INSERT, UPDATE, DELETE.

### `failure_handler_dispatches`

PK `(tenant_id, source_run_id)`; `namespace_id`, `shard`, `workflow_id`, `version_number null`,
`config_revision`, `failure_context jsonb` (protected: closed schema, CHECK `octet_length <= 2048`),
`state` CHECK `IN ('PENDING','ADMITTED','DEAD')`, `attempts smallint` (0..12), `next_attempt_at`,
`handler_run_id uuid null` (UNIQUE with tenant), `last_error_code text null` (closed set), `created_at`,
`updated_at`. FK `(tenant_id, source_run_id)` → `runs`. Index `(shard, next_attempt_at) WHERE state =
'PENDING'`. Grants: SELECT, INSERT, UPDATE, DELETE.

### `test_run_pins`

PK `(tenant_id, run_id, node)`; `namespace_id`, `source_kind` CHECK `IN ('MOCK','RUN_PIN')`,
`payload_digest bytea` (FK to the test run's own payload row, charged once), `source_run_id uuid null`,
`source_node text null`, `floor_level text null` CHECK `IN ('public','internal','pii')`, `created_at`.
Immutable (`ALWAYS` trigger refuses UPDATE). Index `(tenant_id, source_run_id) WHERE source_run_id IS NOT
NULL` for erasure. Grants: SELECT, INSERT, DELETE. Protected.

### Content stores registered with 020 (FR-022-037, coordinator decision C8)

Feature 020's reference discovery (020 `data-model.md`, "Reference discovery") reads foreign keys from
`pg_catalog` plus the content-store registry `crates/infra-postgres/run-content-references.json` of non-FK
copies with run provenance and their erasure hook. This feature registers there `step_cache_entries`
(inline copy of `source_run_id`'s output; hook: invalidate by `source_run_id`, T18) and
`step_cache_hit_provenance` (consumer copies by `source_run_id`; hook: list for 020's erasure policy,
G1172). `test_run_pins.payload_digest` and `step_cache_store_obligations` reference the run's payload rows
by foreign key, so the catalog covers them; `test_run_pins.source_run_id` is registered as a copy with
provenance. Every column is also classified in `storage-classification.json`; a referencing column in
neither place fails 020's migration test.

### `runs` (additive columns)

| Column | Type | Notes |
|---|---|---|
| `result_source` | text null | CHECK `IN ('EARLY_RETURN','END')`; set with `result_digest` |
| `result_available_at` | timestamptz null | |
| `handler_for_run_id` | uuid null | FK `(tenant_id, handler_for_run_id)` → `runs`; UNIQUE `(tenant_id, handler_for_run_id)` where not null |
| `test_tree` | boolean | NOT NULL DEFAULT false; true for `DRAFT_TEST` and its descendants |
| `failure_handler_state` | text null | CHECK `IN ('RUNNING','SUCCEEDED','FAILED','TIMED_OUT','CANCELLED')` |
| `pending_terminal_status` | text null | one of the five handler statuses |

The `origin` CHECK of migration `20261125000000_database_ad_hoc` is extended with `STANDALONE_STEP` and
`DRAFT_TEST`. All new `runs` columns are protected (closed values).

## Consistency invariants (checked by `verify_run` / `assert_consistent` after fault tests)

1. `concurrency_semaphores.holders` = count of `concurrency_holders` for the key = open
   `concurrency.acquired` events without `concurrency.released` in the runs' journals, ignoring events that
   an 018 reset copied and marked `restored`.
2. `concurrency_semaphores.waiters` = count of `concurrency_waiters` for the key; every waiter run is
   `WAITING` with an open `concurrency.queued`.
3. A run with `result_source` has `result_digest` equal to the payload digest of its `result.returned` or
   `end.reached` event, and at most one of them carries a result.
4. A run with `failure_handler_state` has exactly one `failure_handler.started`; a terminal one has one
   `failure_handler.finished` and `status = pending_terminal_status`.
5. A dispatch `ADMITTED` names a run whose `handler_for_run_id` is the dispatch's source run.
6. Every `step.cache_hit` and `step.mocked` references a payload retained by that run; every
   `step.cache_hit` has a matching `step_cache_hit_provenance` row.
7. No row of any concurrency table carries `pool = 'ORDINARY'` for a run with `test_tree = true`, or the
   reverse.

## Retention

- Cache entries: until expiry (purged in bounded batches by the engine maintenance loop) or invalidation;
  scope rows with no entries are deleted once `invalidated_at` (or creation) is older than the 30-day TTL
  ceiling.
- Store obligations: deleted when stored or skipped; obligations older than one hour are dropped and
  counted (`skipped_backlog`).
- Erasure (020, T18): entries with that `source_run_id` and the run's obligations are deleted with byte
  release; hit provenance and pins are kept so 020 can reach the consumer copies, and are deleted with
  their own (consumer) run.
- Semaphores, holders, waiters, requirements: live state only; deleted when released or empty. Rows carry
  the digest `key_version`; a new version is not activated while any exist (ADR-0056 decision 7).
- Handler configuration: until cleared (audited). Dispatches: retained with their source run (FK), so
  provenance outlives neither run nor archival decisions of 020.
- Pins: retained with their test run; test runs follow the run retention of the namespace.

## Lock order summary

chain (if any) → run → shard fence only where the path already holds one (scheduler passes; worker
completions hold none and T1 adds none) → path-specific existing locks (drain fence, session, activity
quota; on admission, slots 1–5 of ADR-0050 decision 7) → `runs` insert or update (and its
`run_active_counts` trigger rows; slot 6) → cache scope → cache entry (readiness transitions) →
semaphores (sorted by pool, scope, group, key; slot 7) → namespace failure-handler policy row
(`FOR SHARE`, terminal path only) → quota, rate and usage rows and the deferred content flush, always last
(slot 8b). Semaphores come after every entity lock a path takes before it acquires, releases or grants a
permit; after a semaphore a transaction locks only the listed transition rows (the failure-handler
policy row) and usage rows, never an existing run, count or activity-quota row. The cache rows and the
failure-handler policy row are the transition rows ADR-0050 decision 7 lets each ADR list; ADR-0056
decision 2 gives every path's order.
