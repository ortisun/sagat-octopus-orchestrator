# Blob reference contract (worker protocol 1.3)

This contract defines the implemented step-output slice of FR-002-008. It is additive to worker protocol 1.2: workers that negotiate 1.2 keep sending inline `output_json`, and servers continue accepting it. Protocol 1.3 adds `WriteBlob`, `ReadBlob`, and `CompleteStep.output_blob_ref`. Large run input over REST and `TaskLease.input_blob_ref` are not implemented; `input_blob_ref` stays reserved and empty.

## Descriptor

A workflow carries a blob as ordinary JSON, never as storage credentials or a provider URL:

```json
{
  "$blob": {
    "id": "019b5d6e-0000-7000-8000-000000000001",
    "size": 1048576,
    "sha256": "abababababababababababababababababababababababababababababababab",
    "contentType": "application/json"
  }
}
```

`id` is a UUID, `size` is the measured byte count, `sha256` is 64 lower-case hexadecimal characters, and `contentType` is a normalized media type without parameters. Object keys are server-generated as `tenants/<tenant>/namespaces/<namespace>/blobs/<id>`.

## Authorization and scope

Every blob RPC carries the full task identity and lease capability: task, run, step, attempt, generation, and lease token. The server verifies worker permission, tenant ownership, current worker binding, the exact current attempt and generation, and an unexpired lease before transfer.

A reservation durably binds the blob to that tenant, namespace, run, step, attempt, and generation. Only that exact scope may attach it to a completion. An exact replay of an already successful `CompleteStep` may resolve its `COMMITTED` descriptor after the lease ended so a lost response remains idempotent; a new report still needs the original completion checks.

A current lease may read an `UPLOADED` or `COMMITTED` blob in its tenant and namespace by unguessable id. A different tenant, namespace, or unknown id returns the same `NOT_FOUND` result. Workers never receive object-store credentials, endpoints, bucket names, or presigned URLs.

## WriteBlob

`WriteBlob` is client-streaming. The first and only first message is `WriteBlobHeader`; every later message is a byte chunk of at most 256 KiB. The header may state an expected size and SHA-256. The server counts and hashes bytes while streaming, enforces configured size, quota, concurrency, and duration limits, and refuses a mismatch.

The catalog is reserved as `UPLOADING` before external I/O. It becomes `UPLOADED` only after the backend accepted the full body and the measured values match. The `blob.uploaded` audit record and this state change share one PostgreSQL transaction. A failure after reservation claims the row as `DELETING`; backend deletion is attempted and the row remains durable with retry evidence if deletion fails. A failed multipart abort can leave provider-managed incomplete parts that are not a readable object and cannot be deleted by object key; deployments must expire them with the provider's incomplete-multipart lifecycle rule.

Steps that declare secrets currently cannot use blob output. The call fails before its body reaches object storage because bounded cross-chunk matching of current raw and JSON-escaped secret values is still pending. Inline output scanning does not prove streamed blob scanning.

## CompleteStep

Exactly one of `output_json` and `output_blob_ref` is set. When `output_blob_ref` is present, `output_sha256` is optional; if present it must match the catalog descriptor. The step transaction changes the exact matching blob from `UPLOADED` to `COMMITTED` together with the step state, journal entry, downstream scheduling, and outbox work. PostgreSQL never commits a `COMMITTED` row again; the application idempotency result answers an exact replay.

The payload stored in the journal and passed to later steps is the descriptor above. Blob bytes do not enter the PostgreSQL transaction.

## ReadBlob

`ReadBlob` is server-streaming. The first response is `ReadBlobHeader`; later responses are byte chunks of at most 256 KiB. The server appends `blob.read` before returning the stream. Repeated reads by the same actor, attempt, blob, and reason collapse to one audit event. If auditing fails, no bytes are returned.

The server verifies the stored size and digest while streaming. A missing backend object referenced by the catalog is an internal integrity failure. Changed, truncated, or extended bytes end the stream with `DATA_LOSS` and `x-sagat-error: BLOB_CORRUPT`; a clean end means the declared byte count and SHA-256 matched.

## Cleanup and retry

Cleanup claims expired `UPLOADING`, expired unused `UPLOADED`, or retry-due `DELETING` rows in bounded batches with `FOR UPDATE SKIP LOCKED`. Claiming changes the state to `DELETING`, increments `deletion_attempts`, records the attempt time, and sets `delete_after` before external deletion. A committed row is never eligible.

After the backend confirms idempotent deletion, a previously finalized upload's catalog row is removed in the same transaction as its `blob.deleted` audit. An upload never confirmed as `UPLOADED` instead retains a `DELETING` tombstone with `deletion_confirmed_at`: a delayed PUT can finish even after a successful delete. The tombstone remains scheduled for idempotent deletion reconciliation, cannot be read or committed, and releases its active upload slot only after the first successful deletion and audit commit. Repeated confirmation adds no duplicate audit. Provider failure or process exit retains retry evidence; deletion is never inferred from a failed call.

Unused uploaded-blob retention starts at `uploaded_at`, not reservation time. Cleanup tombstone compaction and committed-blob retention remain separate pending policy decisions; no finite tombstone expiry is assumed to prove an unknown external PUT can never finish.

## Error mapping

| Condition | gRPC status | Stable metadata |
|---|---|---|
| Stale/forged/expired lease or fence | `ABORTED` | `x-sagat-error: STALE_ATTEMPT` |
| Unknown or cross-boundary blob | `NOT_FOUND` | `x-sagat-error: NOT_FOUND` |
| Bad stream, media type, expected measurement, conflicting inline/blob output, declared-secret upload, or feature disabled | `INVALID_ARGUMENT` | typed `x-sagat-error` reason |
| Pending-blob quota | `RESOURCE_EXHAUSTED` | `x-sagat-error: QUOTA_EXCEEDED` |
| Transfer concurrency | `RESOURCE_EXHAUSTED` | `x-sagat-error: RATE_LIMITED`, `retry-after-ms` |
| Transfer/storage temporarily unavailable | `UNAVAILABLE` | `x-sagat-error: UNAVAILABLE` |
| Stored bytes fail size/digest verification | `DATA_LOSS` | `x-sagat-error: BLOB_CORRUPT` |

## Verification boundary

The repository tests memory and filesystem backends and signed requests against an in-process S3-compatible server. Real AWS S3 and Cloudflare R2 tests require credentials, are ignored by default, and have not been claimed as executed. The local S3-compatible server cannot represent multipart metadata for the full tenant/namespace key within its filesystem filename limit, so its 12 MiB signed-request case uses one part; multipart behavior remains part of the unexecuted real-provider checks.
