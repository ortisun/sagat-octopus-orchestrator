# Sink adapter contracts (draft, feature 020)

Placement follows the cross-feature rule written by 021 in ADR-0055 (coordinator C3, rule 11): fixed-shape
batches to non-database destinations (object storage, stream subscriptions) are sent by the engine through
the guarded egress client holding only that destination's credential (rule 11(b)); no engine or
control-plane process opens a connection to a customer database, whatever the protocol (rule 11(c)), so
ClickHouse and PostgreSQL sinks are written by `apps/database-runner`.
Every record key, object name, token and batch id carries the 032 region epoch (D1615).

Implementation contract for `crates/infra-analytics` (one cargo feature per engine-side backend:
`sink-object-store`, `sink-parquet`) and for the database batch writers (PostgreSQL, ClickHouse) in
`crates/infra-database`, run by `apps/database-runner` (revised 2026-10-27, review S5 and coordinator). Each adapter implements the
application port `analytics::SinkWriter` with one method, `write(batch) → Acked | Transient(code) |
Permanent(code)`, and `probe() → Ok | Failed(code)`. Codes are closed (contracts/errors.md); provider
messages, URLs and bodies never reach logs, spans, the API or the database. Every engine-side network
connection goes through `crates/infra-egress` (public destinations only; loopback, link-local, metadata,
private ranges and configured platform state endpoints refused). Engine-side credentials come from the
sink's secret reference, resolved per batch through the existing secret service (audited
`secret.resolved`, collapsing identical resolutions as today) and dropped after the batch.

## OBJECT_STORAGE

Settings: `provider` (`S3`, `R2`, `GCS`, `AZURE`), `bucket`, `prefix` (≤ 256 bytes, no `..`), `region`,
`endpoint` (optional, egress-checked), `format` (`NDJSON_GZIP` default, `PARQUET` with
`sink-parquet`). Credentials secret: `{"accessKeyId","secretAccessKey","sessionToken"?}` for S3/R2.

- Object name: `<prefix>/sagat.analytics.v1/<kind>/dt=<UTC date of batch plan>/epoch=<e>/shard=<k>/<sinkId>-<e>-<shard>-<batchSeq>.<ndjson.gz|parquet>`
  (`<e>` is the 032 region epoch of the batch, 0 without 032), one object per kind present in the batch;
  a manifest `<prefix>/sagat.analytics.v1/_manifests/<sinkId>/<e>/<shard>/<batchSeq>.json`
  lists objects, record counts and SHA-256.
- Write order: data objects, then manifest. Ack only after a HEAD of every object returns the expected
  size (and checksum where the provider returns one). A retry writes the same names with identical bytes
  (gzip with fixed header fields and level, records in key order).
- Readers (DuckDB `read_ndjson`/`read_parquet`, Athena, Spark) deduplicate with
  `QUALIFY row_number() OVER (PARTITION BY key ORDER BY version DESC) = 1`; documented in
  `docs/runbooks/analytics.md`.
- Gate (G1083): customer endpoints stay refused until the `object_store` HTTP client is proven to go
  through the guarded resolver; until then only endpoints the operator allow-lists are accepted.

## CLICKHOUSE (managed database executor)

Settings: `url` (HTTPS only, TLS verify-full), `database`, `tablePrefix`, `compression` (`gzip`) on the
sink revision, and a credentials secret `{"username","password"}`. ClickHouse is not an engine of the 017
connection catalog, so the destination lives on the sink (G1617); it is validated against the managed
executor's destination policy (operator-approved CIDRs and ports narrowed by namespace, private networks
allowed, platform state endpoints refused — ADR-0035 decision 4), not the engine's egress guard. If a
later feature adds ClickHouse to the 017 catalog, a sink may reference that connection instead.

- Delivery: a bound `database-runner` identity leases the sink's frozen batches over
  `AnalyticsExportService`, calls `ResolveSinkConnection` under the batch lease (settings plus the
  credential, through the same two-phase OpenBao resolution as 017, fenced by the lease) and sends only
  fixed, platform-generated statements over ClickHouse's HTTP interface: one `INSERT INTO
  <db>.<prefix><kind> FORMAT JSONEachRow` per kind with settings
  `insert_deduplication_token='<sinkId>:<e>:<shard>:<batchSeq>:<kind>'` (epoch included, D1615) and
  `deduplicate_insert=1` (replicated tables, or `non_replicated_deduplication_window > 0`, documented);
  then `AckBatch`. The runner's network policy must permit the approved ClickHouse peers and ports.
- Documented DDL (shipped in `docs/runbooks/analytics.md`, never executed by the platform):
  `ENGINE = ReplacingMergeTree(version) ORDER BY (tenant_id, namespace_id, key)`, one table per kind,
  columns matching `sagat.analytics.v1` with unknown future fields ignored by `input_format_skip_unknown_fields=1`.
- Exact reads need `FINAL` or `argMax(…, version)`; merges are eventual (G1085).
- Outcomes: 2xx → ack; 5xx, timeout, connection reset → transient nack; 4xx with an unknown-column or
  type error → permanent nack `SCHEMA_MISMATCH` (`AUTO_PAUSED`/`SINK_SCHEMA_MISMATCH`); 401/403 →
  permanent nack `AUTH_FAILED`. Response bodies are never logged or stored.
- Probe: a zero-record batch of kind `PROBE` leased by the runner, which runs the fixed `DESCRIBE TABLE`
  of each enabled kind and compares it with the expected columns; the outcome is recorded on the sink.

## POSTGRESQL (managed database executor, ADR-0035)

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

## ICEBERG (managed executor of feature 035, added 2026-10-27)

Settings: `connectionRef {name, revision?}` of a feature-035 Iceberg connection in the sink's namespace,
`namespace` (Iceberg namespace, one or more levels) and `tablePrefix`. The connection revision is pinned
by the sink revision as for `POSTGRESQL`; the sink's `maxLevel` must not exceed the connection's
classification ceiling (FR-035-005). No catalog address, storage location or credential is stored on
the sink, and no Iceberg, catalog or object-storage client exists in the engine (ADR-0055 rule 11).

- Delivery: a bound `database-runner` identity (or the executor 035's plan designates for Iceberg I/O)
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
