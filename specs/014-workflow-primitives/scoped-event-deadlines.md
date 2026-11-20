# Qualified event deadline processing checkpoint

Refs: T014-014–017/040, FR-014-004/005/014/015/017. This connects current collection
subscriptions to automatic deadline discovery and native timeout processing. It does not
activate public wait bodies, external delivery, loops/nesting or released-reader rollout.

The existing maintenance deadline port returns StepRef, already capable of exact paths.
Select both persisted path arrays and decode with addressed_step_ref; preserve shard/tenant
exclusion, due ordering and bounded pages. Remove the root-only filter only together with the
typed scheduler consumer. Deduplicate/fairly select by (run, complete instance), never (run,node).
Root candidates retain the legacy root timeout command. Nonroot candidates require a lease
and use exact bounded entry plus the accepted time_out_event_wait_path_instance command.
A placement-free scheduler must count a fenced scoped action, never invent clock authority.

Discovery grants no authority. Reopen current path revision and own Run; acknowledge current
own shard/time, validate exact pin/binding/physical identity and recheck the deadline after
locking. Preserve existing pause semantics: record due outcomes while paused, defer local
routing, and wait until resume before treating an unrouted failed wait as an unhandled
collection failure. Defer every valid unrouted Failed event wait from collection fail-fast
while no Run stop is committed; the exact-path forward-eligibility guard keeps its route
waiting under pause. No new frontier status field is needed. After resume, release the
deferred outcome: a declared onError takes its local handler, while without that handler
StepRouted still leaves the domain failure unhandled for the next normal fail-fast decision.
The domain failure predicate does not change. Other active failures still take the normal
collection fail-fast path; this adds no sibling-read authority to the accepted timeout command.
A committed global stop, ancestor drain or prior event outcome prevents timeout. An unlatched
sibling failure alone is not a new timeout fence; the committed scope/global closure is.
Use existing atomic native effects,
typed errors, current quota recovery and scheduler metrics; no worker lease represents a wait.

Tests use real admitted worker → scheduler start, actual due-clock predicates, qualified
candidate pages/exclusion and actual Scheduler ticks under sagat_app. Prove two equal local
names survive discovery/selection, handled/unhandled paused outcomes and resume, current
fences/revision changes, cold restart/duplicate ticks and legacy root compatibility. Existing
held Run event/timeout/cancel native tests remain the underlying write-boundary proof; do not
claim that those alone prove the new discovery/consumer. No canonical release task closes here.
