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
binding, payload/classification/quota, receipt/delivery and routing commit. The current dispatcher
WaitingStep/cursor/consumer identity is run+node only: it must retain qualified path identity,
paginate without skipping siblings and deduplicate delivery per exact instance while preserving
root compatibility. No body publication is enabled by merely persisting waiting_kind=EVENT.

Prove event-before-wait/lookback, event/timeout/cancel lock orders, stale prepared state and
post-wait authority, duplicate receipt, rollback/lost acknowledgement, tenant/namespace boundaries,
value redaction, paused durable completion/cold resume and no worker for suspension. Keep frozen
OrdinaryLeafV1 closed and compare full/frontier runtime journals. Existing root waits and their
wire semantics remain unchanged. Native/scoped compensation, general nesting, released-profile
rollout and complete release gates remain separately open.
