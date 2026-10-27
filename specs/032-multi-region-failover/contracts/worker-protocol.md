# Worker protocol changes — feature 032 (draft, not implemented)

Canonical file: `specs/001-platform-foundation/contracts/worker.proto` (merged by T032-050). Rules:
ADR-0015 (minor versions negotiated per worker, additive only), `docs/architecture/worker-runtime-protocol.md`.

## What changes

Nothing in the lease, fencing or report semantics (research D1569). Two additions, both additive:

### 1. Error trailers (no new gRPC status)

| gRPC status | `x-sagat-error` | When | Worker action (`sdks/SPEC.md` §6.2, unchanged) |
|---|---|---|---|
| `UNAVAILABLE` | `REGION_PASSIVE` | the engine serves a standby region: `RegisterWorker`, `PollTasks`, `StreamTasks`, and any report (the database is read-only) | wait with backoff, **close the channel and resolve the cell endpoint again** (new, FR-032-025), reconnect |
| `UNAVAILABLE` | `REGION_DRAINING` | the active region is draining: `RegisterWorker`, `PollTasks`, `StreamTasks` only; heartbeats and reports of granted leases are still served | as above |
| `UNAVAILABLE` | `REGION_SUPERSEDED` | the engine saw a higher epoch of its cell at a peer and stopped | as above |
| `UNAVAILABLE` | `DURABILITY_UNCONFIRMED` | `sync_quorum` cell: the engine's durability lease lapsed or its barrier is incomplete; registration, polls and reports are refused | wait with backoff and retry the same call (reports with the same idempotency key) |
| `UNAUTHENTICATED` | (none, unchanged) | the session is not in the authority after a failover | register again (unchanged) |
| `ABORTED` | `STALE_ATTEMPT` (unchanged) | a report for a lease the authority does not hold, including a task id the new authority reissued under another lease token | stop; do not retry the report (unchanged) |

The header comment of `worker.proto` gains the three `UNAVAILABLE` lines. A worker of any earlier minor
already retries `UNAVAILABLE` and re-registers on `UNAUTHENTICATED`; it only lacks the re-resolution,
which matters when its gRPC library keeps a connection to an address the DNS no longer returns.

### 2. Optional fields on `RegisterWorkerResponse`

```proto
message RegisterWorkerResponse {
  string worker_id = 1;
  string session_token = 2;
  string accepted_protocol_version = 3;
  // Added in 1.<next> (feature 032, additive). Empty / 0 on a single-region deployment.
  // Informational: the worker logs them and exposes them in its health output. They are not a fence.
  string cell_id = <n>;       // field numbers assigned at merge time after features 019+ have taken theirs
  string region = <n+1>;      // the region of the engine that registered the session
  int64 region_epoch = <n+2>; // the epoch the engine knew at registration
}
```

The minor version is the next free one when T032-050 merges (protocol 1.6 is the current minor per
feature 017 tasks; feature 019 may take the next). A server never requires these fields from a worker.

### 3. `already_applied` in `sync_quorum` cells

A repeated `CompleteStep`/`FailStep` (or managed report) answered from its receipt with
`already_applied = true` is sent only after the replaying transaction assigned a transaction id and its
commit waited for the quorum (FR-032-009 b). During a partition the worker therefore gets
`UNAVAILABLE` (`COMMIT_OUTCOME_UNKNOWN` or `DURABILITY_UNCONFIRMED`) instead of an acknowledgment the new
authority might not hold. No message changes.

## Conformance

`contracts/sdk-conformance.md` scenarios `region-passive-reconnect`, `region-failover-reregister` and
`region-stale-report` run in all five SDKs against the region-pair harness.
