# Scoped event waits — staged 0.3 boundary

Refs: T014-014–017/040, FR-014-004/005/014/015/016/017. Public event-wait bodies stay
closed until the native delivery and five-SDK acceptance gates pass.

The first pure increment records WaitStarted, WaitSatisfied and WaitTimedOut against an exact
member in both full and bounded replay. Start records its correlation/deadline without a worker;
event and timeout are mutually exclusive terminal outcomes. A handled timeout chooses the local
onTimeout edge, normal satisfaction chooses next, and an unhandled timeout records WAIT_TIMED_OUT.
The ordinary planner has no correlation evaluator/clock and leaves Pending waits to a dedicated
start action. Recorded replay is not itself authority to accept an external delivery or timeout.

Before native activation, preserve the exact pinned kind, complete path, evaluation snapshot,
current Run/shard/ancestor authority and database time through atomic journal/projection, event
binding, payload/classification/quota, receipt/delivery and routing commit. Qualified candidate preparation below retains the complete path and physical UUID, while
receipt construction and native discovery stay root-only. The future receiver must deduplicate
delivery per exact instance while preserving root compatibility. No body publication is enabled by merely persisting waiting_kind=EVENT.

Prove event-before-wait/lookback, event/timeout/cancel lock orders, stale prepared state and
post-wait authority, duplicate receipt, rollback/lost acknowledgement, tenant/namespace boundaries,
value redaction, paused durable completion/cold resume and no worker for suspension. Keep frozen
OrdinaryLeafV1 closed and compare full/frontier runtime journals. Existing root waits and their
wire semantics remain unchanged. Native/scoped compensation, general nesting, released-profile
rollout and complete release gates remain separately open.

## Qualified dispatch candidate preparation

WaitingStep carries the existing shared ExecutionInstanceId plus the stored step UUID,
keeping tenant/namespace/run/event type/correlation/since. The dispatcher cursor uses
(run,instance), ordered in SQL by run, path-node array, path-index array and definition node;
the same tuple controls ORDER BY and the strict after predicate. This is database tuple
ordering, not the derived Rust ExecutionInstanceId::Ord (which interleaves path segments).
No caller or test double may reorder a returned page using the Rust order. No full history is loaded.
The scan marker includes tenant_id, namespace, run, node, both path arrays, stored UUID,
status=WAITING, waiting_kind=EVENT, event type/correlation/since and event_scanned_at IS NULL.
It preserves sibling markers and refuses a stale or foreign candidate. Tenant identity is
explicit even when another tenant reuses UUIDs.

Legacy root delivery consumers stay byte-for-byte wait:{run}/{node}; trigger keys and receipt
metadata stay unchanged. DeliveryRef::to_wait now returns None for nonroot candidates, and
undelivered_event_for refuses them without querying the inbox. Scheduler wake refuses them
before opening its root RunTx and verifies a root candidate's UUID against the transaction's
pinned version/run/instance before any receipt/output. Candidate identity grants no execution
authority. Scan marker comparison includes the selected stored UUID and exact path arrays.

The future scoped receiver and receipt writer must introduce the disjoint bounded namespace
wait-instance:{run}/{stepUuid} together, validate its metadata against the pinned current own
snapshot, and atomically retain the existing instance UUID/path columns in event_deliveries.
No scoped consumer or receipt metadata is constructed by this preparation increment.

The preceding preparation guard restricts native discovery to root paths and adds no scoped
receiver. Both waits_for_event and unscanned_waits require empty path arrays;
mark_wait_scanned keeps the same guard and compares the selected event binding and namespace.
Native tests prove injected scoped rows neither consume a root cursor page nor receive scan
markers. The legitimate root is marked; each independently changed type/key/since and foreign
tenant/namespace candidate is inert. These are storage sentinels, not an executable scoped wait.
This prevents projecting a qualified candidate through the existing root-only wake command.
Only the later proven scoped receiver may widen discovery. External event completion follows
the existing ingress/own-Run receipt authority; scheduler-owned start/timeout additionally
need current shard authority. Applicable ancestor/stop/group eligibility still guards routing.
No invented claim of dispatcher shard ownership is introduced.

Before enabling scoped event publication, fence/drain old execution dispatchers as well as old
scheduler authority: the released run+node matcher does not distinguish scoped waits. Root
consumer compatibility alone is not mixed-version safety for a new body shape. Actual
released-profile active waits/rollout and all remaining delivery/value/clock/race proofs stay open.

