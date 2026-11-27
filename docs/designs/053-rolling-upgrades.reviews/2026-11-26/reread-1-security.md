# Targeted re-read 1 of design 053: security (2026-11-26)

Doc: docs/designs/053-rolling-upgrades.md
Reviewed-commit: aa462893e3ae15be5419e0c093cd7cd80a10881d
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A re-read of the fixes of
`security-full.md`, not a full review. The reviewer read files only and ran nothing.

Full round: Majors 1, 2, 4 and 5 resolved (design 049's and 048's stale names carried by Tasks 049-20 and 048-19);
Majors 3 and 6 partly (below); Nits 1, 3, 4, 5 resolved, Nit 2 partly (the lint job's `permissions: contents: read`).

## Major

1. An evicted row comes back at the process's next heartbeat (`instances.rs`: a row that is gone is registered again),
   with a null applied epoch, so `instance_evict` cannot unblock phase 2. Fix: the runbook stops the process first
   (restart, or rotate the credential when compromised); the evicted instance is recorded and refused at registration;
   a test with the process kept running.

## Nit

1. Form (a)'s `FOR KEY SHARE` needs `UPDATE` privilege, which `sagat_app` lacks; say it runs in an owner-owned definer
   function, as `lease_withdrawal_required()` does, listed in the catalog allowlist.
2. What remains of the lint's self-exemption: owners for the lint, its fixtures, its requirements and the workflow;
   the boundary checks run the base branch's copy; `dynamic-ddl` cannot judge built text (allow only `format()` with a
   literal template from a closed list); the catalog test covers only `public`, not `proowner`, and not both directions
   of the allowlist; the release workflow checks only that the online rules' start is present, not its value.
3. The eviction's history row has no table or action.
4. Suspicion: granting `command_await_durability` to `sagat_retirer` may add a write path; state that it writes nothing.

Counts: Blocker 0, Major 1, Nit 4

All five were fixed at the next commit.
