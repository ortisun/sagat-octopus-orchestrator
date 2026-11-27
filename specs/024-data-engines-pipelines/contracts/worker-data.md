# Draft contract — worker protocol additions (FR-024-004, 019, 021, 023, 038; Decision 024-014)

Excerpt for `specs/001-platform-foundation/contracts/worker.proto`, merged by T024-008. All
additions are optional fields or new messages/RPCs of **one additive minor: the next free minor of
the register in `sdks/SPEC.md` section 9 when this contract merges** (1.7 is feature 041's step logs,
Owner decision 2026-11-15-8; this draft writes it `1.N`). The merge adds the register row naming
feature 024. A worker that registers with a lower minor never receives a task of a new driver or a
data task; the candidate query filters such tasks before its cap and the claim transaction repeats
the check (ADR-0023 run-input precedent). `worker.baseline.json` is regenerated only after reading
its diff. Field numbers below were checked against the canonical
`specs/001-platform-foundation/contracts/worker.proto` on 2026-11-27 (after the correctness review
found that `CompleteStepRequest` field 13 is already feature 041's `StepLogCarry step_logs`, protocol
1.7): `ResolvedDatabase` uses 1 to 15, `WriteBlobHeader` 1 to 10, `CompleteStepRequest` and
`FailStepRequest` 1 to 13. They are confirmed again at merge by a contract test that reads the
canonical file and fails when a number this draft names is already taken in its message (Task
024-008); a field taken by another feature in between moves this draft's number, never the other's.

## Remote engines on `ResolveDatabase`

`ResolveDatabase(ResolveSecretsRequest) returns (ResolvedDatabase)` is unchanged. Exactly one of
the connection fields is set; field 3 keeps PostgreSQL and fields 13 to 15 keep feature 035's lake
connections. Only a runner bound to the driver's profile (`database` or `warehouse`) is answered;
the credential is Principle V's second declared set.

```proto
message ResolvedDatabase {
  // fields 1..12 unchanged (017); 13 iceberg, 14 object_storage, 15 clickhouse unchanged (035)
  MysqlConnection mysql = 16;          // also used for mariadb, see driver
  SqlServerConnection sqlserver = 17;
  BigQueryConnection bigquery = 18;
  SnowflakeConnection snowflake = 19;
  DatabaseDriver driver = 20;          // UNSPECIFIED means postgresql (1.6 behaviour)
  // Opaque, deterministic identity for warehouse jobs/statements of this attempt.
  string request_identity = 21;
  uint64 maximum_bytes_billed = 22;    // bigquery only; 0 is never sent
}
enum DatabaseDriver {
  DATABASE_DRIVER_UNSPECIFIED = 0; DATABASE_DRIVER_POSTGRESQL = 1; DATABASE_DRIVER_MYSQL = 2;
  DATABASE_DRIVER_MARIADB = 3; DATABASE_DRIVER_SQLSERVER = 4; DATABASE_DRIVER_BIGQUERY = 5;
  DATABASE_DRIVER_SNOWFLAKE = 6;
}
message MysqlConnection { string host = 1; uint32 port = 2; string database = 3; DatabaseTlsMode tls = 4; string ca_pem = 5; EgressCeiling egress_ceiling = 6; }
message SqlServerConnection { string host = 1; uint32 port = 2; string database = 3; SqlServerEncryption encryption = 4; string ca_pem = 5; string host_name_in_certificate = 6; EgressCeiling egress_ceiling = 7; }
enum SqlServerEncryption { SQLSERVER_ENCRYPTION_UNSPECIFIED = 0; SQLSERVER_ENCRYPTION_STRICT = 1; SQLSERVER_ENCRYPTION_VERIFY_FULL = 2; SQLSERVER_ENCRYPTION_DEVELOPMENT_PLAINTEXT = 3; }
message BigQueryConnection { string project_id = 1; string location = 2; EgressCeiling egress_ceiling = 3; }
message SnowflakeConnection { string account = 1; bool privatelink = 2; string warehouse = 3; string role = 4; string database = 5; string schema = 6; EgressCeiling egress_ceiling = 7; }
```

