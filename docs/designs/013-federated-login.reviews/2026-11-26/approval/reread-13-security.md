# Targeted security re-read of design 013 for approval, round 13 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: e28824eb
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the round-2 changes (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found the MAC-first order, the locked minting, the reuse rules, the fail-closed empty or stale ring, `revoke`, the answers and the link consumption sound.
Counts: Blocker 0, Major 1, Nit 2

1. **Major. Technical Plan 4 "Refresh tokens", Technical Plan 2.** The re-read on an unknown key id is unbounded, would run in the refresh transaction under the victim's row lock (an OpenBao read amplifier on an unauthenticated endpoint), and a failed re-read answers `invalid_grant`, discarding a usable token. Fix: check the id before `BEGIN`, one single-flight re-read per interval outside any transaction, `503` on a failed re-read; two tests.
2. **Nit. Technical Plan 4.** "The same answer whether the session exists" does not cover timing or the `503` path. Fix: a dummy MAC; decide the `503` before the session lookup.
3. **Nit. Technical Plan 4.** Grace rotations are bounded in time only, and whether they write rows is unstated. Fix: a cap of three per generation; no row of their own.

Outside the security scope: "`RETAINED` keys activated over 7 days ago" should count from the successor's activation.
