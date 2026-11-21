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

## Rate limits (release 0.3.0 slice, US07)

Proposed on 2026-11-13 (D022-001..017, ADR-0088); nothing here exists yet. Created by their own additive
migration (T022-070), tenant-keyed, granted to `sagat_app` with the privileges listed, every column
protected (closed values, patterns, digests, instants) and entered in `storage-classification.json`.

### Domain

`RateLimitPolicy {group: GroupName, key: Option<Expression>, limit: u32, per: Second | Minute | Hour,
queue_timeout: Duration}` at workflow level (run starts) or on a root `activity`/`script` node (attempt
dispatches). An absent key means the single key of the group. Group names follow the rules of
`ConcurrencyPolicy`; a rate-limit group and a concurrency group of the same name are unrelated.

A grant **counts** for an acquirer with window `per` at instant `now` when: (run scope) `granted_at >
now - per`; (step scope) it has no lease instant yet, or `leased_at > now - per`. An acquirer is admitted
when the grants of its key that count are fewer than its `limit` and no admitted waiter is ahead of it
(D022-003, D022-005, D022-009). All instants are database-clock values read inside the transaction.

Run transitions added: admission → `WAITING(RATE_LIMIT)` (buffered) → `RUNNING` (granted) |
`FAILED(RATE_LIMIT_WAIT_TIMEOUT)` | `CANCELLED`. A run buffered for both gates is `WAITING(CONCURRENCY)`
while a concurrency waiter row exists for it and shows both states on the run resource.

Step transitions added: would-be-ready → `WAITING(RATE_LIMIT)` (no attempt, not claimable) → `READY`
(granted) | failure route (`RATE_LIMIT_WAIT_TIMEOUT`). Each retry repeats it after its backoff.

When US03 is built, `RATE_LIMIT_WAIT_TIMEOUT` joins the `kind` values of `FailureContext` with the meaning
of `CONCURRENCY_WAIT_TIMEOUT` (the run never started work; only the namespace handler may apply).

### `rate_limit_windows`

PK `(tenant_id, namespace_id, pool, limit_scope, group_name, key_digest)`; `pool` CHECK `IN
('ORDINARY','DRAFT_TEST')`; `limit_scope` CHECK `IN ('RUN','STEP')`; `group_name` as in
`concurrency_semaphores`; `waiters integer` and `live_grants integer` with CHECK `>= 0`; `grants_total
bigint` (every grant ever made under this row, for the fault-suite check against the journals);
`key_version smallint`; `updated_at`. It is the lock of the key: grant and waiter rows of a key change
only under it. A row with no waiter and no grant row is deleted by the purge (T25). Grants: SELECT,
INSERT, UPDATE, DELETE.

### `rate_limit_grants`

PK `(tenant_id, run_id, node, attempt)`; `node` is `''` and `attempt` 0 for a run-scope grant;
`namespace_id`, `pool`, `limit_scope`, `group_name`, `key_digest` (FK to `rate_limit_windows`, restrict);
`declared_limit integer`, `period_ms integer` CHECK `IN (1000, 60000, 3600000)`; `granted_at timestamptz`;
`leased_at timestamptz null` (run scope: equal to `granted_at`; step scope: null until the pass copies
the lease instant, T23); `shard smallint`. FK `(tenant_id, run_id)` → `runs`. Indexes: `(tenant_id,
namespace_id, pool, limit_scope, group_name, key_digest, leased_at)` for the count; `(shard) WHERE
leased_at IS NULL` for lease recording. A row is deleted when no acquirer can count it any more (older
than one hour after `leased_at`, the longest window), or when its step ends without a lease. Grants:
SELECT, INSERT, UPDATE (`leased_at` only), DELETE.

### `rate_limit_waiters`

PK `(tenant_id, run_id, node)`; `node` is `''` for a buffered run; `attempt integer`; `namespace_id`,
`pool`, `limit_scope`, `group_name`, `key_digest` (FK to `rate_limit_windows`); `declared_limit`,
`period_ms`; `enqueued_at` (database clock of the buffering transaction; ties broken by `run_id`, then
`node`); `deadline_at`; `shard smallint` (= `run_shard(run_id)`). Indexes: `(tenant_id, namespace_id,
pool, limit_scope, group_name, key_digest, enqueued_at, run_id, node)` for oldest first; `(shard,
enqueued_at)` for the grant pass. FK `(tenant_id, run_id)` → `runs`. Grants: SELECT, INSERT, DELETE.

A run buffered for both gates has one row here and one in `concurrency_waiters`; the grant, the timeout
and the cancellation remove both in one transaction.

### Consistency invariants added (checked by `verify_run` / `assert_consistent`)

8. `rate_limit_windows.waiters` = count of `rate_limit_waiters` of the key, and every waiter row belongs
   to a run or step that is `WAITING` with an open `rate_limit.queued` (no later `rate_limit.granted`,
   `rate_limit.wait_timed_out` or `rate_limit.dequeued` for that run, node and attempt).
9. Every `rate_limit.granted` that is not `restored` has, while it can still count, one
   `rate_limit_grants` row with the same `granted_at`; `grants_total` grows by exactly one per such
   event; no run, node and attempt has two.
