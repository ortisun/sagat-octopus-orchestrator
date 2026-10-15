# Data model: durable primitive proposal

**Status**: Proposed. No tables, events, SDK methods or new runtime guarantees are implemented by
this document. Field/constraint names below are the design contract for future tasks.

## 1. Definitions and bounded values

Wire documents remain adapter types; the domain uses typed IDs, bounded values and expression
interfaces. A typed business-value tree may represent state, but domain rules must not depend on
`serde_json`. Reuse existing tenant, namespace, run, node, version, deadline and failure types.

| Entity | Fields / relationships | Validation rules |
| --- | --- | --- |
| PrimitiveDefinition | Pinned IR revision, node id, kind, body reference, output/carry expressions, limit policy | `nodeId` remains 1..64 permitted ASCII characters; each scope is structurally acyclic; body references cannot form recursion. |
| BodyDefinition | Local nodes, entrypoint, explicit return boundary, optional nested scope | Body has one entry and one logical return; no edge enters from or exits to another body; workflow `end` and `continueAsNew` are forbidden inside bodies. Local node count contributes to the existing 10,000 definition-node ceiling. |
| PrimitiveLimits | maxItems, maxConcurrency, maxIterations, maxInstances, maxDepth, timeoutMs | `maxItems` and `maxIterations` are integers in 1..10000; `maxConcurrency` is 1..256; `maxInstances` is 1..100000; `maxDepth` is 1..8; `timeoutMs` is 1..31622400000. Required applicable limits cannot be omitted. Tenant policy may lower them. |
| SwitchCase | Stable case id, predicate expression, target node | 1..32 ordered cases; case ids unique within switch; mandatory default; existing 2048-byte expression limit applies. |
| StateDefinition | schema, initial expression, Query/Update declarations | At most 32 queries and 32 updates; names use existing 1..128 activity-name rules; state and operation result values are at most 256 KiB each. |
| ApprovalPolicy | subject ids and built-in role names, excludeInitiator, timeout | Between 1 and 32 total subjects/roles; subjects are members of the run's tenant/namespace; roles are only `admin`, `developer` or `operator` (never `viewer`, `worker`, `runner`). Custom roles and identity-provider groups do not exist (G39, feature 013), so a narrower approver set lists subjects. `excludeInitiator` defaults true. No dynamic arbitrary role-expression callback. |
| ChainPolicy | maxSegments, timeoutMs | `maxSegments` is 1..10000, including the initial run; `timeoutMs` is 1..31622400000; both required on definitions containing continuation. |

The 256 KiB value / 4 MiB retained per-run / 8 MiB task-context limits continue to apply in addition
to item counts. Snapshot, carry, outputs, update state/receipts and compensation inputs are charged;
no implicit exemption for nested scopes. Blob references keep immutable digest, length and schema
association and require authorized run binding. Collection input is a bounded array: no iterator or
external network fetch in the scheduler. Reading a captured blob through a bounded adapter does not
change membership; digest/schema validation precedes admission.

## 2. Instance identity and scope state

**ExecutionInstance**: `(tenantId, namespaceId, runId, instancePath, definitionNodeId)` is unique.
`instancePath` is a typed sequence of `(scopeNodeId, iterationIndex)` pairs; root path is empty,
indices are zero-based integers in 0..9999 and path depth is at most 8. IDs are not constructed by
concatenating user strings. Retry attempt/generation is a separate dimension and never changes the
logical instance or effect idempotency key. Every lease, result, timer, subscription, compensation
reference and journal event that addresses a dynamic node carries the instance identity.

**IterationScope**: owner instance, kind (`COLLECTION`, `WHILE`, `REPEAT_UNTIL`), lifecycle,
captured input/state reference, cardinality, admitted cursor, completed count, next iteration index,
effective limits, original deadline, reserved slots and ordered result references. Collection
membership is immutable after `ScopeEntered`; duplicate equal item values remain distinct indices.
Only admitted iterations allocate work. Unique `(scopeInstanceId, iterationIndex)` prevents duplicate
admission. Scope lifecycle is `OPEN -> DRAINING -> SUCCEEDED|FAILED|CANCELLED`; `OPEN` may transition
directly to success for an empty collection/false while. DRAINING never admits new iterations.

