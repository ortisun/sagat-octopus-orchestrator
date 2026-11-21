# ADR-0098: A lease the server never handed to the transport is withdrawn, with a new journal event behind a fleet requirement

**Status:** Proposed
**Decided by:** agent, delegate (OD-2026-11-20-6), on the coordinator's choice of options A and B (2026-11-20) and of the fixes of the three reviews (2026-11-21, decisions D006-030 to D006-039). Acceptance is the owner's: decision 1 overturns part of an accepted decision about the guarantees of the worker protocol (ADR-0029 decision 5).
**Refines / supersedes:** **supersedes ADR-0029 decision 5** for the leases the server never handed to the transport (see "Why ADR-0029 decision 5 does not hold here"); refines ADR-0076 (an unknown outcome goes to consultation or review: a withdrawn lease is not an unknown outcome); uses ADR-0082 (requirements are recorded only by the operator) and amends its activation with one generic command (decision 4).
**Enforced by:** `crates/domain/tests/lease_withdrawal.rs` (the transition, the attempt given back, the generation kept, the retry budget counted, cancellation, scoped and bounded paths, the reset rule); the codec test of `step.lease_withdrawn` (`crates/infra-postgres/src/event_codec.rs`); the body tests `withdrawal_of_undelivered_frames` of `crates/api-grpc/src/service.rs` (a frame discarded at the window deadline, a queue drained at token expiry, a dropped body, a frame sent into a closed queue, a frame handed over); `tests/contract/durable-execution/grpc/lease_withdrawal.rs` (barriers after the commit of a claim: a dropped stream, a window that ends, a dropped unary poll, the gate off, `TASK_NOT_STARTED` while leased, after a heartbeat and without the requirement; from a session older than 1.8; the cap of four for each cause; a claim whose consumer is gone between two runs; the refused removal; the requirement re-read under its lock and the off switch; the shutdown drain; the tenant slot; `verify_run` after each withdrawal); a scoped lease withdrawn for each cause on the real store (`tests/integration/workflow-primitives/bounded_worker.rs`); the down guards in `tests/integration/platform-foundation/migrations.rs`; the released-v0.2 profile (`release_upgrade_inflight.rs`, no withdrawal before the activation); the gate tests of `crates/application/src/instances.rs`; `tests/integration/schedules-bulk-operations/lease_withdrawal_activation.rs` (the operator command, the refused removal); the capability parity test, the migration pair `20270208000000_lease_withdrawal` and its down guards; the capability-list check of `scripts/check-architecture.py` (decision 8).

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
   `TaskBody::poll_next` returns it; a unary answer when the handler returns it), in one fenced transaction per run (and per
   instance path for a scoped lease) that acts only on a step still `LEASED` with the issued generation and lease-token hash.
2. The producer of a task stream, and the claim of a unary poll, MUST NOT be aborted in the middle of a claim: told to stop,
   the claim finishes the run transaction it is in, claims no further run, queues nothing more and hands its leases to the
   withdrawal. Both run on the tracker a graceful shutdown waits for, and at most a quarter of the pool withdraws at once
   (decisions D006-035, D006-036 and D006-038).
3. A withdrawal MUST be recorded as `step.lease_withdrawn` (`node`, `attempt`, `generation`, `cause` `NOT_SENT` or
   `NOT_STARTED`), which moves `LEASED` to `READY`, clears the lease and sets the attempt back by one (the generation is kept, so
   the next lease has the next one). It MUST NOT consume an attempt or open a review.
4. The event MUST be written only while the requirement `lease-withdrawal-v1` is recorded for control planes and engines.
   Every control plane and engine of the build lists the capability. The requirement is recorded by the generic operator
   command `control-plane capability-activate lease-withdrawal-v1 --operator NAME [--expect-live N] [--dry-run]` (owner
   role, one bounded transaction). The same command activates every fleet requirement that has a way to be activated
   (feature 022 adds its own entries); only the one-way handover of `schedules-v1` keeps its own command. Until the
   requirement is recorded, a lease that is not sent lapses as before and is counted.
5. A host whose session negotiated protocol 1.8 or later MUST hand back a lease it will not run with `FailStep` code
   `TASK_NOT_STARTED`, retryable, before any handler of it runs; below 1.8 it MUST NOT, and the lease lapses as before
   (decision D006-080: a server below 1.8 would spend the attempt). The code is the host's: an SDK MUST rewrite a `TASK_NOT_STARTED` that a handler raises into an ordinary error. The
   server MUST treat it as a withdrawal (cause `NOT_STARTED`) only when the session that holds the lease negotiated a
   protocol minor that reserves the code (1.8, delivery acknowledgement, ADR-0099), the requirement is recorded, the step is
   still `LEASED` and the cap of decision 7 is not reached. In every other case it is an ordinary failure with the host's
   `retryable` flag. `TASK_NOT_STARTED` MUST NOT be an unknown outcome.
6. A lease that lapses MUST keep its meaning: an unknown outcome, consultation or review.
7. One attempt MUST NOT be withdrawn more than four times (the derivation is decision D006-031: one delivery is withdrawn at
   most once, and four causes can each end one delivery of an honest attempt). Past the cap a `TASK_NOT_STARTED` is the
   ordinary failure and a lease the server did not send lapses as before.
8. The requirement is a replay floor. Its removal MUST be refused while any journal holds `step.lease_withdrawn`, and a
   withdrawal MUST read the requirement under a lock the removal waits for. An engine MAY stop writing new withdrawals with
   `SAGAT_LEASE_WITHDRAWAL=off`, which keeps the capability listed. The capability lists of the registry's checks in SQL
   MUST equal `Capability::ALL`, which `scripts/check-architecture.py` checks (CI runs it at a release) and the test `capability_lists_follow_the_code` of `crates/infra-postgres` checks on every pull request that changes that crate or the application crate. The minor of the session that holds a lease is never read above the build's own ceiling (ADR-0099 decision 9).

### Why ADR-0029 decision 5 does not hold here

ADR-0029 decision 5 says: never release a committed lease because a write, a stream or a client was cancelled, because a
closed stream never proves that a handler did not start. That reason is true for a frame handed to the transport: the host
may have it, and only delivery acknowledgement can tell. It is not true for a frame that `TaskBody::poll_next` never
returned, or a unary answer the handler never returned. No host can hold that lease, and the server knows it. Decision 1
releases only those. A frame handed to the transport keeps ADR-0029 decision 5 (gap G006-001). The host's hand-back
(decision 5) is the host's own statement while it holds the lease, limited to sessions of the minor that reserves the code
and capped by decision 7.

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

Rollback: to stop withdrawing, set `SAGAT_LEASE_WITHDRAWAL=off` on the engines (decision 8); their leases lapse as before.
The requirement cannot be removed once a journal holds `step.lease_withdrawn`, because a build without this decision cannot
replay the event: a new durable event is forward-only. Rolling a binary back below this decision is possible only while no
journal holds the event (the requirement removed first, then the down migration, which refuses otherwise).

## Evidence

`specs/006-sdk-multilanguage/research.md` D006-001 to D006-018, D006-030 to D006-039, D006-080 to D006-089, G006-001 to G006-004, G006-030, G006-031, O006-001; the reviews of 2026-11-21 (`specs/006-sdk-multilanguage/reviews/2026-11-21/`); the CI jobs 136567175914 and
136453650156; the C# output of `flow-rate-limit` (feature 022, PR #251); `specs/006-sdk-multilanguage/stream-completion.md`,
"Leases that never reach a host".
