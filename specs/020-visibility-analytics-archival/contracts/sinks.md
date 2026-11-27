# Sink adapter contracts (draft, feature 020)

*Corrected 2026-11-24 (research Decision 020-002):* the engine sends nothing to a customer destination.
Every sink whose destination is a 017 connection (`OBJECT_STORAGE`, `CLICKHOUSE`, `POSTGRESQL`, `ICEBERG`)
is leased and written by a bound `apps/database-runner` with feature 035's writer for that connection
kind: the `database` profile for PostgreSQL and ClickHouse, the `lake` profile (image `lake-runner`) for
object storage and Iceberg. `STREAM` batches are rows of the platform's own outbox; `PULL` batches are
leased by a customer's exporter. This is stricter than the cross-feature rule written by 021 in ADR-0055
(rule 11), so 020 no longer relies on its clause (b). Every record key, object name, token and batch id
carries the 032 region epoch (D1615).

Implementation contract for the batch entry points 020 adds to 035's writers (`crates/infra-database`,
`crates/infra-lake`, `apps/database-runner/src/lake/`), driven by the runner's lease, resolve and ack
loop over `AnalyticsExportService`. Each writer answers `write(batch) → Acked | Transient(code) |
Permanent(code)` and `probe() → Ok | Failed(code)`. Codes are closed (contracts/errors.md); provider
messages, URLs and bodies never reach logs, spans, the API or the database. Every connection goes through
017's destination policy (operator CIDRs and ports narrowed by namespace, private networks allowed,
platform state endpoints refused); the `lake` profile's object-store client reaches out only through its
relay (D035-048..050). Credentials come from the connection, resolved per batch through
`ResolveSinkConnection` (audited `secret.resolved`) and zeroised when the batch ends.

## OBJECT_STORAGE (lake profile, feature 035's `object_storage` connection)