**IterationCheckpoint**: scope/index, input reference, terminal body outcome, body result, carried
state and condition result, committed journal sequence, return instance. One checkpoint per index.
For `while`, pre-test false ends with initial/current carry; for `repeatUntil`, post-test true ends
with new carry. At each completed body evaluate `carry` against previous carry/body result, then
predicate against new carry. For while the predicate is checked before every body; repeatUntil only
after each body. Carry and condition values are persisted in one transition so restart cannot use
a newer state to change the route. Conditions must return boolean; errors fail with EXPRESSION_ERROR.

Iterations run sequentially for loops. An optional durable delay applies only when another iteration
is needed, after its continuation decision and before that next body's admission. Check the limit
before scheduling the delay: completing iteration `maxIterations-1` and needing another produces
ITERATION_LIMIT. The deadline competes with transitions under the same run lock; timeout first
commit wins, cancels later admission and begins normal drain/compensation.

Collection slots cover active iteration instances, including waits/retries and unreconciled unknown
outcomes. Admission requires an available scope slot **and** tenant-wide active-iteration/task
capacity across all runs. Empty items do not consume a slot. Temporary capacity pressure leaves
work durably waiting for fair scheduler admission; declared hard budget exhaustion fails before
admission with the typed budget error. Reclaim only terminal/drained slots, transactionally.
A policy reduction stops new admissions above its new ceiling but does not kill accepted work.

**EffectRecord**: existing effect identity extended by full instance identity and committed
completion sequence. Compensation keeps the existing rule (FR-003-002, ADR-0005,
`saga::reverse_dependencies`): reverse topological order that respects branch independence, not
reverse completion order. Applied to scopes: within one iteration, the body's reverse topological
order; between iterations of a collection, no order, because iterations are independent branches
(they may compensate concurrently within `maxConcurrency`); between iterations of a sequential loop,
reverse iteration order, because iteration `k+1` depends on the carry of `k`. This refinement of
ADR-0005 is accepted in T014-002. Unknown effects reconcile before retry. A parent scope cannot
claim drain complete while a child instance still has an unknown effect or unresolved compensation.
Failed compensation retains MANUAL_REVIEW/PARTIAL semantics; a loop failure cannot overwrite it with
a simpler FAILED outcome.

## 3. Approval request and receipt

**ApprovalRequest**: opaque request id, tenant/namespace/run/instance, pinned policy, initiator subject,
opened sequence, absolute deadline, status (`PENDING`, `APPROVED`, `REJECTED`, `EXPIRED`, `CANCELLED`),
winning receipt id and outcome sequence. Unique owner instance. No process/worker is reserved.

**ApprovalDecisionReceipt**: request id, decision key, canonical request hash, actor id, decision
(`APPROVE`, `REJECT`), reason code and result. `decisionKey` is 1..128 permitted ASCII characters;
`reasonCode` is optional 1..64 permitted ASCII characters; free-text reasons are not accepted.
Unique `(tenantId, requestId, decisionKey)`. Actor identity is read from auth, never accepted from a
body. Identical key/body returns the same authorized receipt; changed body returns `CONFLICT` with
reason `IDEMPOTENCY_KEY_REUSED`. A different key after a winning decision returns `CONFLICT` with
reason `ALREADY_DECIDED`. Unauthorized requests create redacted security audit evidence but no
decision receipt.

The initiator is the run's recorded `startedBy` (for a continuation successor, the chain's original
one). When it is a machine identity (a trigger, a worker), exclusion removes no person; documentation
says so rather than implying separation of duties for machine-started runs.

At activation compute deadline once from committed time and policy. It makes timer expiry eligible;
it does not reorder a previously racing decision transaction. Response, expiry and cancellation
serialize on the request/run and transition out of PENDING once. A losing internal expiry is an
audited no-op; an external conflicting response gets the typed conflict. Approval/rejection/expiry
output carries only outcome, decision timestamp, actor id when authorized and reasonCode, all under
its declared schema. It never includes credentials, memberships or full policy evaluation details.

## 4. Workflow business state and operations

