# Pull exporter protocol (draft excerpt of `worker.proto`, feature 020)

Additive service in `specs/001-platform-foundation/contracts/worker.proto`, package
`sagat_octopus_orchestrator.v1`, at the next free additive minor (1.7 at the time of writing; the number
is assigned at merge, G1097, ADR-0015). Older workers and SDKs are unaffected: they never call it.
Merged by T020-049. Not implemented.

```proto
// Added in 1.7 (additive, feature 020, ADR-0053). Only an identity with the machine role
// `analytics-exporter` (permission analytics:sink:export) and a current binding to the sink may call it.
// A batch is frozen before it is leased: every lease of the same batch returns identical records.
// Delivery is at-least-once: an exporter that crashes after writing and before AckBatch receives the
// same batch again (same batch id, same record keys); its store must deduplicate by record key.
service AnalyticsExportService {
  rpc RegisterExporter(RegisterExporterRequest) returns (RegisterExporterResponse);
  // Finite credit (ADR-0029 pattern): the server leases at most `max_batches` batches and streams at
  // most `max_records` records and `max_bytes` bytes in this call, then closes the stream. An empty
  // window ends with retry_after_ms > 0.
  rpc LeaseBatches(LeaseBatchesRequest) returns (stream LeasedBatchFrame);
  rpc RenewBatchLease(BatchLeaseRef) returns (BatchLeaseResponse);
  // Commits progress exactly when the lease is current: same token, same generation, not expired on the
  // database clock, batch still LEASED, sink ACTIVE with the same revision. Otherwise ABORTED STALE_ATTEMPT.
  // A lost response is retried with the same request id and answered already_applied = true.
  rpc AckBatch(AckBatchRequest) returns (BatchLeaseResponse);
  // A permanent refusal pauses the sink (AUTO_PAUSED SINK_REJECTED); a transient one releases the lease
  // for a later attempt with backoff. Neither skips the batch.
  rpc NackBatch(NackBatchRequest) returns (BatchLeaseResponse);
  // POSTGRESQL, CLICKHOUSE and ICEBERG sinks only, for a bound database-runner identity holding a current lease of
  // one of the sink's batches: the destination settings pinned by the sink revision (the 017 connection
  // for POSTGRESQL; the sink's ClickHouse settings), the effective limits and the credential, through the
  // same two-phase OpenBao resolution as ResolveDatabase (current state and ACLs checked before and after
  // provider I/O, no lock held across it), fenced by the batch lease token and generation instead of a
  // step attempt. Audited as secret.resolved. The response carries no provider routing, provider token
  // or platform key.
  rpc ResolveSinkConnection(BatchLeaseRef) returns (ResolvedSinkDestination);
}

message RegisterExporterRequest {
  string exporter_name = 1;            // bounded label, logged; never a credential
  string namespace = 2;
  repeated string sink_ids = 3;        // each must be bound to this subject
  string protocol_version = 4;         // newest additive minor understood
  string analytics_schema = 5;         // "sagat.analytics.v1"
}
message RegisterExporterResponse { string session_id = 1; string session_token = 2; string accepted_protocol_version = 3; }

message LeaseBatchesRequest {
  string session_id = 1; string session_token = 2;
  string sink_id = 3;
  int32 max_batches = 4;               // 1..8
  int32 max_records = 5;               // 1..10000
  int64 max_bytes = 6;                 // 1 KiB..16 MiB
}
message LeasedBatchFrame {
  oneof frame {
    BatchHeader header = 1;            // first frame of a batch
    RecordChunk records = 2;           // NDJSON of sagat.analytics.v1 records, at most 256 KiB per chunk
    BatchTrailer trailer = 3;          // record_count, byte_count, content_sha256 of the whole batch
  }
  int64 retry_after_ms = 4;
}
message BatchHeader {
  string batch_id = 1;                 // "<sinkId>:<regionEpoch>:<shard>:<batchSeq>", stable across leases
  string lease_token = 2;              // opaque; the server stores only its hash
  int64 generation = 3;                // +1 every time the lease is taken again after expiry
  int64 expires_at_epoch_ms = 4;
  int32 sink_revision = 5;
  string traceparent = 6;
}
message RecordChunk { bytes ndjson = 1; }
message BatchTrailer { int64 record_count = 1; int64 byte_count = 2; string content_sha256 = 3; }
message BatchLeaseRef { string session_id = 1; string session_token = 2; string batch_id = 3; string lease_token = 4; int64 generation = 5; }
message AckBatchRequest { BatchLeaseRef lease = 1; string request_id = 2; string content_sha256 = 3; }
message NackBatchRequest { BatchLeaseRef lease = 1; string request_id = 2; NackKind kind = 3; string reason_code = 4; }
enum NackKind { NACK_KIND_UNSPECIFIED = 0; NACK_TRANSIENT = 1; NACK_PERMANENT = 2; }
message BatchLeaseResponse { bool already_applied = 1; int64 expires_at_epoch_ms = 2; string sink_state = 3; }
message ResolvedSinkDestination {
  oneof destination {
    ResolvedDatabase postgresql = 1;            // the existing 1.6 message, unchanged
    ResolvedClickHouse clickhouse = 2;
  }
  BatchKind batch_kind = 3;                     // RECORDS or PROBE
}
message ResolvedClickHouse { string url = 1; string database = 2; string table_prefix = 3; string username = 4; bytes password = 5; int64 statement_deadline_ms = 6; }
enum BatchKind { BATCH_KIND_UNSPECIFIED = 0; BATCH_KIND_RECORDS = 1; BATCH_KIND_PROBE = 2; }
```

Rules:

- Lease, renew, ack and nack are each one PostgreSQL transaction that reads the batch row `FOR UPDATE`
  and the sink row `FOR SHARE`; lease expiry is sampled on the database clock after the lock (the
  014/017 post-lock sampling rule).
- An ack must carry the `content_sha256` of the trailer; a mismatch is `INVALID_ARGUMENT` and commits
  nothing (the exporter received a corrupted stream).
- `reason_code` is a closed set (`SCHEMA_MISMATCH`, `AUTH_FAILED`, `DESTINATION_FULL`, `OTHER`); free
  text from the exporter is never stored or logged.
- Unbinding a subject or removing its role makes its next call `PERMISSION_DENIED NOT_BOUND`; a lease it
  holds simply expires.
- Roles: `PULL` batches are leasable by `analytics-exporter` subjects; `POSTGRESQL`, `CLICKHOUSE`
  and `ICEBERG` batches only by `database-runner` subjects (which then call `ResolveSinkConnection`); each needs a
  binding to the sink. PostgreSQL reuses the existing 1.6 `ResolvedDatabase`; `ResolvedClickHouse` is the
  only new credential-bearing message (never logged, dropped when the batch ends).
- Five SDK helpers (`AnalyticsExporter` in TypeScript, Go, Python, C#, Java) implement register, the
  credit loop, renew at one third of the lease, ack/nack and backoff; they never retry an ack with a new
  request id. Shared scenario: `tests/contract/sdk-conformance/scenarios/analytics-exporter.json`.
