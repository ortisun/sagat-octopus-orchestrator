# Data model — feature 024

Status: Approved for implementation (Owner decision 2026-11-11-1), in release 0.3.0 (Owner decision
2026-11-24-1); Architecture decision 0058 accepted (delegate, Owner decision 2026-11-26-164); not
started. Nothing here is implemented. Revised 2026-11-26 against `develop` at `e5bda041`, and on
2026-11-27 after the correctness review (research Decision 024-016).

Every table is tenant-keyed, every foreign key is composite with `tenant_id` (and `namespace_id`
where the parent has it), and every new variable-size column joins
`crates/infra-postgres/storage-classification.json`. The migration versions are taken by the
foundation slice that writes them (Task 024-010), after the latest migration of `develop` at that
time (`20270210000100_step_logs_validate` on 2026-11-26) and after any version another open branch
has claimed; names below are provisional.

## Changes to existing records

| Record | Change | Compatibility |
|---|---|---|
| `database_connection_revisions.config` | Accepts the five new driver shapes beside `postgresql` and feature 035's lake kinds (validated by the domain `ConnectionConfig`), each with `egressCeiling`; existing PostgreSQL and lake JSON is unchanged | Old rows decode as before. A binary without these drivers cannot decode a new-driver row: `config_from` (`crates/infra-postgres/src/database.rs`) raises `corrupt("database driver")`, a `StoreError::Internal`, and the whole connection list fails. The drivers' registry capabilities therefore stay unrecorded until binary rollback is no longer planned (FR-024-038) |
| `worker_bindings.profile` | `worker_bindings_profile_valid` widens from `database`, `lake` to `database`, `lake`, `warehouse`, `data` (Decision 024-002) | Existing rows keep their value; an older engine ignores the column |
| `step_database_credentials` | Unchanged: a new-driver attempt pins its one secret as `secret_role = 'primary'` | — |
| `step_lease_blobs` (feature 035) | Unchanged in shape: the lease transaction of a data attempt writes one row per pinned part (Decision 024-007); its release with the run's retention (a day after the run ended) is what retention waits for before deleting a part | — |
| Instance registry | `serving_instances_capabilities_closed`, `platform_requirements_capability_valid` and `platform_requirement_history_capability_valid` gain `database-engines-v1`, `database-warehouses-v1`, `data-pipelines-v1` (Decision 024-003); `Capability::ALL` follows | Additive; nothing is recorded by the migration |
| `run_database_connections` | Unchanged | — |
| `blobs.purpose` | Gains `DATA_SNAPSHOT_PART`, which requires the same exact fence columns as `STEP_OUTPUT` (run, step, attempt, generation). A guarded trigger allows `COMMITTED → DELETING` **only** for this purpose and only from the retention mark (ADR-0058, superseding ADR-0023 in part; enabled, Owner decision 2026-11-26-163). Run purge (020, ADR-0054) excludes this purpose even though the parts carry the producer run's fence. An `UPLOADED` part whose attempt ends without committing it (failure, lease expiry with a new generation, cancellation, refused completion) is claimed for deletion (`UPLOADED → DELETING`, retry due at once) in the transaction that ends the attempt, and an upload of that fence that finishes later is claimed when it finishes (FR-024-035) | Other purposes keep forward-only states and their 020-owned retention |
| `payloads` content accounting (ADR-0027) | The content-delta trigger measures zero for a row whose digest `data_protected_outputs` lists and whose content is at most 4 KiB: the output document of a `dataTransform` step or a `dataPublish` node (FR-024-027) | Every other payload is measured as before; a listed digest is never refunded if an ordinary row with the same bytes was charged first |
| `inbox_events` | Rows with `source = 'sagat.data'` carry the closed publication document; a CHECK bounds their `payload` to 1 KiB and the content-delta trigger skips them (ADR-0027 `protected_system` by a bounded closed schema, D1267); manifest entry updated | Other sources unchanged |
| Quota resources | Seven new resource names: `database_analytic_timeout_ms`, `database_bytes_billed`, `data_assets`, `data_snapshot_bytes`, `data_concurrency`, `tenant_data_concurrency`, `tenant_data_bytes` | Additive enum; no existing limit changes |
| Content accounting (ADR-0027) | A new class `data` measures `DATA_SNAPSHOT_PART` bytes into `tenant_data_bytes`, charged once when the upload is reserved and measured (D1278); committing the snapshot does not charge again | `tenant_content_bytes` no longer sees these bytes; other blobs unchanged |
| Alert kinds | `DATA_ASSET_STALE`, `DATA_SNAPSHOT_QUARANTINED` (subject: asset name) | Additive enum (AsyncAPI rule 3) |