10. For every key and every acquirer `(limit, period)` seen in its journals, no interval of length
    `period` holds more than `limit` counting instants (run scope: `granted_at_us` of the events; step
    scope: the instants of the matching `step.leased` events). Checked from the journals alone.

### Retention added

Windows, grants and waiters are live state: a waiter lives until its grant, timeout or dequeue; a grant
row at most one hour after its lease; a window row until it is empty. Rows carry the digest
`key_version`; a new version is not activated while any exist (ADR-0056 decision 7, extended by ADR-0088
decision 8). Nothing here is retained content, and nothing is copied from a run's payloads, so 020's
erasure has nothing to register for these tables beyond the foreign keys to `runs`.

### Lock order added

… → `runs` insert or update → semaphores (slot 7) → **rate-limit window rows, sorted by (pool, scope,
group, key), then their grant and waiter rows** → quota and usage rows and the deferred content flush,
always last. The window rows are rate rows of ADR-0050 slot 8b, which ADR-0056 decision 2 already allows
after a semaphore. After a window row a transaction locks no run, count, activity-quota or semaphore row.

## Revision after the reviews of 2026-11-13 (D022-019..034, ADR-0089 proposed)

Where this section and the ones above disagree, this section wins.

### Steps and their gates

Step transitions, replacing the step lines of "Concurrency" and "Rate limits": for a node that declares
`concurrency` or `rateLimit`, `step.released`, `step.retry_due` and `step.retry_requested` lead to
`WAITING(RATE_LIMIT)` or `WAITING(CONCURRENCY)` (the first gate not yet granted, rate first), never to
`READY`; the grant that completes the last declared gate leads to `READY`. A granted, unleased step that
leaves `READY` other than by a lease returns its slot (`rate_limit.returned`) and permit
(`concurrency.released`) and waits again (pause) or ends. `step_concurrency_requirements` is not built: a
step waiting for a permit is a `concurrency_waiters` row with `permit_scope = 'STEP'` and its `node`, and a
step holder is created at grant, not at claim.

### Columns added

| Table | Column | Notes |
|---|---|---|
| `concurrency_semaphores`, `rate_limit_windows` | `key_shard smallint` | stable hash of (tenant, namespace, pool, scope, group, key digest) modulo 64; the shard whose owner grants the key |
| same | `next_eligible_at timestamptz null` | hint kept under the key's lock, early never late; partial index `(key_shard, next_eligible_at) WHERE waiters > 0` |
| `rate_limit_windows` | `max_period_ms integer` | the longest period among its live grants and waiters; the purge keeps a grant for it plus one minute |
| `concurrency_waiters` | `node text` (`''` for a run), PK `(tenant_id, run_id, node)`; `attempt integer` | step waiters |
| `concurrency_holders` | `attempt integer` | the attempt a step permit was granted for (`attempt() + 1`) |
| `rate_limit_grants` | `attempt` is the attempt the grant is for | lease recording accepts only that attempt's `step.leased` committed after the grant |

Every foreign key to `runs` from a flow-control table is `(tenant_id, namespace_id, run_id)` →
`runs (tenant_id, namespace_id, id)` (constraint `runs_tenant_namespace_id_uniq`).

### `flow_control_counts`

PK `(tenant_id, namespace_id, slot)`, `slot` 0..15; `queued_runs bigint`, `live_keys bigint`, both CHECK
`>= 0`. Slot-8b usage rows (ADR-0050). Maintained only by `SECURITY DEFINER` triggers on the waiter rows
(a run counts once whatever its waiter rows) and on the semaphore and window rows (a key counts once per
row); `sagat_app` has `SELECT` only, as for `run_active_counts`.

### `flow_control_key_versions`

PK `version smallint`; `state` CHECK `IN ('ACTIVE','RETIRING','RETIRED')` with at most one `ACTIVE`; `activated_at`,
`retired_at`. Deployment-wide, protected; the key material is configuration, never in a row. Read `FOR
SHARE` in every deciding transaction; changed only by an audited operator command.

### Grants and counters

`waiters`, `holders` and `live_grants` on semaphore and window rows are maintained by `SECURITY DEFINER`
triggers on the holder, waiter and grant rows; `sagat_app` may `UPDATE` only `next_eligible_at` and
`updated_at` of those rows (enough for `SELECT ... FOR UPDATE`). The purge decrements `live_grants` by the
rows it actually deleted.

### Invariants, revised and added

10. *(revised, D022-032)* For each grant `e` of a key with instant `t_e`, limit `limit_e` and period
    `period_e` (run scope: `granted_at_us`; step scope: the instant of the matching `step.leased`), the
    grants of that key whose instants lie in `(t_e − period_e, t_e]` number at most `limit_e`. Checked from
    the journals alone.
11. Every commit in which a step of a gated node leaves `RUNNING`/`RETRY_WAIT` towards readiness contains,
    for that step, either the grants of all its gates or a `*.queued`; no gated step is `READY` without an
    open grant for each declared gate.
12. Every waiter row belongs to a run that is not terminal; a breach raises `flow_control_waiter_orphaned`.
13. At no instant does a key have more granted, unleased steps than the smallest `limit` among them.
