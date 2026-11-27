# Targeted re-read 6 of design 048: security (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: 32d579fffba202282a5a44d5004247e274333beb
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A sixth targeted re-read
of the fixes of re-read 5, not a full review. The reviewer read files only and ran nothing.

Re-read 5: Nit 3 and the suspicion resolved; Nits 1 and 2 partly (below).

No Blocker, no Major.

## Nit

A. The authenticator's own `UNAVAILABLE` (a failed fetch of the identity provider's keys, a session lookup) answers with
   `Retry-After` 5 s before authentication, and Task 048-7's reason would show on the session-lookup path. Fix: add the
   authenticator's refusals to the constant answer and the test.
B. Task 048-9's amend list misses six files that state the public reason (an integration test, two contracts, a runbook,
   doc comments, a hint branch that becomes dead).

A suspicion: the probe's `pg_hba` line, if applied through Patroni's configuration, would stay cluster-wide.

Counts: Blocker 0, Major 0, Nit 2

Both and the suspicion were fixed at `4b988eb4`.
