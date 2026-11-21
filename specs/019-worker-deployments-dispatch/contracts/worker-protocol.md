# Worker protocol minors `1.N`–`1.N+2` — feature 019 (draft excerpt)

The three wire slices take three consecutive minors (D1069): **`1.N`** deployment identity, `RegisterWorkerResponse`
state and `DispatchFacts`; **`1.N+1`** checkpoints (`Checkpoint`, heartbeat and failure details, `DetailsStatus`);
**`1.N+2`** `DetachStep` and `async_completion_allowed`. `1.N` is the next free minor at integration (`1.7` if no other
feature merges one first, G1048). Each minor is a superset of the previous one and of 1.6, and the server accepts it
only when its store declares that slice's path (the `protocol_ceiling` pattern of `crates/application/src/worker.rs`)
and the slice's evidence has passed; otherwise negotiation stays at the highest proven minor. Every addition is a new
field number or rpc; no existing field changes meaning. A worker at 1.0–1.6 is served exactly as today and is
unversioned. A host uses a field only when the accepted minor includes it.

## Additions to `specs/001-platform-foundation/contracts/worker.proto`

```proto
service WorkerService {
  // ... existing rpcs unchanged ...
  // Added in 1.N+2. Detaches the current attempt of a step whose node declares asyncCompletion: the lease
  // expiry becomes the declared detached heartbeat timeout, the session's in-flight slot is released and a
  // task token is returned for heartbeats and completion through REST. Repeating it under the same lease
  // returns the same token with already_applied = true. Errors: PERMISSION_DENIED (NOT_BOUND),
  // ABORTED (STALE_ATTEMPT), INVALID_ARGUMENT (x-sagat-error ASYNC_COMPLETION_NOT_DECLARED),
  // RESOURCE_EXHAUSTED (QUOTA_EXCEEDED: task_tokens).
  rpc DetachStep(DetachStepRequest) returns (DetachStepResponse);
}

// Added in 1.N. Identity of the build serving a deployment.
message WorkerDeploymentIdentity {
  string deployment_name = 1;   // ^[a-z][a-z0-9-]{2,63}$
  string build_id = 2;          // ^[A-Za-z0-9._:+-]{1,128}$
}

message WorkerCapabilities {
  // fields 1-5 unchanged
  WorkerDeploymentIdentity deployment = 6;   // Added in 1.N; absent = unversioned session
}

message RegisterWorkerResponse {
  // fields 1-3 unchanged
  string deployment_version_state = 4;       // Added in 1.N: INACTIVE, RAMPING, CURRENT, DRAINING, DRAINED
}

// Added in 1.N. How this lease was routed and ordered (informational; never authority).
message DispatchFacts {
  int32 priority = 1;                        // 1..5
  string versioning_behavior = 2;            // "pinned", "autoUpgrade" or empty (activity outside deployments)
  string deployment_build_id = 3;            // build that was routed; empty for unversioned
}

// Added in 1.N+1. A checkpoint recorded by an earlier attempt of the same step instance.
message Checkpoint {
  string details_json = 1;                   // canonical JSON, at most the effective heartbeat_detail_bytes
  int32 recorded_by_attempt = 2;
}

message TaskLease {
  // fields 1-18 unchanged
  DispatchFacts dispatch = 19;               // Added in 1.N
  Checkpoint checkpoint = 20;                // Added in 1.N+1: latest checkpoint of earlier attempts, if any
  Checkpoint target_checkpoint = 21;         // Added in 1.N+1: for COMPENSATION/RECONCILIATION, the forward instance's
  bool async_completion_allowed = 22;        // Added in 1.N+2: the node declares asyncCompletion
}

message HeartbeatRequest {
  // fields 1-8 unchanged
  string details_json = 9;                   // Added in 1.N+1: optional checkpoint
  uint64 details_sequence = 10;              // Added in 1.N+1: monotonic per attempt; order key (attempt, epoch, sequence)
}

message HeartbeatResponse {
  // fields 1-2 unchanged
  DetailsStatus details_status = 3;          // Added in 1.N+1
}

enum DetailsStatus {
  DETAILS_STATUS_UNSPECIFIED = 0;            // no details sent, or a server below 1.N+1
  DETAILS_STORED = 1;
  DETAILS_COALESCED = 2;                     // accepted, not stored: within the coalescing interval
  DETAILS_STALE = 3;                         // sequence not newer than the stored one
  DETAILS_REFUSED_SIZE = 4;
  DETAILS_REFUSED_SECRET = 5;                // contains a secret value handed to this attempt
  DETAILS_ENCODING_REQUIRED = 6;             // 031 ENCODING_REQUIRED: non-empty payload plan, no encoded heartbeatSchema
}

message FailStepRequest {
  // fields 1-12 unchanged
  string details_json = 13;                  // Added in 1.N+1: final checkpoint, always stored when valid
}

message DetachStepRequest {
  string task_id = 1; string run_id = 2; string step_id = 3;
  int32 attempt = 4; int64 generation = 5; string lease_token = 6;
  string traceparent = 7;
  ExecutionInstanceIdentity instance = 8;
  string details_json = 9;                   // optional final checkpoint of holder epoch 0
  uint64 details_sequence = 10;              // ordering within epoch 0 (final details take the maximum)
}

message DetachStepResponse {
  string task_token = 1;                     // credential: never log or persist it outside the system it is given to
  int64 detached_expires_at_epoch_ms = 2;
  int64 max_detached_until_epoch_ms = 3;
  bool already_applied = 4;
}
```