`EgressCeiling` and `DatabaseTlsMode` are the existing enums. The `credential` field (8) carries the
codec's closed JSON for the auth mode; endpoints are never in it. `DatabaseFailure` codes gain
`DATABASE_COST_LIMIT`, also a `worker`-origin entry of `sdks/error-catalog.json`.

## Data tasks

Claimed only by the data runner (`apps/data-runner`, role `data-runner`, binding profile `data`),
as the managed kinds `DataTransform` and `DataQuery` (activities `sagat.data.transform`,
`sagat.data.query`). Nothing here is a secret: `ResolveData` carries no credential, no object key and
no provider routing, so no Principle V disclosure set applies.

```proto
service WorkerService {
  // Added in 1.N. Only a current data-runner with data:execute, bound with profile data, that owns
  // this managed data lease may resolve it. Returns the pinned inputs (exact parts), the pinned
  // declarations of the outputs, audits and effective limits.
  rpc ResolveData(ResolveSecretsRequest) returns (ResolvedData);
}

message ResolvedData {
  string namespace_id = 1;
  string tenant_id = 2;                  // telemetry correlation only, never a metric label
  DataEngine engine = 3;
  repeated DataInput inputs = 4;
  repeated DataOutput outputs = 5;       // empty for a dataQuery
  DataLimits limits = 6;
  int64 lease_expires_at_epoch_ms = 7;
  // polars only: digest of the signed design 046 artifact (bundle version 5) and of its layer
  // record for this runner's target; the data runner fetches and verifies the artifact as a script
  // runner does, and the data mode starts the attempt only with a layer it materialized and verified
  // (Gap 024-002 designs who builds the layer).
  string script_artifact_digest = 8;
  string query_statement = 9;            // dataQuery only
  repeated DatabaseValue query_parameters = 10;
  DatabaseLimits query_limits = 11;
  string script_layer_digest = 12;       // polars only
}
enum DataEngine { DATA_ENGINE_UNSPECIFIED = 0; DATA_ENGINE_DUCKDB = 1; DATA_ENGINE_POLARS = 2; }
// The level each input counts at for a dataQuery output is the server's (step_data_inputs.level)
// and is not sent: the data runner never decides a classification.
message DataInput {
  string alias = 1; string asset_name = 2; string snapshot_id = 3;
  repeated DataPart parts = 4;           // snapshot parts, or one step-output Parquet blob
  string step_node = 5;                  // set for a step source (ingestion)
  bytes inline_result_json = 6;          // a step source whose 017 database result is inline (<= 4 MiB)
}
message DataPart { string blob_id = 1; int64 size = 2; string sha256 = 3; int64 rows = 4; }
message DataOutput {
  uint32 index = 1; string asset_name = 2; uint32 declaration_revision = 3;
  string statement = 4;                  // duckdb only
  repeated DatabaseValue parameters = 5; // evaluated by the server
  repeated DataColumn columns = 6;       // pinned contract
  repeated DataAuditCheck audits = 7;
  int64 max_bytes = 8; uint32 max_parts = 9;
}
message DataColumn { string name = 1; string type = 2; bool nullable = 3; uint32 precision = 4; uint32 scale = 5; }
message DataAuditCheck { string name = 1; string kind = 2; string statement = 3; repeated string columns = 4; repeated string accepted = 5; int64 min_rows = 6; int64 max_rows = 7; }
message DataLimits {
  int64 input_bytes = 1; int64 memory_bytes = 2; uint32 cpus = 3; int64 spill_bytes = 4;
  uint32 transform_timeout_ms = 5; uint32 audit_timeout_ms = 6;
  DataBudgetClass budget_class = 7;      // the smallest class of the data mode that covers the limits
}
enum DataBudgetClass { DATA_BUDGET_CLASS_UNSPECIFIED = 0; DATA_BUDGET_CLASS_SMALL = 1; DATA_BUDGET_CLASS_LARGE = 2; }
```

The budget class is what the data runner sends to the data-mode sandbox runner (Architecture
decision 0079 rule 5: the request names a class, never a limit); the other limits are what the data
runner and the host program enforce in software below the class budget.

## Reading and writing parts

