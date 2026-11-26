# Feature 022 — errors, refusal reasons and failure codes (draft)

No new `Error.code` value is added (the closed set in `sdks/error-catalog.json` and the canonical
OpenAPI `Error` schema stays). Refinements are `details.reason` values, `DefinitionProblem.code` values
for publication/validation, and durable step/run failure codes. Messages are safe: they never contain a
key value, a cached value, a pin, a payload or a secret. Every row needs a test (task T022-012).

## REST refusals (`Error.code` + `details.reason`)

| Situation | HTTP | `code` | `details.reason` | Retryable |
|---|---:|---|---|---|
| Test run in a `prod` namespace | 403 | `FORBIDDEN` | `TEST_RUN_PRODUCTION_NAMESPACE` | no |
| Draft moved since `draftRevision` | 409 | `CONFLICT` | `DRAFT_REVISION_CHANGED` | no |
| `startAt` not a root node outside scopes | 422 | `VALIDATION_FAILED` | `INVALID_START_NODE` | no |
| Reachable node needs an upstream pin | 422 | `VALIDATION_FAILED` | `MISSING_PIN` (`details.node`) | no |
| Pin on a non-mockable kind, unknown node, blob or oversized value | 422 | `VALIDATION_FAILED` | `INVALID_PIN` (`details.node`) | no |
| Unpinned 021 `resourceOperation`, 024 data, 025 agent or 030 `serviceCall` node reachable from `startAt` | 422 | `VALIDATION_FAILED` | `TEST_RUN_UNMOCKED_EFFECT` (`details.node`) | no |
| `fromRun` source not disclosable to the caller (secret field, missing grant) | 403 | `FORBIDDEN` | `PIN_NOT_DISCLOSABLE` | no |
| `fromRun` source missing, foreign, not succeeded or another workflow | 404 | `NOT_FOUND` | — (opaque) | no |
| Standalone node with `compensateWith`, `next`, `onError` or several nodes | 422 | `VALIDATION_FAILED` | `INVALID_STANDALONE_STEP` | no |
| Same idempotency key, different body (standalone, test, invalidation, redrive) | 409 | `CONFLICT` | `IDEMPOTENCY_KEY_REUSED` | no |
| Per-key waiter queue full (synchronous start) | 429 | `QUOTA_EXCEEDED` | `CONCURRENCY_QUEUE_FULL` | no |
| Live semaphore keys, namespace `queued_runs`, cache entries, test runs, pins over a ceiling | 429 | `QUOTA_EXCEEDED` | `FLOW_CONTROL_LIMIT` (`details.limit`) | no |
| A start that declares both gates is refused by the key that is full: its permit queue `CONCURRENCY_QUEUE_FULL`, its slot queue `RATE_LIMIT_QUEUE_FULL`; a full namespace `queued_runs` is `RATE_LIMIT_QUEUE_FULL` for any start that declares `rateLimit` (rate first, FR-022-052; D022-065) | 429 | `QUOTA_EXCEEDED` | as stated | no |
| A start whose effective rate limit is zero (`rate_limit_per_second`, `_per_minute` or `_per_hour` of its window) | 429 | `QUOTA_EXCEEDED` | `FLOW_CONTROL_LIMIT` (`details.limit` names the policy) | no |
| Per-key rate-limit buffer full, or namespace `queued_runs` full, for a start that would be buffered (synchronous start; US07) | 429 | `QUOTA_EXCEEDED` | `RATE_LIMIT_QUEUE_FULL` | no |
| Live rate-limit keys over the namespace ceiling (US07) | 429 | `QUOTA_EXCEEDED` | `FLOW_CONTROL_LIMIT` (`details.limit`: `rate_limit_live_keys`) | no |
| Signal-with-Start whose new run would queue for its start's gates, of a signal no `waitEvent` on the start's entry path waits for (the nodes the start's settle passes through in the grant's transaction, every branch counted): refused as with room on the key, nothing created (FR-018-024 as amended by FR-022-018, D022-126) | 409 | `CONFLICT` (018 contract) | `SIGNAL_NOT_ACCEPTED_AT_START` | no |
| Signal-with-Start whose new run would queue, whose payload copied to every wait of its name on the entry path would pass the run's retained-output bound (4 MiB): refused as with room on the key, nothing created (D022-126) | 429 | `QUOTA_EXCEEDED` | — (`details.limit`: `output retained by one run`, as `signalRun`) | no |
| A key listing's cursor sealed more than fifteen minutes ago, or stamped more than one minute ahead of the reader's clock (message "the cursor has expired: read the listing again without a cursor"; D022-100, D022-129) | 400 | `BAD_REQUEST` (canonical) | — | no |
| Reset (018) of a run whose workflow declares a run-level `rateLimit`, with no free slot (a reset never waits; rate first when the start also takes a permit; US07, D022-069) | 409 | `CONFLICT` (018 contract) | `RATE_LIMIT_SLOT_UNAVAILABLE` | no |
| Publication or start using `concurrency` or `rateLimit` before the operator activated the capability through 018's instance-capability registry (review 2026-11-13) | 422 | `VALIDATION_FAILED` | `FLOW_CONTROL_ACTIVATION_PENDING` | no |
| Any call to a process that lacks a capability the registry requires (018's code, reused) | 503 | `UNAVAILABLE` | `BINARY_CAPABILITY_MISSING` | yes |
| Reset (018) of a permit-holding source with no free permit (a reset never queues) | per 018 | 018 contract | `CONCURRENCY_PERMIT_UNAVAILABLE` | no |
| Reset (018) to a point refused by a 022 reset rule (inside a handler phase, after a queue timeout or dequeue, while queued) | per 018 | 018 contract | 018's reset-point refusal reason | no |
| Reset, rerun or batch (018) of a test-tree run | per 018 | 018 contract | `RESET_SOURCE_TEST_RUN` | no |
| Namespace handler revision mismatch | 409 | `CONFLICT` | `REVISION_CONFLICT` | no |
| Handler workflow not in namespace / pinned version unpublished | 422 | `VALIDATION_FAILED` | `INVALID_FAILURE_HANDLER` | no |
| Redrive of a dispatch that is not `DEAD` | 409 | `CONFLICT` | `STATE_CONFLICT` | no |
| Invalidation target names no scope | 404 | `NOT_FOUND` | — | no |
| Capability disabled for a start of an existing version (rollback) | 422 | `VALIDATION_FAILED` | `CAPABILITY_DISABLED` | no |

Existing reasons reused unchanged: `IDEMPOTENCY_KEY_REUSED`, `STATE_CONFLICT`, `REVISION_CONFLICT`,
`PII_REVEAL_NOT_GRANTED` (a `fromRun` pin of a `pii` value without the grant), `ROUTE_NOT_FOUND`.

## Publication and validation (`DefinitionProblem.code`)

| Code | Severity | Rule |
|---|---|---|
| `CACHE_POLICY_INVALID` | error | bad TTL/scope/name/version combination, or `cache` on a kind other than activity/script |
| `CACHE_WITH_EFFECT` | error | `cache` with `compensateWith`, `reconcileWith`, `irreversible`, `highRisk`, or on a database node |
| `CONCURRENCY_POLICY_INVALID` | error | bad limit/group/timeout, timeout above the run deadline, key over the top-level `input` reads other data |
| `CONCURRENCY_SELF_CHILD` | warning | a child node names this workflow and the group/key can coincide |
| `CONCURRENCY_LIMIT_MISMATCH` | warning | an explicit group is already declared by another published definition of the namespace with a different limit (grants then skip ahead, D1196) |
| `RATE_LIMIT_POLICY_INVALID` | error | US07: bad `limit`, `per`, group or timeout, a `limit` above the ceiling of its `per`, a timeout above the run deadline, a top-level key that reads other data than `input`, or `rateLimit` on a kind other than activity/script |
| `RATE_LIMIT_MISMATCH` | warning | US07: an explicit group is already declared by another published definition of the namespace with a different `limit` or `per` (each acquirer then applies its own, D022-009) |
| `FLOW_CONTROL_KEY_UNBOUNDED` | error | a concurrency or rate-limit key reads the run's identity (`run`, whose only field is the run's id), alone or beside other data; the business identifier and the idempotency key are not visible to an expression (FR-022-056, D022-081) |
| `FLOW_CONTROL_KEY_SECRET` | error | a key reads a location classified `secret` (FR-022-057) |
| `FLOW_CONTROL_KEY_PII` | warning | a key reads a location classified `pii`; its label is shown only to callers who may disclose it (FR-022-057) |
| `EARLY_RETURN_PLACEMENT` | error | placement rules of FR-022-008 |
| `FAILURE_HANDLER_REGION` | error | region rules of FR-022-012 |
| `FLOW_CONTROL_COMBINATION` | error | `earlyReturn`, `onFailure`, `concurrency` or (US07) `rateLimit` with `continueAsNew` |
| `ENCODING_READ_BY_CACHE_KEY` | error | owned by 031 (`specs/031-client-side-encryption/contracts/workflow-encoding.md` §5): a cache key reads an encoded location |
| `ENCODING_READ_BY_CONCURRENCY_KEY` | error | owned by 031, which registers it: a run- or step-level concurrency key reads an encoded location |
| `ENCODING_CARRIAGE_MISMATCH` | error | owned by 031: `earlyReturn.result` carries encoded locations that differ from `outputSchema` (as `end.outputExpr`) |
| `UNSUPPORTED_NODE` + `dependency` `SCOPED_CACHE` / `SCOPED_CONCURRENCY` | error | `cache` or node `concurrency` inside a body |
| `UNSUPPORTED_NODE` + `dependency` `SCOPED_RATE_LIMIT` | error | US07: node `rateLimit` inside a body (G1175) |
| `UNSUPPORTED_NODE` (capability disabled) | error | existing code: any of the four capabilities, or (US07) `rateLimits`, not enabled |

