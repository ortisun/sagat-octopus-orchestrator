# Worker protocol excerpt — `AppendStreamEvents` (draft)

DRAFT, not implemented. Merged into `specs/001-platform-foundation/contracts/worker.proto` and
`docs/architecture/worker-runtime-protocol.md` by T026-007. The rpc is offered in the next additive
minor after 1.6, proposed as **1.7**; drafts 019 and 025 may need a minor as well, so the final number
is assigned when the drafts are integrated (G1333). Every older minor keeps its meaning (ADR-0015).

```proto
service WorkerService {
  // ... existing rpcs unchanged ...

  // Added in 1.7 (proposed). Appends Workflow Stream events for the leased attempt. Requires the
  // current lease (task, run, step, instance, attempt, generation, token) on the database clock,
  // a node that declares every channel in `emits`, and events valid against the pinned channel
  // schema. A batch commits atomically: all its new events or none. Replaying a batch returns the
  // stored offsets (already_applied = true per event).
  rpc AppendStreamEvents(AppendStreamEventsRequest) returns (AppendStreamEventsResponse);
}

message StreamEventInput {
  string channel = 1;            // ^[a-z][a-z0-9_.-]{0,62}$, declared in the version and in emits
  int64 publisher_seq = 2;       // >= 0, chosen by the publisher, unique per attempt and generation
  string data_json = 3;          // canonical JSON object, at most 16 KiB (64 KiB hard)
}

message AppendStreamEventsRequest {
  string task_id = 1; string run_id = 2; string step_id = 3;
  int32 attempt = 4; int64 generation = 5; string lease_token = 6;
  string traceparent = 7;
  ExecutionInstanceIdentity instance = 8;
  repeated StreamEventInput events = 9;   // 1..64 (256 hard), total at most 64 KiB (256 KiB hard)
}

message AppendedStreamEvent {
  int64 publisher_seq = 1;
  int64 offset = 2;              // per-run, gap-free, starting at 1
  bool already_applied = 3;      // true when this publisher_seq was stored before with the same data
}

message AppendStreamEventsResponse {
  repeated AppendedStreamEvent events = 1;  // same order as the request
  int32 retry_after_ms = 2;                 // > 0 only with RESOURCE_EXHAUSTED (rate)
}
```

## Refusals (gRPC status + `x-sagat-error` trailer, existing codes)

| Situation | Status | `x-sagat-error` / reason |
|---|---|---|
| Lease expired, superseded, step already completed or failed, wrong token or generation | `ABORTED` | `STALE_ATTEMPT` (stop publishing; nothing stored) |
| Same `publisher_seq` with different data | `ABORTED` | `IDEMPOTENCY_KEY_REUSED` |
| Channel undeclared, not in `emits`, data not an object, schema violation, oversized event or batch | `INVALID_ARGUMENT` | details reason `STREAM_EVENT_INVALID` with the event index and a JSON pointer, never a value |
| A value the step received as a secret appears in the data | `INVALID_ARGUMENT` | `SECRET_IN_OUTPUT` (same scanner as completion) |
| Channel budget, author pool, reserved pool, per-run hard ceiling or tenant content quota | `RESOURCE_EXHAUSTED` | reason `STREAM_LIMIT` with `scope` = `channel`, `author_pool`, `reserved_pool`, `run` or `tenant_content` (not retryable) |
| Per-attempt append rate | `RESOURCE_EXHAUSTED` | reason `STREAM_RATE` with `retry_after_ms` |
| Worker negotiated a minor below the one that defines the rpc | `UNIMPLEMENTED` from older servers; `FAILED_PRECONDITION` + `UNSUPPORTED_PROTOCOL` from newer servers | |
| Identity not bound to the activity, tenant mismatch | `PERMISSION_DENIED` | `NOT_BOUND` (indistinguishable from stale for another tenant's run) |

A refusal never fails the step. The SDK surfaces it to the step code, which decides.

## Server checks, in order

1. Bounded request size and event count before any database access (the existing gRPC limits layer).
2. One transaction: lock the step lease row `FOR SHARE`; sample the database clock after the lock;
   compare token hash, generation, attempt, expiry and session ownership as `Heartbeat` does; read the
   pinned version's channel declarations and the node's `emits`.
3. Validate every event against its channel schema with the cached compiled validator; scan for the
   attempt's resolved secret values.
4. Lock `run_stream_heads` `FOR UPDATE`; per event: existing deduplication key → stored offset (or
   conflict on a different `data_sha256`); else check the channel budget, its pool (author or reserved
   `sagat.`) and the per-run hard ceiling, insert the event with its data inline (never in `payloads`),
   update the channel usage and pool counters and advance the head; check the per-attempt rate.
5. Commit; the deferred ADR-0027 trigger checks `tenant_content_bytes`. A commit failure stores nothing.

The rpc never locks the run row and never writes the journal, the outbox or `telemetry_pending`.

## SDK behaviour (all five, `sdks/SPEC.md` §6 addition)

- `ctx.stream(channel).publish(data)` validates the channel name locally, assigns the next
  `publisher_seq` for the attempt, and buffers; a background flusher sends batches every 100 ms or at
  64 events, whichever comes first; `flush()` is a barrier that returns when every buffered event was
  acknowledged or refused.
- The buffer is bounded (default 1,024 events or 1 MiB); a full buffer makes `publish` return or raise
  `STREAM_BUFFER_FULL` (local, not retryable) rather than block the step forever.
- Transport errors (`UNAVAILABLE`) are retried with the same batch and `publisher_seq` values until the
  lease deadline; `STALE_ATTEMPT` stops the flusher and drops the buffer (the attempt is over).
- Completing or failing the step flushes first (bounded by the lease deadline); events not flushed
  before completion are reported to the step code as not published, never silently dropped.
- Scripts use feature 023's reserved, capability-gated `stream.append` host call (shim protocol of
  feature 023, coordinator decision C17); the supervisor holds the
  lease and calls this rpc. Managed executors (017, 025) call it under their own lease.
