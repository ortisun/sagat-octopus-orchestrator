# Worker, leasing and fencing protocol — details

## Session and capabilities

`RegisterWorker` authenticates the machine/namespace identity, negotiates `protocol_version`, capabilities (`activity_names`, `languages`, `pools`, `max_inflight`) and returns a session with TTL. A worker does not execute a task that declares a capability it lacks.

## Claim

NATS delivery is a **hint**. The worker calls `PollTasks`/stream, which performs a transactional claim in Postgres; the response carries an opaque `lease_token`, `generation`, `attempt`, `deadline` and `idempotency_key`. A NATS callback does not constitute authority to execute without a valid lease.

## Worker loop

```text
connect → register → request credits → claim one or more tasks
for each task:
  validate protocol/tenant scope, deadline, capabilities
  start heartbeat loop (lease renewal)
  execute Activity or Script in isolated executor with deadline
  upload large result to object storage (hash verified)
  CompleteStep or FailStep with token+generation+idempotency_key
  if STALE_ATTEMPT: ignore persisted result, stop executor, emit metric
  ack local delivery; read next tasks respecting max_inflight
```

## Lease settings

- Proposed default parameters: `lease_ttl=30s`, `heartbeat_interval<=lease_ttl/3`, `max_lease_renewal` per step timeouts.
- Heartbeats do not persist the full journal per heartbeat; they renew the row via a lightweight update with fencing.
- A worker that lost its connection may continue the side effect; a stale token prevents the **internal commit**, but does not undo the side effect. The SDK recommends cancel + idempotency for external APIs.
- Task released again after expiration with `generation+1`. An old response receives `STALE_ATTEMPT`.
- Long polling/backoff avoids busy-looping; gRPC streaming can be added without changing semantics.

## Failing step

- `retryable=true`: consume retry budget and schedule `next_attempt_at` with backoff/jitter.
- `retryable=false`: progression to the onError branch, saga compensation or fail policy.
- `deadline_expired`: best-effort cancel and persisted temporal state; if the external effect is unknown, reconciliation is mandatory for a critical connector.
- `fail` and `complete` with the same repeated key return the previously recorded outcome; different request hash under the same key = conflict.

## Flow control

- Local credit control per worker and global quotas per tenant/queue/connector.
- Expose `max_inflight` and capacity dynamically; the scheduler cannot trust in-memory counters after failover.
- Distribution by capability without a subject per run, for bounded queue cardinality.

## Success flow

```mermaid
sequenceDiagram
  participant API as REST Client
  participant DB as PostgreSQL
  participant SCH as Scheduler
  participant NATS as JetStream
  participant W as Worker
  API->>DB: TX insert Run + journal + ready step + outbox
  DB-->>API: commit / 202
  SCH->>DB: claim READY + lease + event + outbox (TX)
  SCH->>NATS: publish task_ref (outbox)
  NATS-->>W: delivery hint
  W->>SCH: Poll/Claim via Worker API
  SCH-->>W: TaskLease (generation/token)
  W->>W: execute Activity (idempotency key)
  W->>SCH: CompleteStep(token/generation/result)
  SCH->>DB: TX step success + event + next ready + outbox
  DB-->>SCH: commit
  SCH-->>W: Applied
```

## Warning: double grant

The diagram above illustrates claim and delivery in the same chain; the implementation must ensure that the lease is delivered **logically only once**, via one canonical operation. Recommended option: the scheduler only creates the READY/outbox hint, and the worker API performs the READY→LEASED transition under lock. Do not implement a prior claim by the scheduler and a second claim by the worker simultaneously. If opting for pre-assignment, write an ADR and corresponding fencing tests.

### Final V1 decision

**The V1 choice is:** the engine writes `READY` + outbox; NATS wakes the worker; the Worker API grants `LEASED` under row lock/CAS and returns the token. The worker executes, renews the heartbeat and confirms. The reconciler re-notifies READY without an active hint and releases expired leases. The "SCH claim READY" step in the diagram above must be interpreted as scheduler validation, not as a lease grant prior to the worker API.

## As implemented (feature 002)

The V1 decision above is what the engine does. The details that the diagrams leave open are fixed by `specs/001-platform-foundation/contracts/worker.proto`, which the Rust server (`crates/api-grpc`) and every SDK are generated from, and by `specs/002-durable-execution/research.md` (D27, D31, D32, D34).

- **Identity.** Every call carries `authorization: Bearer <jwt>`; the token is verified exactly as for the REST API and the caller must be a member of the tenant with the `worker` role (or `admin`). `RegisterWorker` returns a session token (stored hashed, bound to the identity that opened it, expiring when idle); `PollTasks` needs it. There is one live session per worker name and identity: registering again under the same name replaces the session (the old token stops working, so a worker that reconnects registers again), and an identity holds at most 32 sessions (`RESOURCE_EXHAUSTED` beyond that). `languages` and `pools` take at most 16 entries of 1 to 64 letters, digits or `. _ - : +`.
- **Task identity.** `task_id` is `run/node/generation`. A report names the task, the run, the step id, the attempt, the generation and the lease token; all of them must be current. The lease token is a random 256-bit secret shown once in `TaskLease` and stored only as a SHA-256.
- **Errors.** gRPC status plus a stable code in the `x-sagat-error` trailer: `UNAUTHENTICATED`; `PERMISSION_DENIED` (`FORBIDDEN`); `INVALID_ARGUMENT`; `ABORTED` with `STALE_ATTEMPT` (stop, do not retry) or `IDEMPOTENCY_KEY_REUSED`; `RESOURCE_EXHAUSTED` with `retry-after-ms`; `UNAVAILABLE` (retry with the same key). A report about a run of another tenant is indistinguishable from a stale one.
- **Idempotency.** `CompleteStep` and `FailStep` take a key (8 to 160 printable ASCII). The same key and report replays the original answer with `already_applied = true`; the same key with a different report is refused. `output_sha256`, when sent, is checked against the bytes received. `output_blob_ref` is refused until a blob store exists (G19); an empty `output_json` means "no value".
- **Sizes.** An output or signal payload is at most 256 KiB, and a run retains at most 4 MiB of them (the completion that would exceed it fails the step with `RUN_DATA_LIMIT`, not retried). A `PollTasks` response stops growing at 8 MiB, so it is at most 8 MiB plus the last task, and a task carries the outputs its run retains: **a client must accept messages of at least 16 MiB** (the default of most gRPC libraries is 4 MiB) or it will fail on a large run. What does not fit stays claimable.
- **Cancellation.** There is no server push: a cancellation reaches a running attempt through the next `Heartbeat` (`cancel_requested`), and the worker then reports the attempt as stopped.
- **Hints.** NATS subjects are `sagat.v1.task.ready.<tenant>.<activity>` and `sagat.v1.run.finished.<tenant>`; messages carry identifiers only (run, node, activity, trace id), never payloads or tokens. A worker that receives one still has to claim through `PollTasks`.
