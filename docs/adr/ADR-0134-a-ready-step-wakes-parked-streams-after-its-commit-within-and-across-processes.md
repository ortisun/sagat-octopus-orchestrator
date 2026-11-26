# ADR-0134: A ready step wakes parked streams after its commit, within and across processes

**Status:** Proposed (2026-11-24; revised 2026-11-26 after design 052's full review round)
**Decided by:** agent proposal (design 052, first drafted as part B of design 051), to be accepted under Owner decision 2026-11-24-4 once design 052's review is complete
**Refines / supersedes:** refines [ADR-0029](ADR-0029-worker-task-streams-spend-finite-receive-credit.md) (a stream also claims on a wake, not only after its idle interval); supersedes nothing. The next step in a completion's answer, which this record's first draft also decided, is not decided here (design 052, Open Question 3)
**Enforced by (pending until built):** `a_commit_between_an_empty_claim_and_the_park_wakes_the_stream` (Task 052-3), `a_wake_from_another_instance_claims_at_once` and `a_lost_wake_is_bounded_by_idle_retry` (Task 052-4), design 038's guards unmodified

## Context

A stream that finds nothing sleeps `idle_retry` (500 ms) and claims again, so a step that becomes ready waits up to
500 ms, up to 1,500 ms of idle in a chain of three under light load. The broker's `task.ready` hints go through the
outbox, each an acknowledged JetStream publish at the scheduler's pace, and nothing subscribes to them.

## Decision

1. A parked stream MUST register its binding keys and read their wake counters before its claim, and park only if no
   counter moved; a commit that makes a step of those keys ready increments the counter after the commit.
2. With NATS configured, the committing process MUST publish, after the commit and outside it, one core NATS message per
   key made ready and not leased by the same transaction, on `sagat.wake.v1.>` (outside the platform stream's filter, so
   JetStream never stores it); each process serving worker streams subscribes and wakes by key. A lost wake costs at
   most `idle_retry`; paths with no commit of a step stay bounded by it.
3. A wake MUST wake at most as many parked streams of the key as steps were made ready, coalesced within 10 ms; a woken
   stream refused by discovery admission returns to parking.

## Considered options

1. **A local registry with a best-effort core NATS wake (chosen).** Fast, lossy by design, bounded by `idle_retry`.
2. **The outbox's `task.ready` hint.** Durable and slow; a wake needs to be fast and may be lost.
3. **`LISTEN` and `NOTIFY`.** Its commit-time lock serializes the commits of every notifying transaction.
4. **A shorter `idle_retry`.** An empty claim per idle stream per interval.

## Consequences

- Good: a step's wait for a lease under light load follows the commit, not a polling interval.
- Bad: one core publish per key made ready; a forged wake costs an empty claim.
- Harder: the wake must never be relied on; every test also holds with it lost.
- Compatibility: an older process ignores the wake subjects; nothing is stored.
- Rollback: the setting `SAGAT_WAKE` turns it off.

## Evidence

Design 052 and its review reports (`docs/designs/052-hot-path-dispatch.reviews/2026-11-26/`). To be added when built: the
wake tests and the measurement of Task 052-5.
