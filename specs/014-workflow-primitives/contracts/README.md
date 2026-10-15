# Accepted contract design for feature 014

**Status: architecture accepted by ADR-0030/0031/0032; canonical contracts/runtime pending.** Current
examples use `sagat-octopus-orchestrator.io/v1` and the deployed canonical contracts in
`specs/001-platform-foundation/contracts/`. The design below is normative for implementation;
T014-006 adds machine-verifiable contracts and runtime mapping. Acceptance does not make the
operations available or prove their release tests.

## 1. Additive IR and capability validation

Keep `sagat-octopus-orchestrator.io/v1` and extend the canonical `workflow.schema.json` in place,
as accepted by [ADR-0032](../../../docs/adr/ADR-0032-additive-workflow-ir.md). Preserve every
previously accepted root form and stored-run meaning. Historical unsupported `foreach` and
`humanApproval` shapes remain unsupported, not aliases for the new kinds. Worker minor 1.5 carries
scoped task identity independently of the IR version. Publication checks supported and enabled
capabilities; an unavailable kind returns `VALIDATION_FAILED` with reason `UNSUPPORTED_NODE` before
storing a runnable version. Immutable runs pin the exact workflow version, and required capabilities
come from that definition plus explicit stored identity/chain data, not from a second `apiVersion`.

The document adds a `bodies` map, optional `state` declarations, `chainPolicy` and optional
`limits: {maxInstances,maxDepth}`. Omitted limit fields default to 100000 and 8 respectively;
valid ranges are 1..100000 and 1..8, and tenant policy may lower them.
Named bodies have local nodes/entrypoint/return boundary. Definition graph, each body and body
reference graph remain acyclic. Closed-region validation forbids cross-body references, workflow
ends in bodies and overlapping ownership. Paths/expressions cannot dynamically name arbitrary
nodes, secrets or capabilities. The existing evaluator is authoritative: source at most 1024 bytes, 256 tokens, depth 16,
list literals at most 64, constructed/concatenated lists at most 4096 and generated strings at most
64 KiB. The domain's 2048-byte wrapper is not an expanded evaluator allowance. No object/map literal,
mutation operator or general code callback is added; expressions may select existing objects.

A body is `{entrypoint, nodes}` with exactly one node `{id, type: "return", result}`. The return
node has no successor and its expression produces the body result; it is forbidden in the root
graph. Body nodes cannot be `end` or `continueAsNew`. A collection's `result` expression reads
`bodyResult`, `item` and `index`; body expressions read `item`/`index` (collections) or
`iteration`/`carry` (loops), plus input/state and local `steps.<node>.output`. Local completed outputs
are resolved by scoped instance, never by flattening equal node names across iterations.

The canonical `workflow.schema.json` checks document shape. Existing root schema branches remain
compatible; new kinds and body declarations use closed bounded shapes. Publication must additionally check
UTF-8 expression bytes/grammar, referenced targets/body closure/recursion, unique case/node ids,
aggregate authored-node count, schema byte/depth limits and enabled runtime capabilities. JSON
Schema `maxLength` counts characters, so its bound is not proof of the evaluator's byte limit.

| Kind / declaration | Required fields | Semantics |
| --- | --- | --- |
| `forEach` | `items`, `body`, `result`, `maxItems`, `maxConcurrency`, `next` | Evaluate/capture bounded array once; invoke body at each index; ordered array of `result` values. SDK `map` emits this same node. |
| `while` | `condition`, `body`, `initial`, `carry`, `maxIterations`, `timeoutMs`, `next` | Pre-test; false initially returns initial carry; each body computes new carry before next test. |
| `repeatUntil` | Same as `while` | Post-test; execute at least once, stop when condition becomes true. |
| `approval` | `policy`, `timeoutMs`, `onApprove`, `onReject`, `onExpire` | Single eligible decision or expiry; all three routes declared; no activity runs while waiting. |
| `switch` | ordered `cases[{id,when,target}]`, `default` | First boolean true wins; evaluated predicate error fails; recorded decision is replayed. The shape has no `onError` route. |
| `state` | `schema`, `initial`, optional `queries`/`updates` maps | Initialize typed business state with run creation; operation schemas and bounded expressions are pinned. |
| query entry | `argsSchema`, `resultSchema`, `expression`, `disclosure: declared-business-result-v1` | Read only; snapshot of input/state/completed outputs/args, with authorized projection. |
| update entry | `argsSchema`, optional `validate`, `state`, `result`, `resultSchema`, `disclosure: declared-business-result-v1` | Atomic validator/state reducer/result; no I/O, activity, wait or child invocation. |
| `continueAsNew` | `input`, `state` (if state exists) | Top-level terminal boundary; same-version successor, quiescence and chain policy required; no `next`. |
| `return` | `result` | Body-only terminal boundary; exactly one per body, no successor. |
| `chainPolicy` | `maxSegments`, `timeoutMs` | Required if continuation is present; count includes initial run; original chain timeout never resets. |

The runtime graph exposes `switchSelection` as CASE with local `caseId` and pinned target, DEFAULT
with its pinned target, or null before evaluation. Its ordered edges use `kind: case` with `caseId`
and `kind: default`. The journal records `switch.evaluated` once; telemetry derives
`workflow.switch.evaluate` and a bounded `workflow_primitive_transitions_total` observation from
that committed event. Case IDs never become metric labels.