## Durable failure codes (journal, step failure, run cause)

| Code | Where | Meaning |
|---|---|---|
| `CONCURRENCY_WAIT_TIMEOUT` | run (queued) or step (ready) | the queue deadline passed before a permit; non-retryable for a run, ordinary failure route for a step |
| `RATE_LIMIT_WAIT_TIMEOUT` | run (buffered) or step (waiting for a slot) | US07: the queue deadline passed before a slot; non-retryable for a run, ordinary failure route for a step; the wait spent no attempt |
| `FLOW_CONTROL_KEY_INVALID` | step | a step-level key failed to evaluate or exceeds 256 bytes; failure route, no attempt spent, never granted (FR-022-054) |
| `FLOW_CONTROL_LIMIT` | step | a step-level key would exceed the namespace's live-keys ceiling; failure route, no attempt spent (FR-022-054) |
| `RATE_LIMIT_QUEUE_FULL`, `CONCURRENCY_QUEUE_FULL`, `FLOW_CONTROL_LIMIT` | child node | the child run's start was refused by a full buffer, `queued_runs` or the live-keys ceiling; retryable as a class, through the node's failure route (a `child` node has no retry policy in IR v1); no child run exists (FR-022-055, D022-080) |
| `CAPABILITY_DISABLED`, `FLOW_CONTROL_ACTIVATION_PENDING`, `BINARY_CAPABILITY_MISSING` | child node | the child run's start was refused because the deployment switched the capability off, the fleet has not activated it, or this process does not serve the gate (or the child's definition has a collection or a loop, G1197); through the node's failure route; no child run exists (FR-022-055, D022-080) |
| `EXPRESSION_ERROR` | run | existing: an `earlyReturn.result` or a top-level concurrency or rate-limit key failed to evaluate |
| `RUN_DATA_LIMIT` | run | existing: an early result over 256 KiB, or a cache-hit copy over the run's retained limit |
| `STORAGE_QUOTA_EXCEEDED` | run | existing: the copy of a hit or an early result exceeded the tenant content quota (ADR-0027 path) |
| `FAILURE_HANDLER_TIMEOUT` | handler outcome only | the handler exceeded `onFailure.timeoutMs`; the run keeps its pending status |

## Dispatch error codes (`failure_handler_dispatches.last_error_code`, closed)

`WORKFLOW_NOT_FOUND`, `NO_ACTIVE_VERSION`, `VERSION_NOT_PUBLISHED`, `QUOTA_EXCEEDED`, `PUBLISHER_NOT_AUTHORIZED`,
`UNAVAILABLE`.

## gRPC worker protocol

Unchanged. No draft invents an encrypted-field code: encoded-field refusals are 031's `ENCODING_*` codes
(coordinator decision C7). Mocked and cached steps never produce a lease. A step waiting for a permit or for a rate
slot (US07) is not `READY`: it waits outside it (`WAITING`, wait kind `CONCURRENCY` or `RATE_LIMIT`; ADR-0089 decision
1, FR-022-019) until the grant of its last declared gate, so no claim ever considers it and no attempt is spent.
*Corrected 2026-11-24 (decision D022-077 item 3):* this paragraph said that a step waiting for a permit stays `READY`
and is skipped by `PollTasks`, the design ADR-0089 replaced.
