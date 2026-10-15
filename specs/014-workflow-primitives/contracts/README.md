# Proposed contracts for feature 014

**Status: design proposal — not a supported API or executable workflow schema.** Current examples
use `sagat-octopus-orchestrator.io/v1` and the canonical contracts in
`specs/001-platform-foundation/contracts/`. This document is not installed in those contracts.
T014-002/T014-006 accept the design, allocate revision identifiers and add machine-verifiable
contracts before runtime implementation. Proposed names below are normative for that review;
they are not code snippets users can call today.

## 1. IR revision and validation

Use a new explicitly supported IR revision for structured scopes, state declarations and
continuation; keep `v1` parsing and behavior unchanged. Do not silently permit unknown kinds in v1.
The publish validator knows the supported revision and enabled primitives; unsupported revision or
kind returns `VALIDATION_FAILED` with reason `UNSUPPORTED_NODE` before storing a runnable version. Immutable runs pin the exact
revision and workflow version, independently of product/worker protocol version.

The proposed document adds a `bodies` map, optional `state` declarations and `chainPolicy`.
Named bodies have local nodes/entrypoint/return boundary. Definition graph, each body and body
reference graph remain acyclic. Closed-region validation forbids cross-body references, workflow
ends in bodies and overlapping ownership. Paths/expressions cannot dynamically name arbitrary
nodes, secrets or capabilities. Existing expression grammar/type validation and evaluation budgets
apply; no general code callback is accepted.

| Kind / declaration | Required fields | Semantics |
| --- | --- | --- |
| `forEach` | `items`, `body`, `result`, `maxItems`, `maxConcurrency`, `next` | Evaluate/capture bounded array once; invoke body at each index; ordered array of `result` values. SDK `map` emits this same node. |
| `while` | `condition`, `body`, `initial`, `carry`, `maxIterations`, `timeoutMs`, `next` | Pre-test; false initially returns initial carry; each body computes new carry before next test. |
| `repeatUntil` | Same as `while` | Post-test; execute at least once, stop when condition becomes true. |
| `approval` | `policy`, `timeoutMs`, `onApprove`, `onReject`, `onExpire` | Single eligible decision or expiry; all three routes declared; no activity runs while waiting. |
| `switch` | ordered `cases[{id,when,target}]`, `default` | First boolean true wins; evaluated predicate error fails; recorded decision is replayed. |
| `state` | `schema`, `initial`, optional `queries`/`updates` maps | Initialize typed business state with run creation; operation schemas and bounded expressions are pinned. |
| query entry | `argsSchema`, `resultSchema`, `expression` | Read only; snapshot of input/state/completed outputs/args, with authorized projection. |
| update entry | `argsSchema`, optional `validate`, `state`, `result`, `resultSchema` | Atomic validator/state reducer/result; no I/O, activity, wait or child invocation. |
| `continueAsNew` | `input`, `state` (if state exists) | Top-level terminal boundary; same-version successor, quiescence and chain policy required; no `next`. |
| `chainPolicy` | `maxSegments`, `timeoutMs` | Required if continuation is present; count includes initial run; original chain timeout never resets. |

Loops may also specify `delayMs` in 1..31622400000, applied only between needed iterations. Absence
means no delay. Shared run `maxInstances` and nesting limits are enforced even when individual node
limits fit. ForEach's body sees immutable `{item,index}` plus captured input and declared state;
indices have no language-specific iteration semantics. Loop bodies see `{iteration,carry}`. Body
`result` and loop `carry` expressions are evaluated and recorded by the same bounded adapter as
conditions. A map output is always index-ordered; there is no unordered mode in this increment.

Numeric limits, payload ceilings and uniqueness rules are specified in `data-model.md`. SDK builders
perform useful local validation but the server owns all decisions and revalidates the document.
A builder cannot turn language `for`, `await`, Promise/Task concurrency or arbitrary callbacks into
durable behavior implicitly. No generated pseudo-API is added to current examples.

## 2. Proposed HTTP operations

All paths are relative to the existing `/v1` REST base. New operations use existing authentication,
namespace resolution, canonical error envelope and request/trace headers. They never accept tenant
or actor identity from the request body. The path design is additive; OpenAPI details remain an
unchecked implementation task, not a substitute contract served today.

