# Targeted re-read 4 of design 048: security (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: 6e923c171dc89e9725ca3dee92cba0949d238de7
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A fourth targeted re-read
of the fixes of re-read 3, not a full review. The reviewer read files only and ran nothing.

Re-read 3: Nit 1 and both suspicions resolved; Nits 2, 3 and 4 partly (below). Checked with no finding: the column
grants and definer attributes, the waiting connection, the returned identity, `registry_first_confirmation`, reasons
shown to authenticated callers.

No Blocker, no Major.

## Nit

1. The headline still says "at most 27 s", where a request that starts just before the gate closes runs several
   statements; the real bound is 22 s plus the HTTP request timeout of 15 s.
2. The control plane's readiness is on the public router and names the reason, so an anonymous caller would learn of a
   database reason; it has no admin listener for readiness.
3. The gRPC admission refusal runs before authentication and names a reason; it is not in the rule or the file table.
4. The isolated node has no port on `db-access` to route to; name the mechanism and how the routed container recovers.

A suspicion for the correctness reviewer: with every closed-gate refusal before authentication at `Retry-After` 10 s,
the report window's arithmetic (which uses 2 s) would be 318 s.

Counts: Blocker 0, Major 0, Nit 4

All four and the suspicion were fixed at `9d192bee`: every refusal before authentication (HTTP, public readiness, gRPC
admission) answers `Retry-After` 2 s with no reason, so the window stays 310 s; the reasons go to operators only; the
bound is 37 s for HTTP; the probe reaches the isolated node through `db-probe` and is stopped after the check.
