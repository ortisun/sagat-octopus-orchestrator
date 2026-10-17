# Accepted contract design for feature 014

**Status: architecture accepted by ADR-0030/0031/0032; bounded run Query/Update is enabled and the
remaining primitive runtime is incomplete.** Current examples use
`sagat-octopus-orchestrator.io/v1` and the deployed canonical contracts in
`specs/001-platform-foundation/contracts/`. The design below is normative for implementation;
availability still depends on each capability's runtime and release evidence.

## 1. Additive IR and capability validation

Keep `sagat-octopus-orchestrator.io/v1` and extend the canonical `workflow.schema.json` in place,
as accepted by [ADR-0032](../../../docs/adr/ADR-0032-additive-workflow-ir.md). Preserve every
previously accepted root form and stored-run meaning. Historical unsupported `foreach` and
`humanApproval` shapes remain unsupported, not aliases for the new kinds. Worker minor 1.5 carries
scoped task identity independently of the IR version. Publication checks supported and enabled
capabilities; an unavailable kind returns `VALIDATION_FAILED` with reason `UNSUPPORTED_NODE` before
storing a runnable version. Immutable runs pin the exact workflow version, and required capabilities
come from that definition plus explicit stored identity/chain data, not from a second `apiVersion`.

The public parser accepts a closed `state` declaration with the already enabled basic root-node
set. This is the publication boundary for run-addressed Query/Update. A document that combines
`state` with `bodies`, approval, collection, loop or body-return forms remains rejected. The one
combined continuation form described below is available.

The optional top-level `limits` block is publishable in every shape the runtime can run (D297).
An author may lower `maxInstances` (1..100000) and `maxDepth` (1..8) below the platform ceilings
and never raise them: a value outside the range is refused at publication, an omitted field keeps
its ceiling, and tenant policy can lower the run's budget further but never raise it. A document
without the block is unchanged, byte for byte and digest for digest. A definition whose bodies
nest deeper than its own `maxDepth` is refused. Run creation captures the smaller of the platform
ceiling and the author's value; a chain successor captures its own, so `maxInstances` bounds each
segment.

Every activated node instance counts, the `end` node included; admitting a collection or loop
iteration charges its body's nodes plus the compensation and lookup instances its activities may need (an activity and its `return` cost two; an activity with `compensateWith` and `reconcileWith` costs four). The next work beyond
the budget is a durable run failure, never a silent wait:

| Next work | Failure code | When it is decided |
| --- | --- | --- |
| a root node beyond `maxInstances` | `INSTANCE_LIMIT` | by the planner, as the stop that replaces the activation |
| a collection whose iterations cannot all be admitted | `INSTANCE_LIMIT` | before the collection enters |
| more collection items than `maxItems` | `ITERATION_LIMIT` | before the collection enters |
| a loop's first body or successor does not fit | `INSTANCE_LIMIT` | before the loop enters |
| a loop's next body (or its successor once it is done) does not fit | `INSTANCE_LIMIT` | at the checkpoint, before any delay is scheduled; the completed iterations are kept |
| a loop needs another body after `maxIterations` | `ITERATION_LIMIT` | at the checkpoint |

`INSTANCE_LIMIT` is not routed to an `onError` node, which would need an instance of its own. A
tenant policy that lowers a running run below its accepted work pauses further collection
admission (accepted work is never killed and nothing is deleted); a loop reports the lowered
budget as `INSTANCE_LIMIT` at its next checkpoint.

The continuation parser retains `chainPolicy` as a bounded domain policy and `continueAsNew` as a
typed terminal with its input and optional state expressions. The public runtime accepts two closed
root shapes: an activity followed directly by a continuation transferring input and, when declared,
the exact `state` snapshot; or an activity followed by one condition that selects between the
same continuation and an `end` without output. Run
creation writes the initial chain membership atomically. The scheduler evaluates the transfer and
commits the predecessor, successor, chain head, journal, audit and outbox under the chain and run
compare-and-swap guards. Reaching `maxSegments` stops the current run with durable `CHAIN_LIMIT`,
marks the segment `FINAL` and latches the chain outcome so another tick cannot repeat the boundary.
An input/state transfer expression that fails evaluation stops the same current run with durable
`EXPRESSION_ERROR`, audit and `FINAL` in one fenced transaction; a stale evaluated snapshot cannot
fail a newer run version.
Arbitrary state transfer, bodies, triggers, `onError`, non-activity entrypoints and all wider graph
combinations remain rejected at publication; top-level `limits` are accepted.

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
| `while` | `condition`, `body`, `initial`, `carry`, `maxIterations`, `timeoutMs`, `next`; optional `delayMs`, `onError` | Pre-test; false initially returns initial carry; each body computes new carry before next test. With `onError`, every loop failure follows that edge (below). |
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

