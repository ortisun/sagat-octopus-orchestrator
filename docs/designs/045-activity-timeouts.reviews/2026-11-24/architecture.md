# Architecture review of design 045 (2026-11-24)

Doc: docs/designs/045-activity-timeouts.md
Reviewed-commit: 67cc649e
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and its severity is kept, with the author's answer.
The reviewer read the working trees (no shell); the design file was unchanged from 67cc649e there; Architecture decisions 0098
and 0099 were read from the local branch of the delivery-acknowledgement worktree.

Counts: Blocker 1, Major 10, Nit 15

## Blocker

1. **Unclaimable rows fill the claim's candidate window** (`claimable_query!`, `crates/infra-postgres/src/workers.rs`, the oldest
   `READY` rows up to `min(64, 4 × wanted)`): a 1.8 session behind 64 handler-sourced or expired rows claims nothing, poll after
   poll, and steps of older versions starve (principle IV, ADR-0089's context, FR-038-002). *Answer: accepted*: the three
   conditions go into all four branches of the candidate query (the precedent is the `$5 >= 5` branch of ADR-0032 decision 5), the
   lease decision in the domain skips such a step without aborting the run's other leases, and a parity test ties the two;
   `heartbeatFrom: "handler"` requires `scheduleToStartMs`, so such a row cannot wait without bound.

## Major

2. **Two exits for a gate waiter, without refining ADR-0089** (its decision 5, one timeout routine). *Answer: accepted*: the close
   deadline becomes one of the waiter's deadlines inside ADR-0089's single timeout routine (the earlier wins), a waiter past it is
   not eligible under decision 4, and ADR-0110 states that it refines ADR-0089 decisions 4 and 5.
3. **A withdrawal cannot keep a start deadline the claim cleared.** *Answer: accepted* (also correctness, Major 1); the event's
   value is the one source (also correctness, Major 9).
4. **Paused runs are not addressed.** *Answer: accepted* (Decision 045-019; also correctness, Major 7).
5. **Reset and operator retry are undefined for the deadlines**: reset copies `step.released` and `step.retry_due` with absolute
   deadlines from the source. *Answer: accepted* (Decisions 045-018 and 045-020): a reset recomputes the deadline fields of the
   steps it makes claimable from the reset's own time; the rule goes into feature 018's reset-rule registry
   (`specs/018-schedules-bulk-operations/contracts/journal-and-events.md`) with its test; an operator's retry clears the close
   deadline and writes a start deadline.
6. **The 3,000 ms minimum rests on a one-machine measurement**, below the 6 s receive bound. *Answer: accepted*: 10,000 ms, above the
   receive bound plus a heartbeat bound; the loss of delivery-acknowledgement protection below 18,000 ms is stated as a consequence
   in ADR-0110 and shown as a warning in the Studio's form (Gap 045-004 records the case for an 18 s minimum).
7. **`heartbeatFrom: "handler"` has no real publication gate and no liveness bound.** *Answer: accepted*: the capability
   `activity-timeouts-handler-v1`, listed only by builds that offer 1.9; `scheduleToStartMs` is required with the handler source.
8. **The design edits an applied migration** (the invalid-index list of `20261017000000_scheduler_shards.up.sql`). *Answer:
   accepted*: the new migration has its own check.
9. **Heartbeat load and lease holding have no ceiling or budget.** *Answer: accepted*: the minimum of 10,000 ms bounds a lease's
   beat rate to three times today's; a heartbeat-rate scenario in Task 045-17; ADR-0110's consequences state both costs, including
   a long `heartbeatTimeoutMs` holding a dead worker's step and its concurrency for that long.
10. **The rules have no fitness checks, and no row in `docs/engineering/architecture-principles.md`.** *Answer: accepted*: a parity
    test of claim eligibility (candidate query against the domain), a static check in `scripts/check-architecture.py` that the
    deadline columns are assigned only from event values and never by a withdrawal, a reset-registry test, a pause barrier test, and
    the row; `scripts/check-capacity-activation.py` is run on the per-node time-to-live (Task 045-7).
11. **Cross-feature tasks have no `Owner-gate:` line.** *Answer: accepted*: the tasks that build into features 006 and 022 cite Owner
    decision 2026-11-24-4 (and 2026-11-24-1 for scope).

## Nit (each accepted unless said)

12. Not every writer of a step takes the run lock: corrected.
13. A fresh grant's expiry is not capped by the attempt deadline: corrected; the cap is on renewal only.
14. The list grammar is narrower than the server's: documented, as for correctness Major 12.
15. A worker can send the two new codes: the server maps them to `WORKER_ERROR`.
16. CUJ 2 and jitter: `jitter: false`.
17. An overlaid policy can be invalid: `RETRY_OVERRIDE_INVALID`.
18. The compensation policy also follows row 3: said.
19. Which code wins when both deadlines passed: the earlier deadline's; a tie goes to `SCHEDULE_TO_CLOSE_TIMEOUT`.
20. A provisional lease at a close-capped attempt deadline is withdrawn, then fails `SCHEDULE_TO_CLOSE_TIMEOUT`: said.
21. `deadline: startToClose` would be written for older versions: written only when the cause column is set.
22. The fingerprint, and `heartbeat_timeout_ms = 0`: the delay joins the fingerprint; the field always carries the time-to-live that
    applies.
23. The down migration's rule for a `NOT NULL DEFAULT 0` column: "a non-default value".
24. A handler-sourced host that stops beating never learns of a cancellation, and `script` nodes cannot declare `reconcileWith`:
    both said; a script's timed-out attempt goes to review.
25. Wrong references (the journal document, the task number of Gap 045-001, the heading of Decision 045-009, the note on field 28):
    corrected; the journal fields go to `specs/001-platform-foundation/contracts/` and the journal-and-events contract of feature 018.
26. The Studio's time left must not use the browser's clock: it is computed from the view's server time.