The rollout prerequisite remains an unimplemented release blocker, not a guarantee supplied
by this identity port. Inbox claims have no dispatcher generation/token, so scheduler fencing
or waiting one claim TTL cannot prove an old dispatcher stopped. Before any scoped EVENT row
may be created, a separate cold-cutover/activation proof must stop and join the actual old
dispatcher processes while holding an in-flight claim and prevent restart with their old
configuration/credentials (or use a separately proven compatible activation fence). It must
show the retained event is delivered once by the new receiver after restart. No such runtime
fence/test is implemented here. Scoped publication and creation remain closed until that
mechanism and actual released-profile barrier pass; manual wording alone is not acceptance.


## Pure explicit-clock start command

start_event_wait_instance and its exact-path counterpart accept the evaluated correlation
result and an explicit instant. They require the exact nonroot declared WaitEvent in pristine
Pending state and current global/ancestor forward eligibility. A successful evaluation records
the key and now+the pinned timeout. An expression failure records EXPRESSION_ERROR and the
existing local error route (including the empty route for an unhandled failure). No sibling
key/deadline or output is changed. Duplicate start, pause, stop/drain, cancellation, wrong
kind/path, malformed Pending metadata and clock overflow are refused before recording.

This is pure command preparation only: its caller must prove evaluation snapshot/CAS, native
scheduler authority and the database clock before atomic journal/subscription persistence.
It creates no subscription, event_since/scanned metadata, task, receipt or external authority.
No native caller or public body capability is added. Native timeout and external event acceptance,
lookback, payload/classification/quota, rollback and mixed-version activation remain pending.


## Pure explicit-clock timeout command

The full and exact-path timeout commands require an exact declared event wait with its
recorded Event binding, a pristine Waiting state and active global/ancestor outcome eligibility.
They compare explicit now with the recorded deadline and derive handled only from the pinned
onTimeout declaration. An early timeout is inert. A handled timeout selects its local timeout
route; an unhandled timeout records WAIT_TIMED_OUT and the normal local error route, if any.
Paused runs may record the timeout outcome; routing remains deferred until explicit resume.
Completed, cancelled, stopped/draining, malformed, wrong-kind or foreign-path sources are inert.

This retains root first-commit semantics: an event may win after the deadline until timeout
commits. The pure recorded-event/timeout ordering tests are not a PostgreSQL race proof or
authority to retain an external event. Native timeout production still needs current own shard,
post-wait database time, revision/physical binding CAS, subscription update and atomic routing;
the future receiver needs its separate current ingress/own-run authority and payload receipt.
No new native caller, public capability, scoped consumer or mixed-version activation is enabled.


## Exact-context correlation evaluation

The additive ExpressionEvaluator correlation_key_instance port preserves root delegation;
legacy adapters refuse nonroot calls until they implement the new method. JsonEvaluator uses
the existing bounded canonical environment for the exact instance path. Collection item/index,
loop carry/iteration and local step outputs use their own path; a same-name root or sibling
output is never fallback. Existing scalar conversion and 256-byte/control-character correlation
key validation are unchanged, including UTF-8 byte limits and numeric/boolean keys.

Evaluation remains lazy over requested variable roots. Constants and explicit own run
input/state/id can evaluate without scope_inputs; that result grants no membership or execution
authority. Requested item/index/carry/iteration must exist in the proper context. The future
native caller must validate membership, immutable captured input, snapshot revision, current
scheduler authority and post-wait database time before invoking the pure start command.
No native caller or public shape is enabled by the evaluator port. Invalid computed lookup
values retain the interpreter's existing canonical masking; no separate logging path is added.

## Native lifecycle checkpoints

The subsequent explicit native transaction ports now persist scoped wait start and timeout with
own-shard post-lock database time and exact binding CAS; the collection drain coordinator also
disposes those bindings with canonical identity checks and atomic reservation/slot retirement.
These use the named immutable structured-codec fixture and real admission/activity completion.
The lifecycle gate covers start, timeout, pause/cold explicit routing, malformed source, rollback,
user cancellation, fail-fast sibling drain, both timeout/cancel Run-lock orders and full/frontier
journal parity. See [storage](./scoped-event-storage.md), [drain](./scoped-event-drain.md) and the
[command/results ledger](./implementation-0.3.md) for exact evidence and limitations.

No dispatcher candidate discovery, scoped receipt/payload receiver, public body, SDK capability
or automatic event-wait scheduler producer is enabled by these checkpoints. The enforceable
old-dispatcher cutover, actual released-profile proof and final resource/SDK/release gates remain
required before production scoped EVENT activation.