**WorkflowState**: owner run, immutable definition state schema, initialized value/reference,
state revision and the run version (`Run.version`, the existing count of applied journal events)
of its last change. Initialization is atomic with run start. State revision starts at 0 and
increments exactly once per successful Update. In the first increment only an Update changes
declared state: workflow nodes read `state` in their expressions but cannot assign it, and no node
waits on a state predicate. Both are pending decisions, not implied features. `expectedStateRevision`
is the Update's optimistic-concurrency token. It conflicts only with another Update, unlike the run
version, which every activity completion and timer moves.

**Value disclosure**: the API returns no payload value today; ADR-0028 allows only authorized
structural inspection and requires a new decision for any value disclosure. Query results, Update
results and receipt replays disclose declared values, so US06 requires that decision (T014-002)
before release. It names the permission, the audit and the redaction that apply.

**QueryDefinition**: name, argument schema, result schema, expression. Query evaluates against one
consistent persisted snapshot with `{input, state, completedResults, args}`; secrets, live leases,
credentials and non-authorized payload fields are unavailable. Returns result, `runId`,
`runVersion` and `stateRevision`; it never updates workflow journal/state. Authorized query access
is audited in an access-audit record, with no arguments/results. Query on retained terminal state
is allowed; removed state returns `NOT_FOUND`. Limits/schema failures return typed errors.

**UpdateDefinition**: name, argument schema, optional validator expression (default true), state
expression, result expression, bounded declared-state schema. Evaluation order: validate arguments,
check expected state revision, evaluate validator, compute new state, validate state, compute result
and validate result. Expressions see the same pre-state and arguments; result may explicitly
reference `newState`. They cannot call activities, invoke children, wait or cause external effects.

**UpdateReceipt**: tenant/namespace, address scope (`RUN` or `CHAIN`), target id, operation name,
`updateKey`, canonical request hash, actual applied run id, status (`APPLIED`, `REJECTED`), rejection
code or result reference, before/after state revision, run version at commit and actor id.
`updateKey` is 1..128 permitted ASCII characters; expected state revision is an optional
nonnegative integer. The hash includes operation, arguments, expected state revision and expected
current run id, excluding trace/request ids.
Unique `(tenantId, namespaceId, addressScope, targetId, updateKey)`; reusing a key for another
operation also conflicts. Schema/authorization/rate failures before operation admission have no
receipt; admitted business-validator/revision rejections have stable rejection receipts and audits.
Transient infrastructure failure leaves no committed receipt and is retryable with the same key.

New Update operations are allowed only in RUNNING/WAITING. PAUSED, COMPENSATING, MANUAL_REVIEW,
CANCELLING and terminal states reject new operations. An authorized replay of an already stored
receipt is returned regardless of later run state. A successful update emits `StateUpdated` and
commits all records together; it creates no task, timer or subscription. A rejected update changes no
business state and emits no workflow state event, but its receipt/audit is durable. No intermediate
ACCEPTED state is exposed: an operation is atomically APPLIED or REJECTED.

## 5. Execution chain and continuation

**ExecutionChain**: tenant/namespace, `chainId` (initial run id), current run id, segment count,
maxSegments, original chain deadline, current/final outcome, cancel-requested flag and revision.
Unique chain head; every run has additive chain metadata, with singleton behavior when it never
continues. Retain the definition version and initiating identity. Chain lifetime/admission charges
are separate from per-run retained-data limits; continuation cannot renew the original deadline or
reset tenant usage/rate/retention counters.

**ChainSegment**: chain id, segment index (0..9999), run id, predecessor id, optional successor id,
completion kind (`FINAL`, `CONTINUED`), explicit transfer references and parent-child link if any.
Unique `(tenantId, chainId, segmentIndex)`, unique predecessor and at most one successor per run.
A continued old run is `SUCCEEDED` with `completionKind=CONTINUED`; final success is SUCCEEDED/FINAL.
Other terminal outcomes are FINAL. This distinguishes segment completion from process completion.

A continuation boundary is legal only in the top-level workflow graph, with no unfinished sibling
scope, activity lease, child, timer, subscription, approval, unknown external outcome or
compensation obligation. There is no pending-message state to drain: a signal is accepted only by a
step already waiting for it and consumed in that transaction (`signalRun` in the canonical OpenAPI),
so an open wait is the only message obligation and it already blocks the boundary. It is outside
saga bodies. A completed saga with no remaining obligations is allowed. Normal finalization rules
ensure abandoned branches are drained before this check; no implicit cancel-and-continue shortcut
exists.

