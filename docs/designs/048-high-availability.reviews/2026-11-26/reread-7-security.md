# Targeted re-read 7 of design 048: security (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: 4b988eb4e01dfd9fd9c9905603dc8827506a7d17
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A seventh targeted
re-read of the fixes of re-read 6, not a full review. The reviewer read files only and ran nothing.

Re-read 6: Nit B and the suspicion resolved; Nit A resolved in the requirement, not carried by the task (Nit 1).

No Blocker, no Major.

## Nit

1. Task 048-9 and the file table do not carry the authenticator's mapping (`error.rs`'s `AuthError` arm, `extract.rs`)
   or name its causes in the test; map only at those call sites, since `unavailable()` also serves after authentication.
2. The webhook ingress (`routes/events.rs`, `auth/webhooks.rs`) answers a failed key fetch with 5 s before
   authentication; add it to the list, the mapping and the test.
3. (Suspicion.) Patroni may rewrite `pg_hba.conf` at the demotion, removing the probe's line, and the check would pass
   without a positive control. Fix: a node-local `hba_file` Patroni does not manage, a positive control, removal in a
   `trap`.

Counts: Blocker 0, Major 0, Nit 3

All three were fixed at `f3180b01`.
