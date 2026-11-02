# Errors, failure codes, journal events, alert kinds and the task token (draft)

No new `Error.code` is added (D1071): every condition below refines an existing code of
`sdks/error-catalog.json` with a stable `details.reason` (REST) or `x-sagat-error` trailer value (gRPC). SDKs expose
it as `SagatError.reason`; the catalog's remediation for the code applies, refined by the reason.

## REST and gRPC reasons

| Code | Reason | Where |
|---|---|---|
| `VALIDATION_FAILED` | `DEPLOYMENT_OVERLAP`, `ACTIVITY_PUSHED`, `RESERVED_ACTIVITY`, `PATTERN_IN_USE`, `MISSING_CAPABILITIES` | deployment create/update (catalog fence) |
| `VALIDATION_FAILED` | `MISSING_CAPABILITIES` (+ `details.activities`), `VERSION_HAS_NO_POLLERS`, `RAMP_EQUALS_CURRENT` | routing change without `acceptGaps` |
| `VALIDATION_FAILED` | `ACTIVITY_OUTSIDE_DEPLOYMENT`, `DEPLOYMENT_VERSION_MISMATCH` (gRPC `INVALID_ARGUMENT`) | registration |
| `FORBIDDEN` | `VERSION_RETIRED`, `BINDING_DOES_NOT_COVER_DEPLOYMENT` (gRPC `PERMISSION_DENIED`) | registration of a retired build; version creation by an identity whose binding does not cover every pattern |
| `CONFLICT` | `ROUTING_REVISION_CONFLICT`, `VERSION_NOT_DRAINED`, `VERSION_IS_ROUTED`, `DEPLOYMENT_IN_USE` | routing, retirement, delete |
| `CONFLICT` | `STALE_ATTEMPT` | task heartbeat/completion whose token secret matched an **ended** attempt of the step (REST) |
| gRPC `ABORTED` | `STALE_ATTEMPT` with `x-sagat-error` detail `DETACHED` or `PUSH_HELD` | any lease-token worker call (`Heartbeat`, `CompleteStep`, `FailStep`, `ResolveSecrets`, `ResolveDatabase`, `ReadBlob`, `WriteBlob`, `DetachStep`) on a lease whose holder is `detached` or `push` |
| `CONFLICT` | `IDEMPOTENCY_KEY_REUSED`, `RUN_NOT_MUTABLE` | task completion; repin of a terminal run |
| `CONFLICT` | `NOT_DETACHED` | token secret matched the **current** lease, but a worker session holds it, or the push invoker holds it for an attempt not allowed to complete asynchronously |
| `NOT_FOUND` | (none) | any token whose secret matches neither the current lease nor a retained ended lease of that step (forged, malformed, foreign, unknown); foreign deployment, endpoint, policy |
| `FORBIDDEN` | (none) | task API caller without `task:complete`, or a machine identity whose binding does not cover the step's activity (checked after the secret) |
| `VALIDATION_FAILED` | `ASYNC_COMPLETION_NOT_DECLARED` (gRPC `INVALID_ARGUMENT`) | Detach |
| `VALIDATION_FAILED` | `DISPATCH_EXPRESSION`, `UNSUPPORTED_CAPABILITY`, `PRIORITY_OUT_OF_RANGE` | admission, publication |
| `VALIDATION_FAILED` | `PUSH_DESTINATION_DENIED`, `ACTIVITY_IN_DEPLOYMENT`, `ACTIVITY_ALREADY_PUSHED`, `AUDIENCE_NOT_TARGET`, `REGION_INVALID` | push endpoint save |
| `VALIDATION_FAILED` | `DISPATCH_KEY_LEVEL` (an `{expr}` fairness key above `internal`); 031's `ENCODING_READ_BY_FAIRNESS_KEY` (it reads a 031 encoded location) and `ENCODING_NOT_PRODUCIBLE` (`heartbeatSchema` of a `script` declares encoding) | publication; codes owned by 031 (`specs/031-client-side-encryption/contracts/encoded-field-registry.md`) |
| heartbeat `details_status` / REST `detailsStatus` | 031's `ENCODING_REQUIRED` | checkpoint of an attempt with a non-empty payload plan and no encoded `heartbeatSchema` of `activity`; the lease renewal still commits |
| `VALIDATION_FAILED` | `WARM_RESIDUE_NOT_ACKNOWLEDGED` | warm policy |
| `VALIDATION_FAILED` | `SECRET_IN_OUTPUT` | external completion output containing a handed secret value |
| `QUOTA_EXCEEDED` | resource name (`worker_deployments` (per namespace; also the 100 non-retired / 1,000 total versions per deployment), `push_endpoints`, `push_concurrency`, `tenant_push_concurrency`, `task_tokens`, `script_warm_containers`, `tenant_script_warm_containers`) | admission |

