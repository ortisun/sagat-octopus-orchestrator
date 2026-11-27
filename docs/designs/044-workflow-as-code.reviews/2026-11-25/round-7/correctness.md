# Correctness review of design 044, round 7 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: e0a23b09f7ddbe225e80110c2cf25f3da960b730
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Round 6's Majors A, B,
C, D and F are closed, E partly closed and G closed for the bound itself; every round-6 Nit is closed.

## Major

1. A settled code abort still journals `compensation.planned` and `saga.compensated`, which stay `EXCLUDE`, so no reset
   point after a caught abort is eligible; make them `COPY` once settled and say what the reset rebuilds.
2. The `HISTORY_LIMIT` closing window refuses every completion (all past 51,200), a retry could open window after
   window, and nothing ends the run when a bare-transaction writer uses the window up: judge completions against the
   window's bound, end the run in that transaction, a pass for other writers, no retry inside a window; tests per writer.
3. The `RETAINED_BYTES_LIMIT` window has no size, end or flag, so its retry repeats the review.
4. Continue-as-new accepted in the completion takes feature 014's chain locks after the run, the reverse of the chain
   cancel's order (a deadlock), and a refused handoff has no outcome: one place for the handoff and stated refusals; a
   barrier test.
5. A function's abort with a member in flight plans only completed members, so a member that succeeds later is never
   compensated: the abort journals `saga.aborting` and feature 003 plans once members settle.
6. Initiators copied to a successor or a child keep another journal's sequences and escape the exclusion: copy at 0.

## Nit

7. The dependency rule names B's outcome instead of A's in section 10.1.
8. "Opens the activation" is ambiguous when a failed start sits in the window.
9. A feature 022 grant is both "not waking" and "waking"; derived and scope steps' outcomes must not be visible.
10. Pause refusals and timeouts are not journaled as not counted, so `verify_run` disagrees.
11. No test for refusing `reapply: UPDATES` on a code run.
12. The history-limit reasoning of Non-functional requirement 044-001, the review's trigger, the window bound's record.
13. The order of the unseen-events check against the completion's own immediate effects.
14. Calls that end the run thrown inside a saga body.
15. A detached child blocks continue-as-new for ever.
16. Command records do not carry the accepting version.
17. A continuously busy run cannot be moved.

Counts: Blocker 0, Major 6, Nit 11
