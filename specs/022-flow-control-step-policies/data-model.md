# Data model — feature 022 flow control and step policies

Proposed; nothing here exists yet. Every table is tenant-keyed, carries the namespace, is created by one
additive migration allocated after `20261220000000` (D1191), is granted to the runtime role `sagat_app`
with the minimum privileges listed, and has each variable-size column entered in
`crates/infra-postgres/storage-classification.json` (ADR-0027). Classes: **content** (charged to
`tenant_content_bytes`), **protected** (bounded by a closed schema, pattern or size check with a test).

*Amended 2026-11-24 (decision D022-220):* each slice of the rest of the feature adds its own additive migration,
numbered after the latest on the tree when it is written (`20270209000100` at `d8d3cf79`), not one migration after
`20261220000000`.

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

*Amended 2026-11-24 (the rest of 022): see [Revision of 2026-11-24](#revision-of-2026-11-24-the-rest-of-022), which wins where it differs: a cacheable step waits `WAITING(CACHE_LOOKUP)` and leaves it by `step.cache_hit` or `step.cache_missed`
(decision D022-203).*

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

*Amended 2026-11-24 (the rest of 022): see [Revision of 2026-11-24](#revision-of-2026-11-24-the-rest-of-022), which wins where it differs: no `output` column and no byte trigger; the entry references `payloads (tenant_id, digest)`
(decision D022-205).*

### `step_cache_hit_provenance`

PK `(tenant_id, consumer_run_id, node)`; `namespace_id`, `source_run_id`, `entry_id`, `payload_digest`,
`created_at`. Written in T1 next to the copied payload, so 020's erasure can find every run that received
a copy of a source run's output. Index `(tenant_id, source_run_id)`. FK `(tenant_id, consumer_run_id)` →
`runs`; no FK to the source (archival of the source must not cascade). Grants: SELECT, INSERT, DELETE.
Protected.

*Amended 2026-11-24 (the rest of 022): see [Revision of 2026-11-24](#revision-of-2026-11-24-the-rest-of-022), which wins where it differs: the foreign key to `runs` includes the namespace (decision D022-213).*

### `step_cache_store_obligations`

PK `(tenant_id, run_id, node, attempt)`; `namespace_id`, `scope_digest`, `key_digest`, `ttl_seconds`
(1..2,592,000), `created_at`, `lease_owner uuid null`, `lease_expires_at null`, `tries smallint`. FK
`(tenant_id, run_id)` → `runs`. Index `(lease_expires_at NULLS FIRST, created_at)`. Grants: SELECT,
INSERT, UPDATE (lease columns), DELETE. Protected. The obligation references the completed attempt's
payload row by FK. Erasure of the run deletes its obligations (T18); the
writer skips an obligation whose source payload no longer exists.

*Amended 2026-11-24 (the rest of 022): see [Revision of 2026-11-24](#revision-of-2026-11-24-the-rest-of-022), which wins where it differs: namespaced foreign keys, the payload referenced by `(tenant_id, payload_digest)`.*

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

*Amended 2026-11-24 (the rest of 022): see [Revision of 2026-11-24](#revision-of-2026-11-24-the-rest-of-022), which wins where it differs: namespaced foreign key; the row is locked at slot 3 by the dispatcher (decision D022-214).*

### `test_run_pins`

PK `(tenant_id, run_id, node)`; `namespace_id`, `source_kind` CHECK `IN ('MOCK','RUN_PIN')`,
`payload_digest bytea` (FK to the test run's own payload row, charged once), `source_run_id uuid null`,
`source_node text null`, `floor_level text null` CHECK `IN ('public','internal','pii')`, `created_at`.
Immutable (`ALWAYS` trigger refuses UPDATE). Index `(tenant_id, source_run_id) WHERE source_run_id IS NOT
NULL` for erasure. Grants: SELECT, INSERT, DELETE. Protected.

*Amended 2026-11-24 (the rest of 022): see [Revision of 2026-11-24](#revision-of-2026-11-24-the-rest-of-022), which wins where it differs: namespaced foreign key; the payload by `(tenant_id, payload_digest)`; written at slot 6 with the run.*

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
| `flow_control_gates` | jsonb null | *Added 2026-11-20 (T022-022, D022-049; migrations `20270203000000`, `20270203000100`).* The gates of the run's start as the reducer left them (phase, pool, group, 12-hex key label, a wait's deadline; never a key's value), the same shape as `steps.flow_control_gates`; CHECK `runs_flow_control_gates_bounded` (an object of at most 1,024 bytes); written only by a commit with a run-scope gate event; protected |
| `failure_handler_state` | text null | CHECK `IN ('RUNNING','SUCCEEDED','FAILED','TIMED_OUT','CANCELLED')` |
| `pending_terminal_status` | text null | one of the five handler statuses |

The `origin` CHECK of migration `20261125000000_database_ad_hoc` is extended with `STANDALONE_STEP` and
`DRAFT_TEST`. All new `runs` columns are protected (closed values).

*Amended 2026-11-24 (the rest of 022): see [Revision of 2026-11-24](#revision-of-2026-11-24-the-rest-of-022), which wins where it differs: `handler_for_run_id`'s foreign key includes the namespace; `test_tree` is copied by every child
admission and never changes (decisions D022-212, D022-213).*

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
   *Amended 2026-11-24 (decision D022-205):* a `step.cache_hit`'s payload is the entry's payload row, referenced, not
   copied.
7. No row of any concurrency table carries `pool = 'ORDINARY'` for a run with `test_tree = true`, or the
   reverse.
   *Amended 2026-11-24 (decision D022-212):* over the concurrency and rate-limit tables; checked by
   `support::flow_control::key_consistency`.

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

*Amended 2026-11-24 (the rest of 022): see [Revision of 2026-11-24](#revision-of-2026-11-24-the-rest-of-022), which wins where it differs: the as-built order of a commit (evaluation rounds before `persist`) is restated there, and the
failure-handler policy row is read without a lock (decision D022-208).*

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

*Revised 2026-11-25 (task T022-105, decision D022-084; migration `20270213000200`):* built as `flow_control_counts
(tenant_id, namespace_id, slot 0..15, queued_runs bigint)`, CHECK `queued_runs >= 0`, foreign key `(tenant_id,
namespace_id)` → `namespaces`; the slot is `get_byte(uuid_send(run_id), 15) & 15`, the slots of `run_active_counts`;
`flow_control_counts_track` (`SECURITY DEFINER`, `search_path = pg_catalog, pg_temp`, every name schema-qualified, not
executable by the runtime role) runs after each insert and delete of a waiter row of either table and counts a run-scope
row only, a run once: it adds one only when the run has no run-scope waiter row in the other table and takes one only
when none is left there. The column `live_keys` is not built: the live keys are counted from the semaphore and window
rows of the digest versions in force.

*Lock order, corrected 2026-11-25 (batch 3 reviews, correctness Major 1):* a slot row is locked after the run's active-run
counter row (slot 6) and after the start's keys (slot 7), on every path that writes or deletes a run-scope waiter row.
The grant of a queued start can end the run in its own transaction (an entry that is an `end` node), so it locks the
run's active-run counter row before the start's keys, as an admission does: run, slot 6, keys, slot 8b
(`concurrency_locks::the_grant_of_a_queued_start_that_ends_takes_its_counter_row_before_the_key_and_the_queued_count`).

### `flow_control_held_signals`

*Added 2026-11-25 (task T022-109, decision D022-087; migration `20270214000000`).* The signal of a Signal-with-Start
whose new run queued, held until the grant of the run's start delivers it. PK `(tenant_id, run_id)` (one held signal a
run); `namespace_id`; `name text` (CHECK 1 to 128 characters, the signal's name as the request gave it); `payload_digest
bytea` (CHECK 32 bytes), foreign key `(tenant_id, payload_digest)` → `payloads`; `actor text` (the requesting subject,
CHECK 1 to 255) and `trace_id text` (CHECK the W3C format when present), so the delivery is audited as the caller's;
`held_at timestamptz`. Foreign key `(tenant_id, namespace_id, run_id)` → `runs (tenant_id, namespace_id, id)` `ON DELETE
CASCADE`. Written by the creation transaction of the run, deleted by the grant that delivers it or by the commit that
ends the queued run (audited `run.signal_dropped`). `sagat_app`: `SELECT`, `INSERT`, `DELETE`. Invariant: a row exists
only for a run whose start waits for its gates.

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
    *Amended 2026-11-24 (decisions D022-203, D022-204):* "either the grants of all its gates, a `*.queued`, a
    `step.cache_hit` or a `step.mocked`"; a gated step that a hit or a pin completes holds no grant.
12. Every waiter row belongs to a run that is not terminal; a breach raises `flow_control_waiter_orphaned`.
13. At no instant does a key have more granted, unleased steps than the smallest `limit` among them.

### Revision of 2026-11-20 (keys review; D022-058, D022-059)

- `concurrency_semaphores.oldest_wait_at timestamptz` null: the oldest `enqueued_at` of the key's waiters, kept by the
  count trigger in the transaction that adds or removes a waiter (`LEAST` on an insert; on a delete the minimum of the
  waiters that remain, read after the trigger locks the key's row `FOR UPDATE` by its own statement); CHECK
  `concurrency_semaphores_oldest_wait_iff_waiters` `(waiters = 0) = (oldest_wait_at IS NULL)` (added `NOT VALID` by
  `20270205000000`, validated by `20270205000100`). Holds after every row trigger of a statement that adds and removes
  several waiters of one key.
- Indexes in the order of the key listing: `concurrency_semaphores_listing_idx (tenant_id, namespace_id,
  COALESCE(oldest_wait_at, 'infinity'), pool, permit_scope, group_name, key_digest)` and
  `concurrency_semaphores_group_listing_idx (tenant_id, namespace_id, group_name, COALESCE(oldest_wait_at, 'infinity'),
  pool, permit_scope, key_digest)`; `concurrency_holders_key_age_idx` (the key's columns, then `acquired_at`) replaces
  `concurrency_holders_key_idx`.
- Invariant 12 is also checked over `rate_limit_waiters`; an empty key row and a key whose `oldest_wait_at` is not its
  waiters' oldest are breaches (`support::flow_control::key_consistency`).

### Revision of 2026-11-20 (rate gate served; D022-064, D022-065)

- A run's grant row stores `leased_at = granted_at` whichever routine grants it (the CHECK `rate_limit_grants_shape`
  already required it); it is never given back and stops counting by time.
- A step's lease instant is copied by the lease pass (transaction T23) from the granted attempt's `step.leased` journaled after the
  grant: `greatest(step.leased.occurred_at, granted_at)`. Only the grant row is locked.
- The purge (transaction T25) deletes a leased grant when `leased_at + max_period_ms + 1 minute < now` (exactly at that instant it is
  kept), under its window row's lock; the window row goes with its last grant and waiter.
- `rate_limit_live_keys` counts a window while it has a waiter or a grant that is unleased or leased within its
  `max_period_ms`; a window whose grants all aged out does not count, purged or not. *Reversed by the revision below.*
- Quota resources added: `rate_limit_live_keys` (count), `rate_limit_per_second`, `rate_limit_per_minute`,
  `rate_limit_per_hour` (policies; the effective limit of a declaration is the lower of it and the policy of its window).

### Revision of 2026-11-20 (rate re-review; D022-066)

- `rate_limit_live_keys` counts the namespace's `rate_limit_windows` rows under the digest versions in force, as
  `concurrency_live_keys` counts `concurrency_semaphores`, without a namespace lock (OD-2026-11-20-11: decisions that
  create keys at the same moment pass it by at most the new keys the others create). The quota usage counts the same
  rows (every version, as the semaphores' usage).
- A decision that lacks room first reclaims windows of its own tenant and namespace with `waiters = 0`, `live_grants`
  between 1 and 64, under a version in force, not one of its own keys, and whose every grant is one the purge may
  delete at that instant (`leased_at IS NOT NULL AND leased_at + max_period_ms + 1 minute < clock_timestamp()`, the
  purge's predicate): at most as many as it lacks, `ORDER BY updated_at`, `FOR UPDATE SKIP LOCKED`; then deletes their
  grants. `rate_limit_window_prune` deletes each window at the commit. Writers of grant rows stay the gate's files
  (`readiness_gate.rs`) and the two maintenance statements.
- A rest of a rate key is `SELECT … FOR UPDATE` of the window where `next_eligible_at` is the selection's hint, then
  `UPDATE … SET next_eligible_at` computed from the grants counted at the key's `selected_at`; a grant or a recorded lease
  sets `next_eligible_at = least(next_eligible_at, instant)`.

### Revision of 2026-11-20 (rate visibility; task T022-078, decision D022-067)

- `rate_limit_windows.oldest_wait_at timestamptz` null: the oldest `enqueued_at` of the window's waiters, kept by the
  count trigger `rate_limit_window_count` in the transaction that adds or removes a waiter (`LEAST` on an insert; on a
  delete the minimum of the waiters that remain, read after the trigger locks the window row `FOR UPDATE` by its own
  statement; `NULL` when the counter reaches 0), as `concurrency_semaphores.oldest_wait_at`. CHECK
  `rate_limit_windows_oldest_wait_iff_waiters` `(waiters = 0) = (oldest_wait_at IS NULL)`, added `NOT VALID` by
  `20270207000000` and validated by `20270207000100`.
- Indexes in the order of the rate-limit key listing: `rate_limit_windows_listing_idx (tenant_id, namespace_id,
  COALESCE(oldest_wait_at, 'infinity'), pool, limit_scope, group_name, key_digest)` and
  `rate_limit_windows_group_listing_idx (tenant_id, namespace_id, group_name, COALESCE(oldest_wait_at, 'infinity'),
  pool, limit_scope, key_digest)`.
- Nothing is journaled for the visibility: `Run.rateLimit`, its position and estimate and the listing are reads.
- *Revised 2026-11-20 (decision D022-068 item 5):* invariant 4 holds for the rate column as for the semaphores': after
  every statement, `oldest_wait_at` is not distinct from the oldest `enqueued_at` of the window's waiters; the
  end-of-test consistency check of feature 022 compares the two for every window of the tenant.

### Revision of 2026-11-24 (re-review of the rate faults; decisions D022-076 and D022-078)

- `concurrency_waiters.pinned_limit integer` null and `rate_limit_waiters.pinned_limit integer` null: the declaration
  the run's version pins (`limit` of the policy), before any ceiling, written with every new waiter row; `NULL` in a
  row written before `20270213000000`. `declared_limit` keeps its meaning on the waiter rows: the filed limit, the
  declaration clamped by the namespace's ceiling at admission, as the row's `*.queued` event records it. CHECK
  `concurrency_waiters_pinned_limit_valid` and `rate_limit_waiters_pinned_limit_valid` `(pinned_limit IS NULL OR
  pinned_limit BETWEEN declared_limit AND 10000)`, added `NOT VALID` by `20270213000000` and validated by `20270213000100`. The table
  grants of `sagat_app` cover the column (`SELECT, INSERT, DELETE`); a fixed-size column has no storage class.
- Every decision (admission, the grant routine, the place estimate, the run view) bounds a waiter by
  `least(coalesce(pinned_limit, declared_limit), the namespace's current ceiling)` (of its window's length for a slot,
  `concurrency_limit` for a permit): a lowered ceiling applies to queued waiters, and so does a raised one, up to the
  declaration. The holder and grant rows still record the limit the grant was decided with.
- The next-eligible hint (decision D022-076 items 1 and 2): a waiter queued while its gate had room sets
  `next_eligible_at = clock_timestamp()` whatever it was; one queued at a full window sets `next_eligible_at =
  least(next_eligible_at, the first instant a grant the window counts for the waiter's own period ages out of it)`,
  and a hint absent becomes `clock_timestamp()`.

## Revision of 2026-11-24 (the rest of 022)

Decisions D022-203 to D022-219, proposed ADR-0135 to ADR-0137. Where this section and the ones above disagree, this
section wins. Every table below is tenant-keyed, carries the namespace, references `runs` by `(tenant_id, namespace_id,
run_id)` to `runs (tenant_id, namespace_id, id)` (constraint `runs_tenant_namespace_id_uniq`, decision D022-213) and
`payloads` by `(tenant_id, digest)`, is created by the migration of the slice that first writes it, is granted to
`sagat_app` with the privileges listed, and has every variable-size column in `storage-classification.json`.

### Steps, added states

- A node that declares `cache`: its first `step.released` in a run leads to `WAITING(CACHE_LOOKUP)`; `step.cache_hit`
  (with `step.completed`, attempt 0) leads to `SUCCEEDED`; `step.cache_missed {outcome}` leads to `READY`, or to
  `WAITING(RATE_LIMIT | CONCURRENCY)` when the node also declares a gate (decided in the same commit). A retry never
  returns to `CACHE_LOOKUP`. A step committed `WAITING(CACHE_LOOKUP)` exists only while its run is paused. *(Revised
  2026-11-25, decision D022-238: no root step is released while its run is paused, so no commit leaves a step
  `WAITING(CACHE_LOOKUP)`: the wait kind exists in the reducer's state only, `steps.waiting_kind` does not gain it, and
  `persist`'s refusal of an undecided lookup keeps it so.)*
  `steps.waiting_kind` gains `CACHE_LOOKUP`.
- A pinned node of a draft test run: an upstream pin is `SUCCEEDED` by `step.mocked` at admission; a downstream pin's
  `step.released` leads to `SUCCEEDED` by `step.mocked` in the same commit, before any lookup or gate.

### `step_cache_entries`, revised

| Column | Type | Notes |
|---|---|---|
| `tenant_id`, `namespace_id`, `scope_digest` | | FK to `step_cache_scopes` |
| `key_digest` | bytea | 32 bytes; PK `(tenant_id, namespace_id, scope_digest, key_digest)` |
| `key_version` | smallint | the digest key version the key was filed under; during a rotation a lookup computes the digests of the versions in force, as a gate decision does (ADR-0093); an entry of a retired version simply misses and expires (D1168) |
| `entry_id` | uuid | UNIQUE `(tenant_id, entry_id)` |
| `payload_digest` | bytea | 32 bytes; FK `(tenant_id, payload_digest)` → `payloads (tenant_id, digest)`; replaces `output` and `output_digest` |
| `carrier_digest` | bytea | 32 bytes |
| `classification_rules` | integer | copied from the producing version |
| `source_run_id`, `source_node`, `source_attempt`, `source_seq` | uuid, text, integer, bigint | provenance; no FK (archival of the source must not cascade); index `(tenant_id, namespace_id, source_run_id)` for erasure |
| `created_at`, `expires_at` | timestamptz | database clock; CHECK `expires_at > created_at` |

Grants: SELECT, INSERT, DELETE; an `ALWAYS` trigger refuses UPDATE. No byte trigger: the value is the producer's payload
row, already charged. The namespace's entries are counted by a quota resource (task T022-202).

### Other tables, revised

| Table | Change |
|---|---|
| `step_cache_scopes` | `workflow_id`, `version_id` FKs include the namespace; locked `FOR UPDATE` only by an invalidation (T4), which locks nothing after it but usage rows |
| `step_cache_hit_provenance` | PK `(tenant_id, namespace_id, consumer_run_id, node)`; FK `(tenant_id, namespace_id, consumer_run_id)` → `runs`; `payload_digest` FK to `payloads`; index `(tenant_id, namespace_id, source_run_id)` |
| `step_cache_store_obligations` | PK `(tenant_id, namespace_id, run_id, node, attempt)`; FK `(tenant_id, namespace_id, run_id)` → `runs`; `payload_digest` FK to `payloads` |
| `failure_handler_dispatches` | PK `(tenant_id, namespace_id, source_run_id)`; FK `(tenant_id, namespace_id, source_run_id)` → `runs`; `handler_run_id` UNIQUE `(tenant_id, handler_run_id)` and FK `(tenant_id, namespace_id, handler_run_id)` → `runs`; `config_revision` is the revision read without a lock by the terminal commit |
| `test_run_pins` | PK `(tenant_id, namespace_id, run_id, node)`; FK `(tenant_id, namespace_id, run_id)` → `runs`; `payload_digest` FK to `payloads`; `source_run_id` without FK, index `(tenant_id, namespace_id, source_run_id) WHERE source_run_id IS NOT NULL` |
| `namespace_failure_handlers` | FK `(tenant_id, namespace_id)` → `namespaces`; `workflow_id` FK includes the namespace; never locked by a deciding transaction (decision D022-208) |
| `runs` | `handler_for_run_id`: FK `(tenant_id, namespace_id, handler_for_run_id)` → `runs`, UNIQUE `(tenant_id, handler_for_run_id)` where not null; `test_tree`: set at admission, copied by every child admission, immutable (trigger) |

### Content stores registered (decision D022-217)

Feature 020 is not on the tree. The rest of 022 creates `crates/infra-postgres/run-content-references.json` in the shape
of 020's data model ("Reference discovery") with its own entries: `step_cache_entries.source_run_id` (hook: delete by
source run, T18), `step_cache_hit_provenance.source_run_id` and `test_run_pins.source_run_id` (hook: list by source
run). The payload references are foreign keys, which 020's catalog-driven discovery finds. A contract test fails when a
022 column that references run content is neither a foreign key nor an entry of the file.

### Invariants, added

14. Every commit in which a step of a node that declares `cache` is first released contains, for that step, a
    `step.cache_hit` or a `step.cache_missed`, unless its run is paused, in which case the step is
    `WAITING(CACHE_LOOKUP)`; no step of such a node is `READY` or leased before one of them. *(Revised 2026-11-25,
    decision D022-238: "a `step.cache_hit`, a `step.cache_missed` or a `step.mocked`", with no paused clause.)*
15. Every `step.mocked` belongs to a run with `test_tree = true`, has a `test_run_pins` row whose `payload_digest` is its
    payload, and no `step.leased` of that node exists in the run.
16. Every run with `handler_for_run_id` has a dispatch row `ADMITTED` naming it, and at most one run names a given
    failed run (the unique index).

### Lock order, as built (replaces "Lock order summary" for the rest)

A transition on an existing run: chain → run (a shard fence only where a pass already holds one, a leaf lock) → the
active-run counter row when the commit ends a run that holds a permit → cache scope → cache entry (`FOR SHARE`, the
lookup round) → semaphores, then windows, each sorted (the gates round) → `persist` (the `runs` row and the transition's
rows; the namespace handler policy row read without a lock) → quota, rate and usage rows and the deferred content flush,
last. A later lookup round of the same commit may read cache rows after the semaphores: the exclusive takers of cache
rows (T3, T4, T5, T18) lock nothing after one but usage rows (ADR-0135 decision 7). An admission follows ADR-0050
decision 7, as amended (dispatch row 3, source runs 3R, draft workflow row 4b, pins 6, gates 7).

## Revision of 2026-11-25 (the reviews of the rest of 022)

Decisions D022-222 to D022-240 (the rulings of [reviews/2026-11-25/](reviews/2026-11-25/)). Where this section and the
ones above disagree, this section wins.

### Node fingerprint (decisions D022-225, D022-226)

`NodeFingerprint` = digest of (the node's canonical IR without the fields `next`, `onError`, `cache.ttlSeconds`,
`cache.name`, `concurrency`, `rateLimit`, `retry`, `timeoutMs`, `compensationRetry`, `compensationTimeoutMs`; the
classification rules version). The canonical IR includes the activity name or artifact digest, the canonical key
expression, the author `version`, the `outputSchema` carrier and the sorted logical secret references, and any field a
later feature adds (an input mapping, a binding) by construction. `KeyDigest` is computed under
`KeyPurpose::StepCache` (1 KiB by default, 4 KiB ceiling), for each digest key version in force (ADR-0093).

### `step_cache_entries`, columns added or changed

| Column | Type | Notes |
|---|---|---|
| `payload_digest` | bytea | FK `(tenant_id, payload_digest)` → `payloads (tenant_id, digest)` `ON DELETE RESTRICT` |
| `secret_refs` | text[] | the producing node's sorted logical secret references (names, never values); at most the node's secrets ceiling; protected |
| `producer_worker_id` | uuid | the worker identity of the completed attempt's lease; a hit requires it to hold now a binding that covers the activity |
| `created_at` | timestamptz | the producing completion's instant (its store obligation's `created_at`), not the insert's; dead when at or before the scope's `invalidated_at` |
| `expires_at` | timestamptz | `created_at` + `ttlSeconds`; the runtime role may lower it (`UPDATE (expires_at)` only); the `ALWAYS` trigger refuses any other update and any raise |

Grants: SELECT, INSERT, DELETE, `UPDATE (expires_at)`. Counted by a usage row kept by a `SECURITY DEFINER` trigger with
`SET search_path = pg_catalog, pg_temp` (as `flow_control_counts`); an expired entry counts until purged.

### `step_cache_store_obligations`, revised

PK `(tenant_id, namespace_id, run_id, node, attempt)`; `scope_digest`, `key_digest`, `key_version` copied from the
step's `cache_lookup` projection; `payload_digest` FK `ON DELETE RESTRICT`; `ttl_seconds`; `created_at` (the
completion's instant). The lease columns are removed: the writer takes no lease or fence, only `FOR UPDATE SKIP LOCKED`.
Written only for the outcomes `miss`, `expired`, `invalidated`; counted by a usage row as the entries are; a full
ceiling skips the insert (counted `skipped_backlog`).

### `steps`, column added

| Column | Type | Notes |
|---|---|---|
| `cache_lookup` | jsonb null | written with `step.cache_missed`: scope digest, key digest and key version of the lookup (bounded, protected); read by T2 to write the store obligation; never a key value |

### `runs`, columns added or changed (decisions D022-223, D022-224, D022-228, D022-237)

| Column | Type | Notes |
|---|---|---|
| `handler_tree` | boolean | NOT NULL DEFAULT false; true for a dispatched handler run, an `onFailure~h` child, every child of a handler-tree run, and every reset or rerun whose source or initiator is a handler-tree run (*2026-11-26, decision D022-279:* "initiator" reads "whose request's principal holds `RunRecovery`"; *2026-11-27, decision D022-310:* and every start whose principal holds `RunRecovery`); never dispatched |
| `handler_kind` | text null | CHECK `IN ('DISPATCHED','REGION_CHILD')`; `REGION_CHILD` is the `onFailure~h` child form |
| `handler_for_run_id` | uuid null | only when `handler_kind = 'DISPATCHED'` (CHECK); FK `(tenant_id, namespace_id, handler_for_run_id)` → `runs`; UNIQUE `(tenant_id, handler_for_run_id)` where not null; never inherited by a reset or rerun |
| `lineage_run_id` | uuid | ~~NOT NULL; the source's `lineage_run_id` (read under the source's identity guard) for a reset or a rerun whose initiator is a machine identity or a handler-tree run; otherwise the run's own id, so a reset or rerun a member starts begins a lineage of its own (decision D022-241, O1176 (b)); the initiator's principal kind is recorded with the reset or rerun provenance~~ *(struck 2026-11-25, decision D022-270: superseded by D022-242 and D022-245; see "Revision of 2026-11-25 after the re-reviews")* |
| `failure_handler_state` | text null | CHECK `IN ('RUNNING','CANCELLING','TIMING_OUT','SUCCEEDED','FAILED','TIMED_OUT','CANCELLED')`; the journal's outcome maps `SUCCEEDED` → `SUCCEEDED`, `FAILED` → `FAILED`, `FAILURE_HANDLER_TIMEOUT` → `TIMED_OUT`, `CANCELLED` → `CANCELLED` |
| `test_tree` | boolean | immutable by a `BEFORE UPDATE OF test_tree` trigger, `ENABLE ALWAYS`; at insert a child's flag must equal its parent's |

The `origin` CHECK gains `STANDALONE_STEP` and `DRAFT_TEST` added `NOT VALID` and validated by a second migration; the
unique index on `handler_for_run_id` is built in its own migration (decision D022-239).

### `failure_handler_dispatches`, revised

Adds `lineage_run_id` (the source run's) ~~with UNIQUE `(tenant_id, namespace_id, lineage_run_id)`: at most one dispatch
per lineage, a lineage linking only through machine identities and handler-tree runs (decision D022-241)~~ *(struck
2026-11-25, decision D022-270: the unique index is `WHERE lineage_first` and every reset and rerun extends the lineage,
decision D022-242).* `attempts` is incremented and committed before each admission is tried. Inserted only by the terminal path
of `persist` (a recorded deviation of ADR-0050 decision 7), locked only by the dispatcher and the redrive. The
pending-dispatch ceiling refuses a new or re-enabled handler configuration, never an insert. `config_revision` is the
~~revision read with the run's definition~~ revision read by the left join of the statement that locks the run (struck
and replaced 2026-11-26, decision D022-283; D022-255).

### `namespace_failure_handlers` and `publication_policies`

`namespace_failure_handlers` is never locked by a deciding transaction; its `revision` and `enabled` are read ~~with the
run's definition~~ by the left join of the statement that locks the run (struck and replaced 2026-11-26, decision
D022-283; D022-255). `publication_policies` gains `draft_executions_enabled boolean NOT NULL DEFAULT false` (decision
D022-227): with `required_approvals > 0`, draft test runs and standalone activity and script steps are refused unless it
is true; changed through the publication-policy route and audited.

### Invariants, revised and added

- Invariant 5 reads: a dispatch `ADMITTED` names a run whose `handler_kind` is `DISPATCHED` and whose
  `handler_for_run_id` is the dispatch's source run.
- Invariant 14 (revised): every commit in which a step of a node that declares `cache` is first released contains, for
  that step, a `step.cache_hit`, a `step.cache_missed` or a `step.mocked`.
- Invariant 16 (revised): every run with `handler_kind = 'DISPATCHED'` has a dispatch row `ADMITTED` naming it; at most
  one run names a given failed run, and at most one dispatch row exists per lineage.
- 17. No run with `handler_tree = true` has a dispatch row as its source, and every child of a handler-tree or test-tree
  run carries its parent's flag.
- 18. A `step_cache_entries` row's `created_at` is after its scope's `invalidated_at`, or the entry is dead and never
  served; every `step.cache_hit` names an entry whose `secret_refs` the consumer's publisher could use at the hit's
  commit (checked by the hit's test, not replayable from the journal).
- 19. An upstream pin's `step.routed` releases nothing: no `step.released` of a node outside what `start_at` reaches.

### Lock order, revised

A transition on an existing run: chain → run (a shard fence only where a pass already holds one) → the active-run counter
row when the commit ends a run → the secret-access and membership locks of a cacheable step's secrets (shared,
ascending) → cache scope → cache entry (`FOR SHARE`) → semaphores, then windows, each sorted → `persist` → usage rows,
last. The exclusive takers of cache rows take their store obligations first and lock nothing after a cache row but usage
rows (T3, T4, T5, T18). An admission follows ADR-0050 decision 7: its slot 7 is run-scope gates, cache scope, cache
entry, step-scope gates.

## Revision of 2026-11-25 after the re-reviews (decisions D022-242 to D022-259)

The second rulings ([reviews/2026-11-25/design-rereviews-and-rulings.md](reviews/2026-11-25/design-rereviews-and-rulings.md)).
Where this section and any above disagree, this section wins; in particular the `lineage_run_id` rule of decision
D022-241 above is reversed.

### `runs`, revised

| Column | Type | Notes |
|---|---|---|
| `lineage_run_id` | uuid null | read as `coalesce(lineage_run_id, id)`; filled by a `BEFORE INSERT` trigger with the source's lineage for every reset and rerun, whoever starts it (so an older binary and the chain insert keep working); immutable by a `BEFORE UPDATE OF` trigger, `ENABLE ALWAYS` (decisions D022-242, D022-245) |
| `handler_tree`, `handler_kind` | | immutable by a `BEFORE UPDATE OF` trigger, `ENABLE ALWAYS`; at insert a child's `handler_tree` equals its parent's, and the runs of a handler region's `child` nodes and of a dispatched handler's `child` nodes are `true` (decisions D022-243, D022-245) |
| `failure_handler_state` | text null | follows the journal: `RUNNING` from `failure_handler.started`; `CANCELLING` or `TIMING_OUT` from `failure_handler.draining {cause}` (`TIMING_OUT` for `TIMEOUT`, `CANCELLING` for every other cause); `SUCCEEDED`, `FAILED`, `TIMED_OUT` or `CANCELLED` from `failure_handler.finished` (decision D022-246) |

The overdue pass (`crates/infra-postgres/src/maintenance.rs`, `overdue_runs`) excludes `failure_handler_state IS NOT
NULL` (decision D022-247).

### `failure_handler_dispatches`, revised (decision D022-242)

| Column | Notes |
|---|---|
| `lineage_run_id` | the source run's `coalesce(lineage_run_id, id)` |
| `lineage_first` | boolean NOT NULL; UNIQUE `(tenant_id, namespace_id, lineage_run_id) WHERE lineage_first`; replaces the unique index on every row of a lineage |
| `state` | CHECK `IN ('PENDING','ADMITTED','DEAD','SUPPRESSED_LINEAGE')`; a redrive turns `DEAD` or `SUPPRESSED_LINEAGE` into `PENDING` and never changes `lineage_first` |

The terminal commit inserts `lineage_first = true ... ON CONFLICT (tenant_id, namespace_id, lineage_run_id) WHERE
lineage_first DO NOTHING`; when that inserts nothing, a second statement inserts `lineage_first = false`, state
`SUPPRESSED_LINEAGE` (counted `skipped_lineage`, the outcome journaled on the run). The namespace dispatch budget is a
quota resource per window with a per-workflow share (decision D022-243); a dispatch beyond it stays `PENDING` with a
later `next_attempt_at` and raises an alert.

### Cache tables, revised (decisions D022-252 to D022-254)

| Table | Change |
|---|---|
| `step_cache_entries` | `secret_ids uuid[]` (sorted secret ids, never names) replaces `secret_refs`; `producer_subject text` (the producing worker's or, for a script, runner's subject; bounded, protected) replaces `producer_worker_id`; `created_at` is the database instant of the producing attempt's lease (`step.leased`), and `expires_at` runs from it |
| `step_cache_key_invalidations` (new) | PK `(tenant_id, namespace_id, scope_digest, key_digest)`; `invalidated_at`; written by an invalidation by key (T5) after its lock; read by T3 and by the hit; deleted with its scope; grants SELECT, INSERT, UPDATE (`invalidated_at`), DELETE; protected |
| `step_cache_store_obligations` | `leased_at` (the attempt's lease instant) added; one column `UPDATE` grant (`created_at`, never changed: an `ALWAYS` trigger refuses every update) so that `FOR UPDATE SKIP LOCKED` works as `sagat_app` |
| `step_cache_names` (new) | PK `(tenant_id, namespace_id, cache_name)`; `writer_workflow_ids uuid[]` (the workflows that may write the name), `revision`, `updated_by`, `updated_at`; an administrator's setting, audited; a `namespace`-scope store by another workflow is skipped (counted) |
| `step_cache_scopes` | inserted by an invalidation only for a scope a published version can derive |
| usage | one usage row per namespace holds both counts (entries and store obligations), kept by `SECURITY DEFINER` triggers with `SET search_path = pg_catalog, pg_temp` that schema-qualify `public.` tables |

`steps.cache_lookup` is copied from the source step by a reset (`step.cache_missed` is `REMATERIALIZE`, decision
D022-253).

### Invariants, revised and added

- Invariant 16 (revised): at most one dispatch row with `lineage_first = true` per lineage; every run with
  `handler_kind = 'DISPATCHED'` is named by an `ADMITTED` dispatch row.
- Invariant 18 (restated, decision D022-252): no `step.cache_hit` names an entry that was dead at the hit's commit
  (expired, or created at or before its scope's invalidation or its key's tombstone).
- 20. `runs.failure_handler_state` equals the state the journal's `failure_handler.*` events give; a domain case
  rebuilds it from the journal after each commit.
- 21. No run with `test_tree` or `handler_tree` true has a continue-as-new successor (decision D022-249).

### Lock order, as built (replaces the revised orders above)

A transition on an existing run: entity locks (chain → run, the run's statement reading the handler configuration by a
left join, `FOR UPDATE OF runs`) → the rounds (lookup: secret-access and membership locks, binding advisory lock,
shared, then cache scope, tombstones, entry `FOR SHARE`; then gates: semaphores, then windows, sorted) → `persist` (the
active-run counter row when the commit ends the run, the give-backs, the rest, the dispatch insert followed only by the
run's own `run.finished` row, corrected 2026-11-26, decision D022-280) → usage rows. The
counter row after cache rows makes no cycle: deciders lock cache rows shared only, and no exclusive taker of a cache row
takes the counter row (decision D022-255).

## Revision of 2026-11-25 after the third reads (decisions D022-260 to D022-270)

- `run.finished` of a commit that inserts a dispatch row is appended after the two dispatch statements; its
  `namespace_dispatch` detail equals the row's state; the terminal path updates no `run_events` row (D022-260).
- `failure_handler.draining` has one `cause`; `later_causes` is removed; a further stop request while draining is
  refused (D022-261).
- `failure_handler_dispatches.attempts` counts admissions tried only; a deferral by the budget moves `next_attempt_at`
  alone (D022-264); `last_error_code` gains `CONTINUE_AS_NEW_REFUSED` (D022-263).
- `step_cache_entries.producer_subject` is immutable and pre-read without a lock before the producer's locks, then
  re-checked on the entry read `FOR SHARE` (D022-265).
- `run_recovery_namespaces` (new): PK `(tenant_id, subject, namespace_id)`; `created_by`, `created_at`; an
  administrator's audited setting binding a `RunRecovery` subject to a namespace, as worker bindings are; grants
  SELECT to `sagat_app`, written only through the administrator's route *(the grants superseded 2026-11-26 by the
  section "Revision of 2026-11-26 after the re-reads", decision D022-273)*. `memberships_role_valid` gains `RunRecovery`
  (its `down` refuses while such a membership exists) (D022-266).
- The draft-executions setting is guarded by an advisory key per namespace, shared by admissions, exclusive in
  `set_policy` (D022-269).
- Invariant 22: every `run.finished` whose commit inserted a dispatch row carries `namespace_dispatch` equal to that
  row's state at the commit. *(Restated 2026-11-26, decision D022-280; see the next section.)*

## Revision of 2026-11-26 after the re-reads (decisions D022-272 to D022-285)

The re-reads of the third-read fixes ([reviews/2026-11-26/design-fixes-responses.md](reviews/2026-11-26/design-fixes-responses.md)).
Where this section and any above disagree, this section wins.

### `run_recovery_namespaces`, revised (decisions D022-273, D022-276, D022-277)

Replaces the line of the section above, whose SELECT-only grant could not serve a route that runs as `sagat_app`.

| Column or rule | Notes |
|---|---|
| `tenant_id`, `subject`, `namespace_id` | PK `(tenant_id, subject, namespace_id)` (the membership statement reads by subject on every request); index `(tenant_id, namespace_id)` for the listing and the namespace's foreign key; FK `(tenant_id, namespace_id)` → `namespaces (tenant_id, id)` `ON DELETE CASCADE`; ~~FK `(tenant_id, subject)` → `memberships (tenant_id, subject)` `ON DELETE CASCADE`~~ *(replaced 2026-11-27, D022-298, by the key `(tenant_id, subject, role)`, see the last section)*; `subject` CHECK as `worker_bindings_subject_valid` (1 to 255 characters, no control character) |
| `created_by`, `created_at` | the administrator's subject (bounded) and `clock_timestamp()` |
| Grants | SELECT, INSERT, DELETE to `sagat_app`; no UPDATE (nothing to change); *2026-11-27 (D022-297):* so `sagat_app` cannot lock a row, and every row lock goes through `public.sagat_lock_run_recovery_binding` |
| Usage | the quota delta and flush triggers of `worker_bindings` (`sagat_quota_row_delta` over `subject`; `sagat_quota_flush_deltas`), `ENABLE ALWAYS` |
| Readers | the membership statement (the bound set of a `run-recovery` member, as an array, on every request; empty while `platform_requirements` does not hold `failure-handler-v1`); an admission or a command by such a member (its row `FOR SHARE`, slot 4a after the memberships) *(2026-11-27, D022-297, D022-302, D022-314: through `public.sagat_lock_run_recovery_binding`, after the memberships where the path takes them; also `recheck_requester` and the payload disclosure transaction)* |
| Writers | `bindRunRecovery` (INSERT, idempotent) and `unbindRunRecovery` (DELETE), each with its audit row; nothing else |
| Classification | the subject is an identifier, protected as `worker_bindings.subject` is |

`memberships_role_valid` gains `run-recovery` (the stored spelling, lowercase with a hyphen as `database-runner`). A
`BEFORE INSERT OR UPDATE OF role` trigger on `memberships`, `ENABLE ALWAYS`, refuses `run-recovery` while
`platform_requirements` does not hold `failure-handler-v1` (decision D022-276). The `down` refuses while a
`run-recovery` membership or a binding row exists.

### `runs.handler_tree`, restated (decision D022-279)

A reset or rerun is a handler-tree run when its source run is one or when the request's principal holds `RunRecovery`;
such a reset or rerun of a definition that continues as new is refused at admission (`CONTINUE_AS_NEW_REFUSED`), so
invariant 21 holds for it.

### The dispatch statements and invariant 22 (decision D022-280)

The two dispatch statements are followed in the run's commit only by the `run.finished` row of the run's own journal,
then usage rows.

- Invariant 22 (restated): `run.finished.namespace_dispatch` is present exactly when a dispatch row for this source run
  exists, and is `PENDING` exactly when that row's `lineage_first` is true (`SUPPRESSED_LINEAGE` otherwise); a later
  redrive does not change it.

### Revision of 2026-11-26 after the second security re-read (decisions D022-286 to D022-294)

- ~~`memberships` gains a second trigger, `AFTER UPDATE OF role ... FOR EACH ROW WHEN (OLD.role IS DISTINCT FROM
  NEW.role)`, `ENABLE ALWAYS`, that deletes every `run_recovery_namespaces` row of the subject in the tenant, as
  `memberships_role_change_revokes_reveal_grants` does for reveal grants (D022-286).~~ *(Superseded 2026-11-27 by
  D022-298: a restricting foreign key refuses the role change instead; see the last section.)*
- The functions of both `memberships` triggers (the activation gate of D022-276 and the role change) are `SECURITY
  DEFINER`, `SET search_path = pg_catalog, pg_temp`, name `public.platform_requirements`, `public.memberships` and
  `public.run_recovery_namespaces` with their schema, and are `REVOKE ALL ... FROM PUBLIC` (D022-290). *(Amended
  2026-11-27, D022-300: "and the role change" is struck; the functions are the activation gate's, the audit trigger's
  and the lock function's.)*
- `bindRunRecovery` reads the subject's role through `sagat_lock_memberships` (`FOR SHARE`) in its own transaction
  before the insert (D022-286). *(2026-11-27, D022-298: kept for the typed answer; the foreign key closes the race.)*
- The membership statement loads the bound set with each namespace's id and slug, so a namespace addressed by id or
  slug is checked against the scope before any lookup (D022-294).
- The completion of a pending reset (`recheck_requester`) reads the actor's binding row of the entry's namespace `FOR
  SHARE` after `sagat_lock_memberships` when the role is namespace-bound (D022-288). *(2026-11-27, D022-297, D022-301:
  through `public.sagat_lock_run_recovery_binding`, with the namespace carried by `PendingAdmission.namespace_id`.)*
- The migration that widens `memberships_role_valid` and adds both triggers lands with task T022-256, not before it
  (D022-291). *(2026-11-27, D022-300: "both triggers" reads "the activation trigger, the audit trigger, the restricting
  foreign key and the lock function".)*

## Revision of 2026-11-27 after the third re-reads (decisions D022-297 to D022-314)

The third re-reads ([reviews/2026-11-26/design-fixes-responses.md](reviews/2026-11-26/design-fixes-responses.md), third
part). Where this section and any above disagree, this section wins.

### `run_recovery_namespaces` and `memberships`, revised (decisions D022-297 to D022-300)

| Column or rule | Notes |
|---|---|
| `tenant_id`, `subject`, `namespace_id` | as revised 2026-11-26: PK `(tenant_id, subject, namespace_id)`, index `(tenant_id, namespace_id)`, FK `(tenant_id, namespace_id)` → `namespaces` `ON DELETE CASCADE`, the subject CHECK |
| `role` (new) | `text NOT NULL DEFAULT 'run-recovery'`, `CHECK (role = 'run-recovery')`; written by nobody but the default |
| `run_recovery_namespaces_membership_fk` (new) | `(tenant_id, subject, role)` → `memberships (tenant_id, subject, role)` `ON UPDATE RESTRICT ON DELETE CASCADE`; replaces the key `(tenant_id, subject)`. A role change of a subject with a binding is refused (SQLSTATE `23503`, reported by provisioning as `RUN_RECOVERY_BINDINGS_EXIST`); a bind for a subject of another role violates it (`422 NOT_A_RECOVERY_SUBJECT`); a membership deletion cascades (D022-298) |
| `memberships_subject_role_key` (new, on `memberships`) | `UNIQUE (tenant_id, subject, role)`, the target of the key; redundant with the PK for uniqueness, needed for the reference; one more index written on a membership insert, which provisioning alone does |
| Grants | SELECT, INSERT, DELETE to `sagat_app`, unchanged |
| Lock function (new) | `public.sagat_lock_run_recovery_binding(p_tenant uuid, p_subject text, p_namespace uuid) RETURNS TABLE(namespace_id uuid)`, `LANGUAGE sql VOLATILE SECURITY DEFINER SET search_path = pg_catalog, pg_temp`, `... FOR SHARE OF b`; `REVOKE ALL ... FROM PUBLIC`; `GRANT EXECUTE` to `sagat_app` (D022-297) |
| Readers | the membership statement (the bound set, with each namespace's id and slug, on every request; empty while `platform_requirements` does not hold `failure-handler-v1`); through the lock function: an admission or a command of a bound actor in slot 4a, after the memberships where the path takes them (D022-277, D022-314), `recheck_requester` of a pending reset (D022-288, D022-301), the payload disclosure transaction after `sagat_lock_memberships` (D022-302) |
| Writers | `bindRunRecovery` (INSERT) and `unbindRunRecovery` (DELETE), each with its audit row; a referential cascade from `memberships` or `namespaces` (DELETE), audited by the trigger below |
| Audit trigger (new) | `AFTER DELETE ... FOR EACH ROW`, `ENABLE ALWAYS`; its function writes one `run_recovery.unbound` row (actor `system:provisioning`, reason "removed by a referential cascade") into `public.audit_records` only when `pg_trigger_depth()` is above 1, that is for a delete issued by a referential action; the route's delete is audited by the route (D022-299) |
| Role-change trigger | none: D022-286's trigger is not built (D022-298) |

The functions of the migration (D022-300): the activation trigger's and the audit trigger's, executable by no role;
the lock function, executable by `sagat_app` alone. All three `SECURITY DEFINER`, `SET search_path = pg_catalog,
pg_temp`, every table named with its schema; read from `pg_proc` by the migration test of T022-253. The migration lands
in the pull request of T022-256; its `down` refuses while a `run-recovery` membership or a binding row exists.

### `runs.handler_tree`, extended (decision D022-310)

Every admission whose principal holds `RunRecovery` (start, reset, rerun) is a handler-tree run, `handler_kind` null;
such an admission of a definition that continues as new is refused (`CONTINUE_AS_NEW_REFUSED`), so invariant 21 holds
for it, and invariant 17 keeps it from being dispatched.

### The scoped lookups of a bound principal (decision D022-305)

For a namespace-bound principal the statements that look up a run, a workflow or a request by id carry `AND
namespace_id = ANY($bound)`, so a row outside the scope is not returned; a tenant principal's statements are unchanged.
