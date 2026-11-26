# Targeted security re-read of design 013 for approval, round 46 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 9cbf07ae1011c99eef7d102196b4e8e518cbee71
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the fixes of the fourth correctness and egm-review re-reads (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found the transport visibility, the allowlist, the Studio redemption, the verification order, the lock order and the generation numbering sound.
Counts: Blocker 0, Major 0, Nit 1

1. **Nit. Technical Plan 12, `revoke [--resume]`; Technical Plan 3, `signin_sessions`.** The commit point lives only in the audit row, written after the ring's check-and-set, so a crash between them leaves `--resume` nothing to continue from, and the likeliest implementation resumes from an earlier revocation and exits 0; and "random value older than the commit point" names no column and no clock. Fix: record the commit point in the ring document in the same check-and-set, `--resume` reading it and refusing when there is none; a column written by the database for when the random value was set.