| Method/path | Body/query | Success result / authorization |
| --- | --- | --- |
| `GET /runs/{runId}/approvals` | bounded cursor/page size | Requests visible under `run:read` + `approval:read`; omit policy/actor details without their permission. |
| `POST /approvals/{approvalId}/decisions` | `decisionKey`, `decision: APPROVE|REJECT`, optional `reasonCode` | 200 durable decision receipt; requires `approval:decide` plus current policy eligibility. |
| `POST /runs/{runId}/queries/{name}` | schema-checked `args` | 200 `{runId,result,runVersion,stateRevision}`; requires `run:read` + `run:query` for the named operation. |
| `POST /runs/{runId}/updates/{name}` | `updateKey`, `args`, optional `expectedStateRevision` | 200 APPLIED receipt/result; 422 durable business rejection or 409 durable revision conflict; requires `run:update` for the named operation. |
| `GET /chains/{chainId}` | none | 200 chain head, status, revision, predecessor/final links; requires `run:read` on the chain's namespace. |
| `GET /chains/{chainId}/runs` | bounded cursor/page size | 200 ordered segment metadata, no implicit payloads; requires `run:read`. |
| `POST /chains/{chainId}/queries/{name}` | `args`, optional `expectedRunId` | Query one current-head snapshot; returns actual run id, run version and state revision; expected-head mismatch is 409. |
| `POST /chains/{chainId}/updates/{name}` | `updateKey`, `args`, optional `expectedRunId`, `expectedStateRevision` | Apply to current head in same lock/transaction as resolution, or 409 expected-head conflict; receipt deduplicates across rollover. |
| `POST /chains/{chainId}/signals` | existing signal fields plus required `signalKey`, optional `expectedRunId` | Existing signal rules on the resolved head: accepted only if it is waiting for the signal, otherwise 409 `STATE_CONFLICT`; result names the resolved run id; the chain-scoped key prevents delivery to another segment. |
| `POST /chains/{chainId}/cancel` | `cancelKey`, optional `expectedRunId` | Durable cancellation acceptance/result; latch chain cancellation, stop handoff, cancel current run under existing drain/child policies. |

**Permissions (proposal, accepted in T014-002)**: new permissions follow the existing
`resource:action` form. `approval:read` goes to `admin`, `developer`, `operator` and `viewer`;
`approval:decide`, `run:query` and `run:update` go to `admin`, `developer` and `operator`.
`worker` and `runner` get none. A role permission is necessary, not sufficient: approval also
requires the request's eligibility policy.

**Value disclosure**: no current API returns payload values; ADR-0028 permits only authorized
structural inspection and requires a new decision before any value is disclosed. Query results,
Update results and replayed receipts return declared values, so these operations are not released
until that decision is accepted (T014-002). It fixes the permission above, the audit and redaction.

Pagination reuses existing cursor/page bounds rather than introducing an unbounded list. Chain-level
operations are explicit opt-ins; run-level methods never change address scope implicitly. Read APIs
expose no payload unless the declared operation/result permissions permit it. Authorize every
request, including receipt replay, before looking up or returning deduplicated content.

Update receipt returns `updateKey`, actual `runId`, optional `chainId`, status, result or rejection
code and before/after state revision. New requests on a non-mutable run return `CONFLICT` with
reason `RUN_NOT_MUTABLE`; when the run has continued, reason `RUN_CONTINUED` with authorized
successor metadata. An identical previously committed receipt is replayed before checking terminal
state. Receipt retention ends with the applicable retained resource; an authorized missing resource
returns `NOT_FOUND`, never a new operation on a successor. A rejected/failed transport request must not make an SDK invent a new
idempotency key automatically.

**Approval authorization**: permission to send signals, publish a workflow or approve publication
does not imply permission to decide a runtime approval. Both named role/subject policy and current
authorization must pass, including initiator exclusion. Unknown/cross-tenant resources both return
404 `NOT_FOUND` to the caller, with a restricted security audit for the attempted access.
A decision key is scoped to the request. Same-key changed body is 409 `CONFLICT` with reason
`IDEMPOTENCY_KEY_REUSED`; another key after final decision is 409 `CONFLICT` with reason
`ALREADY_DECIDED`. An excluded initiator gets 403 `FORBIDDEN` with reason `INITIATOR_CANNOT_DECIDE`,
like the publication approval's `AUTHOR_CANNOT_DECIDE`. Expiry vs decision obeys first commit,
not caller timestamps. The server supplies all outcome timestamps.

## 3. Segment, chain and rollover behavior

