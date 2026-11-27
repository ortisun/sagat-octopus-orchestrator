# Architecture review of design 045, round 2 (2026-11-24)

Doc: docs/designs/045-activity-timeouts.md
Reviewed-commit: 5033df9d
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author; every finding and its severity kept, with the author's answer. Architecture decisions 0098 and
0099 were read from the local delivery-acknowledgement worktree.

Counts: Blocker 0, Major 10, Nit 16

First round: Majors 3, 4, 7, 8, 9 and Nits 12 to 26 closed; Blocker 1 and Majors 2, 5, 6 partly closed and Major 11 not closed
(below). The contested Decisions 045-015 and 045-016 hold.

## Major

1. **The close floor cannot be the same in the claim query and in the domain.** *Answer: accepted*: the journaled close cutoff
   (correctness round 2, Major 1).
2. **The gate timeout at a close deadline has no event contract** and conflicts with feature 022's built routine. *Answer: accepted*:
   with the close cutoff the earlier, the routine journals the dequeue of each open gate and `step.failed` with
   `SCHEDULE_TO_CLOSE_TIMEOUT` in one transaction; `wait_timed_out` only when the queue deadline came first; a `deadline_cause` column
   on 022's two waiter tables.
3. **The design let a person retry an unknown outcome, which feature 037 forbids.** *Answer: accepted*: the operator rule applies to a
   known failure; an unsettled step is settled first (`OutcomeUnresolved`, unchanged).
4. **The heartbeat minimum is argued from 1.8 rules but offered to every host.** *Answer: accepted*: a `heartbeatTimeoutMs` below
   30,000 ms is leased only to a 1.9 host, through `min_protocol_minor`, under `activity-timeouts-protocol-v1`; at or above 30,000 ms
   any host is served, as today.
5. **The `Owner-gate:` lines will fail the checker when ticked; the cited decisions are not on this branch.** *Answer: accepted in
   part*: the lines cite Owner decision 2026-11-24-1, the scope decision that puts the dependent features in 0.3.0; before those tasks
   are ticked, an owner row whose `Covers` names them must exist, and the report to the coordinating session asks for it (a
   delegate's row cannot open a gate under Rule 5).
6. **The attempt deadline and the lease expiry are computed in SQL.** *Answer: accepted*: for a version of level 1 or 2, `step.leased`
   carries the attempt deadline and its cause; the lease expiry is lease data outside the one-source rule and outside the replay
   comparison.
7. **The publication hold locks the registry epoch**, diverging from Architecture decision 0082. *Answer: accepted*: a shared advisory
   lock on the requirement's key, taken exclusively by the operator's functions; Architecture decision 0110 records that it refines
   0082 decision 1, and the registry's privilege test is updated.
8. **The replay floor cannot detect this design's transitions**, and the ADR contradicted itself. *Answer: accepted*: the version's
   level, one indexed predicate; the ADR and the design say the same.
9. **The pause shift has no size bound and no path for body instances.** *Answer: accepted*: one statement at the pause and one at the
   resume over the run's claimable rows, root and path, from the row's own window.
10. **A step that can never be leased can be published.** *Answer: accepted*: `scheduleToCloseMs` at least the floor plus 1,000 ms.

## Nit (each accepted unless said)

1. Decision 045-017 revised in place.
2. Decision 045-011's old reasoning is marked as the first text; the gaps are in order.
3. `due_start_releases` is not the precedent: no run join is needed any more.
4. The pause after an idle tick is 400 to 600 ms: the bound is 5 × 600 = 3,000 ms and the criterion is 4 s at the 99th percentile.
5. The entry's delays have the 86,400,000 maxima.
6. The cross-field rules belong to the domain's constructors; `infra-ir` maps them.
7. The domain sees host features, not a protocol number.
8. `run.resumed` is a setter of the start deadline; the operator's hold is a review (`step.needs_review`).
9. Architecture decision 0110 refines Architecture decision 0099 decision 5 and ADR-0029 decision 6.
10. Reset needs no rule (a reset point has no open work).
11. "1.9" means the minor this design takes when its contract merges.
12. The minimum is decided now: 18,000 ms.
13. The refusal is named `ACTIVITY_TIMEOUTS_NOT_ACTIVE` and listed with the other reasons.
14. The waiter's cause column is on feature 022's tables, said in Non-functional requirement 045-003.
15. The migrations sort after 022's and Architecture decision 0098's; the down migration restores the closed lists and refuses once
    the history names the capabilities.
16. Feature 019's `asyncCompletion.heartbeatTimeoutMs` shares the name: the guide explains the difference.
