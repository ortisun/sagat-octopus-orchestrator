# ADR-0133: A poll stops after a run of contended probes, and starts spread by its process under contention

**Status:** Proposed (2026-11-24; revised 2026-11-26 after design 052's full review round)
**Decided by:** agent proposal (design 052, first drafted as part B of design 051), to be accepted under Owner decision 2026-11-24-4 once design 052's review is complete
**Refines / supersedes:** refines design 038's FR-038-004 (`docs/designs/038-claim-transition-efficiency.md`), within Owner decision 2026-11-12-3 as Owner decision 2026-11-24-526 records: a poll's order stays a preference and no candidate is dropped, only deferred, but its walk is bounded and its start may be spread across processes; follows feature 019's FR-019-013 (a rotation within one priority); leaves ADR-0030, ADR-0050, ADR-0056 and ADR-0078's lock order unchanged; supersedes nothing
**Enforced by (pending until built):** `a_minor_four_session_reaches_its_forward_work_behind_nine_scoped_compensations` (Task 052-2); `a_poll_stops_after_eight_probes_that_met_contention`, `unreadable_runs_ahead_do_not_end_the_poll`, `a_static_backlog_behind_eight_locked_runs_is_leased_within_one_hundred_milliseconds`, `the_ineligible_sample_is_walked_after_the_bound` (Task 052-3); the offset's unit test, `a_poll_alone_keeps_the_order_of_the_scan`, `k_polls_in_flight_together_probe_once_per_lease` unmodified and `a_second_round_after_contention_probes_once_per_lease_in_each_of_two_instances` (Task 052-4); design 038's guards unmodified

## Context

Design 038 made a stale candidate cheap but not rare: probes are two thirds of all database round trips, 58 per lease in
the closed-loop scenario of 2026-11-12, a scenario whose workers spin (`specs/038-claim-transition-efficiency/research.md`).
A poll reads the same window of candidates as every other poll, walks all of it even after many probes found their runs
locked, and spreads its start only among the polls of its own process. Design 052's earlier drafts proposed a queue of
ready steps claimed with `SKIP LOCKED`; its review showed what that queue needs (the scan's filters joined under its
lock, one row per run and activity for fan-out, a safe switch and backfill, a trigger that can change under design 053's
rules).

## Decision

1. A poll MUST stop walking its eligible window after 8 consecutive probes that met contention (a run lock skipped, no
   ready step left, a step leased meanwhile); a refusal of this build or session (an unreadable definition, the protocol
   minor, a pool refusal) does not count. A poll so stopped with candidates untried fetches its window once more, past
   what it tried and without the runs it probed, then walks the ineligible sample uncounted, and if it leased nothing with work left asks again
   after 50 ms instead of reporting empty.
2. While a namespace is contended in a process (from a contended probe until a poll ends having met none, or 1 s
   after the last), the polls of that namespace in that process MUST start at an offset of the process's registry
   identifier modulo the window's length, added to design 038's turn, within one priority; an uncontended poll keeps the
   scan's order.
3. A queue of ready steps is not built until design 052's Success criterion 052-005 (at most 4 probes per lease, two
   instances, an open loop) is missed; then it is a design of its own.

## Considered options

1. **A bounded and spread poll (chosen).** No table, no trigger, no protocol change; the scan and every guard unchanged;
   measured before anything heavier.
2. **A queue of ready steps claimed with `SKIP LOCKED`** (design 052's earlier drafts). Probes per lease near one, at the
   cost of a high-turnover table and the machinery its review listed.
3. **The queue as the authority** (Windmill's model). It moves the lease decision out of the run's transaction.
4. **Row locks on `steps` in the candidate scan.** Design 038's reasons: locks before the chain row, rows the limit
   discards, a poller that cannot lease hiding the head.
5. **A shared turn across processes.** A hot counter per namespace.
6. **Jumping to another offset after k empty probes, without fetching again.** The window is a snapshot shared with the
   polls in flight; its later candidates are as likely leased as its first.
7. **A session's hash for the spread** (the second draft). Two sessions of one process collide one time in four.

## Consequences

- Good: fewer wasted probes across replicas with no new state; a poll alone keeps the oldest-first order.
- Bad: a bounded poll defers eligible work deeper in its window to its second fetch or its 50 ms retry; the counters
  show how often.
- Harder: the threshold of the queue's design is a measurement, not a guess.
- Compatibility: nothing in the schema or the protocol changes; an older process walks its whole window; runs a build
  cannot read never end its polls, since only contention counts.
- Rollback: the settings `SAGAT_POLL_BOUND` and `SAGAT_POLL_SPREAD` turn each off.

## Evidence

Design 052 and its review reports (`docs/designs/052-hot-path-dispatch.reviews/2026-11-26/`); design 038 and its research
register. To be added when built: the barrier tests and the measurement of Task 052-5.
