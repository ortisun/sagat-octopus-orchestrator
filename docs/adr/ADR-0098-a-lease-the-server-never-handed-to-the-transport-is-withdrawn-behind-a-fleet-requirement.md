# ADR-0098: A lease the server never handed to the transport is withdrawn, with a new journal event behind a fleet requirement

**Status:** Proposed
**Decided by:** agent, delegate (OD-2026-11-20-6), on the coordinator's choice of options A and B (2026-11-20); the correctness, security and architecture reviews are still to run.
**Refines / supersedes:** refines ADR-0029 (worker task streams spend finite receive credit: its "after commit, before delivery, the lease lapses") and ADR-0076 (an unknown outcome goes to consultation or review: a withdrawn lease is not an unknown outcome); uses ADR-0082 (requirements are recorded only by the operator). Supersedes none.
**Enforced by:** `crates/domain/tests/lease_withdrawal.rs` (the transition, the attempt given back, the generation kept, the retry budget counted, cancellation, scoped and bounded paths, the reset rule); the codec test of `step.lease_withdrawn` (`crates/infra-postgres/src/event_codec.rs`); the body tests `withdrawal_of_undelivered_frames` of `crates/api-grpc/src/service.rs` (a frame discarded at the window deadline, a queue drained at token expiry, a dropped body, a frame sent into a closed queue, a frame handed over); `tests/contract/durable-execution/grpc/lease_withdrawal.rs` (barriers after the commit of a claim: a dropped stream, a window that ends, a dropped unary poll, the gate off, `TASK_NOT_STARTED` while leased, after a heartbeat and without the requirement); the gate tests of `crates/application/src/instances.rs`; `tests/integration/schedules-bulk-operations/lease_withdrawal_activation.rs` (the operator command); the capability parity test and the migration pair `20270208000000_lease_withdrawal`.

## Context

The engine commits a lease, then sends it. When the send does not happen (a claim cut off at the end of its window, a queued
frame discarded at the deadline, a producer aborted between a commit and the send, a body dropped because the client went
away) nothing takes the lease back: it lapses after the lease time-to-live. Since feature 037 a lapsed lease of a declared
`Activity` is an unknown outcome, so work that never ran becomes `step.needs_review`. It was seen in CI in `reset` (Go) and in
`flow-rate-limit` (C#). The server knows, in each of those cases, that the frame never left the process. A host can also drop a
lease it received (it is stopping, a frame exceeds its credit) and knows it ran nothing.

Constraints: the journal is replayed by every control plane and engine, so a new event must not reach a replica that does not
know it; a release-0.2 binary does not register in the instance-capability registry, so no activation can see it; recording a
requirement is the operator's act only (ADR-0082 decision 1); feature 037's rule that an unknown outcome goes to review must
not be weakened.

## Decision

1. The server MUST withdraw every lease it committed and did not hand to the transport (a frame is handed to the transport when
   `TaskBody::poll_next` returns it; a unary answer when the handler returns it), in one fenced transaction per run that acts
   only on a step still `LEASED` with the issued generation and lease-token hash.
2. The producer of a task stream MUST NOT be aborted in the middle of a claim: it is told to stop, finishes the claim, queues
   nothing more and hands its leases to the withdrawal.
3. A withdrawal MUST be recorded as `step.lease_withdrawn` (`node`, `attempt`, `generation`, `cause` `NOT_SENT` or
   `NOT_STARTED`), which moves `LEASED` to `READY`, clears the lease and sets the attempt back by one (the generation is kept, so
   the next lease has the next one). It MUST NOT consume an attempt or open a review.
4. The event MUST be written only while the requirement `lease-withdrawal-v1` is recorded for control planes and engines.
   Every control plane and engine of the build lists the capability. The requirement is recorded by the operator command
   `control-plane lease-withdrawal-activate` (owner role, one bounded transaction, `--operator`, `--expect-live N`,
   `--dry-run`). Until it is recorded, a lease that is not sent lapses as before and is counted.
5. A host MUST hand back a lease it will not run with `FailStep` code `TASK_NOT_STARTED`, retryable, before any handler of it
   runs. The server MUST treat it as a withdrawal (cause `NOT_STARTED`) when the requirement is recorded and the step is still
   `LEASED`, and as an ordinary retryable failure otherwise. `TASK_NOT_STARTED` MUST NOT be an unknown outcome.
6. A lease that lapses MUST keep its meaning: an unknown outcome, consultation or review.

## Considered options

1. **Withdraw what the server knows it did not send, with a new event behind a fleet requirement (chosen)** — fixes every case
   the server can see without changing a guarantee of the protocol; an older replica never meets the event.
2. **Reuse `step.retry_scheduled` and `step.retry_due`** — older replicas replay them, but they mean "an attempt failed",
   consume the attempt and the retry budget, and fail a step without retries for good.
3. **Reuse `step.released`** — it moves `PENDING` to `READY` only; an older replica replaying it on a `LEASED` step fails.
4. **Treat an expiry with no heartbeat as a known outcome** — unsafe: a handler can have its effect before its first heartbeat.
5. **Delivery acknowledgement** — closes the frames lost on the wire too, but changes the protocol (a new minor in six SDKs);
   approved by the owner for 0.3.0 (OD-2026-11-20-12) and designed in its own ADR. It builds on this one (the withdrawal and
   its event).

## Consequences

Good: a task that never reached a handler goes back to `READY` instead of to an operator; the attempt number of the task that
never began is reused, so the idempotency key the handler sees is the same; the meaning of a lapse is untouched.

Bad: one more journal event in the events contract, the replay, the reset rules and every reader of the journal; one more
capability and one more operator command; nothing changes on a fleet until the operator activates it; a frame lost on the wire
and a lost unary answer still lapse (G006-001) until delivery acknowledgement; a process that stops between a claim and its
withdrawal still leaves the lease to lapse (G006-002).

Rollback: remove the requirement first (the runbook's procedure), then roll the binary back; journals that hold
`step.lease_withdrawn` cannot be replayed by a binary older than this decision, which is why the event is written only after the
activation. Removing the requirement stops new withdrawals at once (within one heartbeat).

## Evidence

`specs/006-sdk-multilanguage/research.md` D006-001 to D006-010, G006-001 to G006-003, O006-001; the CI jobs 136567175914 and
136453650156; the C# output of `flow-rate-limit` (feature 022, PR #251); `specs/006-sdk-multilanguage/stream-completion.md`,
"Leases that never reach a host".
