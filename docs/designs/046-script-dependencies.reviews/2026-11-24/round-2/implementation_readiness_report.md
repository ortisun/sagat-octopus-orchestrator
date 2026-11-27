Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: c00d7999f24de5eef7cbe2e2a95d1059bf27f03e
Reviewer: egm-implementation-readiness (egm-review stage 9, round 2)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Counts: Blocker 1, Major 7, Nit 2

Round 1: answered 2, 5, 6, 8, 11; answered with leftovers 4, 9, 10; partly 1 and 7; not answered 3.

1. **Blocker.** The gateway needs the policy by digest, the tenant's reservations and the token status from the control plane; no route provides them, and the Helm policy lets the gateway reach only upstreams; token issuance and rotation have no task.
2. **Major.** The REST contract does not exist yet (waivers, findings, version packages, SBOM, package rules, deleting an upload, unblock, forced import, the draft `manifest`); no task implements several of them; Requirement 046-011's tests are in no task.
3. **Major.** The claim and materialization answers carry neither the quarantine cut nor `maxLayerBytes`.
4. **Major.** An upload goes "to the package store first" but only the gateway holds the store credential and has no write route.
5. **Major.** Who measures the new public bytes for the fetch budget, from which data, in which transaction.
6. **Major.** The runner side of the test-run path has no task; the test-run claim's fields are undefined; its acceptance test is in no task.
7. **Major.** Phase A's acceptance needs the battery built in Phase E; the gateway's filtering depends on rule crates of the same step.
8. **Major.** Task T046-21 names no files or test, and most metrics and all audit records come from builder A's crates.
9. **Nit.** Missing `Refs:` and tests (Requirements 046-001, 046-003, 046-006; Non-functional requirement 046-002).
10. **Nit.** Task T046-14 omits `workers.rs`; the store administration routes have no task; the materialization identity has no table.
