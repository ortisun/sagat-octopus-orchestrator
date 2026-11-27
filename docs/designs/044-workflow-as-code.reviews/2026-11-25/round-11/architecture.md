# Architecture review of design 044, round 11 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 9bdb707d84f3fef3c190979cde128256f2b8159a
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. All five round-10 Majors
are closed. The ADRs, the layering, the code claims, the arithmetic and the fitness functions hold.

## Major

1. The one cancellation function has no case that stops a run the function itself ended (acknowledgement,
   continue-as-new after a request, latched handoff): a closed `CancelOrigin` whose last three stop at once; a check
   counting every `stop_run(` call site; a test.
2. The `outcome_seq` check cannot tell a code run's step from a graph node named `c-1`: a `steps.code_step` column used by
   the check and the index; a test.
3. The continue-as-new chain check takes the chain lock after the run lock, and its read is outside the budget: an
   unlocked read counted in the budget, the continuation pass's re-check as the authority.

## Nit

1. "A forced cancellation" remains once.
2. A test and a decision still say 768 KiB.
3. Architecture decision 0103 names the 256-command test.
4. A stale constraint name remains.
5. `JournalRow` does not list `server_detail`.
6. Two source-text rules hosted in the integration suite instead of `apps/execution-engine`.
7. Replay-check bounds counted in tasks or items.
8. `REPLAY_PENDING`'s next attempt time when the verdict is recorded.
9. An SDK replayer test listed under the server's security tests.
10. Open Questions names the gaps up to 044-023.
11. "Only the first cancellation request is journaled" without "cooperative" in two places.

Counts: Blocker 0, Major 3, Nit 11
