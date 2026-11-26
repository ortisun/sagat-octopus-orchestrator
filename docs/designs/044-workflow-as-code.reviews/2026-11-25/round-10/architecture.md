# Architecture review of design 044, round 10 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 4a943e7c15d9ca73e9c1afa6274ad5da8cfb5cbb
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. All three round-9 Majors
are closed. The constitution conformance, the layering, the ADRs and the code claims hold.

## Major

M1. The server-side fields live in two places, and resets are kept away from the one they need: one model,
    `server_detail`, its readers named, copied by the reset; a test.
M2. A move item's replay-check phase does not fit feature 018's executor (one due item at a time, exhaustion after ten
    refusals or an hour): a `REPLAY_PENDING` defer cause that is not a refusal, checks created ahead for upcoming items,
    a creation time, a separate busy count; tests.
M3. A cancellation without grace after a graceful request is undefined: the arm, first-request-wins for graceful
    requests only, the allowlist; a test.
M4. The migration holds `ACCESS EXCLUSIVE` on `runs`, `steps` and the journal while it validates and indexes: split the
    validation into a second migration, build indexes as the scheduler-shards migration does, state the lock time.
M5. A red test refuses replay checks while paused, against the rule that they go on.

## Nit

1. The cache's drop rule against incremental reads; the 768 KiB bound leaves no room for the addressed step.
2. The route table: worker queries' `409`, the binding permission, `graceMs` on a graph run.
3. A dangling fragment in Requirement 044-009.
4. "A forced cancellation" remains once.
5. The cancellation allowlist names only the deadline pass.
6. The CI budget pattern does not match the new use-case file.
7. More callers of the cache's rename.
8. Draft 048 (expand and contract, validation later, `platform_requirement_activate_v2`) is missing from section 12.
9. Stale register entries for the esbuild review and the `golang.org/x/tools` version.
10. The `outcome_seq` guard names no mechanism.
11. Which events count in Gap 044-025's fallback.
12. A Python SDK test listed under the server's security tests.
13. The `sagat.` refusal applies to the publication parse only; a restore test.
14. No check for the single history projection.

Counts: Blocker 0, Major 5, Nit 14