| Operation | Explicit run target | Explicit chain target |
| --- | --- | --- |
| Get / Query | Inspect that retained segment, including a continued terminal segment | Resolve and read one consistent current head; return actual run id. |
| Existing SDK completion wait | Stop when that run becomes terminal; return SUCCEEDED + CONTINUED metadata on handoff | New `waitForChainCompletion` follows authorized links until FINAL; timeout only stops client waiting. |
| Signal | Existing exact-run behavior; continued terminal run is not forwarded | Resolve the head and apply the existing signal rule in the same transaction: a waiting step consumes it, otherwise 409 `STATE_CONFLICT`; it is never held for a successor. |
| Update | Run-scoped key; a new operation on continued run returns RUN_CONTINUED | Chain-scoped key returns original applied/rejected receipt across rollover, never applies twice on different segments. |
| Cancel | Cancels only that nonterminal run; a terminal old segment is unchanged | Sets chain cancel latch and cancels current run; no later continuation can pass the latch. |
| Child completion | Parent's child instance follows its child chain | Intermediate continued segments emit no parent completion; final outcome is applied once. |

`waitForChainCompletion` is a proposed language-idiomatic addition; existing `waitForCompletion`
behavior (where provided) and polling helpers remain segment waits. Documentation must not label
an old run's SUCCEEDED as final chain success. No automatic redirect on a 409 or hidden retry with a
new key. An optional `expectedRunId` enables a caller to reject rollover instead of accepting head
resolution. Without it, a chain-target caller explicitly chooses whichever head wins serialization.

Chain signal/cancel keys use the same 1..128 permitted ASCII constraint as update keys, scoped by
`(tenant,namespace,chainId,operationKind,key)` and canonical content hash. Shared chain/run locking
serializes continuation with commands. If a command commits first, the handoff sees its state,
pending signal or cancellation latch. If handoff commits first, a chain command targets the new
head or fails its expected-head precondition. A run command stays bound to the old run. Signals are
never buffered (`signalRun` in the canonical OpenAPI): one is accepted only by a step waiting for it and consumed in that
transaction, so there is no pending signal to drain, and an open wait already blocks the handoff.
No new semantics are added to external broadcast events: existing namespace event subscriptions
and inbox rules continue, and an open subscription prevents handoff.

A child's parent link uses original child-instance identity and chain id, not only its first run id.
The default and only initial child policy is to await the entire chain. A user can explicitly wait
for a segment via its run id; no hidden change to existing direct run waits is permitted.

## 4. Worker and SDK compatibility

Add full instance identity to task, heartbeat, completion, cancellation, reconciliation and secret-
capability addressing using a negotiated next worker minor (allocate after baseline check; baseline
1.4 is recorded in feature 006). Old root workflows still accept the old addressing forms. New
scoped tasks are eligible only for a worker that negotiated support, with unchanged lease/generation
checks. SDK retry logic does not implement the workflow loop; it handles transport safely and passes
idempotency/instance identity. The server remains authoritative about scheduling and quota.

Every SDK gains idiomatic declarative builders for the six node types/state operations plus the
explicit approval, query/update and chain client operations. Shared names in this proposal indicate
semantics, not a mandate for identical language casing. All five must emit byte-identical canonical
IR and pass the same operation/fault fixtures before enabling the advertised capability. Old SDKs
can run old definitions; unsupported definitions produce an actionable typed version error.

## 5. Proposed typed errors

Every error retains canonical `code`, safe `message`, `retryable`, `traceId`, `requestId`; request id
is server-generated. Details contain only authorized bounded identifiers/limits, never expressions,
arguments, state, reason text, role membership or secret content.

`Error.code` is a closed enum (`BAD_REQUEST`, `UNAUTHENTICATED`, `FORBIDDEN`, `NOT_FOUND`, `CONFLICT`,
`VALIDATION_FAILED`, `RATE_LIMITED`, `QUOTA_EXCEEDED`, `UNAVAILABLE`, `INTERNAL`). This feature adds
**no** code: an SDK that parses the enum exhaustively must keep working, for the same reason the run
status enum gains no `CONTINUED`. New meaning goes into `details.reason`, reusing existing reasons
where one fits, and each new reason gets a row in the remediation table (`ErrorCode::remediation`).
Durable workflow failures are step/run failure codes (an open set, like `EXPRESSION_ERROR`), not API
error codes.