A loop that declares `onError` routes every loop failure to that edge: a permanent body failure,
the loop `timeoutMs` and an expression error (D294), as the iteration limit already did. The active
iteration drains first (its member steps stop, no new iteration or delay is admitted), the loop scope
ends `FAILED` with its failure code retained on the owner step, and the run continues at the edge in
the same transaction that finishes the drain: an activity target becomes `READY` with its task
outbox, an `end` target finishes the run through the normal terminal path including `run.finished`
and the outbound fan-out. The run is never asked to stop, so it stays `RUNNING`/`WAITING` and no
`run.stop_requested` is journaled. A loop without `onError` keeps failing the run after the drain,
and a run-level cancellation always finishes the run `CANCELLED` and never takes the edge, even when
it lands during a failure drain. The public loop subset accepts an `end` or an activity as the `onError`
target (D400): a recovery activity is an ordinary root activity (a worker takes it with any worker
protocol), it may chain to further activities and must end at an `end`, and it declares no `onError`,
`compensateWith`, `reconcileWith`, `irreversible` or `highRisk` of its own. An activity after a loop's
success edge, or anywhere else in the root graph, stays unsupported.
An expression error while evaluating the loop's initial carry or condition at entry happens before
any scope or iteration exists, so there is nothing to drain: when the loop declares `onError`, its
pending owner fails with `EXPRESSION_ERROR` (the ordinary `step.failed` event) and the edge is taken
in the same transaction, with the run never asked to stop (D401). A loop without `onError` still
fails the run with `EXPRESSION_ERROR`.

### Bodies that compensate (T014-014..021, D380-D389)

A published collection or loop body is a chain of one to eight activities followed by its `return`
(`MAX_BODY_CHAIN`, an `onError` on a body activity stays refused). Each activity may declare
`compensateWith`, `reconcileWith`, `compensationRetry`, `compensationTimeoutMs`, `irreversible`
and `highRisk` with the meaning they have at the root; `irreversible` and `compensateWith` on one
step are refused as at the root. Branching, `saga`, timers, child workflows and nested scopes inside a
body remain closed.

A scope whose body declares any of `compensateWith`, `irreversible` or `highRisk` is its own saga.
When it stops for a failed iteration, the storage quota, an outer cancellation or a loop timeout, the
active iterations drain as before, and then the iterations that completed (and those whose last
attempt had an unknown outcome) are undone before the scope finishes:

| Where | Order |
| --- | --- |
| Inside one iteration | Reverse dependency order of the body chain: the compensation of a later activity finishes before the one of an earlier activity starts. |
| Between iterations of a collection | None. At most `maxConcurrency` iterations have a compensation in flight. |
| Between iterations of a loop | Last iteration first; iteration `k` starts after the nearest later iteration that had anything to undo has finished. |

The plan is journaled once as `scope.compensation_planned` (the instances to undo, the ones each waits
for, and the effects that cannot be undone) and the end as `scope.compensated` with the outcome
`COMPENSATED`, `PARTIAL` or `COMPENSATION_FAILED`. The scope owner stays `WAITING` on `COLLECTION` or
`LOOP` meanwhile and the run is `COMPENSATING`, or `MANUAL_REVIEW` when a compensation waits for a
person. The run ends with the worse of the stop and the outcome: a failure fully undone is
`COMPENSATED`; a cancellation fully undone is `CANCELLED`; an irreversible effect, an unknown outcome
that nothing could undo, or a compensation skipped by an operator is `PARTIAL`; a compensation that
failed for good is `COMPENSATION_FAILED`. A stop that arrives while the scope compensates changes
none of this, and a compensation that waits for a person keeps waiting through a cancellation (stricter
than the root rule of D66, O381). A loop `onError` edge is taken only after the failure was fully
undone.

