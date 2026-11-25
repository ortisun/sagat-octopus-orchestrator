Doc: docs/designs/047-oauth-integration-hub.md
Reviewed-commit: e1d438b0ff4e17994feb29517a4d0cd2582b1349
Reviewer: correctness (repository agent `.claude/agents/correctness-reviewer.md`, round 2)
Agent-blob: correctness-reviewer
Counts: Blocker 1, Major 10, Nit 3

Condensed by the author; every finding and severity kept. Round 1: answered 4, 7, 8, 10; in part 1, 2, 3, 5, 6, 9, 11, 12.

1. **Blocker.** A disconnect between the provider's rotation and the store makes the holder stop and drop the only valid refresh token; the revocation pass does not wait for the lease nor look at pending rows, revokes the old token and records `REVOKED` while the new grant lives; a late vault write after destruction leaves an orphan. Fix: once the provider answered, always finish the insert and the vault write (only activation is fenced); the pass waits for the lease to lapse and revokes the newest refresh revision with a vault entry, pending ones included; a barrier test between call and store.
2. **Major.** "Vault write complete" has no durable signal; a crash between the write and the mark sends the old token again. Fix: probe the write-once reference of every earlier pending row, as feature 017's reconcile does.
3. **Major.** An adopted access token has no expiry; adoption could pick a reconnect's pending rows. Fix: store expiry, scopes and origin on pending rows; adopt only refresh-origin rows.
4. **Major.** Superseded pending rows and a crashed callback's rows are never retired or revoked and fill the bound of three. Fix: activation retires lower-fence pending rows; a pass revokes and destroys them.
5. **Major.** The monotonic clock starts at the reply, not before the statement; a pause between the check and the send overlaps the next holder; a timeout is an unknown outcome at rotating providers; the Risks table overstates. Fix: start the clock before sending; state the residual window and the retry after a timeout.
6. **Major.** Lock order: disconnect locks credential then resource, deletion the reverse (deadlock); two deletions naming one credential each see the other (write skew). Fix: one order; lock the credential before counting.
7. **Major.** Which fence the callback reads; a reconnect racing a refresh; start or callback on a terminal credential; a user's disconnect failing on a bumped fence. Fix: each writer increments with a state predicate; refuse terminal states; outcomes per race order.
8. **Major.** A start call with an empty list is not counted; "30 per attempt" cannot be enforced on rows keyed by attempt and resource. Fix: a per-attempt row with a start flag and a total.
9. **Major.** Delivery checks only the publisher's `resource:use`; the initiator's too.
10. **Major.** One `invalid_client` marks a registration `BROKEN` and every credential on it, a shared one included; no recovery. Fix: repeated or confirmed failures; a recovery; per-credential updates in order.
11. **Major.** No unique keys for (tenant, kind, item, version) with the digest and for (tenant, request key); the import row shape.
12. **Nit.** A client-credentials credential with a null access revision never matches "equals"; the exchange's expiry base.
13. **Nit.** `RESOURCE_UNAVAILABLE` at delivery for a blocked hub version, while Requirement 047-022 blocks only new admissions.
14. **Nit.** Nothing drives the disconnect after a withdrawal; a failing holder does not clear its lease; waiters after a crashed holder never acquire; the paused-holder test needs an injectable clock.
