# Security review of design 044, round 4 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: ba5a3fdd02abfc57e03b76efc4a9fb2e237b5c45
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Round 1's Blockers 1 to 3
are closed; round 3's Majors 2, 3, 4, 7, 9, 10, 11 and 12 are closed; the rest are partly closed and raised below. No
cross-tenant path, open metric label, injection or new egress was found.

## Major

1. Graph publication reserves only `sagat.database.`, so a graph activity named `sagat.workflow:<type>:<digest>` would be
   leased to a workflow session and could resolve secrets; reserve `sagat.` at graph publication too, refuse a reserved
   name on a step that is not `WORKFLOW_TASK`, scan published versions before recording the requirement.
2. The new RPCs do not refuse an activity, managed-runner or script-runner lease, so an activity lease could read the
   whole history; a lease-kind matrix per RPC; `workflowTypes` only on ordinary bindings of the worker role.
3. Eager leases bypass the claim's admission (quotas, breakers, pools, capacity, gate grants, approvals); route them
   through it; the eager workflow task counts in the bucket.
4. The approval initiator of an engine-held step is undefined; an update's admitter can approve the effect it caused;
   exclude every principal who drove the run up to that command.
5. Another principal's update key answers `409` where a missing key answers `404`, an existence oracle; scope keys by
   run and admitting principal.
6. Worker-query arguments and answers in `payloads` stay forever and outside the content quota; keep them inline and
   bounded in the row.
7. Feature 016's carriers are closed (workflow input and output, node `outputSchema`); the envelope schemas are not
   carriers, so the redaction cannot be built; amend feature 016's carriers, and treat every other payload as `pii`.

## Nit

8. A signal sender can fill the 4 MiB and turn later outputs into unknown outcomes; byte headroom; per-principal shares.
9. No blob owner kind for uploads under a workflow lease.
10. An idempotent completion replay is answered before the re-checks.
11. The move function cannot prove a replay check the runtime role can write; claim only what it guarantees; the pin's
    primary key.
12. The pause should cover query and replay-check tasks.
13. Target grammars and the digest form at registration.
14. Replay-check verdicts and digests are worker-supplied and unbounded.
15. `PYTHONHASHSEED=0` is allowed.
16. `workflowType` is not unique per namespace.
17. The fragment could set a gate's `limit` and `group`.
18. Audit history reads of checks and queries; `secret` fields in workflow history.
19. Unsalted content digests are never to be returned on reads; say and test it.

Counts: Blocker 0, Major 7, Nit 12