A compensation is a worker task of kind `COMPENSATION` with the typed scope identity of the iteration
it undoes (`target_instance`, worker protocol 1.5), the forward node as `targetNode` and the key of
the forward effect as `targetIdempotencyKey`. Its own key is stable across retries and restarts and
derived from the workflow version, the scope path and the forward node. A step whose outcome is
unknown inside an iteration is reconciled by the lookup of that iteration before it is repeated or
compensated; the three verdicts conclude it as at the root. Compensation has the lease, fence, retry,
timeout, breaker and audit of every activity, and a compensation whose budget is spent waits on
`OPERATOR`.

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
or actor identity from the request body. The implemented run Query/Update, chain Query/Update, approval
decision and chain inspect/signal/cancel shapes are mirrored in the canonical OpenAPI. The reviewed
workflow shapes above create chains and hand off through the scheduler. Both stateful shapes
preserve the declared state digest, schema, size and revision in the successor transaction.
Chain Query/Update have seeded PostgreSQL/HTTP evidence. Shared conformance in all five SDKs
covers initial-head Query plus public Query and Update after stateful handoff, exact Update replay
and a stale expected-head Query. The segment list is paged (below). Command races against the
roll-over, a boundary that waits for open work, child continuation chains and the shared
three-segment and child scenarios have PostgreSQL, fault and five-SDK evidence (see
`research.md`, "Decisions of the chains increment"). A continuation whose entrypoint is a
`waitEvent` and wider continuation graphs remain gated.

| Method/path | Body/query | Success result / authorization |
| --- | --- | --- |
| `GET /runs/{runId}/approvals` | bounded cursor/page size | Requests visible under `run:read` + `approval:read`; omit policy/actor details without their permission. |
| `POST /approvals/{approvalId}/decisions` | `decisionKey`, `decision: APPROVE|REJECT`, optional `reasonCode` | 200 durable decision receipt; requires `approval:decide` plus current policy eligibility. |
| `POST /runs/{runId}/queries/{name}` | schema-checked `args` | 200 `{runId,result,runVersion,stateRevision}`; requires `run:read` + `run:query` for the named operation. |
| `POST /runs/{runId}/updates/{name}` | `updateKey`, `args`, optional `expectedStateRevision` | 200 APPLIED receipt/result; 422 durable business rejection or 409 durable revision conflict; requires `run:update` for the named operation. |
| `GET /namespaces/{namespaceId}/chains/{chainId}` | none | 200 current head, revision, final outcome and ordered segment links; requires `run:read`. |
| `POST /namespaces/{namespaceId}/chains/{chainId}/signals/{signalName}` | required `expectedRunId`, `payload`; `Idempotency-Key` header | 202 with the root waits woken; requires `run:signal`; a missing matching wait or changed head is 409. Scoped waits remain gated. |
| `POST /namespaces/{namespaceId}/chains/{chainId}/cancel` | required `expectedRunId`, `reason`; `Idempotency-Key` header | 202 durable cancellation or exact replay; requires `run:operate`; latches chain cancellation and cancels the current head atomically. |
| `POST /namespaces/{namespaceId}/chains/{chainId}/queries/{name}` | `args`, optional `expectedRunId` | 200 current-head declared result plus the actual `runId`; requires `run:read` + `run:query`; a changed expected head returns 409. |
| `POST /namespaces/{namespaceId}/chains/{chainId}/updates/{name}` | `updateKey`, `args`, optional `expectedRunId` and `expectedStateRevision` | 200 current-head APPLIED receipt/result; durable 422 rejection or 409 revision conflict; requires `run:update`. Exact receipt replay stays bound to its original resolved run after rollover. |
| `GET /namespaces/{namespaceId}/chains/{chainId}/runs` | optional `cursor`, `limit` (1..200, default 50, clamped) | 200 `{items, nextCursor}` in segment order, each item `{index, runId, status, completionKind, predecessorRunId, successorRunId, createdAt, finishedAt}` and no input, output or state; the cursor is opaque and append-stable; requires `run:read`; a foreign or missing chain is the same 404 (with the restricted denial audit); a malformed cursor or limit is 400; never cached. |