## Entities and state

### Data asset

- **Status:** `ACTIVE ⇄ PAUSED → DELETED` (terminal tombstone; history kept). `PAUSED` refuses new
  publications and admission of runs that write the asset; reads of retained snapshots continue.
- **Kind:** `MANAGED` (snapshots, pointer, contract, audits) or `EXTERNAL` (connection + opaque
  locator, observations only). Kind is immutable.
- **Pointer:** `current_snapshot_id` and a monotonic `pointer_version` (0 = never published). Every
  pointer move increments the version by one and writes one publication record. The tombstone
  transaction clears the pointer (`current_snapshot_id` null) with a publication record of origin
  `TOMBSTONE`, so no pointer names a snapshot that retention may retire.

### Snapshot

```
completion ──► AUDITED ──pointer CAS──► PUBLISHED ──retention──► RETIRED
           ├─► QUARANTINED ───────────────────────retention──► RETIRED
           └─► ABANDONED (stale auto-publication that no later node reads) ──retention──► RETIRED
AUDITED ──passed over by a publication, or window elapsed, and no live-run pin──► ABANDONED ──retention──► RETIRED
```

- A snapshot row is created only by a completion transaction whose parts the engine verified
  (FR-024-023), in state `AUDITED`, `QUARANTINED` or `ABANDONED`. There is no `STAGING` row:
  uploaded but uncommitted parts are ordinary `UPLOADED` blobs, claimed for deletion by the
  transaction that ends their attempt (the ADR-0023 time to live is only a backstop).
- A snapshot pinned by a live run (its own staged snapshot, a `from` input, an output base, a
  `run_data_snapshot_refs` row) never moves to `ABANDONED`; after the run ends, retention applies the
  rule. The unpublished window counts from `created_at` whatever the state, so a snapshot that is
  abandoned late is retired in the same retention pass once its window has passed.
- `QUARANTINED` and `ABANDONED` are never publishable (Decision 1259); a `dataPublish` that meets `ABANDONED`
  treats it as stale (`onStale`), and a `from` input that meets it fails `DATA_CONFIGURATION_INVALID`.
  Rollback targets only snapshots that were `PUBLISHED`.
- The snapshot trigger refuses `RETIRED` for a snapshot that an asset's `current_snapshot_id` names.
- Content columns (parts, counts, fingerprint, provenance, audits, producer fence) are immutable;
  only `state`, `published_at`, `retain_until` and `retired_at` move, through a guarded trigger that
  allows exactly the transitions above.

### Pins

- **Admission pin** (`run_data_assets`): the declaration revision of every asset a node reads,
  writes or observes, fixed in the run admission transaction (like 017 connection pins).
- **First-lease pins** (`step_data_inputs`, `step_data_output_bases`): the input snapshot per alias
  and the pointer version per output, fixed in the transaction that grants attempt 1; retries reuse
  them. A `dataQuery` pins its inputs the same way.
