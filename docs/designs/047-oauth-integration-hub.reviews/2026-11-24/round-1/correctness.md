Doc: docs/designs/047-oauth-integration-hub.md
Reviewed-commit: f6fef1d6c9277d7f88b5b159177b0cbd7af5decd
Reviewer: correctness (repository agent `.claude/agents/correctness-reviewer.md`)
Agent-blob: correctness-reviewer
Counts: Blocker 1, Major 8, Nit 3

Condensed by the author from the reviewer's output; every finding and severity is kept.

1. **Blocker.** Activation compares only the fence and no other writer moves it: a disconnect during a refresh leaves a rotated, never-revoked token on a terminal row; a reconnect is overwritten by a stale holder; a lapsed holder's `invalid_grant` moves a healthy credential to `NEEDS_REAUTHORIZATION`. Fix: every writer of state or pointers increments and compares the fence and states the expected state; a disconnect waits for a held lease and revokes the newest refresh revision; tests in both orders.
2. **Major.** The fence protects activation, not the provider call: a paused holder sends the old refresh token after its lease lapsed; the store step has no deadline. Fix: check elapsed time before sending; a deadline on the store; reword to "at most one activated"; a paused-holder test.
3. **Major.** After the write and before activation the rotated token is durable behind a feature 017 intent that `reconcile` can verify; the design discards it. Fix: the next holder adopts the newest unactivated intent first; an expected outcome per crash point.
4. **Major.** Acquire does not check that a refresh is still needed; a second caller refreshes again. Fix: compare the observed access revision in acquire.
5. **Major.** The instant `expires_in` is measured from is unstated. Fix: the database `now()` returned by acquire plus `expires_in`.
6. **Major.** Feature 017's `ManagedSecretService` needs a principal with `SecretManage`, a `SecretName`, the secrets quota, an actor in its binding, two audit records per write, and its recovery is "resubmit the value", impossible for a token. Fix: a broker principal and entry point, naming, quota for token revisions, a bounded number of live revisions, retirement in the activation transaction, a lost write means reauthorization.
7. **Major.** Check and mark of `state` as two steps lets a double submit exchange twice (the provider may then revoke); a Studio retry after success gets a conflict. Fix: one conditional update; a repeat by the same principal returns the outcome; a concurrent double-submit test.
8. **Major.** Nothing drives `PENDING` after a crash, `DISCONNECTING` after a crash, or revocation after a resource deletion; delivery not stated to refuse non-`ACTIVE`. Fix: a leased reconciliation pass; the deletion transition in the deletion's transaction; the state rechecked after a token request.
9. **Major.** "Each part in one transaction" versus "one transaction per request"; the idempotency key is unnamed; same version with a different digest; key reuse with another body. Fix: one transaction per integration; uniqueness on tenant, kind, item and version with a digest compare; `IDEMPOTENCY_KEY_REUSED`.
10. **Nit.** Refresh outcomes `invalid_client`, `unauthorized_client`, `invalid_scope` have no state; `access_denied` and a slow person's `invalid_grant` are not `502`; waiters should fail at once on `invalid_grant`; the waiter's wake-up is unstated.
11. **Nit.** The initial state of a client-credentials credential; `tenant_id` missing from the activation statement.
12. **Nit.** The re-ask budget must be counted atomically; `last_used` on every delivery makes a hot row.