**Operator decisions inside a scope.** `POST /runs/{runId}/operations` accepts an optional
`instancePath` (the scopes around the step, outermost first, each with its iteration, as the runtime
graph reports them) for `RETRY_STEP`, `RETRY_COMPENSATION`, `SKIP_COMPENSATION`, `RESOLVE_STEP` and
`MARK_MANUAL_REVIEW`. `stepId` is the local node (`reserve~c` for the compensation of `reserve`).
The request is refused with `VALIDATION_FAILED` for `CANCEL`, `PAUSE`, `RESUME` and `RERUN`, which
apply to the whole run. Permissions, the reason, the idempotency key, the audit record and the
`409` rules are those of a root decision (`docs/runbooks/sagas.md`).

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
| Existing SDK completion wait | Stop when that run becomes terminal; the run read carries `chain` (`SUCCEEDED` with `completionKind` `CONTINUED` on handoff, additive and absent outside a chain) | New `waitForChainCompletion` follows authorized links until FINAL; timeout only stops client waiting. |
| Signal | Existing exact-run behavior; continued terminal run is not forwarded | Resolve the head and apply the existing signal rule in the same transaction: a waiting step consumes it, otherwise 409 `STATE_CONFLICT`; it is never held for a successor. |
| Update | Run-scoped key; a new operation on continued run returns RUN_CONTINUED | Chain-scoped key returns original applied/rejected receipt across rollover, never applies twice on different segments. |
| Cancel | Cancels only that nonterminal run; a terminal old segment is unchanged | Sets chain cancel latch and cancels current run; no later continuation can pass the latch. |
| Child completion | Parent's child instance follows its child chain | Intermediate continued segments emit no parent completion; final outcome is applied once. |

`waitForChainCompletion` is available in all five SDKs and polls authorized chain snapshots until
`finalOutcome` is present; its timeout only stops the client wait. Existing `waitForCompletion`
behavior and polling helpers remain segment waits. Documentation must not label
an old run's SUCCEEDED as final chain success. No automatic redirect on a 409 or hidden retry with a
new key. Signal and cancel require `expectedRunId`, so a delayed caller never applies its command
to a replacement head. Chain Query and Update make the precondition optional because each locks the
chain before resolving one head. For Update, the optional value is part of the canonical request
hash: exact replay uses the same presence/value and returns the original receipt across rollover;
a new key with an obsolete value fails with `CHAIN_HEAD_CHANGED`.

Chain signal/cancel use a mandatory 8..160 printable ASCII `Idempotency-Key`, scoped by
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

