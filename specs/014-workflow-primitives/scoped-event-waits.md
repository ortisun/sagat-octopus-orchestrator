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