- `ReadBlob` of a `DATA_SNAPSHOT_PART` blob, or of a step-output Parquet blob named by a step
  source, is allowed only when a `step_lease_blobs` row (feature 035, migration
  `20270111000000_lake_blob_pins`) names it for the calling attempt and generation; the lease
  transaction writes one such row per pinned part (Decision 024-007). A part uploaded by that same
  attempt and generation may also be read (the audit phase reads its own parts back, because the
  transform container's `/out` tmpfs is gone). Any other blob id answers like a missing blob.
  Generic `ReadBlob` by a SQL managed lease stays refused (017). Two pieces are new code, not the
  lake path reused as is: the data pin writer (feature 035's `lake_blob_pin` admits only committed
  `STEP_OUTPUT` blobs of the same run), which admits committed `DATA_SNAPSHOT_PART` parts of the
  pinned snapshots of any run of the namespace (never a part in state `DELETING`), a committed step
  output of the same run, and, in a run created by an 018 reset, the source run's committed step
  output that a restored pin names by id and digest; and a `ManagedKind::DataTransform` /
  `DataQuery` branch of the `ReadBlob` gate (`check_blob_access`), which today admits only
  `ManagedKind::Lake`. Task 024-031 tests both.
- `WriteBlobHeader` gains `BlobPurpose purpose = 11` (`BLOB_PURPOSE_UNSPECIFIED` keeps today's
  step-output meaning) with `BLOB_PURPOSE_DATA_SNAPSHOT_PART`; content type
  `application/vnd.apache.parquet`; the reservation records the exact fence and counts against the
  data pending-upload ceiling and `tenant_data_bytes`. The finalization of such an upload rechecks
  that its fence is still the step's live lease; when the attempt has ended it claims the blob for
  deletion in the same transaction.

## Completion

```proto
message CompleteStepRequest {
  // fields 1..12 unchanged; 13 is StepLogCarry step_logs (1.7, feature 041), which a data runner
  // never sends (Requirement 041-016)
  DataStepResult data_result = 14;      // set only for data transforms
}
message DataStepResult {
  repeated DataOutputResult outputs = 1;
}
message DataOutputResult {
  uint32 index = 1;
  repeated DataPartResult parts = 2;    // blob ids returned by WriteBlob, in order
  repeated DataAuditOutcome audits = 3;
  // The SHA-256 of each part the data runner staged into the audit attempt directory, in order;
  // must equal the committed parts' digests. A data-runner self-check, not proof of what the audit
  // container read (its outcomes rest on the sandbox, Gap 1249).
  repeated string staged_part_sha256 = 4;
}
message DataPartResult { string blob_id = 1; int64 rows = 2; }
message DataAuditOutcome { string name = 1; string outcome = 2; int64 failing = 3; }
```

The engine treats `rows` and audit outcomes as claims: it verifies parts itself (data runtime
contract) and the completion transaction decides `AUDITED`, `QUARANTINED`, `ABANDONED` (a stale
auto-publication that no later node reads) or a failure. When the object store is unavailable while
the engine reads a footer, `CompleteStep` answers `UNAVAILABLE` and changes nothing, and the data
runner sends the same request again with the same idempotency key. The output document a transform
completion stores is the closed, protected record of FR-024-027 (at most 4 KiB), never charged to
content quota. The
idempotency key, fingerprint and receipt replay rules of `CompleteStep` are unchanged; the
fingerprint covers `data_result`. A `dataQuery` completes with inline `output_json` only (at most
256 KiB, no blob output) in the 017 typed-cell shape, validated by the 017 result validator; the
engine stores it as the step's classified output. DuckDB is not a `DatabaseDriver` value. The data
runner never sends a step-log carry (Requirement 041-016; the ingest refuses its role).

## Heartbeats and cancellation

Unchanged. The data runner heartbeats from claim until report; cancellation or lease loss ends the
attempt through the attempt API (`DELETE /v1/attempts/{id}`), after the attempt socket has stopped;
a report after cancellation follows the 017 cancellation-acknowledgement rule and never commits
parts. The cancellation, a `FailStep`, a refused completion and a lease expiry that grants a new
generation each claim the attempt's uncommitted `DATA_SNAPSHOT_PART` uploads for deletion in their
own transaction; a `WriteBlob` of that fence that finishes afterwards is claimed when it finishes.
