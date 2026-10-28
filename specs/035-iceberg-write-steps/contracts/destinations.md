# Destination write contracts (draft, feature 035)

Per-kind behaviour of synchronous destinations (FR-035-038..042). Common rules are in FR-035-036;
derived nodes and policies in D035-038, D035-054 and D035-055; receipts in
[receipts.schema.json](receipts.schema.json). Revised after review round 1 (D035-052, D035-053,
D035-060, D035-061, D035-064).

| Kind | Executor | Activity | Recovery rule | Automatic retry of an uncertain write |
|---|---|---|---|---|
| `iceberg` | lake deployment | `sagat.database.destination.iceberg` | effect key in snapshot history (D035-030) | yes, after a conclusive lookup |
| `postgresql` | database deployment | `sagat.database.destination.postgresql` | constraint-backed key, one transaction | yes: a proven effect contract (ADR-0035 decision 11) |
| `object_storage` | lake deployment | `sagat.database.destination.object_storage` | effect-key object name, create-only or checked, record checksum | yes |
| `clickhouse` | database deployment | `sagat.database.destination.clickhouse` | effect key as the insert deduplication token | only when both deduplication windows are proven (D035-060) |
| `broker_stream` | engine; the shard owner settles the step | none | delivery rows and payload committed with the source completion | the pipeline's retries; duplicates possible |

The runner's own clients (`tokio-postgres`, `reqwest`, `object_store`) resolve through
`NetworkPolicy::resolve`, follow no redirect, ignore proxy variables and use only the resolved
credentials (D035-061).

## postgresql

- Discovery uses fixed statements in `crates/infra-database`: the columns of `<schema>.<relation>` from
  `information_schema.columns`, and the constraints from `pg_catalog.pg_constraint` and `pg_index`.
  The key column set must equal a primary key or a non-partial unique constraint; otherwise the write
  fails `DESTINATION_KEY_CONSTRAINT_MISSING`.
- Statements quote identifiers only after matching them exactly against the discovered names, and
  bind every value as a parameter:
  - `insert`: `INSERT INTO "s"."r" ("c1",…) VALUES ($1,…),… ON CONFLICT ("k",…) DO NOTHING`;
  - `upsert`: `… ON CONFLICT ("k",…) DO UPDATE SET "c1" = EXCLUDED."c1", …`, key columns excluded;
  - the optional `effectKeyColumn` receives the destination effect key on every row.
- Each destination write is one explicit transaction under 017's statement and lock deadlines, with
  at most 1,000 rows per statement (chunked inside the transaction).
- Values follow 017's typed cells. A value the column cannot hold losslessly fails
  `DESTINATION_SCHEMA_MISMATCH`, naming the column.

## clickhouse

- The target database is the connection's `database`; the IR declares only the table.
- Identifier checks: the table must exist in `system.tables` and every mapped column in
  `system.columns`, matched exactly.
- Request: `POST <url>/?query=INSERT INTO {db:Identifier}.{t:Identifier} ({c1:Identifier}, …) FORMAT JSONEachRow`,
  with the parameters percent-encoded (`param_db`, `param_t`, …). Settings:
  - `insert_deduplication_token=<effect key>`;
  - `deduplicate_insert=enable` on servers ≥ 26.2, otherwise `insert_deduplicate=1`;
  - `async_insert=0` and `input_format_skip_unknown_fields=0`.
  The body is NDJSON of the mapped records. Credentials go in headers, never in the URL.
- At the start of every attempt the writer reads the engine and its deduplication settings
  (`replicated_deduplication_window` and `…_seconds`, or `non_replicated_deduplication_window`).
  Before the first insert it records the table's per-partition maximum block number from
  `system.parts`, durably with the attempt. If no durable per-attempt checkpoint exists to hold it,
  a gap is recorded and every uncertain insert is unknown.