| Situation | Durable failure code | API `code` (HTTP) | `details.reason` | Retryable |
| --- | --- | --- | --- | --- |
| IR revision/kind/worker capability unavailable | — | `VALIDATION_FAILED` (422) | `UNSUPPORTED_NODE` (existing) | false |
| Cycle, escape, overlap, illegal continuation location, recursive body | — | `VALIDATION_FAILED` (422) | `INVALID_SCOPE` | false |
| Next work exceeds a declared hard budget | `ITERATION_LIMIT` / `INSTANCE_LIMIT` / `CHAIN_LIMIT` | command: `QUOTA_EXCEEDED` (429) with `details.limit` | — | false |
| Existing byte/context bound exceeded | `RUN_DATA_LIMIT` (existing) | command body too large: `BAD_REQUEST` (413) with `PAYLOAD_TOO_LARGE`; run's retained data would exceed: `QUOTA_EXCEEDED` (429) with `details.limit`, as signals do today | existing | false |
| Expression type/evaluation/schema failure | `EXPRESSION_ERROR` (existing) | `VALIDATION_FAILED` (422) | `EXPRESSION` (existing) or `INVALID_OPERATION_STATE` | false |
| Caller lacks permission or approval eligibility | — | `FORBIDDEN` (403) | `NOT_ELIGIBLE`; `INITIATOR_CANNOT_DECIDE` | false |
| Missing, expired or inaccessible resource | — | `NOT_FOUND` (404) | — | false |
| Another decision already won | — | `CONFLICT` (409) | `ALREADY_DECIDED` (existing) | false |
| Same key, different canonical request | — | `CONFLICT` (409) | `IDEMPOTENCY_KEY_REUSED` (existing) | false |
| Expected state revision or chain head does not match | — | `CONFLICT` (409) | `REVISION_CONFLICT` (existing); `CHAIN_HEAD_CHANGED` | false |
| Declared validator returned false | — | `VALIDATION_FAILED` (422) | `UPDATE_REJECTED` | false |
| Mutation targets an ineligible state or an old segment | — | `CONFLICT` (409) | `RUN_NOT_MUTABLE`; `RUN_CONTINUED` | false |
| Signal to a head that is not waiting for it | — | `CONFLICT` (409) | `STATE_CONFLICT` (existing) | false |
| Boundary reached with unresolved obligations | `CONTINUATION_NOT_QUIESCENT` | — | — | false |
| Rate or temporary capacity | — | `RATE_LIMITED` / `QUOTA_EXCEEDED` (429, Retry-After) | existing | true; reuse the same key |
| Worker or scheduler no longer owns the attempt/transition | existing stale-fence handling | existing worker error | — | false |

Workflow deadlines use existing TIMED_OUT behavior; request wait deadlines do not implicitly cancel
runs/updates. T014-006 adds the new reasons and their remediation rows to the canonical OpenAPI
and to `crates/api-http/src/error.rs` together, and checks each mapping against the existing ones.

## 6. Telemetry and audit

Proposed OTel spans: `workflow.scope.admit`, `workflow.iteration.commit`, `workflow.approval.decide`,
`workflow.query`, `workflow.update`, `workflow.continue`. Link predecessor/successor traces and retain
W3C trace context; put run/instance/chain/actor identifiers only in authorized spans or redacted logs,
not metric labels. Tenant context is mandatory internally; metrics use bounded tenant service tier
or a bounded authorized tenant aggregation, never unbounded raw tenant/run IDs.

Metrics: `workflow_primitive_transitions_total{kind,outcome}`, `workflow_primitive_active{kind}`,
`workflow_primitive_admission_wait_seconds{kind}`, `workflow_approval_wait_seconds{outcome}`,
`workflow_operation_duration_seconds{kind,outcome}`, `workflow_primitive_rejections_total{kind,code}`,
`workflow_continuation_total{outcome}` and backlog/reconciliation counters. Kind/outcome/code are
bounded enums. All samples and structured logs exclude payload, state, arguments, expressions and
secret markers. The journal-derived runtime graph (`/runs/{runId}/graph`, `/graph/changes`;
ADR-0016) and the telemetry projector (ADR-0017) expose instance index, scope, selected case and
chain links with lag; telemetry is never the authoritative workflow graph. A Studio or projector
that does not know a new revision shows its nodes as unsupported and keeps working (T014-045).

Durable audit covers opening/deciding/expiring approval, decision denial, query access, update
application/rejection, continuation, cancel/manual decisions and quota/policy changes. Workflow
mutations and their audits are atomic; query/security-denial audits use the existing access/security
audit path. A failed audit write fails the operation closed. Retain only subject ids, operation kind,
resource refs, outcome/code, time and trace correlation under existing audit access policy.
