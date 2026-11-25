# ADR-0133: A lean queue of ready steps is claimed with SKIP LOCKED, and it is an index of candidates, not an authority

**Status:** Proposed (2026-11-24)
**Decided by:** agent proposal (design 051, Part B), to be accepted under Owner decision 2026-11-24-4 once design 051's review is complete
**Refines / supersedes:** refines [ADR-0030](ADR-0030-structured-instances-and-continuation-chains.md) and [ADR-0078](ADR-0078-capacity-group-lock-order-and-possession-protocol.md) (the claim's lock order gains the queue row before the run row); supersedes nothing
**Enforced by:** the barrier tests of the claim's lock chain with a queue row held (Task 051-22), design 038's guards passing unmodified, the reconciliation check that fails the suites on a repaired queue (Task 051-23), and design 038's barrier test of K pollers with probes per lease at most 1.1

## Context

Design 038 made a stale candidate cheap but not rare: probes are two thirds of all database round trips, 58 per lease in
the closed-loop scenario of 2026-11-12, because every poller reads the same window of candidates with no row lock and
tries each in its own transaction (`specs/038-claim-transition-efficiency/research.md`). Its window order is per
process, so replicas still collide. It left "a separate task queue table claimed with one statement, as Windmill does"
and row locks in the scan for an architecture decision.

## Decision

1. Every leasable step that becomes `READY` MUST get a row in `ready_steps`, by a trigger on `steps`, in the transaction
   that made it ready. Nothing else inserts.
2. A claim MUST take its next candidate from `ready_steps` with `FOR UPDATE SKIP LOCKED`, delete it in the claim's
   transaction, and then claim that run exactly as today, from the run row on. A lease is decided only from the step
   rows read under the run lock.
3. Only claims and the queue's sweeper lock a queue row, always with `SKIP LOCKED` and before any run row. No other
   transaction locks or deletes a queue row; a queue row whose step is no longer ready is deleted by the claim or the
   sweeper that finds it.
4. A reconciliation MUST add a queue row for every leasable ready step without one and count it; the integration suites
   fail when it adds any.
5. The queue's order is `ready_at` within the session's (tenant, namespace, activity) keys; feature 019 adds its
   priority and fairness keys to the queue's index and order.

## Considered options

1. **A queue as an index, claimed with `SKIP LOCKED` (chosen).** One probe per lease, the run lock and every guard of
   features 002, 009 and 010 unchanged, no lock cycle (a claim holding a queue row waits only for locks whose holders
   never wait for a queue row).
2. **The queue as the authority** (the step's state in the queue row, one statement per claim). Faster still, and it
   moves the lease decision out of the run's transaction, changing principle I's boundary and every guard.
3. **Row locks on `steps` in the candidate scan.** Takes run-related locks before the chain lock, locks rows the outer
   limit discards, and lets a poller that cannot lease hide the head of the queue (design 038's reasons).
4. **Design 038's window order across processes** (a shared turn). Needs a shared counter per namespace, itself a hot
   row.

## Consequences

- Good: probes per lease near one whatever the number of pollers and replicas; the candidate scan of `steps` leaves the
  hot path.
- Bad: a table with high turnover and its own vacuum settings; one trigger insert per ready step and one delete per
  lease.
- Harder: every path that makes a step ready is covered by the trigger, so none can forget; a new leasable kind must be
  added to the trigger's condition.
- Compatibility: additive; an old binary that claims by the scan still works against the new schema (its claims leave
  queue rows that the next claim or the sweeper deletes).
- Rollback: a setting turns the queue claim off; the scan is still there as the reconciliation's query.

## Evidence

Design 051 and its review reports; design 038 and its research register. To be added when built: the barrier tests and
the measurement of Task 051-12.
