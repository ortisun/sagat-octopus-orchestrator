# Targeted correctness re-read of design 013 for approval, round 1 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 5d451eb9
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: a re-read of the text changed since the reviewer's full review of 2026-11-25 (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept.
Counts: Blocker 0, Major 4, Nit 6

1. **Major. Technical Plans 3, 4 and 6, Requirement 013-013.** The hand-off of recomputation beyond 100 members, and of a rule change's job, is recorded nowhere, so a crash or a second rule change loses demotions (constitution principle I). Fix: a durable marker or job row with a rules revision and a cursor in the change's transaction; compare-and-set; restart on a moved revision; two tests.
2. **Major. Technical Plan 13, Architecture decision 0050's addition, Task 013-11.** SCIM requests take the settings row `FOR SHARE` and a demotion of an admin takes it `FOR UPDATE`; two such SCIM requests deadlock, and the barrier test passes on the deadlock. Fix: choose the strength before the first lock; roll back before restarting; the tests assert one success and one `LOCKOUT_GUARD`.
3. **Major. Requirement 013-018, Technical Plan 4 "Redeem".** The code is consumed before `SESSION_CONFLICT` is decided, so a committed refusal spends it, and an expired code leaves the person stuck. Fix: check the cookie before consuming; the Studio's path after expiry; redeem one code twice in the test.
4. **Major. Requirement 013-021, Technical Plan 6.** A mapped demotion the last-administrator guard refuses would fail the whole SCIM request forever or loop the job. Fix: the group change commits, the demotion is skipped, audited and shown, not retried until the rules or groups change; a test.
5. **Nit. Technical Plan 4, "Signing keys".** After a revoke, a fresh replica keeps signing with the revoked key for up to two intervals, spending codes for refused tokens. Fix: check the handle's `kid` in the transaction and roll back.
6. **Nit. Technical Plan 12.** A new attempt key seals at once while other replicas cannot open it until their refresh. Fix: add it as an opening key first; a test.
7. **Nit. Requirement 013-026.** The grace covers one lost response; a second loss revokes the session. Fix: extend it, or state the limit.
8. **Nit. Requirement 013-009, Technical Plans 3 and 6.** A purged code no longer marks its attempt as used while the cookie lives 10 minutes. Fix: a tombstone until the attempt expires.
9. **Nit. Technical Plan 4, "Lock order".** Whether a group is named is read before the shared fence; an assignment between would let an authority change run under the shared fence. Fix: re-read under the fence and restart; a barrier test.
10. **Nit. Technical Plan 6.** "A genuine reactivation is sent again" assumes provider retries after a `409`; whether it counts toward `scimFailing` is unstated. Fix: the fixtures show it; state the count.