A continuation workflow can be started by a `child` node (D301; it was refused with `INTERNAL` before
this increment). The chain's runs are created settled, so only the link tells the parent, once, and only
with the chain's final outcome; a continued segment tells it nothing. The chain keeps the child's deadline
(the smaller of the parent's, the node's and the chain's required timeout) as its original one. Cancelling the
parent (or settling its step under a cancelling policy) latches the link under the parent's lock only; the
hand-off reads the latch under the chain, head-run and link locks and refuses, and the scheduler cancels
the chain's current head. A hand-off locks the parent first (key share), then the chain and the head run, the
order every writer of a child uses. The successors are runs of the parent's tree and spend its
1,024-run budget; a full tree ends the chain with the durable `CHAIN_LIMIT`, as the segment limit does.

A continuation boundary that finds open work waits (D296): a running or ready step, a timer, a subscription,
an approval, an unsettled child, an unknown effect or an unfinished compensation. It is retried until the work
settles or the original deadline ends the chain `TIMED_OUT`; nothing about the wait is stored and no error is
returned to any caller. A namespace or tenant that has no room for the successor makes it wait the same way:
the successor replaces the predecessor and never needs more room than the predecessor held.

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
| Combination accepted by the design whose prerequisite is not built (an approval in a collection or loop body, D295) | — | `VALIDATION_FAILED` (422) | `UNSUPPORTED_NODE` (existing), plus `details.dependency` = `SCOPED_COMPENSATION`, `details.node`, `details.kind` | false |
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
| Query/Update arguments do not match the declared argument schema | — | `VALIDATION_FAILED` (422) | `INVALID_ARGUMENTS` | false |
| `updateKey` outside 1..128 printable ASCII | — | `VALIDATION_FAILED` (422) | `INVALID_UPDATE_KEY` | false |
| `decisionKey` outside 1..128 permitted ASCII | — | `VALIDATION_FAILED` (422) | `INVALID_DECISION_KEY` | false |
| `reasonCode` outside 1..64 machine characters, or free text | — | `VALIDATION_FAILED` (422) | `INVALID_REASON_CODE` | false |
| Mutation targets an ineligible state or an old segment | — | `CONFLICT` (409) | `RUN_NOT_MUTABLE`; `RUN_CONTINUED` | false |
| Signal to a head that is not waiting for it | — | `CONFLICT` (409) | `STATE_CONFLICT` (existing) | false |
| Boundary reached with unresolved obligations | — (the boundary waits, D296; `CONTINUATION_NOT_QUIESCENT` is a scheduler wait reason, never a stored failure) | — | — | — |
| Rate or temporary capacity | — | `RATE_LIMITED` / `QUOTA_EXCEEDED` (429, Retry-After) | existing | true; reuse the same key |
| Worker or scheduler no longer owns the attempt/transition | existing stale-fence handling | existing worker error | — | false |

`tests/contract/workflow-primitives/errors.rs` reads this table: every reason it introduces has a
specific remediation row and is named in the OpenAPI `Error.details` description (D292).

Workflow deadlines use existing TIMED_OUT behavior; request wait deadlines do not implicitly cancel
runs/updates. T014-006 adds the new reasons and their remediation rows to the canonical OpenAPI
and to `crates/api-http/src/error.rs` together, and checks each mapping against the existing ones.

## 6. Telemetry and audit

OTel span design: `workflow.scope.admit`, `workflow.iteration.commit`, `workflow.approval.decide`,
`workflow.query`, `workflow.update`, `workflow.continue`, `workflow.switch.evaluate` and
`workflow.unsupported`. Link predecessor/successor traces and retain
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
Scoped graph nodes use a stable `scopeNode[index]/localNode` id and carry `nodeId` plus an ordered
`instancePath`; only persisted scoped instances appear. The journal change feed identifies the
complete path of each touched instance, so a change after scoped work is a precise delta (`reset`
stays false) and clients never apply a local node update to the wrong iteration.
Routes are edges: an approval node has `approve`, `reject` and `expire` edges to its three pinned
targets, and its `approvalDecision` (`{kind, target}`; `PENDING`, `APPROVED`, `REJECTED`, `EXPIRED`
or `CANCELLED`) says which one the recorded outcome took. It never names the approver, the decision
key or the reason code. A collection or loop owner has a `begin` edge to the entry step of each
admitted iteration (`items[2]/work`), and the body's own edges are repeated per iteration.
A journal entry of a kind the projector does not know is not dropped silently and does not stop the
run's projection: it becomes one `workflow.unsupported` span and one
`workflow_primitive_transitions_total{kind="unsupported",outcome="unsupported"}` increment. The span
carries the run, the scope path and the sequence-derived span id, never the unknown event's type
or payload.
When chain metadata has been initialized, graph and changes responses include `chainId`,
`segmentIndex`, `segmentCount`, `currentRunId`, predecessor/successor run IDs, completion kind and
final outcome in `chain`. A run without retained chain metadata reports `chain: null`.

Durable audit covers opening/deciding/expiring approval, decision denial, query access, update
application/rejection, continuation, cancel/manual decisions and quota/policy changes. Workflow
mutations and their audits are atomic; query/security-denial audits use the existing access/security
audit path. A failed audit write fails the operation closed. Retain only subject ids, operation kind,
resource refs, outcome/code, time and trace correlation under existing audit access policy.
