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
| `EARLY_RETURN_PLACEMENT` | error | placement rules of FR-022-008 |
| `FAILURE_HANDLER_REGION` | error | region rules of FR-022-012 |
| `FLOW_CONTROL_COMBINATION` | error | `earlyReturn`, `onFailure` or `concurrency` with `continueAsNew` |
| `ENCODING_READ_BY_CACHE_KEY` | error | owned by 031 (`specs/031-client-side-encryption/contracts/workflow-encoding.md` §5): a cache key reads an encoded location |
| `ENCODING_READ_BY_CONCURRENCY_KEY` | error | owned by 031, which registers it: a run- or step-level concurrency key reads an encoded location |
| `ENCODING_CARRIAGE_MISMATCH` | error | owned by 031: `earlyReturn.result` carries encoded locations that differ from `outputSchema` (as `end.outputExpr`) |
| `UNSUPPORTED_NODE` + `dependency` `SCOPED_CACHE` / `SCOPED_CONCURRENCY` | error | `cache` or node `concurrency` inside a body |
| `UNSUPPORTED_NODE` (capability disabled) | error | existing code: any of the four capabilities not enabled |

## Durable failure codes (journal, step failure, run cause)

| Code | Where | Meaning |
|---|---|---|
| `CONCURRENCY_WAIT_TIMEOUT` | run (queued) or step (ready) | the queue deadline passed before a permit; non-retryable for a run, ordinary failure route for a step |
| `EXPRESSION_ERROR` | run | existing: an `earlyReturn.result` or a top-level concurrency key failed to evaluate |
| `RUN_DATA_LIMIT` | run | existing: an early result over 256 KiB, or a cache-hit copy over the run's retained limit |
| `STORAGE_QUOTA_EXCEEDED` | run | existing: the copy of a hit or an early result exceeded the tenant content quota (ADR-0027 path) |
| `FAILURE_HANDLER_TIMEOUT` | handler outcome only | the handler exceeded `onFailure.timeoutMs`; the run keeps its pending status |

## Dispatch error codes (`failure_handler_dispatches.last_error_code`, closed)

`WORKFLOW_NOT_FOUND`, `NO_ACTIVE_VERSION`, `VERSION_NOT_PUBLISHED`, `QUOTA_EXCEEDED`, `PUBLISHER_NOT_AUTHORIZED`,
`UNAVAILABLE`.

## gRPC worker protocol

Unchanged. No draft invents an encrypted-field code: encoded-field refusals are 031's `ENCODING_*` codes
(coordinator decision C7). Mocked and cached steps never produce a lease; a step waiting for a permit is simply not
returned by `PollTasks` (it stays `READY`, no attempt spent), exactly as for an exhausted activity quota.