- Retry rule: an uncertain insert is retried automatically only when deduplication is confirmed, the
  elapsed time is below the window seconds minus 60 s, **and** the block-number delta is below the
  window blocks. Otherwise the outcome is `OUTCOME_UNKNOWN` (G035-008).
- Responses: 2xx → success; 5xx, timeout or reset → retryable; 4xx for an unknown column or a type
  error → `DESTINATION_SCHEMA_MISMATCH`; 401 or 403 → `DESTINATION_AUTH_FAILED`.
- The probe runs as activity `sagat.database.clickhouse.probe` and records the deduplication facts it
  found with the connection's probe result.

## object_storage

- Name: `<prefix>/<workflow>/<destination>/dt=<run admission date UTC>/<hex sha-256(effect key)>.<parquet|ndjson>`.
- Metadata: `sagat-records-sha256` (the SHA-256 of the canonical NDJSON form of the mapped records,
  which is stable across attempts, unlike Parquet bytes) and `sagat-effect-key-sha256`.
- When the probe confirmed `conditionalCreate = true`, the write uses `PutMode::Create`. On a
  precondition failure it reads the existing object's `sagat-records-sha256`: equal → `ALREADY_PRESENT`;
  different → `DESTINATION_OBJECT_CONFLICT` (review).
- When `conditionalCreate = false`, the write does a `HEAD` with the same comparison, then a plain put.
  The gap between `HEAD` and put is documented: two attempts with the same key write the same records.
- The write never overwrites or deletes. Multipart uploads above 64 MiB carry the condition on
  completion where the provider supports it (not R2), and are checked afterwards otherwise.

## broker_stream

- Sources: `output`, `outputPath` or `inline` only, refused at publication otherwise
  (`BROKER_SOURCE_NOT_INLINE`), so the completion transaction never reads object storage.
- Subscription: an ADR-0025 subscription of kind NATS, KAFKA or SQS (REDIS after 033) with
  `purpose = STEP_OUTPUT` and an `egressCeiling` (`pii` requires the 016 reveal grant). Referencing
  one needs `event:manage` at publication. A subscription also selected for run events is refused
  with `DESTINATION_SUBSCRIPTION_SHARED`.
- In the source step's completion transaction:
  1. Re-read the subscription under lock. If it is disabled, deleted, or its ceiling is below a
     written field's level, the destination step fails in this transaction and its policy applies.
  2. Store the mapped records in `payloads` (run content).
  3. Insert one `webhook_deliveries` row per record, with:
     - `kind = 'STEP_OUTPUT'`;
     - `event_id = uuid_v5(<platform step-output namespace UUID>, <effect key> + "/" + <index>)`;
     - `run_id`, `step_key` and `payload_digest`;
     - a `payload` that holds only the envelope: the existing versioned event shape with type
       `step.output.v1`, the record index and count, and no data.
  The delivery worker inserts the record data from `payloads` when it sends.
- Bounds: a record whose sent body would exceed 65,536 bytes fails the destination step in the
  completion transaction (`DESTINATION_DELIVERY_BOUND_EXCEEDED`); total delivery bytes per completion
  are capped at 4 MiB by default, 16 MiB at most.
- Settlement (D035-053): the delivery worker commits only delivery state plus an outbox hint
  `destination.delivery.settled`. The scheduler shard owner completes the waiting destination step
  once every delivery of its `step_key` is `DELIVERED`, or applies the failure policy when one is
  `DEAD`, by reading the delivery rows; a reconciliation scan covers lost hints.
- Retry and redrive (D035-068): a broker destination step has no activity to lease. Under `hold`,
  `RETRY_STEP` on it returns it to `WAITING(DELIVERY)`: the shard owner re-inserts any missing delivery
  row idempotently (unique on subscription and `event_id`, from the retained payload digest), the
  operator redrives dead deliveries with the existing ADR-0025 action, and T11 settles the step again
  once every delivery is terminal. Under `continue` the step stays `SKIPPED` and the redrive is the
  ADR-0025 delivery redrive alone; no step changes state.
