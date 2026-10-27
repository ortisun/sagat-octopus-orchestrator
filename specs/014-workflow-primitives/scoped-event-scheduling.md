# Scoped event collection scheduling checkpoint

Refs: T014-014–017/040, FR-014-004/005/014/015/017. This bounded decision connects
collection scheduling to accepted native start and deferred-routing commands. Timeout
discovery, external receiver invocation, public-body activation and released-reader rollout
are separate decisions. Existing public publication refusal stays enforced.

## Candidate and transaction

Add a private collection event-wait scheduling module, separate from timer/control decisions.
One eligible active member progresses per collection transition. A pinned WaitEvent in Pending
can start; a valid Succeeded wait or Failed wait with a declared onError and an unrouted
outcome can release its deferred local route after resume. Waiting, cancelled and already-routed members are not candidates.
Do not schedule a worker or grant event waiting any worker capability.

Both collection entry methods already require Hold::Lease before these private helpers
(advance_collection_from_run_in_view and advance_collection_in_view). Generic View::holds
acceptance of Hold::Nothing is unreachable here. A placement-free scheduler must retain the
existing typed FenceUnavailable result before mutation; it grants no singleton clock authority.

Both full/frontier fail-fast classification must defer a valid unrouted Failed event wait
with a declared onError until that local route is committed. This applies while paused too,
when scheduling must remain no work. Do not call it handled before routing: preserve the
domain failure predicate, defer only this scheduler decision, retain other active failures,
and never defer a committed Run stop/drain. A wait without onError follows normal fail-fast.

The bounded path loads its exact snapshot, checks the transition revision and current
forward-progress eligibility and evaluates correlation_key_instance from that captured data
before locking the Run. Pending correlation errors remain typed input to the accepted pure
start command, which records EXPRESSION_ERROR without exposing the expression/key in logs.
Exact begin_path revision fencing prevents a stale evaluated answer from being applied.
Then acknowledge current own scheduler authority and obtain its post-lock database clock.
Start with start_event_wait_path_instance or release_pending_path and persist addressed events
with no outputs. A changed revision/no-work result does not count as progress.

The full-model oracle uses its already-owned Run/data and the same exact-instance evaluator,
then acknowledges current own scheduler authority before the clocked start or local routing.
It uses start_event_wait_instance/release_pending_instance_path and the accepted full native
writer. Both models enforce pinned kind, valid wait state, own Run, ancestor/global eligibility,
and the native captured/current binding checks. They preserve pause/stop/drain semantics.

## Failure and evidence

Reuse typed errors, existing collection storage-quota drain, actor/tracing and native
transactional journal/projection/telemetry/quota behavior. Handle content quota refusal from
both persist and commit after dropping/consuming the failed transaction; reopen the existing
scope drain under its original transition authority/revision. Other failures remain errors.
No schema, migration, permission, SDK or public activation change is required by this slice.

Use actual scoped worker completion followed by collection scheduling in both full/frontier
modes. Verify two same-name members get only their own captured item keys, pinned windows and
one journal start each, repeated scheduling is no work and no new worker hint exists. Add
expression-failure, paused start/deferred-route, stopped-state and current quota controls.
Existing native lifecycle/identity/fence tests remain authority evidence; final public and
released-profile gates remain open. No canonical task closes from this checkpoint alone.