`STALE_ATTEMPT` and `NOT_DETACHED` over REST return `409` only after the token's secret matched a lease of that step
(current or retained ended), so they reveal nothing to a caller who does not hold the real token; every other token
is `404`, whatever the run, attempt or generation it names. Receipts (same key and body → original answer) are keyed
by tenant, principal, task and key, and are read only after that check.

## Durable step failure codes (journal `failure_code`)

`DETACHED_TIMEOUT` (retryable per policy), `PUSH_UNAVAILABLE` (retryable), `PUSH_FUNCTION_ERROR` (retryable),
`PUSH_CREDENTIAL_UNAVAILABLE` (retryable: the revision's secret binding no longer resolves for its `boundBy`),
`PUSH_REJECTED`, `PUSH_ASYNC_NOT_ALLOWED`, `PUSH_RESPONSE_INVALID`, `PUSH_DESTINATION_DENIED` (non-retryable).
`OUTCOME_UNKNOWN` keeps its reserved meaning (feature 003). A code a function or external caller sends that is not
a code is recorded as `WORKER_ERROR`, as today.

## Journal events (additive)

Detail keys are snake_case like every existing journal event (`crates/infra-postgres/src/event_codec.rs`; coordinator
decision C16). Each event type declares its reset rule in 018's reset-rule registry
(`specs/018-schedules-bulk-operations/contracts/journal-and-events.md`, coordinator decision C1); 018 refuses a reset
point whose prefix contains an event type without a declared rule.

| Event | Detail (no values) | Written by | Reset rule (018 registry) |
|---|---|---|---|
| `run.started` | + `dispatch {priority, fairness_key_digest, fairness_weight, versioning_behavior}` | admission | `COPY` (the new run keeps the source's dispatch options; its fairness key is re-read from the source run row) |
| `run.deployment_pinned` | `deployment`, `build_id`, `source` | claim, admission (child), handoff | `REMATERIALIZE`: the reset transaction re-creates the `run_deployment_pins` row with the same version under the shared catalog and deployment fences; if that version is `RETIRED`, no pin is re-created and the new run pins at its first claim from the current routing (journaled with `source: reset`) |
| `run.deployment_repinned` | `deployment`, `from`, `to`, `actor`, `reason` (bounded) | repin, retirement | `REMATERIALIZE` (folded into the re-created pin's final version, as above) |
| `step.leased` | + `dispatch_mode`, `deployment_build_id`, `push_endpoint {name, revision}`, `checkpoint {attempt, digest, size}` | claim | `COPY` (added detail of an existing event) |
| `step.detached` | `attempt`, `generation`, `expires_at`, `max_until` | Detach, push `202` | `COPY` as history; a detached attempt still open at the reset point is an open attempt for 018 and is never carried into the new run (its token stays bound to the source run) |
| `step.failed`, lease expiry, timeout | + `checkpoint {digest, size}` | existing writers | `COPY` (added detail) |

Checkpoint rows (`step_checkpoints`) are not journal events and are never copied by a reset: a new run never inherits
checkpoints (FR-019-016).

The journal listing and the runtime graph stay payload-free (ADR-0028): the fairness key appears only as its digest,
an HMAC-SHA-256 under a per-tenant key (never a plain hash of a low-entropy value); the raw key is retained content
on the run row, classified (`public` or `internal`), and shown only to `payload:read-internal` holders.

## Alert kinds (AsyncAPI `ops.alert.*` and `Alert.kind`, enumeration grows)

`VERSION_DRAIN_STALLED` (a draining version has open pins and no live session for longer than the threshold),
`PUSH_ENDPOINT_FAILING` (breaker open or `DENIED`), `UNVERSIONED_POLLERS_IDLE` (unversioned sessions bound to
activities routed to a build). Subjects are a deployment name, an endpoint name or an activity name (≤120
characters), never a run or payload. Receivers must accept unknown kinds (`docs/event-schema-evolution.md`).

## Task token format

`sgtt1.` followed by base64url (no padding) of a fixed binary layout:

| Bytes | Field |
|---|---|
| 1 | layout version (`1`) |
| 16 | namespace id |
| 16 | run id |
| 32 | SHA-256 of the instance identity (path and definition node) |
| 4 | attempt (big endian) |
| 8 | generation (big endian) |
| 32 | lease token |

The server decodes it, takes tenant and namespace from the authenticated principal and the path, finds the step by
run and instance digest **within that tenant and namespace**, and first compares SHA-256(lease token), in constant
time, with the current lease hash and the retained hashes in `step_ended_leases` (written only for leases that were
detached or push-held). Any mismatch, a tenant/namespace mismatch or a missing step is `NOT_FOUND`. Only after a
match are attempt and generation compared and receipts read (see the table above). The token is a credential: it is never logged, put in a URL, a span, an
audit record or an error; `crates/telemetry/src/redact.rs` scrubs the `sgtt1.` prefix.
