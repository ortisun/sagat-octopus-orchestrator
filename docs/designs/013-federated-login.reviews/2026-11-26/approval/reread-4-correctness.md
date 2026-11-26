# Targeted correctness re-read of design 013 for approval, round 4 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: a8820014ea23e0d9a2b5305437956597d29e197a
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: a re-read of the changes since round 3 (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. Nothing in the changed text reopens an earlier finding. Uncounted, the reviewer also noted that the verification filters `keys` (Task 013-8) and `signin` (Task 013-9) match few of the listed test names unless the module files carry those names.
Counts: Blocker 0, Major 0, Nit 5

1. **Nit. Technical Plan 12, `revoke`.** The random-value batches also rotate sessions signed in under the new key after the ring write, and running `revoke` again is a second revocation, not a resume. Fix: record the ring write's commit point and rotate only sessions older than it; a `revoke --resume` that writes no ring version; tests.
2. **Nit. Technical Plan 4, "Verification order and locking" and "Grace and retirement".** Whether the generation's class and the grace rows are read from the locked row is unstated; from the unlocked read, two requests with one current token both classify it current and the second's outcome is undefined. Fix: the class decided on the locked row; a barrier test of two refreshes of one token.
3. **Nit. Technical Plan 12, the attempt ring.** No command is named as dropping `RETAINED` attempt keys, and dropping 10 minutes after promotion is two intervals short of an attempt sealed by a stale replica. Fix: drop after 10 minutes plus two intervals, by the next `attempt-key rotate`.
4. **Nit. Technical Plan 4, the refresh token format; Technical Plan 3.** The grammar refuses generation 0 but nothing says the first generation is 1. Fix: generations start at 1, a `CHECK`, a redemption-to-first-refresh test.
5. **Nit. "Lock order"; the maintenance passes.** The maintenance loop's session batches have no stated lock order and can deadlock with `revoke`'s. Fix: `ORDER BY tenant_id, id FOR UPDATE` in a subquery, in `lock_order.rs`.
