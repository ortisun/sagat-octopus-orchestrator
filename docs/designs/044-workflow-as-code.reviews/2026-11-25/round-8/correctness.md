# Correctness review of design 044, round 8 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 3884f9f3db9c7ec9c7405f2970a02985c4249711
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. All six round-7 Majors
are closed.

## Major

A. Feature 014's continuation pass, as built, refuses or blocks a code run at each of its checks (boundary step,
   quiescence, the version columns, the latch, the entry), and `version: active` is checked at the completion and
   resolved at the handoff: record the resolved version at the completion; specify the pass's code arm; a barrier test.
B. Continue-as-new and acknowledge-cancellation have no command record written by the completion, so a refused handoff
   leaves the chain, `verify_run`, the reducer and an exported replay disagreeing: journal a record for each.

## Nit

1. The claim's 51,200 rule inside the window.
2. The completion's own immediate effects and the unseen check.
3. The between-statements crash test cannot fail as written: release the lock after the kill and wait for the backend.
4. Reset points do not exist for chained or child code runs (FR-018-035).
5. Rematerialized timers' fire times and waits' deadlines.
6. A move off a `HISTORY_LIMIT` review; the remedies of `CLOSING_WINDOW_COMMAND`.
7. A pause refusal can still be counted at the lease's expiry.
8. A scope closed with a member still running.
9. "Opens the activation" in Requirement 044-010 and section 1.8.
10. Whether a terminal completion's stop-drain rows count against the window's bound.

Counts: Blocker 0, Major 2, Nit 10
