# ADR-0133: A poll ends after a run of empty probes, and starts spread by its session under contention

**Status:** Proposed (2026-11-24; revised 2026-11-26 after design 052's full review round)
**Decided by:** agent proposal (design 052, first drafted as part B of design 051), to be accepted under Owner decision 2026-11-24-4 once design 052's review is complete
**Refines / supersedes:** refines design 038's FR-038-004 (`specs/038-claim-transition-efficiency/`): a poll's order stays a preference, but its walk is bounded and its start may be spread across processes; leaves ADR-0030, ADR-0050, ADR-0056 and ADR-0078's lock order unchanged; supersedes nothing
**Enforced by (pending until built):** `a_poll_ends_after_eight_probes_that_leased_nothing`, `the_samples_behind_the_eligible_window_are_tried_and_not_counted`, `a_poll_alone_keeps_the_order_of_the_scan`, `a_session_that_met_contention_starts_spread_by_its_hash`, design 038's barrier test of K pollers run with two claimers, and design 038's guards unmodified (Task 052-2)

## Context

Design 038 made a stale candidate cheap but not rare: probes are two thirds of all database round trips, 58 per lease in
the closed-loop scenario of 2026-11-12, a scenario whose workers spin (`specs/038-claim-transition-efficiency/research.md`).
A poll reads the same window of candidates as every other poll, walks all of it even after many probes found their runs
locked, and spreads its start only among the polls of its own process. Design 052's earlier drafts proposed a queue of
ready steps claimed with `SKIP LOCKED`; its review showed what that queue needs (the scan's filters joined under its
lock, one row per run for fan-out, a safe switch and backfill, a trigger that can change under design 053's rules).

## Decision

1. A poll MUST end its walk after 8 consecutive probes that leased nothing; the ineligible and pool-refused samples
   behind the eligible window are tried as today and not counted.
2. A poll whose session's previous poll met contention (ended by the bound, or began with an empty probe) MUST start its
   eligible window at an offset of the session's hash (modulo 8) times what it asks for, added to design 038's turn; a
   poll that met none keeps the scan's order.
3. A queue of ready steps is not built until design 052's Success criterion 052-001 (at most 4 probes per lease with two
   instances in an open-loop run) is missed; then it is a design of its own.

## Considered options

1. **A bounded and spread poll (chosen).** No table, no trigger, no protocol change; the scan and every guard unchanged;
   measured before anything heavier.
2. **A queue of ready steps claimed with `SKIP LOCKED`** (design 052's earlier drafts). Probes per lease near one, at the
   cost of a high-turnover table and the machinery its review listed.
3. **The queue as the authority** (Windmill's model). It moves the lease decision out of the run's transaction.
4. **Row locks on `steps` in the candidate scan.** Design 038's reasons: locks before the chain row, rows the limit
   discards, a poller that cannot lease hiding the head.
5. **A shared turn across processes.** A hot counter per namespace.

## Consequences

- Good: fewer wasted probes across replicas with no new state; a poll alone keeps the oldest-first order.
- Bad: a bounded poll can leave eligible work deeper in its window to the next poll; the counters show how often.
- Harder: the threshold of the queue's design is a measurement, not a guess.
- Compatibility: nothing in the schema or the protocol changes; an older process walks its whole window.
- Rollback: the settings `SAGAT_POLL_BOUND` and `SAGAT_POLL_SPREAD` turn each off.

## Evidence

Design 052 and its review reports (`docs/designs/052-hot-path-dispatch.reviews/2026-11-26/`); design 038 and its research
register. To be added when built: the barrier tests and the measurement of Task 052-5.
