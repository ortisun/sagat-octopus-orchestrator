# Data model — feature 020

Draft. Every table is tenant-keyed, every foreign key carries the tenant, every query filters tenant and
namespace. Grants are for the runtime role `sagat_app`; "definer" means a `SECURITY DEFINER` function
with `search_path = pg_catalog, pg_temp`, every name qualified, `EXECUTE` revoked from `PUBLIC` and
granted to `sagat_app` only (the ADR-0020/ADR-0027 pattern). Variable-size columns are added to
`crates/infra-postgres/storage-classification.json` in the same migration (ADR-0027). Migrations are
additive; each `down` refuses while rows of the new feature exist rather than deleting customer data.

## 1. Search attributes

### `search_attribute_registry`

| Column | Type | Notes |
|---|---|---|
| `tenant_id`, `namespace_id` | uuid | FK `(tenant_id, namespace_id)` → `namespaces` |
| `id` | uuid | UUIDv7; `UNIQUE (tenant_id, id)` |
| `name` | text | `^[a-z][a-zA-Z0-9_]{0,63}$`; `UNIQUE (tenant_id, namespace_id, name)` |
| `value_type` | text | `KEYWORD, TEXT, INT, DOUBLE, BOOL, DATETIME, KEYWORD_LIST` |
| `level` | text | `PUBLIC, INTERNAL, PII` (`SECRET` refused by CHECK) |
| `state` | text | `ACTIVE, RETIRED` |
| `revision` | integer | optimistic concurrency, ≥ 1 |
| `registered_by_version` | uuid | first version that declared it (FK to `workflow_versions`) |
| `created_at`, `updated_at` | timestamptz | database clock |

Rules: `value_type` never changes (trigger); `level` only moves to a stricter value (trigger compares the
order `PUBLIC < INTERNAL < PII`); `RETIRED` is final. Grants: SELECT, INSERT, UPDATE. Classification:
`name` protected (bounded pattern). Count per namespace is a quota resource `search_attributes`.

### `run_search_attributes`

| Column | Type | Notes |
|---|---|---|
| `tenant_id`, `namespace_id`, `run_id` | uuid | FK `(tenant_id, run_id)` → `runs` |
| `attribute_id` | uuid | FK `(tenant_id, attribute_id)` → registry |
| `element` | smallint | 0 for single-valued types; 0..63 for `KEYWORD_LIST` |
| `value_keyword` | text | ≤ 1,024 bytes (CHECK) |
| `value_text` | text | ≤ 8 KiB |
| `value_tsv` | tsvector | `GENERATED ALWAYS AS (to_tsvector('simple', value_text)) STORED` |
| `value_int` | bigint | |
| `value_double` | double precision | finite (CHECK) |
| `value_bool` | boolean | |
| `value_datetime` | timestamptz | |
| `set_seq` | bigint | journal sequence of the `search_attributes.upserted` event |

PK `(tenant_id, run_id, attribute_id, element)`. CHECK: exactly one value column is non-null.
Indexes (partial, one per type, so an upsert touches one secondary index):
`(tenant_id, namespace_id, attribute_id, value_keyword, run_id) WHERE value_keyword IS NOT NULL`,
likewise for `value_int`, `value_double`, `value_bool`, `value_datetime`; GIN on `value_tsv`.
Grants: SELECT, INSERT, UPDATE, DELETE (rows of a run are replaced by an upsert). Classification:
`value_keyword`, `value_text`, `value_tsv` are `content` (charged to `tenant_content_bytes`).

Invariant (FR-020-006): for every run, the rows equal the fold, in sequence order, of its
`search_attributes.upserted` events (each references a canonical JSON payload `{name: value|null}`).
`search_attributes.rejected` events carry `{name, reason}` in `detail` with a closed reason
(`TYPE_MISMATCH`, `TOO_LARGE`, `TOO_MANY_ELEMENTS`, `RETIRED`, `ENCRYPTED`, `EXPRESSION_ERROR`,
`RUN_LIMIT`) and no payload. The upsert payload is charged as retained content like any payload; the
016 disclosure operation answers `unavailable`/`NOT_DISCLOSABLE` for that sequence (values are read,
gated per attribute, through `getRunSearchAttributes`), and `listRunJournalEvents` shows only its digest.

