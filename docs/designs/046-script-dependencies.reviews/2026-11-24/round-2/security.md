Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: c00d7999f24de5eef7cbe2e2a95d1059bf27f03e
Reviewer: security (repository agent `.claude/agents/security-reviewer.md`, round 2)
Agent-blob: security-reviewer
Counts: Blocker 0, Major 5, Nit 5

Condensed by the author; every finding and severity kept. Round 1: all 19 answered, with residuals in findings 1, 3, 4, 6 and 8 below.

1. **Major.** The gateway's sources of truth are unspecified: if it learns of a reservation only after an upstream fetch, a reserved name is sent upstream; the job-finished call needs a credential or is an oracle. Fix: reserved names and filter inputs in a signed per-job document the runner forwards; a reserved name never causes an upstream request.
2. **Major.** The gateway holds every tenant's private files and is trusted beyond its isolation: a compromised gateway can exfiltrate private packages, report `private` and `uploadedAt`, and swap an npm tarball whose lock hash is SHA-512 while the runner checks a SHA-256 address. Fix: private files behind a credential the upstream-fetching process cannot use; `private` from `private_packages`; the runner verifies the lock's own `integrity`; GET-only egress; restate the residual.
3. **Major.** Blocks are checked at lease, and materialization (up to 10 minutes) follows; malware imported meanwhile starts with the run's secrets. Fix: materialization and the pre-start fetch refuse a blocked artifact; a barrier test.
4. **Major.** The control plane parses an unauthenticated feed without decompression or record bounds and auto-blocks; a poisoned source can block every tenant's common packages in one commit or exhaust memory. Fix: fetch and parse in a sandbox container with bounds; hold an import that adds blocks above a threshold for operator confirmation; correct the egress claim.
5. **Major.** Runner disk exhaustion: the install volume has no quota, `fsize` limits single files, bytecode is outside `maxLayerBytes`. Fix: quota-backed volumes or metering with a kill at the bound; bytecode counted.
6. **Nit.** No permission for removing a malware block; `sagat_app` can delete any block; the import role is unnamed. Fix: a trigger refusing deletion of import blocks unless the advisory is withdrawn; the permission; the role.
7. **Nit.** Channel bounds conflict: 192 KiB decoded exceeds the 240 KiB line; 4,000 requests exceed 400 channel lines; path matching does not handle encodings, `..`, `//` or queries.
8. **Nit.** Upload pipeline: no internal upload route; nobody checks that the archive's name is the reserved one or bounds decompression.
9. **Nit.** Cross-tenant attempts at the gateway and materialization are not audited (Principle V).
10. **Nit.** Decision 046-010 (names per namespace) contradicts Decision 046-027; mark it superseded.
