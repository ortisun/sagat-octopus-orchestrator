# Pull exporter protocol (draft excerpt of `worker.proto`, feature 020)

Additive service in `specs/001-platform-foundation/contracts/worker.proto`, package
`sagat_octopus_orchestrator.v1`, at the next free additive minor of the register in `sdks/SPEC.md` §9
when this contract merges (*corrected 2026-11-24:* 1.7 is feature 041's, 1.8 delivery acknowledgement's,
1.9 design 045's, so 1.10 or later at the time of writing; G1097, ADR-0015, research Decision 020-009).
Older workers and SDKs are unaffected: they never call it. Batch leases are not task leases: delivery
acknowledgement (ADR-0099) and lease withdrawal (ADR-0098) do not apply to them. Merged by T020-049.
Not implemented.

```proto
// Added in 1.<next> (additive, feature 020, ADR-0053). Only an identity with the machine role
// `analytics-exporter` (PULL sinks) or `database-runner` (connection-backed sinks and CONNECTION
// archives), the permission analytics:sink:export and a current binding to the sink may call it.
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
  // POSTGRESQL, CLICKHOUSE, OBJECT_STORAGE and ICEBERG sinks and CONNECTION archives only, for a bound
  // database-runner identity holding a current lease of one of the sink's batches (or of the archival
  // job): the 017 connection pinned by the sink or policy revision, the effective limits and the
  // credentials, through the same two-phase OpenBao resolution as ResolveDatabase (current state and
  // ACLs checked before and after provider I/O, no lock held across it), fenced by the lease token and
  // generation instead of a step attempt. Audited as secret.resolved. The response carries no provider
  // routing, provider token or platform key.
  rpc ResolveSinkConnection(BatchLeaseRef) returns (ResolvedSinkDestination);
  // CONNECTION archives (feature 020, retention): leases the export of one archival job. The stream is
  // the sagat-run-archive-v1 document in chunks, then a trailer with its size and SHA-256. The ack
  // carries the size and SHA-256 the runner read back from the object it wrote create-only; the job is
  // VERIFIED only when both equal the trailer.
  rpc LeaseArchive(LeaseArchiveRequest) returns (stream LeasedBatchFrame);
  rpc AckArchive(AckArchiveRequest) returns (BatchLeaseResponse);
}
message LeaseArchiveRequest { string session_id = 1; string session_token = 2; string namespace = 3; }
message AckArchiveRequest { BatchLeaseRef lease = 1; string request_id = 2; int64 object_size = 3; string object_sha256 = 4; }

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
  // The existing message, unchanged: `postgres` (1.6) or feature 035's `iceberg`, `object_storage` or
  // `clickhouse` (fields 13 to 15), exactly one set; `statement`, `parameters` and `operation` unset.
  ResolvedDatabase connection = 1;
  BatchKind batch_kind = 2;                     // RECORDS, PROBE or ARCHIVE
  string table_prefix = 3;                      // or the object prefix; from the sink or policy revision
  string target_namespace = 4;                  // POSTGRESQL schema or ICEBERG namespace; empty otherwise
}
enum BatchKind { BATCH_KIND_UNSPECIFIED = 0; BATCH_KIND_RECORDS = 1; BATCH_KIND_PROBE = 2; BATCH_KIND_ARCHIVE = 3; }
```

Rules:

- Lease, renew, ack and nack are each one PostgreSQL transaction that reads the batch row `FOR UPDATE`
  and the sink row `FOR SHARE`; lease expiry is sampled on the database clock after the lock (the
  014/017 post-lock sampling rule). An archive lease locks the `archival_jobs` row instead; its
  `batch_id` is `archive:<regionEpoch>:<runId>` and it is renewed with `RenewBatchLease`.
- An ack must carry the `content_sha256` of the trailer; a mismatch is `INVALID_ARGUMENT` and commits
  nothing (the exporter received a corrupted stream).
- `reason_code` is a closed set (`SCHEMA_MISMATCH`, `AUTH_FAILED`, `DESTINATION_FULL`, `OBJECT_CONFLICT`,
  `CONNECTION_UNUSABLE`, `OTHER`); free text from the exporter is never stored or logged.
- Unbinding a subject or removing its role makes its next call `PERMISSION_DENIED NOT_BOUND`; a lease it
  holds simply expires.
- Roles: `PULL` batches are leasable by `analytics-exporter` subjects; `POSTGRESQL` and `CLICKHOUSE`
  batches only by `database-runner` subjects on the `database` profile, `OBJECT_STORAGE` and `ICEBERG`
  batches and `CONNECTION` archives only by `database-runner` subjects on the `lake` profile (each then
  calls `ResolveSinkConnection`); each needs a binding to the sink (an archive needs a binding to the
  namespace's retention destination). *Corrected 2026-11-24:* every kind reuses the existing
  `ResolvedDatabase` (feature 035 added its `iceberg`, `object_storage` and `clickhouse` fields), so the
  draft's `ResolvedClickHouse` is dropped and this service adds no credential-bearing message.
- Six SDK helpers (`AnalyticsExporter` in TypeScript, Go, Python, C#, Java and Rust) implement register,
  the credit loop, renew at one third of the lease, ack/nack and backoff; they never retry an ack with a
  new request id. They lease `PULL` batches only. Shared scenario:
  `tests/contract/sdk-conformance/scenarios/analytics-exporter.json`.