Loops may also specify `delayMs` in 1..31622400000, applied only between needed iterations. Absence
means no delay. Shared run `maxInstances` and nesting limits are enforced even when individual node
limits fit. Each forward activation reserves its possible declared compensation/reconciliation
identities inside the lifetime instance budget; derived activation uses that reservation without a
second charge, including after a policy reduction. Retry does not allocate another logical instance. ForEach's body sees immutable `{item,index}` plus captured input and declared state;
indices have no language-specific iteration semantics. Loop bodies see `{iteration,carry}`. Body
`result` and loop `carry` expressions are evaluated and recorded by the same bounded adapter as
conditions. A map output is always index-ordered; there is no unordered mode in this increment. A referenced
input/blob array may contain up to 10,000 items within the capture/value/context byte limits, subject
to bounded materialization evidence; this does not increase CEL literal/constructed-list limits.
The engine assembles the indexed aggregate separately from expression list concatenation.

Numeric limits, payload ceilings and uniqueness rules are specified in `data-model.md`. SDK builders
perform useful local validation but the server owns all decisions and revalidates the document.
A builder cannot turn language `for`, `await`, Promise/Task concurrency or arbitrary callbacks into
durable behavior implicitly. No generated pseudo-API is added to current examples.

## 2. HTTP operation design

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

**Permissions (accepted by ADR-0030/0031/0032)**: new permissions follow the existing
`resource:action` form. `approval:read` goes to `admin`, `developer`, `operator` and `viewer`;
`approval:decide`, `run:query` and `run:update` go to `admin`, `developer` and `operator`.
`worker` and `runner` get none. A role permission is necessary, not sufficient: approval also
requires the request's eligibility policy. Namespace `run:query`/`run:update` grants authorize all
declared operation names in that namespace; no per-name ACL or custom role is implied.

**Value disclosure**: ADR-0031 accepts only explicitly declared business-operation results. Each
query/update declaration includes `disclosure: declared-business-result-v1` and a closed bounded
result schema (recursively declared object keys, no arbitrary additional properties, bounded
arrays/strings/depth, maximum 256 KiB serialized result). A schema annotation alone is not disclosure
permission. Result values/declared keys are intentionally visible to the authorized operation caller;
no generic raw payload selector/export is added. Responses use `Cache-Control: no-store`. Current
authorization and successful audit precede any result or receipt replay. The evaluator receives no
secret capability, credentials, network or callback; preserve protected-secret checks and do not
claim heuristic classification of arbitrary business data. Normal graph/SSE/journal/log/audit/trace
responses remain payload-free, and ADR-0028 structural inspection remains value/key-free. US06
release still requires its real security, transaction and all-five conformance evidence.

The initial closed result-schema vocabulary has one explicit `type` per schema: object, array,
string, integer, number, boolean or null. Objects declare `properties` and `additionalProperties:
false`, with optional `required`; arrays declare `items` and `maxItems`; strings declare `maxLength`.
Numeric bounds and descriptions are optional. References/combinators, arbitrary object keys and
unbounded array/string results are not part of this initial result contract. Schema compilation,
64 KiB schema bytes, finite nesting and the aggregate serialized-result bound are checked by the
adapter in addition to document-shape validation; argument/state schemas retain their existing
local-schema validation path.

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

`waitForChainCompletion` is a planned language-idiomatic addition; existing `waitForCompletion`
behavior (where provided) and polling helpers remain segment waits. Documentation must not label
an old run's SUCCEEDED as final chain success. No automatic redirect on a 409 or hidden retry with a
new key. An optional `expectedRunId` enables a caller to reject rollover instead of accepting head
resolution. Without it, a chain-target caller explicitly chooses whichever head wins serialization.

Chain signal/cancel keys use the same 1..128 permitted ASCII constraint as update keys, scoped by
`(tenant,namespace,chainId,operationKind,key)` and canonical content hash. Shared chain/run locking
serializes continuation with commands. If a command commits first, the handoff sees its state,
consumed-signal transition or cancellation latch. If handoff commits first, a chain command targets the new
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
capability addressing using worker minor 1.5 allocated by ADR-0030 after checking the 1.4 baseline. Old root workflows still accept the old addressing forms. New
scoped tasks are eligible only for a worker that negotiated support, with unchanged lease/generation
checks. SDK retry logic does not implement the workflow loop; it handles transport safely and passes
idempotency/instance identity. The server remains authoritative about scheduling and quota.

Every SDK gains idiomatic declarative builders for the six node types/state operations plus the
explicit approval, query/update and chain client operations. Shared names in this design indicate
semantics, not a mandate for identical language casing. All five must emit byte-identical canonical
IR and pass the same operation/fault fixtures before enabling the advertised capability. Old SDKs
can run old definitions; unsupported definitions produce an actionable typed unsupported-capability error.

## 5. Typed errors

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
| IR version/kind/worker capability unavailable | — | `VALIDATION_FAILED` (422) | `UNSUPPORTED_NODE` (existing) | false |
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

OTel span design: `workflow.scope.admit`, `workflow.iteration.commit`, `workflow.approval.decide`,
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
that does not know a new capability shows its nodes as unsupported and keeps working (T014-045).

Durable audit covers opening/deciding/expiring approval, decision denial, query access, update
application/rejection, continuation, cancel/manual decisions and quota/policy changes. Workflow
mutations and their audits are atomic; query/security-denial audits use the existing access/security
audit path. A failed audit write fails the operation closed. Retain only subject ids, operation kind,
resource refs, outcome/code, time and trace correlation under existing audit access policy.