## Semantics

- **Registration.** With `deployment` set, the deployment must exist (`NOT_FOUND`), the build must not be retired
  (`PERMISSION_DENIED`, `x-sagat-error: VERSION_RETIRED`), every advertised activity must match the deployment's
  patterns (`INVALID_ARGUMENT`, `ACTIVITY_OUTSIDE_DEPLOYMENT`) and, for an existing version, be within its frozen
  manifest (`DEPLOYMENT_VERSION_MISMATCH`). The first registration of an unknown build creates the version
  `INACTIVE`, only if the identity's binding covers every pattern of the deployment (`PERMISSION_DENIED`,
  `BINDING_DOES_NOT_COVER_DEPLOYMENT`); at most 100 non-retired and 1,000 total versions per deployment
  (`QUOTA_EXCEEDED`). The binding check is unchanged and comes first.
- **Claim.** `PollTasks` and `StreamTasks` return only leases the session is eligible for under the routing at the
  claim's commit (ADR-0051). Order: strict priority, weighted fairness between keys, oldest first. A session of an
  `INACTIVE` version receives nothing.
- **Heartbeat details.** Sent only at `1.N+1` or later. A refusal never fails the renewal; `details_status` says what
  happened. Hosts send details with the next scheduled beat (no extra beats), and send the final details with
  `FailStep` or `DetachStep`.
- **Detach.** Allowed only while the lease is current and the node declares `asyncCompletion`. After it **every**
  call presenting that lease token — `Heartbeat`, `CompleteStep`, `FailStep`, `ResolveSecrets`, `ResolveDatabase`,
  `ReadBlob`, `WriteBlob`, `DetachStep` other than an exact replay — answers `ABORTED`/`STALE_ATTEMPT` with
  `DETACHED` (the host stops beating), and completion goes through REST with the token. Leases held by the push
  invoker are refused the same way (`PUSH_HELD`), so a decoded task token is useless on gRPC. A host never detaches on
  its own: the handler asks for it.
- **Cancellation.** A detached attempt learns cancellation through the REST heartbeat (`cancelRequested`).

## SDK host surface (all five SDKs, idiomatic names)

- Worker option `deployment: {name, buildId}`; `host.deploymentVersionState` after registration.
- Activity context: `ctx.heartbeat(details)` records details for the next beat (latest wins, bounded locally with
  the effective limit, `PAYLOAD_TOO_LARGE` raised before sending); `ctx.checkpoint` returns the previous attempt's
  checkpoint (`{details, recordedByAttempt}`) or none; `ctx.targetCheckpoint` for compensation/reconciliation;
  `ctx.dispatch` exposes the informational `DispatchFacts`.
- `ctx.detach()` returns the task token when `async_completion_allowed`; the handler then returns a sentinel
  (`AsyncPending`) and the host reports nothing further for that task.
- Client: `tasks.heartbeat(token, details?)`, `tasks.complete(token, output, idempotencyKey)`,
  `tasks.fail(token, {code, retryable, safeMessage}, idempotencyKey)`; start options `priority`, `fairnessKey`,
  `fairnessWeight`, `versioningBehavior`, sent only when `PlatformVersion.features` contains `dispatch-options`.
- `verifyPushEnvelope(rawBody, headers, keys, expected{tenantId, namespaceId, endpoint}, now)`; `expected` is
  required (see [push-envelope.md](push-envelope.md)).
- Checkpoint ordering: a host numbers `details_sequence` per attempt from 1; after a detach the external system numbers
  its REST `detailsSequence` from 1 in holder epoch 1 (D1076).