Transfer expressions select successor input and state; their schemas, sizes, blobs and authorization
are validated before commit. The successor uses the same immutable definition version. The handoff
records old completion, new run/state/initial work, chain head/revision, lineage, quota accounting,
audit and outbox together. If maxSegments would be exceeded, the boundary fails with CHAIN_LIMIT;
if required quota is temporarily unavailable, it remains durably waiting before handoff; the old run
is not marked complete. Original deadline remains enforceable while waiting.

A parent awaiting a child follows its child's chain to FINAL; intermediate segments never emit
`ChildSucceeded`/`ChildFailed`. Parent cancellation propagates to the chain's current member through
the existing child policy. Child lineage is carried atomically; the final chain outcome notifies the
parent once under the original parent child-instance identity.

Tree budget: every run with the same root counts against the existing child-run tree budget
(`MAX_TREE_RUNS` = 1,024, all runs ever created under the root, depth at most 8). A successor of a
continuing child is a new run under the parent's root, so a child chain's segments consume that
budget; reaching it fails the boundary with the existing tree-limit failure, whatever `maxSegments`
allows. A successor of a root run starts its own tree, because its predecessor is quiescent. Children
started inside collection bodies consume the same budget per iteration.

Chain lineage is distinct from the existing `rerunOf` (RERUN_AS_NEW). A rerun of any segment starts
an independent run with `rerunOf` set and a new singleton chain; it never joins or forks the old chain.

## 6. Transaction boundaries

All rows/FKs/uniqueness include tenant and namespace where applicable. All writes use cancellation-
safe begin, run journal CAS and current scheduler owner/fence; worker completions also validate
lease token, attempt and generation. The transaction includes applicable quota deltas and audits.

| Transition | One transaction MUST contain | Crash/retry invariant |
| --- | --- | --- |
| Enter collection/loop | captured source/carry, scope record, first decision, admitted instances/tasks/timers, journal, outbox | No re-snapshot or duplicate first iteration. |
| Complete iteration / admit next | fenced result, checkpoint, slot release/reservation, scope status, next tasks/delay, journal, outbox | No duplicate result, free slot or next iteration. |
| Choose switch case | selected case/result, chosen successor tasks, journal, outbox | Replay never reruns choice. |
| Open approval | request/policy/deadline, waiting state, timer, journal, audit, outbox | Pending request and expiry cannot be separated. |
| Decide/expire/cancel approval | winner/receipt, audit, timer cancellation, selected successor or run drain, journal, outbox | One winning outcome, losing transition audited. |
| Apply Update | receipt/result, new state/revision, run journal, audit, outbox notification of the change | Lost response can replay the receipt; no result without state; no task is scheduled by it. |
| Reject admitted Update | rejection receipt and audit under same run/chain lock | Stable rejection; no business-state mutation. |
| Continue | predecessor terminal event, successor/start/state/tasks, links/head, parent lineage, transfers, audit, outbox | Zero or one successor; never a terminal predecessor with missing successor. |
| Chain-target command | chain head resolution, dedup receipt/inbox, current-run mutation and all its records | Rollover cannot split resolution from application. |

Lock order is chain (if involved), run ids in stable sorted order, then scope/request/quota resources
in stable key order. Direct-run mutations for a chain member use the same chain-before-run order.
No caller response acknowledges a mutation before commit. Query uses a consistent read snapshot;
a read audit must succeed before returning its data, without adding a workflow transition.

## 7. Retention, migration and recovery

Receipts and chain links remain available through the applicable run/chain retention window;
return `NOT_FOUND` after authorized retention removal, never recreate a prior mutation using
an expired receipt. API docs must state this finite deduplication window; no infinite idempotency
promise. Garbage collection checks live chain, child, saga and blob references before deletion.
Snapshot/checkpoint storage is charged and deleting one segment cannot strand the active chain.

Migration adds nullable/defaulted root identity and completion fields for old rows, tenant-scoped
constraints and new primitive tables. Old journal events replay to the empty root path; never rewrite
history or assign new meaning to old event types. New events are versioned. Rollback disables new
starts/publications but retains compatible readers until all new-revision runs have drained.
