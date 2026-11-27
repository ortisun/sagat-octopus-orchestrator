# Targeted re-read of design 047, second pass: security (2026-11-26)

Doc: docs/designs/047-oauth-integration-hub.md
Reviewed-commit: d51179f9c41ff88d81911bd8e806638dbaad9166
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output, condensed by the author; every finding and severity is kept. A confirmation of the fixes of
`reread-1-security.md`; the reviewer read the design, the diff of the fixes and the data model, and ran nothing.

## The previous findings

1. Answered for retired and abandoned revisions (finding 1 below brings the failure back through `PENDING` rows).
2. Answered for disconnecting and disconnected credentials (the new rule causes findings 1 and 2). 3 to 7: answered
(in-flight slots from unexpired leases and broker-call rows under the window lock; opt-in predicates, column grant,
trigger and `404`; two foreign-keyed columns with `tenant_id`; 1 in flight and an even share, every call through the
windows; `INDEX` blocks only, pinned `search_path`, `EXECUTE` revoked from `PUBLIC`).

## New findings

1. Major. The sweeper revokes "every `PENDING` row older than the lease plus the store deadlines", rows of `ACTIVE`
   credentials included, which are the rows the adopt step activates. A holder that stores, marks and crashes before
   activating, with nobody asking for a token within about 45 s, has its row revoked: with Google the whole grant ends;
   with rotation the row holds the only valid refresh token; with a same-value provider it is the live token. The
   credential goes to `NEEDS_REAUTHORIZATION`, skipping finding 1's guard. Fix: revoke only rows of disconnecting or
   disconnected credentials; adopt a stale row of a live credential, and once replaced destroy it in the vault only;
   `a_stale_pending_revision_of_a_live_credential_is_adopted_not_revoked`.
2. Nit (suspicion). The rule takes a disconnecting credential's `PENDING` rows at once, including rows whose vault write
   is still running; closing an unmarked row and then a late write and a crash leave a live token untracked, with
   `REVOKED` recorded. Fix: skip unmarked rows younger than the store deadlines, or claim the write-once reference first.

Counts: Blocker 0, Major 1, Nit 1

## The author's answers (both fixed; Decision 047-062)

1. The sweeper revokes only rows of disconnecting or disconnected credentials; a stale row of a live credential is adopted
and, once replaced, destroyed in the vault only; the test named. 2. An unmarked row is taken only after the lease and
store deadlines; `the_sweeper_waits_for_a_vault_write_still_running`.