Settings: `connectionRef {name, revision?}` of an `object_storage` connection of the sink's namespace
(its storage provider, credentials, confinement, prefix and formats are the connection's, FR-035-040),
`prefix` (≤ 256 bytes, no `..`, under the connection's prefix), `format` (`NDJSON_GZIP` default or
`PARQUET`, among the connection's formats). *Corrected 2026-11-24:* no provider, bucket, endpoint or
secret on the sink (O035-005).

- Object name: `<connection prefix>/<prefix>/sagat.analytics.v1/<kind>/dt=<UTC date of batch plan>/epoch=<e>/shard=<k>/<sinkId>-<e>-<shard>-<batchSeq>.<ndjson.gz|parquet>`
  (`<e>` is the 032 region epoch of the batch, 0 without 032), one object per kind present in the batch;
  a manifest `<connection prefix>/<prefix>/sagat.analytics.v1/_manifests/<sinkId>/<e>/<shard>/<batchSeq>.json`
  lists objects, record counts and SHA-256.
- Write order: data objects, then manifest, each create-only (`PutMode::Create` where the provider
  supports it, else an existence check first, as FR-035-040 does). An existing object whose
  `sagat-records-sha256` metadata equals the batch kind's records digest is the write's success; a
  different one is a permanent nack `OBJECT_CONFLICT` (sink `AUTO_PAUSED`/`SINK_REJECTED`), never an overwrite. Ack
  only after a HEAD of every object returns the expected size. So a retry needs equal records (records in
  key order), not byte-identical files, and Parquet needs no deterministic encoder.
- Readers (DuckDB `read_ndjson`/`read_parquet`, Athena, Spark) deduplicate with
  `QUALIFY row_number() OVER (PARTITION BY key ORDER BY version DESC) = 1`; documented in
  `docs/runbooks/analytics.md`.
- *Closed 2026-11-24:* the gate of G1083 no longer applies; the engine has no object-store route to a
  customer bucket.

## CLICKHOUSE (database profile, feature 035's `clickhouse` connection)

Settings: `connectionRef {name, revision?}` of a `clickhouse` connection of the sink's namespace (its URL,
HTTPS and verify-full TLS, database, CA and credentials are the connection's, FR-035-041) and
`tablePrefix`. *Corrected 2026-11-24:* the owner's answer to O035-005 replaces the sink's own URL and
secret (G1617 closed); the destination is checked by 017's destination policy, not the engine's egress
guard.

- Delivery: a bound `database-runner` identity leases the sink's frozen batches over
  `AnalyticsExportService`, calls `ResolveSinkConnection` under the batch lease (the connection's
  settings and credential in `ResolvedDatabase.clickhouse`, through the same two-phase OpenBao resolution
  as 017, fenced by the lease) and sends, with 035's ClickHouse writer, only
  fixed, platform-generated statements over ClickHouse's HTTP interface: one `INSERT INTO
  <db>.<prefix><kind> FORMAT JSONEachRow` per kind with settings
  `insert_deduplication_token='<sinkId>:<e>:<shard>:<batchSeq>:<kind>'` (epoch included, D1615) and
  `deduplicate_insert=1` (replicated tables, or `non_replicated_deduplication_window > 0`, documented);
  then `AckBatch`. The runner's network policy must permit the approved ClickHouse peers and ports.
- Documented DDL (shipped in `docs/runbooks/analytics.md`, never executed by the platform):
  `ENGINE = ReplacingMergeTree(version) ORDER BY (tenant_id, namespace_id, key)`, one table per kind,
  columns matching `sagat.analytics.v1` with unknown future fields ignored by `input_format_skip_unknown_fields=1`.
- Exact reads need `FINAL` or `argMax(…, version)`; merges are eventual (G1085).
- Uncertain outcome (a lost response, a retry that may fall outside the deduplication window): the batch
  is inserted again. Unlike 035's step writes, which go to review (FR-035-041), analytics records are
  grow-only with a key and version, so a duplicate is a reader concern (the rule of D1671).
- Outcomes: 2xx → ack; 5xx, timeout, connection reset → transient nack; 4xx with an unknown-column or
  type error → permanent nack `SCHEMA_MISMATCH` (`AUTO_PAUSED`/`SINK_SCHEMA_MISMATCH`); 401/403 →
  permanent nack `AUTH_FAILED`. Response bodies are never logged or stored.
- Probe: a zero-record batch of kind `PROBE` leased by the runner, which runs the fixed `DESCRIBE TABLE`
  of each enabled kind and compares it with the expected columns; the outcome is recorded on the sink.

## POSTGRESQL (database profile, ADR-0035)

Settings: `connectionRef {name, revision?}` of a 017 connection in the sink's namespace, `schema`,
`tablePrefix`. The connection revision is resolved and pinned when the sink revision is created (as run
admission pins it); a later connection edit needs a new sink revision. The connection must be in execute
mode and usable by the sink's disclosure owner (017 ACL checks at revision creation and at every lease).
No host, port or credential is stored on the sink, and no PostgreSQL client exists in the engine.

- Delivery: a bound `database-runner` identity leases the sink's frozen batches over
  `AnalyticsExportService` (contracts/analytics-export-protocol.md), calls `ResolveSinkConnection` under
  the batch lease (the 017 two-phase credential resolution, fenced by the lease instead of a step
  attempt), and writes the batch with fixed platform templates
  `INSERT INTO <schema>.<prefix><kind> (…) VALUES … ON CONFLICT (tenant_id, key, version) DO NOTHING`
  in one explicit transaction with 017's statement and lock deadlines, destination policy (operator
  CIDRs narrowed by namespace, private networks allowed) and TLS identity checks; then `AckBatch`.
- Effect contract: the insert is idempotent by key, which is the declared, proven effect contract
  ADR-0035 decision 11 requires before a write may be retried; an ambiguous commit is simply retried
  (no manual review), and duplicates are absorbed by the key.
- Documented DDL with latest-state views (`DISTINCT ON (key) … ORDER BY key, version DESC`); the
  platform never runs DDL. The platform's own database is refused (017 rule).
- Probe: a zero-record `PROBE` batch; the runner reads the documented tables' columns from
  `information_schema` (fixed statement) and compares them with the expected columns.
- Outcomes: commit → ack; serialization/connection errors → transient nack; undefined table/column or
  type error → permanent nack `SCHEMA_MISMATCH`; authentication → permanent nack `AUTH_FAILED`; a paused,
  deleted or denied connection → `AUTO_PAUSED`/`CONNECTION_UNUSABLE`.
- SQL text, parameters and rows are never logged (017 log rules); runner diagnostics use the closed
  vocabulary.

## ICEBERG (lake profile of feature 035, added 2026-10-27)

Settings: `connectionRef {name, revision?}` of a feature-035 Iceberg connection in the sink's namespace,
`namespace` (Iceberg namespace, one or more levels) and `tablePrefix`. The connection revision is pinned
by the sink revision as for `POSTGRESQL`; the sink's `maxLevel` must not exceed the connection's
classification ceiling (FR-035-005). No catalog address, storage location or credential is stored on
the sink, and no Iceberg, catalog or object-storage client exists in the engine (ADR-0055 rule 11).

- Delivery: a bound `database-runner` identity on the `lake` profile (*corrected 2026-11-24:* 035's plan
  places Iceberg I/O there, with the Iceberg semantics in a per-attempt sandboxed PyIceberg host)
  leases the sink's frozen batches, calls `ResolveSinkConnection` under the batch lease (035's catalog
  and storage credentials, through the 017 two-phase OpenBao resolution, fenced by the lease) and writes
  one append commit per record kind to `<namespace>.<prefix><kind>` on `main`, with the effect key
  `<sinkId>:<e>:<shard>:<batchSeq>:<kind>` (epoch included, D1615) in the snapshot summary; then
  `AckBatch` once every kind of the batch committed.
- Effect contract: before each commit the executor runs 035's lookup (FR-035-018) for that key and
  skips a kind whose commit already landed; commits are conditional on the searched table state. When
  the lookup is inconclusive (history expired) it appends again: records carry `key` and `version`, and
  readers deduplicate with `QUALIFY row_number() OVER (PARTITION BY key ORDER BY version DESC) = 1` as
  for object storage, so no manual review is needed (D1671).
- Documented tables (shipped in `docs/runbooks/analytics.md`, never created by the platform): one table
  per kind, columns matching `sagat.analytics.v1`, partitioned by day of the batch plan, Parquet files.
  Iceberg favours few large commits: the runbook recommends a batch policy near the maximums and table
  maintenance by the owner (snapshot expiry, compaction), and states that expiry shortens the lookup
  window (G035-005).
- Probe: a zero-record `PROBE` batch; the executor loads each enabled kind's table and compares its
  current schema with the expected columns.
- Outcomes: commit → ack; catalog or storage unavailable, commit conflict past 035's retry bound →
  transient nack; schema mismatch or unsupported table (FR-035-011) → permanent nack `SCHEMA_MISMATCH`;
  authentication → permanent nack `AUTH_FAILED`; a table location outside the connection's warehouse →
  `AUTO_PAUSED`/`SINK_DESTINATION_DENIED`; a paused, deleted or denied connection →
  `AUTO_PAUSED`/`CONNECTION_UNUSABLE`.
- Record values, file locations (which can embed partition values) and provider responses are never
  logged (FR-035-030).

## STREAM

Settings: `subscriptionId` of a webhook or broker subscription of the namespace (ADR-0025) **dedicated to
the sink**: it receives no run events of its own (refused `STREAM_SUBSCRIPTION_SHARED`), so an analytics
backlog cannot trip ADR-0025's switch-off (20 consecutive dead or 10,000 pending) for the namespace's
operational events. No adapter of its own: the ack transaction inserts one delivery row per part
(contracts/asyncapi.fragment.yaml) and the cursor advances with it; delivery, retry and redrive are the
pipeline's. A delivery of kind `analytics.batch` that ends `DEAD` auto-pauses the sink
(`STREAM_DELIVERY_DEAD`): a dead part is otherwise a silent skip. Redrive the delivery, then resume. A
subscription that is disabled or deleted auto-pauses the sink (`SINK_DESTINATION_DENIED`).

## PULL

Settings: `leaseSeconds` (default 60, max 600), `maxExporters` (default 2, max 8). Batches are leased
over `AnalyticsExportService` (contracts/analytics-export-protocol.md); the engine never connects out.
