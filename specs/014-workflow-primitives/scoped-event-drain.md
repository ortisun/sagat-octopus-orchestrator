# Scoped event-wait collection drain checkpoint

Refs: T014-014–017/040, FR-014-004/005/014/015/017. This bounded checkpoint follows native
start/timeout storage. It does not activate public bodies, event delivery or general nesting.

There are three independent eligibility planes: the pure CollectionIterationDrained reducer,
both full/frontier scheduler drain predicates, and PgCollectionStore::drain_collection_iteration.
All must recognize a well-formed WaitingOn::Event suspension as owning no worker. Invalid
attempt/generation/cancel/retry/control/routed/timed-out/failure metadata must remain refused.
The domain intrinsic suspension predicate needs no definition; terminal timeout policy still
uses the pinned definition in the existing complete projection validator. Scheduler predicates
additionally check the pinned WaitEvent kind and keep the existing collection/stop/chain fences.

The native coordinator keeps chain → Run → current own shard → existing quota/scope/member
lock order, expected Run revision and current drain reason. Read the current complete member
rows under FOR UPDATE in the same transaction. Validate every pinned event-wait row (including
Pending and terminal) with the shared logical/physical predicate and canonical namespace/UUID.
These locked rows are its authoritative current read set; the port does not accept a stale
event candidate prepared outside the Run lock. No new public port fields, schema or grants.

For a waiting event, perform an exact guarded update with the locked old status/routed,
namespace/UUID/path/node and event type/key/deadline/since plus the shared physical predicate.
Set Cancelled/routed and clear waiting/deadline/type/key/since/scan/retry/cancel fields. A scan
marker is optimization, not binding authority. Require exactly one row updated. Preserve the
existing bulk drain behavior for other supported states, adding subscription cleanup. Do not
manufacture a generic failure from an already waiting event entry: such an inconsistent
failure-entry source is refused. Existing valid Pending failure-entry handling is unchanged.

Journal, projections, retired reservations, run/tenant active slots, telemetry and outbox commit
or roll back together. Retain existing compensation/unknown-outcome ownership rules; never
release a slot that still owns unresolved worker/child/approval work. A timeout committed first
retains its outcome; cancellation committed first prevents a later timeout. Use actual Run lock
barriers and PostgreSQL wait observation, not sleeps. Current native fixtures use an immutable
owner-inserted structured-codec version and actual admitted activity → wait routing; they do not
prove public publishing, old-dispatcher cutover or released-profile compatibility.

Required evidence: actual domain and native REDs; cancellation and fail-fast sibling drain;
full/frontier parity; complete binding cleanup and sibling isolation; current authority/revision
and corrupted metadata refusal; rollback after projection/journal work; replay; both timeout/
cancel lock orders; no new worker work and correct retention/reservation accounting. Keep these
canonical tasks unchecked until the remaining receiver/nesting/resource/release gates pass.
