# Draft contract — worker protocol additions (FR-024-004, 019, 021, 023, 038; D1271)

Excerpt for `specs/001-platform-foundation/contracts/worker.proto`, merged by T024-008. All
additions are optional fields or new messages/RPCs of one additive minor after 1.6 (written **1.7**
here; the coordinator allocates the final number across the parallel drafts). A worker that
registers with a lower minor never receives a task of a new driver or a data task; the candidate
query filters such tasks before its cap and the claim transaction repeats the check (ADR-0023
run-input precedent). `worker.baseline.json` is regenerated only after reading its diff.

## Remote engines on `ResolveDatabase`

`ResolveDatabase(ResolveSecretsRequest) returns (ResolvedDatabase)` is unchanged. Exactly one of
the connection fields is set; field 3 keeps PostgreSQL.

```proto
message ResolvedDatabase {
  // fields 1..12 unchanged (017)
  MysqlConnection mysql = 13;          // also used for mariadb, see driver
  SqlServerConnection sqlserver = 14;
  BigQueryConnection bigquery = 15;
  SnowflakeConnection snowflake = 16;
  DatabaseDriver driver = 17;          // UNSPECIFIED means postgresql (1.6 behaviour)
  // Opaque, deterministic identity for warehouse jobs/statements of this attempt.
  string request_identity = 18;
  uint64 maximum_bytes_billed = 19;    // bigquery only; 0 is never sent
}
enum DatabaseDriver {
  DATABASE_DRIVER_UNSPECIFIED = 0; DATABASE_DRIVER_POSTGRESQL = 1; DATABASE_DRIVER_MYSQL = 2;
  DATABASE_DRIVER_MARIADB = 3; DATABASE_DRIVER_SQLSERVER = 4; DATABASE_DRIVER_BIGQUERY = 5;
  DATABASE_DRIVER_SNOWFLAKE = 6;
}
message MysqlConnection { string host = 1; uint32 port = 2; string database = 3; DatabaseTlsMode tls = 4; string ca_pem = 5; }
message SqlServerConnection { string host = 1; uint32 port = 2; string database = 3; SqlServerEncryption encryption = 4; string ca_pem = 5; string host_name_in_certificate = 6; }
enum SqlServerEncryption { SQLSERVER_ENCRYPTION_UNSPECIFIED = 0; SQLSERVER_ENCRYPTION_STRICT = 1; SQLSERVER_ENCRYPTION_VERIFY_FULL = 2; SQLSERVER_ENCRYPTION_DEVELOPMENT_PLAINTEXT = 3; }
message BigQueryConnection { string project_id = 1; string location = 2; }
message SnowflakeConnection { string account = 1; bool privatelink = 2; string warehouse = 3; string role = 4; string database = 5; string schema = 6; }
```

The `credential` field (8) carries the codec's closed JSON for the auth mode; endpoints are never
in it. `DatabaseFailure` codes gain `DATABASE_COST_LIMIT`.

## Data tasks

```proto
service WorkerService {
  // Added in 1.7. Only a current runner with data:execute that owns this managed data lease may
  // resolve it. Returns the pinned inputs (exact parts), the pinned declarations of the outputs,
  // audits and effective limits. Contains no credential, no object key and no provider routing.
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
  // polars only: digest of the signed feature-005 artifact; the runner fetches and verifies it
  // through the existing script-runner artifact route, as for any script step.
  string script_artifact_digest = 8;
  string query_statement = 9;            // dataQuery only
  repeated DatabaseValue query_parameters = 10;
  DatabaseLimits query_limits = 11;
}
enum DataEngine { DATA_ENGINE_UNSPECIFIED = 0; DATA_ENGINE_DUCKDB = 1; DATA_ENGINE_POLARS = 2; }
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
}
```

## Reading and writing parts

- `ReadBlob` of a `DATA_SNAPSHOT_PART` blob is allowed only for a part listed in the calling
  attempt's `ResolvedData.inputs` or uploaded by that same attempt and generation (the audit phase
  reads its own parts back, because the transform container's `/out` tmpfs is gone); any other blob
  id answers like a missing blob. Generic `ReadBlob` by a managed database lease stays refused
  (017).
- `WriteBlobHeader` gains `BlobPurpose purpose = 11` (`BLOB_PURPOSE_UNSPECIFIED` keeps today's
  step-output meaning) with `BLOB_PURPOSE_DATA_SNAPSHOT_PART`;
  content type `application/vnd.apache.parquet`; the reservation records the exact fence and
  counts against the data pending-upload ceiling and `tenant_data_bytes`.

## Completion

```proto
message CompleteStepRequest {
  // existing fields unchanged
  DataStepResult data_result = 13;      // set only for data transforms
}
message DataStepResult {
  repeated DataOutputResult outputs = 1;
}
message DataOutputResult {
  uint32 index = 1;
  repeated DataPartResult parts = 2;    // blob ids returned by WriteBlob, in order
  repeated DataAuditOutcome audits = 3;
  repeated string audited_part_sha256 = 4; // must equal the committed parts' digests, in order
}
message DataPartResult { string blob_id = 1; int64 rows = 2; }
message DataAuditOutcome { string name = 1; string outcome = 2; int64 failing = 3; }
```

The engine treats `rows` and audit outcomes as claims: it verifies parts itself (data runtime
contract) and the completion transaction decides `AUDITED`, `QUARANTINED` or a failure. The
idempotency key, fingerprint and receipt replay rules of `CompleteStep` are unchanged; the
fingerprint covers `data_result`. A `dataQuery` completes with inline `output_json` only (at most
256 KiB, no blob output) in the 017 typed-cell shape, validated by the 017 result validator; the
engine stores it as the step's classified output. DuckDB is not a `DatabaseDriver` value.

## Heartbeats and cancellation

Unchanged. The runner heartbeats from claim until report; cancellation or lease loss stops the
containers; a report after cancellation follows the 017 cancellation-acknowledgement rule and never
commits parts.