- **Per-attempt read allow-list** (feature 035's `step_lease_blobs`): every lease of a data attempt,
  the first and each retry, writes one row per part of its pinned inputs for that attempt and
  generation; `ReadBlob` of the data runner is allowed exactly those (Decision 024-007).

## Tables

### `data_assets`

| Column | Type | Constraint |
|---|---|---|
| `tenant_id`, `namespace_id`, `id` | uuid | PK `(tenant_id, namespace_id, id)`; FK `(tenant_id, namespace_id)` → `namespaces` |
| `name` | text | the domain `Slug` rule (3–63, `[a-z0-9-]`, starts with a letter, ends with a letter or digit, no `--`), same CHECK as `database_connections.name`; UNIQUE `(tenant_id, namespace_id, name)`; immutable |
| `kind` | text | `MANAGED`/`EXTERNAL`; immutable |
| `status` | text | `ACTIVE`/`PAUSED`/`DELETED`; `DELETED` irreversible |
| `current_revision` | integer | > 0; deferred FK to `data_asset_revisions`; +1 per revision |
| `current_snapshot_id` | uuid null | deferred FK `(tenant_id, namespace_id, id, current_snapshot_id)` → `data_snapshots`; null for `EXTERNAL` and after a tombstone |
| `pointer_version` | bigint | ≥ 0; +1 per publication; 0 for `EXTERNAL` |
| `last_published_at`, `last_observed_at` | timestamptz null | database clock |
| `created_at`, `updated_at` | timestamptz | `clock_timestamp()` |

Trigger: identity, name, kind and `created_at` immutable; `EXTERNAL` rows keep a null pointer; pointer may change only together with an
insert into `data_asset_publications` of the same version (checked by a deferred constraint
trigger). Quota: `data_assets` counts `ACTIVE` and `PAUSED` rows per namespace.

### `data_asset_revisions`

`(tenant_id, namespace_id, asset_id, revision)` PK; `declaration jsonb` (object, ≤ 131,072 bytes,
canonical JSON, class `content`); `status`; `actor` (1–255, no control characters); `request_id`
uuid with UNIQUE `(tenant_id, namespace_id, actor, request_id)`; `created_at`. Updates refused by an
`ALWAYS` trigger. At most 1,000 revisions per asset (a final tombstone revision is allowed at the
ceiling, as in 017). The declaration holds: contract (columns, key, compatibility), audits,
freshness, retention and producer `{workflow, node}` (managed), or connection, locator and
producer (external).

### `data_snapshots`

| Column | Notes |
|---|---|
| `tenant_id`, `namespace_id`, `asset_id`, `id` | PK `(tenant_id, namespace_id, asset_id, id)`; UNIQUE `(tenant_id, namespace_id, id)` |
| `declaration_revision` | FK to the revision the producer was admitted with |
| `state` | `AUDITED`, `QUARANTINED`, `PUBLISHED`, `ABANDONED`, `RETIRED`; created as `AUDITED`, `QUARANTINED` or `ABANDONED` |
| `run_id`, `node_id`, `attempt`, `generation` | producer fence; FK `(tenant_id, namespace_id, run_id)` → `runs` |
| `base_pointer_version` | the pointer version captured at the producer's first lease |
| `part_count` (1–32), `row_count`, `byte_count` (bigint ≥ 0) | measured by the engine |
| `schema_fingerprint` | bytea(32): SHA-256 of the canonical contract column list it matched |
| `created_at`, `published_at`, `retain_until`, `retired_at` | database clock |

Index `(tenant_id, namespace_id, asset_id, state, created_at)`.

### `data_snapshot_parts`

PK `(tenant_id, namespace_id, snapshot_id, part_index)` (`part_index` 0–31); `blob_id` UNIQUE per
tenant/namespace, FK → `blobs(tenant_id, namespace_id, id)` whose `purpose` must be
`DATA_SNAPSHOT_PART` and whose fence equals the snapshot's; `size_bytes`, `sha256` (32 bytes) equal
to the blob row; `row_count`. Immutable.

### `data_snapshot_inputs` (provenance)

PK `(tenant_id, namespace_id, snapshot_id, alias)`; `alias` (SQL identifier, ≤ 63 bytes); either
`input_asset_id`, `input_snapshot_id` (FK → `data_snapshots`) or, for a step source,
`input_run_id`, `input_node_id` (the producing step of the same run). Immutable. Lineage reads it
and shows a step source as a `source` node (its connection or activity name).

### `data_snapshot_audits`

PK `(tenant_id, namespace_id, snapshot_id, check_name)`; `check_name` (bounded identifier,
protected); `outcome` `PASSED|FAILED|ERROR`; `failing_count` bigint null. No values. Immutable.

### `data_asset_publications`

PK `(tenant_id, namespace_id, asset_id, pointer_version)`; `snapshot_id` (null only for
`TOMBSTONE`), `previous_snapshot_id` null; `origin` `RUN_AUTO|RUN_NODE|API_PUBLISH|API_ROLLBACK|TOMBSTONE`;
`run_id`, `node_id` null (set for run origins; for `RUN_NODE` the `dataPublish` node, while the
producer binding was checked against its `from` node); `actor`; `justification` text null (≤ 1,024
bytes, no control characters, class `content` under ADR-0027 because it is tenant-authored text, so
an API publication or rollback is refused `QUOTA_EXCEEDED` at a full content quota; required for
API origins only; run origins and `TOMBSTONE` carry none, so they are never charged and a tombstone,
a deletion, still works at a full quota as ADR-0027 requires); `published_at`. A
`TOMBSTONE` record writes no inbox event. Immutable; excluded from run purge and owned by ADR-0058
(G1254).

### `data_publication_receipts`

PK `(tenant_id, namespace_id, actor, idempotency_key)`; `fingerprint` bytea(32) over the canonical
request (asset, snapshot, kind, expected version, justification); `asset_id`, `pointer_version`;
`created_at`. Same key and fingerprint replays the stored result after current authorization;
different content conflicts. Purged after 7 days.

### `run_data_assets`

PK `(tenant_id, run_id, node_id, asset_id)`; `namespace_id`; `revision` (FK to the revision);
`role` `READ|WRITE|OBSERVE`. Written in the run admission transaction; immutable; FK to `runs` with
`ON DELETE CASCADE` like `run_database_connections`.

### `step_data_inputs` and `step_data_output_bases`

`step_data_inputs`: PK `(tenant_id, run_id, node_id, alias)`; `namespace_id`; `source`
`ASSET|STAGED|STEP`; for `ASSET`/`STAGED` the `asset_id` and `snapshot_id` (FK); for `STEP` the
`source_node_id` and either the committed output `blob_id` (FK to `blobs`, purpose `STEP_OUTPUT`) and
its `sha256`, or the inline output digest; `level` (`public|internal|pii`, never null): the level
the input counts at for a `dataQuery` output (FR-024-016): the highest level of the pinned
revision's columns for `ASSET`/`STAGED`; for `STEP` `pii` unless the source step's output is a 016
carrier whose pinned schema declares a level for every field (then the highest), and `pii` for a
017 database result, which only a `dataTransform` may pin (publication refuses it as a `dataQuery`
input); `pinned_at`. A CHECK makes exactly one form present; a `STAGED` pin is
accepted only for a snapshot in state `AUDITED` or `PUBLISHED` (D1277), and resolves the snapshot
of the named node's successful completion. In a run created by an 018 reset, a rematerialized
`STEP` pin may name the source run's committed output blob, admitted by the run's reset link and
the restored blob id and digest (never a caller-chosen id). `step_data_output_bases`: PK `(tenant_id, run_id, node_id,
asset_id)`; `base_pointer_version`, `base_snapshot_id` null, `pinned_at`. Both written once in the
first-lease transaction and immutable. Index `step_data_inputs (tenant_id, namespace_id,
snapshot_id)` for the retention check.

### `definition_data_edges`

PK `(tenant_id, namespace_id, workflow_version_id, node_id, asset_id)`; `role`
`READ|WRITE|OBSERVE`. Written in the publication transaction of a workflow version from its
immutable document; the lineage query joins it with the active-version pointer. Immutable.

### `data_asset_observations`

PK `(tenant_id, namespace_id, asset_id, run_id, node_id)`; `observed_at`. Written in the completion
transaction of a successful declared write by the external asset's producer on its connection
(D1275). The latest 1,000 per asset are kept; older rows have no inbound reference and are deleted.

### `run_data_snapshot_refs`

PK `(tenant_id, namespace_id, run_id, snapshot_id)`; `source_node_id` (the node of the source run
that committed it); `reason` `RESTORED`. Written only by an 018 reset transaction that
rematerializes a restored `data.snapshot_committed` (below), so retention keeps a source run's
snapshot that the reset run may still read through `from` or publish through `dataPublish`; those
two resolve a node's snapshot through this row when the node's completion is in the restored
prefix. Immutable; FK to `runs` `ON DELETE CASCADE` and to `data_snapshots`.

### `data_protected_outputs`

PK `(tenant_id, digest)`; `node_kind` `DATA_TRANSFORM|DATA_PUBLISH`; `created_at`. Written in the
data completion and the `dataPublish` transaction before the step's output payload, for the closed
output document (asset names, snapshot ids, states, pointer versions and counts; at most 4 KiB). The
`payloads` content-delta trigger measures zero for a listed digest whose content is at most 4 KiB.
Immutable; class `protected_system`.

Freshness has no table: for a managed asset it is computed from the `created_at` of the snapshot
the pointer names (its producer's completion), never from `last_published_at`, so a rollback or a
late publication of old data does not refresh it; for an external asset from `last_observed_at`;
with the pinned policy and the database clock. Alerts reuse the 008 alert table.

## Journal events and reset rules (018 registry, coordinator decision C1)

Payload-free, snake_case detail keys; older readers ignore them (decoders read by name). Each type
declares its rule in 018's reset-rule registry: the exhaustive `match` of `crates/domain/src/execution/reset.rs`
(`ResetRule::Copy`, `Rematerialize`, `Exclude`, `Refuse`), documented in
`specs/018-schedules-bulk-operations/contracts/journal-and-events.md`.
Step events of data nodes (`step.leased`, `step.completed`, `step.failed`) are existing types and keep
018's rules for them; a `dataQuery` adds no type.

| Event type | Written in | Detail | Reset rule |
|---|---|---|---|
| `data.inputs_pinned` | first-lease transaction of a data node | `inputs[]` (`alias`, `level`, `asset_id` + `snapshot_id`, or `source_node` + the output `blob_id` and `sha256` or the inline digest), `output_bases[]` (`asset_id`, `base_pointer_version`) | `REMATERIALIZE`: the reset transaction re-creates `step_data_inputs` and `step_data_output_bases` for the new run from the restored detail, a step source naming the source run's committed output (admitted through the reset link and the restored id and digest); it refuses the reset point (`CONFLICT`, reason `SNAPSHOT_RETIRED`) if a pinned snapshot is `RETIRED`, read under the asset head lock. A later publication CAS against the restored base is stale if the pointer moved, and follows `onStale` |
| `data.snapshot_committed` | data completion | `asset_id`, `snapshot_id`, `state` (`AUDITED`/`QUARANTINED`/`ABANDONED`), `row_count`, `byte_count`, `part_count` | `REMATERIALIZE`: inserts `run_data_snapshot_refs` for the new run so retention keeps the source run's snapshot and a later `from` or `dataPublish` of the reset run resolves it; refused (`SNAPSHOT_RETIRED`) if it is `RETIRED`. The snapshot stays the source run's; nothing is re-staged |
| `data.published` | auto-publishing completion, `dataPublish` activation | `asset_id`, `snapshot_id`, `pointer_version`, `origin` | `COPY`: history of a shared-state change the source run made; the reset run never re-publishes, and a restored `dataPublish` is not re-executed |
| `data.publish_skipped` | completion or `dataPublish` with `onStale: skip` | `asset_id`, `snapshot_id`, `observed_pointer_version` | `COPY` |
| `data.observation_recorded` | completion of a declared external write | `asset_id` | `COPY`: the reset run records no second observation |

API publications and rollbacks are not run events; they are audited and recorded in
`data_asset_publications` only.

## Content-store registry entry (020, coordinator decision C8)

| Store | Purge and erasure rule |
|---|---|
| `blobs` rows of purpose `DATA_SNAPSHOT_PART` and their bytes | Excluded from 020's run purge (020 data model, purge step 5) although they carry the producer run's fence; owned by ADR-0058: deleted only by this feature's retention under the asset's declared policy (Owner decision 2026-11-26-163). Erasure of individual records inside a snapshot is G1250 |
| `data_snapshots`, `data_snapshot_parts`, `data_snapshot_inputs`, `data_snapshot_audits`, `data_asset_publications` | Excluded from 020's run purge and owned by ADR-0058 (020 adds the matching excluded rows): purging a producer run must not remove the metadata behind a current pointer, a pin or a provenance edge. Only this feature's retention may prune metadata of `RETIRED` snapshots that nothing references, which is not designed yet (G1254) |
| `inbox_events` rows of source `sagat.data` | ADR-0012 deduplication window, as other inbox rows |
| `data_protected_outputs` rows | Fixed-size digests with no content; kept in this delivery. A payload row they name is purged with its run like any payload (020), and the listing only makes its measurement zero while it exists |

## Transactions on these records

| Transaction | Writes together | Locks, in order |
|---|---|---|
| Asset declaration write | revision, head, audit, receipt | namespace data-catalog fence (ADR-0050 slot 4a) → membership → asset head `FOR NO KEY UPDATE` → usage |
| Workflow publication | version, `definition_data_edges`, audit (existing publication transaction) | existing order; asset heads `FOR SHARE` ascending before inserting edges |
| Run admission | run, `run_data_assets`, connection pins, journal, initial tasks, outbox, quotas, audit | the merged admission lock order of ADR-0050 (018): asset heads `FOR SHARE` ascending in slot 4b, after 017 connection and 021 resource/variable heads, before secrets, run rows, semaphores and usage (D1273) |
| First managed lease | lease, `step_data_inputs`, `step_data_output_bases`, `step_lease_blobs` rows of the pinned parts, journal (pinned ids), outbox | run → existing claim guards (drain fence, session, binding with profile `data`, policy) → asset heads `FOR SHARE` ascending |
| Later lease (retry) | lease, `step_lease_blobs` rows of the same pinned parts for the new attempt and generation, outbox | run → existing claim guards; no asset lock (the pins are fixed) |
| `ResolveData` (every attempt) | audit `data.snapshot_read` (identical resolutions of one attempt/generation collapse, as 017 `secret.resolved`) | run → current runner/session/binding guards; no asset lock |
| Data completion | snapshot(s), parts (blobs `UPLOADED → COMMITTED`, no second quota charge, D1278), provenance, audits, step, `data_protected_outputs` row and the step's output, journal, next tasks, outbox; with `publish: auto` also pointer CAS, publication record, abandonment of passed-over `AUDITED` snapshots that no live run pins, inbox event, webhook delivery rows, audit | run → existing managed completion guards (binding, session, membership) → asset heads ascending (`FOR NO KEY UPDATE` if publishing, else `FOR SHARE`), taken before any row that references them, with status and producer rechecked |
| Attempt end without a commit (`FailStep`, a refused completion, lease expiry that grants a new generation, cancellation) | the existing step transition, journal and outbox; the attempt's `UPLOADED` `DATA_SNAPSHOT_PART` blobs of that fence `→ DELETING` with an immediate retry time | run → existing guards → the blob rows of that fence; no asset lock |
| `dataPublish` node | pointer CAS, publication record, `data_protected_outputs` row and the node's output, inbox event, delivery rows, journal, next tasks, outbox, audit | run → asset head `FOR NO KEY UPDATE` |
| API publish / rollback | pointer CAS, publication record (justification charged to content quota; a full quota refuses the whole transaction with `QUOTA_EXCEEDED`), receipt, inbox event, delivery rows, audit | current membership guard → asset head `FOR NO KEY UPDATE`; no run lock |
| Asset tombstone | revision, status `DELETED`, pointer cleared with a `TOMBSTONE` publication record, audit, receipt | namespace data-catalog fence (slot 4a) → membership → asset head `FOR NO KEY UPDATE` |
| Declared external write completion (database or `icebergWrite` step with `assets.writes`) | the existing completion writes, plus `data_asset_observations`, `data_assets.last_observed_at` and the journal event `data.observation_recorded`, or, when the recheck fails, an audited refusal and no observation | run → existing managed completion guards → external asset heads `FOR NO KEY UPDATE` ascending, with `ACTIVE` and the producer of the current revision rechecked under that lock |
| 018 reset (the data part of the reset transaction) | `step_data_inputs`, `step_data_output_bases` and `run_data_snapshot_refs` of the new run, from the restored `data.*` events | 018's order for a run-creating transaction: asset heads of every referenced snapshot `FOR SHARE` ascending in slot 4b, then each snapshot's state read under that lock (`SNAPSHOT_RETIRED` refusal), then the rows above, before the new run's rows |
| Retention mark | snapshot `RETIRED`, parts `DELETING` with attempt/retry fields, data bytes still charged, audit | asset head `FOR NO KEY UPDATE` → pin recheck with plain reads, including `run_data_snapshot_refs`, the pointer, and that no `step_lease_blobs` row names a part |
| Retention confirm | blob tombstone confirmation, bytes released | blob row; no asset or run lock |

No transaction performs object-store or provider I/O while holding these locks, and none takes a
run or authority lock after an asset head lock (D1273).

## Grants for `sagat_app`

SELECT and INSERT on every new table (including `data_protected_outputs`, which is never updated or
deleted); UPDATE on `data_assets` and `data_snapshots` only (guarded by
the triggers above); DELETE only on `data_publication_receipts` and on observations beyond the
kept window. Snapshot metadata is never deleted in this delivery, by run purge or otherwise (G1254). No
TRUNCATE, no trigger disable. Retention and the monitor use the same runtime role; the
`runtime_privileges` sweep covers every new statement.

## Retention

| What | Kept while | Then |
|---|---|---|
| Current snapshot | always | — |
| Previously published snapshots | among the latest `retainPublished` (default 5, hard 50) | `RETIRED`; parts deleted (Owner decision 2026-11-26-163) |
| Any snapshot pinned by a non-terminal run (input, output base, own staged snapshot, or a `run_data_snapshot_refs` row of a reset run) | the run is live, and 24 hours after it ends; never moved to `ABANDONED` while the run is live | eligible again |
| `AUDITED` unpublished | `retainUnpublishedDays` (default 7, hard 30) from `created_at` | `ABANDONED`, and `RETIRED` in the same pass |
| `QUARANTINED`, `ABANDONED` | `retainUnpublishedDays` from `created_at` (not from the time it became abandoned) | `RETIRED` |
| `RETIRED` metadata (rows, provenance, audits, publications) | always in this delivery; never removed by run purge | pruning of unreferenced `RETIRED` metadata by this feature's retention is G1254 |
| Uncommitted part uploads | until the transaction that ends their attempt (failure, refused completion, lease expiry with a new generation, cancellation) | claimed for deletion there; the ADR-0023 upload time to live (24 h) remains a backstop only |

Retention deletes the bytes of `RETIRED` snapshots under the retention the asset manager declared,
audited, and releases their charge only after confirmed deletion (Owner decision 2026-11-26-163;
ADR-0058 decision 7). Tombstoning an asset clears its pointer and makes every snapshot except
live-pinned ones eligible. Deleting data here does
not reach copies that left the platform (query results read by a person, webhook deliveries,
exports); this is stated in the runbook. Record-level erasure inside a snapshot is G1250.
