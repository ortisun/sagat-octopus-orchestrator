# Targeted security re-read of design 013 for approval, round 3 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 66189ab7
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the fixes of round 2 (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer judged all seven findings of round 2 closed, the round-2 scenario now refused, the epoch ordering sound, and found no new Blocker or Major.
Counts: Blocker 0, Major 0, Nit 2

1. **Nit. Requirement 013-010.** If "no requirement" is stored as an empty set, setting the first requirement removes nothing and is not a raise, so CUJ 5's first requirement would record no floor and a later removal would revive its cut-off sessions. Fix: say that with no requirement every value is accepted, so the first requirement is a raise; start the lowering test from no requirement.
2. **Nit. Requirement 013-019, Tasks 013-9, 013-10.** `a_reenabled_connections_old_session_cannot_renew` is in no task. Fix: add it to Task 013-9's red tests, covering renewal and the CLI's refresh.
