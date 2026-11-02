# Scoped event wait storage: start/timeout boundary

Refs: T014-014–017/040, FR-014-004/005/014/015/017. This is a sequence of bounded native
increments, not public activation or a complete event receiver. Actual commands and results
are recorded in [implementation-ledger.md](./implementation-ledger.md).

## Scope and fixture provenance

The first checkpoint persists Pending → Waiting and correlation-start failure of an existing
exact event wait in both full and bounded transactions. The second checkpoint also persists due
timeout closure through the explicit native transaction port. Neither adds a scheduler producer.
Keep public parsing, root-only dispatcher discovery,
legacy consumers and frozen OrdinaryLeafV1 unchanged. Reject native scoped WaitSatisfied until
its own receipt/payload transaction is implemented. No schema, role, wire field or service is added.

Use the existing test-only structured codec and immutable owner-inserted version pattern.
The native fixture admits an ordinary activity entry through real CollectionStore, completes it
through real scoped WorkerService, and reaches a Pending local wait by its actual recorded route.
Never change a published version or relabel a timer row to manufacture that source. Tests using
the internal codec are not public-publishing, released-profile or production-cold-read evidence.
Cold full/path replay in this fixture still uses the explicitly named structured test codec.

## Clock and authority

A start or timeout requires an acknowledged own-run scheduler guard in that same transaction.
A failed correlation start emits StepFailed(EXPRESSION_ERROR) plus routing, not WaitStarted: also
recognize that pinned scoped WaitEvent/previous-Pending shape for this guard. Unrelated failures
and root ingress keep their current boundaries. A refused later own hold clears the acknowledgement.
A successful holds_shard retains its existing SHARE guard and post-lock database-time sample;
a parent creation guard does not authorize this operation. Track this fact on the native full/path
transaction so ignoring a refused hold cannot authorize a clocked event-wait write. The sample
proves authority after its lock wait, not continuous TTL validity through COMMIT. Existing root
wait creation, user ingress, workers and other primitives retain their current boundaries.

The start deadline must equal that own-fenced sample plus the pinned timeout, with checked
arithmetic. Timeout uses the recorded deadline and that own-fenced sample, never host time or
a timestamp from the earlier evaluation snapshot. Acquire chain → own Run → own shard as today;
retain revision/snapshot and physical binding checks, existing quota order and one-run mutation.

## Binding and physical metadata

An event wait never owns an activity, worker lease, retry, hint, child or approval capability.
Validate its complete logical lifecycle and physical metadata in full/path/frontier readers and
in both guarded writers. Share the physical predicates and before-write read state; reject
malformed rows instead of normalizing them. Compare tenant/namespace/run/path/node/stored UUID,
pinned kind, source status/routed and current event type/key/deadline/since with the prepared read.

For this immutable, single-start instance, derive the accepted-event lower bound from
recorded deadline − pinned timeout − pinned lookback, using checked arithmetic. It must match
stored event_since. This binds the physical subscription without adding an empty DTO or loading
journal history. Pure start computes deadline from the same fenced time; all values have the
canonical millisecond/public and microsecond/storage representation. No legal retry or pause
recalculates this bound. A changed binding without a Run revision must not be overwritten.

Pending has no event binding/output/worker metadata. Waiting has the pinned type, exact key,
deadline/lower bound and no output. Timeout and correlation-start failure clear subscription/scan fields;
handled timeout retains its recorded route/flag, unhandled timeout retains WAIT_TIMED_OUT or its
local error route, correlation failure retains EXPRESSION_ERROR. Other failure codes and
unsupported native event-satisfaction/output states remain refused in this boundary. A scan marker
is optimization, not authority; current root-only discovery never creates one for a scoped wait.

Journal, projection, routing, reservation changes, retained usage, telemetry/outbox and optional
audit commit or roll back together through existing native machinery. Suspension creates no
worker task or lease. No external event value, delivery receipt or scoped consumer is introduced.

## Required evidence for each checkpoint

Observe genuine REDs before each production correction. The start checkpoint proves real runtime-role Pending source,
full/path start parity, both same-name siblings and checked lookback binding, explicit
absence of worker work, current own-fence and parent/stale/expired refusal, actual post-lock time,
snapshot CAS plus binding tampering without revision, rollback and replay.
Include refused/skipped own authority on the correlation-failure start shape, not only WaitStarted.
Use held locks/barriers, not sleeps. Preserve root dispatch/control/timer regressions.
Public entry/activity→wait parsing stays closed, and native WaitSatisfied remains rejected.

The timeout checkpoint additionally proves full/path due-time parity, recorded binding CAS,
handled/unhandled/local-error outcomes, subscription/scan cleanup, paused deferred route and cold
resume, current post-lock clock/authority and atomic rollback. A start pass does not close these
proofs. External event/timeout/cancel races belong to the later receiver/drain checkpoints.
Paused cold routing here uses an explicit native path command after the actual audited Resume;
it does not establish automatic scheduler discovery of a scoped event wait.

## Collection drain checkpoint

The later bounded [collection drain checkpoint](./scoped-event-drain.md) extends the reducer,
both scheduler eligibility guards and the independent PgCollectionStore writer together. It
validates the current locked binding and deterministic identity, clears since/scan with the
subscription, retires reservations atomically and preserves the timeout-first outcome. Real
cancellation-first/timeout-first Run-lock barriers, sibling failure, native rollback and
full/frontier journal parity are recorded in the ledger. This goes beyond projection formatting
or pure StepCancelled replay. No public body, loop event drain or external receiver is activated.

The later receiver still needs exact consumer/receipt identity, current ingress/own-run authority,
payload classification/retention/quota, lookback and real event/timeout/cancel lock orders. Production
scoped EVENT row activation additionally requires the enforceable old-dispatcher cutover and actual
released-profile barrier described in scoped-event-waits.md. These remain release blockers.