### `run_views`

`(tenant_id, namespace_id, id)` PK; `name` (≤ 128 bytes, unique per namespace and owner for
`PRIVATE`, per namespace for `NAMESPACE`), `query` text (≤ 4 KiB, content), `columns` text[] (≤ 16
names), `sort` text, `visibility` `PRIVATE|NAMESPACE`, `owner` subject, `revision`, timestamps.
Grants: SELECT, INSERT, UPDATE, DELETE. Quota resource `run_views` (namespace count).

## 2. Journal note (generalized `telemetry_pending`)

Added columns (by the additive migration): `namespace_id uuid`, `moved_xid xid8 NOT NULL DEFAULT '1'`,
`moved_at timestamptz`, `tombstone_at timestamptz`. The **trigger and index are not created by the
migration**: `control-plane analytics enable` (migration owner role, recorded in the migration history)
installs a `BEFORE INSERT OR UPDATE` trigger that fills `namespace_id` from `runs` on insert and sets
`moved_xid = pg_current_xact_id()` and `moved_at = clock_timestamp()` when the row is inserted or
`head_seq`/`tombstone_at` changes (the telemetry projector's own `applied_seq` updates do not move it),
and the index `telemetry_pending_moved_idx ON (tenant_id, namespace_id, run_shard(run_id), moved_xid,
run_id)`, built `CONCURRENTLY`. With analytics disabled the write path is unchanged. Existing notes
(constant default, no rewrite) are reached by backfill, not live discovery. The 24 h note purge
(`purge_projected`) additionally keeps a note while any **non-`DELETED`** sink of its namespace (paused,
auto-paused, failing and throttled included) has a lower applied sequence for the run, until that sink's
hold expires; the expiry increments the sink's `retention_gaps`. Writers are unchanged; a test asserts
every journal-appending transaction moves the note (`assert_consistent`).

## 3. Analytics sinks

| Table | Key | Content |
|---|---|---|
| `analytics_sinks` | `(tenant_id, namespace_id, id)` | `name` (unique per namespace), `current_revision`, `state` (`ACTIVE, PAUSED, AUTO_PAUSED, FAILING, THROTTLED, DELETED`), `state_reason` (closed), `disclosure_owner` (subject, for `PII`), `activation_xid xid8`, timestamps |
| `analytics_sink_revisions` | `(tenant_id, sink_id, revision)` | `kind` (`OBJECT_STORAGE, CLICKHOUSE, POSTGRESQL, ICEBERG, STREAM, PULL`), `settings jsonb` (closed per kind, no credential, ≤ 16 KiB), `credentials_secret` (secret name or null), `record_kinds text[]`, `workflows uuid[]` (≤ 64), `batch_policy jsonb`, `max_level` (`INTERNAL, PII`), `start_position`, `actor`, `request_id`, `created_at`; immutable (ALWAYS trigger) |
| `analytics_sink_cursors` | `(tenant_id, sink_id, shard)` | `after_xid xid8`, `after_tenant`, `after_run`, `next_batch_seq`, `oldest_pending_at`, backfill `after_run`/`upper_run`/`done`, `retention_gaps bigint` |
| `analytics_envelope_cursors` | `(tenant_id, sink_id, kind)` | as above for `inbound_event`/`delivery` over `moved_xid` |
| `analytics_sink_runs` | `(tenant_id, sink_id, run_id)` | `applied_seq`, `tombstoned`, `updated_at`; deleted for terminal, fully applied runs after the ledger window |
| `analytics_batches` | `(tenant_id, sink_id, shard, batch_seq)` | `region_epoch` (032 epoch at plan, 0 without 032; part of every record key, object name, token and batch id of the batch), `state` (`PLANNED, LEASED, ACKED, REJECTED, SUPERSEDED, SKIPPED`), `revision`, window from/to, `record_count`, `byte_count`, `content_sha256`, `attempts`, `next_attempt_at`, `last_outcome` (closed), pull lease (`holder`, `token_hash`, `generation`, `expires_at`), `planned_at`, `acked_at` |
| `analytics_batch_runs` | batch key + `run_id` | `from_seq`, `upto_seq`, `tombstone` |
| `analytics_exporter_bindings` | `(tenant_id, namespace_id, sink_id, subject)` | `created_by`, `created_at` |
| `analytics_export_usage` | `(tenant_id, day)` | exported bytes (quota `analytics_export_bytes_per_day`) |

Kind-specific settings: `POSTGRESQL` holds `connectionRef {name, revision}` of a 017 connection (the
revision is pinned in the sink revision, like a run admission pin; no host or credential of its own);
`CLICKHOUSE` holds `url`, `database`, `tablePrefix` and a credentials secret reference (ClickHouse is not a
017 catalog engine, G1617), validated against the managed executor's destination policy; `STREAM` holds
the id of a subscription dedicated to the sink (refused if it receives run events); `ICEBERG` holds
`connectionRef {name, revision}` of a feature-035 Iceberg connection, `namespace` and `tablePrefix`; `PULL` holds lease
settings. Exporter bindings name a subject whose role is `analytics-exporter` (for `PULL`) or
`database-runner` (for `POSTGRESQL`, `CLICKHOUSE` and `ICEBERG`). `analytics_batches.kind` is `RECORDS` or `PROBE`
(a zero-record batch a database-runner leases to run the sink's fixed probe statement).

Constraints: one open batch per `(tenant_id, sink_id, shard)` (partial unique index `WHERE state IN
('PLANNED','LEASED')`); `shard BETWEEN 0 AND 63`; `generation ≥ 0`; lease columns all null or all set.
Grants: SELECT, INSERT, UPDATE on all; DELETE on `analytics_sink_runs`, `analytics_batches`,
`analytics_batch_runs` (ledger purge after the window) and bindings. Classification: `settings`,
`record_kinds`, `workflows` content (tenant configuration); everything else protected (closed codes,
ids, counters).

State transitions of a sink: `ACTIVE ↔ PAUSED` (user); `ACTIVE → AUTO_PAUSED` (permanent rejection,
credential unusable, disclosure revoked, destination denied); `ACTIVE ↔ FAILING` (transient threshold /
next ack); `ACTIVE ↔ THROTTLED` (export quota / next day); any → `DELETED` (tombstone; cursors and
open batches retired; ledger kept for its window). Resume from `AUTO_PAUSED` requires a new revision or
an explicit resume after the cause is cleared.

Derivation never reads mutable state: a `run_snapshot` at `upto_seq` takes its status from the last
status event at or before `upto_seq`, its counts from indexed scans of that journal prefix and its
attribute values from folding the `search_attributes.upserted` events of the prefix, so a retried batch
derives the same snapshot even if the run moved since the plan. A snapshot is emitted for a run in a
batch when the range changes its status or attributes, or when it is the run's first batch for the sink.

Excluded runs: a run whose origin is 022's `DRAFT_TEST`, or that belongs to a `test_tree`, is skipped at
discovery and never appears in any record, archive or `RunSelector` page (D1619).

Restored history (018 reset): `journal_event` records of events 018 marks as restored carry
`restoredFrom {runId, seq}`, and restored attempts are not re-emitted as `step_attempt` records (they
were exported under the source run), matching how telemetry skips the restored prefix.

Batch transitions: `PLANNED → ACKED | REJECTED | SUPERSEDED | SKIPPED`; `PLANNED → LEASED → ACKED`
(pull); `LEASED → PLANNED` on lease expiry (generation + 1).

### Envelope rows

`inbox_events` and `webhook_deliveries` gain `analytics_rev bigint NOT NULL DEFAULT 0` and `moved_xid
xid8 NOT NULL DEFAULT '1'`. Their `BEFORE INSERT OR UPDATE` triggers (`analytics_rev + 1`,
`pg_current_xact_id()`) and `(tenant_id, namespace_id, moved_xid)` indexes are installed only by
`control-plane analytics enable --envelopes`; until then a revision asking for `inbound_event` or
`delivery` is refused (`ENVELOPES_NOT_ENABLED`). Rows of `webhook_deliveries` with
`kind = 'analytics.batch'` are measured as `content` by the storage manifest (D1105); every other kind
keeps its protected closed schema.

## 4. Canonical record levels (D1100)

| Record field | Level |
|---|---|
| ids (tenant, namespace, run, workflow, version, parent, root, chain, subscription, delivery), node, instance path, attempt, generation, status, codes, event type, source, timestamps, durations, counts, trace id | `internal` |
| `run_snapshot.correlationId`, `run_snapshot.startedBy`, `run_snapshot.businessId` (018), `journal_event.actor` (for `user:` actors), `inbound_event.subject`, `inbound_event.correlationKey` | `pii` |
| search attribute values | registry level, and only when the run's pinned version declared `export` for the attribute; otherwise the name with `withheld` reason `NOT_DECLARED` |
| declared record fields | effective level from the record schema and its sources (D1091) |

A sink with `max_level = INTERNAL` receives `pii` fields as absent with a `withheld` entry naming the
field; no sink receives `secret`.

## 5. Retention and archival

### `retention_policies`

`(tenant_id, namespace_id)` PK; `hot_retention_days` (1..3650 or null), `archive` (`REQUIRED, NONE`),
`archive_values` (`METADATA_ONLY, UP_TO_INTERNAL, UP_TO_PII`), `destination_kind` (`PLATFORM,
OBJECT_STORAGE`), `destination_settings jsonb` (content), `credentials_secret`, `sink_hold_days`
(1..30), `revision`, `updated_by`, `updated_at`. Effective days = min(namespace, tenant default,
operator ceiling) (quota resource `run_retention_days`, ADR-0027 resolution). Grants: SELECT, INSERT,
UPDATE.

### `runs` additions

`purged_at timestamptz`, `archive_state text NOT NULL DEFAULT 'NONE'` (`NONE, ARCHIVED,
PURGED_WITHOUT_ARCHIVE, ERASED`), `archive_object text` (≤ 512 bytes, protected pattern),
`archive_sha256 bytea`. No constraint is relaxed: `input_digest` of a stub points to the tenant's
tombstone payload (canonical `{"$purged":true}`, one row per tenant, inserted by the purge function), so
an older engine reads a stub as a run without history. A stub keeps every other column;
`correlation_id` and 018's `business_id` are set to null at purge (018's immutability trigger consults
the purge authorization, G1103); `started_by` is replaced by `system:erased` only on erasure.

## 6. Archival jobs

`archival_jobs`, PK `(tenant_id, run_id)`: `namespace_id`, `reason` (`RETENTION, ERASURE`), `state`,
`attempts`, `next_attempt_at`, `lease_holder`, `lease_expires_at`, `object_key`, `object_size`,
`object_sha256`, `values_level`, `requested_by`, `blocked_reason`, timestamps.

```
ELIGIBLE ──claim──▶ EXPORTING ──PUT ok──▶ EXPORTED ──HEAD+digest ok──▶ VERIFIED ──▶ PURGING ──▶ PURGED
    │                  │ PUT fails (retry)        │ mismatch / missing          ▲
    │                  └──────────────▶ EXPORTING └──────▶ BLOCKED(ARCHIVE_INTEGRITY) (manual)
    └── archive = NONE ─────────────────────────────────────────────────────────┘
```

Each arrow is one transaction; object-store I/O happens between transactions with the job leased
(`FOR UPDATE SKIP LOCKED`, expiry on the database clock). `PURGING` is entered only from `VERIFIED` or
from `ELIGIBLE` when `archive = NONE`, and is not reversible.

### Purge privilege

`run_events` and `payloads` are append-only by `ALWAYS` triggers and the runtime role has no `DELETE`
on `runs`, `steps`, `payloads` or `run_events`. Purge uses the definer function
`purge_run_content(tenant, run, job_token)` which (1) locks the job and requires `VERIFIED` (or
`archive = NONE`) with a matching token and a policy revision that is an audited change by a human
principal, (2) on the first chunk locks the run row `FOR UPDATE` (conflicting with the ADR-0039 guard's
key-share lock, so a concurrent rerun either commits first or then sees the purge) and rechecks
eligibility under the run-tree lock order, (3) records `(pg_current_xact_id(), tenant, run)` in the
private table `retention_purge_authorizations` (no runtime grant), which the append-only trigger
functions consult before allowing a `DELETE` of that run's rows, (4) deletes run-owned rows in bounded
chunks, (5) deletes candidate payloads and run-purpose blobs (`STEP_OUTPUT`, `RUN_INPUT` → `DELETING`)
that nothing still references, never `DATA_SNAPSHOT_PART` blobs (feature 024, ADR-0058), (6) on erasure
calls the registered invalidation hooks (022 `step_cache_entries` by `source_run_id`), (7) updates the
stub, the note (`tombstone_at`) and the job, and writes `run.purged` audit. No session setting or caller
argument can authorize a deletion. Content counters are released by the existing ADR-0027 triggers.

**Content-store registry (D1616, coordinator C8).** `crates/infra-postgres/run-content-references.json`
records, for every place that holds run content, how purge and erasure treat it. The entries other drafts
own are registered by those drafts in their data models with a pointer here; 020 owns the file and its
test.

| Store (owner) | Link to the run | Purge (retention) | Erasure |
|---|---|---|---|
| Journal, steps, timers, subscriptions, attribute rows (002/014/020) | FK | deleted | deleted |
| `payloads` rows (002) | FK from every referencing column | deleted when unreferenced | same |
| `blobs` `STEP_OUTPUT`, `RUN_INPUT` (ADR-0023) | catalog scope / FK | to `DELETING` when unreferenced | same |
| `blobs` `DATA_SNAPSHOT_PART` (024) | snapshot catalog | **excluded** (ADR-0058 owns retention) | excluded |
| 024 snapshot metadata: `data_snapshots`, `data_snapshot_parts`, `data_snapshot_inputs`, `data_snapshot_audits`, `data_asset_publications` | producer run fence / provenance | **excluded** from run purge (ADR-0058 owns their retention, even when the producing run is purged) | excluded (record-level erasure inside a snapshot is 024's G1250) |
| 024 run-scoped data links: `run_data_assets`, `step_data_inputs`, `step_data_output_bases`, `run_data_snapshot_refs` | FK run | **excluded**: left in place on the stub (runs are never deleted); ADR-0058 decides what they mean for snapshot retention | excluded |
| 018 retained Update arguments | `run_events.payload_digest` of `state.updated` | with the journal | with the journal |
| 018 `schedule_revisions.input_digest`, `batch_jobs.payload_digest` | FK to `payloads` | keep the payload (reference) | keep (not run content) |
| 019 `step_checkpoints` | FK run | deleted | deleted |
| 022 `step_cache_entries` | `source_run_id` (no FK) | kept to their own TTL (022) | deleted by `source_run_id` |
| 022 `step_cache_hit_provenance` | `consumer_run_id` FK, `source_run_id` | deleted with the consumer run | finds every consumer of an erased source; their copied payload is reported |
| 022 `test_run_pins.payload_digest` | FK to `payloads` | keep the payload (reference) | reported if it pins the erased run |
| 025 transcripts (`agent_transcript_entries`, tool inputs/results) and request staging blobs | FK run / attempt | purged with the run's content | same; data sent to providers stays outside (025 G1285) |
| 026 `run_stream_events` | FK run | deleted (or earlier by `stream_retention_seconds`) | deleted |
| 030 `service_operation_messages`, `service_operations` digests | FK to caller run and `payloads` | settled messages by 030's own retention; digests keep payloads | deleted for the erased run; handler run handled as its own run |

**Reference discovery.** Candidate payloads/blobs are checked against every foreign key that references
`payloads` or `blobs` in `pg_catalog` (so 018 `schedule_revisions.input_digest`, `batch_jobs.payload_digest`
and 022 `test_run_pins.payload_digest` are covered when those features exist) plus a checked registry
`crates/infra-postgres/run-content-references.json` of non-FK copies with run provenance and their erasure
hook. A migration test fails when a referencing column or a content column with run provenance is in
neither (the pattern of `storage-classification.json`). Each referencing column needs an index (G1099).

## 7. Retention of the feature's own records

| Record | Retention |
|---|---|
| attribute rows | with run content (purge) |
| saved views | until deleted |
| sink revisions | while the sink exists, plus tombstone |
| batches, batch runs (ledger) | 30 days default, 90 max (`analytics_ledger_days`) |
| `analytics_sink_runs` | terminal and fully applied: removed after the ledger window |
| archival jobs | `PURGED` rows kept 90 days, then removed |
| run stubs | indefinitely in v1 (G1089, O1082) |
