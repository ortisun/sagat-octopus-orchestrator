# ADR-0134: A ready step wakes parked streams after its commit, and a completion may lease the next step of the run to the session that completed

**Status:** Proposed (2026-11-24)
**Decided by:** agent proposal (design 052, first drafted as part B of design 051), to be accepted under Owner decision 2026-11-24-4 once design 052's review is complete; design 051's first review found it not ready, and design 052 answers those findings
**Refines / supersedes:** refines [ADR-0029](ADR-0029-worker-task-streams-spend-finite-receive-credit.md) (a stream also claims on a wake, not only after its idle interval) and [ADR-0015](ADR-0015-sdks-meet-the-server-by-minor-version-and-are-held-to-one-conformance-suite.md) (the protocol minor after delivery acknowledgement's, which holds 1.8, adds `want_next`, `session_token`, `next_task` and `next_declined`); supersedes nothing
**Enforced by:** the wake tests and the lost-wake test (Task 051-24), the cross-process wake test (Task 051-25), the conformance scenario `next-task-in-completion` in six SDKs (Tasks 051-26 and 051-27), design 038's guards unmodified

## Context

A stream that finds nothing sleeps `idle_retry` (500 ms) and claims again; a unary poll that finds nothing returns
`retry_after_ms = 500`. Under light load a step can wait up to 500 ms before anyone asks for it, three times in a chain
of three. Broker hints are published and nobody reads them. The answer to a completion cannot carry the next task, so a
worker that serves the next activity polls for it.

## Decision

1. After a transaction that made steps ready commits, the process MUST wake its parked streams whose bindings cover
   those steps' (tenant, namespace, activity); a stream reads its keys' wake counters before it claims and parks only if
   none moved. Unary polls are unchanged. With a notification transport configured, each engine
   process MUST also wake on the outbox hint of a ready step. A wake is a hint: a lost or extra wake changes no state,
   and `idle_retry` stays the upper bound.
2. The wake MUST NOT use `NOTIFY`, whose commit-time lock serializes the commits of every transaction that issues it.
3. A session at that minor MAY set `want_next` with its session token on `CompleteStep`. The completion's transaction
   then MUST call the claim's own per-run routine for the first ready step of the same run that the session serves, in
   the claim's lock order after the run row, and MUST return the lease in `next_task` only if every check passes without
   writing a refusal; otherwise the step stays ready for an ordinary claim. The lease follows the withdrawal rules of
   ADR-0098 and ADR-0099, and a replay never re-issues an earlier token.
4. Wake keys MUST NOT be metric labels.

## Considered options

1. **An in-process registry plus the existing hint, and an opt-in next task (chosen).** No change to the commit path,
   no new authority, a lease decided where every lease is decided.
2. **`LISTEN` and `NOTIFY`.** Built in, and serializes commits at the cluster level.
3. **A shorter `idle_retry`.** More empty claims, each a database round trip, for every idle stream.
4. **Eager dispatch of every ready step to any session** (push to workers). Feature 019's dispatch and routing, not
   this design's.

## Consequences

- Good: a step's wait for a lease under light load falls from up to 500 ms to one claim; a chain's next step needs no
  poll at all for a worker that serves it.
- Bad: a registry of parked streams per process; a subscriber per engine process to the hint transport; one more
  protocol field in six SDKs.
- Harder: the completion path runs the claim's checks; design 038's guards and the conformance scenario keep them equal.
- Compatibility: additive minor; older SDKs and servers ignore the fields.
- Rollback: settings switch both off; the polling path is unchanged underneath.

## Evidence

Design 052, and the review reports of design 051's first draft (`docs/designs/051-competitive-benchmark.reviews/2026-11-24/`); `crates/api-grpc/src/service.rs` (the idle sleep); `crates/infra-nats/src/lib.rs`
(hints published, not consumed). To be added when built: Success criterion 051-005's measurement.
