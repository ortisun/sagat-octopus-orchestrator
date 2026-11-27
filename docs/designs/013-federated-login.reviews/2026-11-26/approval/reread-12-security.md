# Targeted security re-read of design 013 for approval, round 12 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 9a760bd7
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a confirmation of the fix of round 11 (Owner decision 2026-11-25-2, step 4); condensed by the author. The reviewer judged it closed, including a re-run after a crash.
Counts: Blocker 0, Major 0, Nit 1

1. **Nit. Requirement 013-026, Task 013-8.** The check-and-set rule applies to `revoke` too, but not what `revoke` does when it loses its write to a `rotate`; copied from `rotate`, it would leave the leaked key, and the random-value rotation may already have run. Fix: `revoke` retries a bounded number of times, then fails audited; it is done only once its write commits; a barrier test.
