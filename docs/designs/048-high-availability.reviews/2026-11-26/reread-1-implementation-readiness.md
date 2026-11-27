# Targeted re-read 1 of design 048: egm-implementation-readiness (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: a29b72b279a265328a1a0570c120c994d81953f7
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A targeted re-read of the
fixes of the full round (`implementation_readiness_report.md`, at 1e283510), not a full review.

Full round: Blocker 1 and Majors 1, 3 to 10, 12 resolved; Majors 2 and 11 partly (Majors 5 and 6); Nits 2, 4, 5
resolved; Nit 1 moot; Nits 3 and 6 partly.

## Major

1. `WriteTx`'s interface is undefined for about 233 sites (the methods' signatures, how it reaches `DatabaseIdentity`
   from stores built on a bare `PgPool`, whether the `fetch()` rule applies inside it).
2. Read-only snapshot transactions (three sites) have no type.
3. No task adapts the single-database environments (`compose.yaml`, the test support's pools, the `scale` harness,
   `drill.sh`, the Kubernetes dependencies) to the flag and the confirmation.
4. Tasks 048-9 and 048-10 depend on feature 032's region code, which does not exist.
5. (Full-round Major 2, partly.) What replaces the five module variants, which carry the keys callers reconcile with.
6. (Full-round Major 11, partly.) `input-schema-inventory` has no profile.

## Nit

1. Two more copies of `confirmed_abort`.
2. `a_role_setting_of_synchronous_commit_local_is_refused` misnames an outranked setting.
3. Task 048-8 names `crates/scheduler`, which holds no SQL; its dependency on design 053's cut-off.
4. Task 048-2 does not add the target to `select-test-targets.py`; Task 048-15 names no job.
5. Task 048-6 does not name the timeouts it exercises, or what happens if it proves a cancel.
6. Task 048-13 names no rule files or expressions; no task reads `SAGAT_DB_COMMIT_DEADLINE_MS`.

Counts: Blocker 0, Major 6, Nit 6

All twelve were fixed at `b7cf600b`; the region check and its tests became feature 032's, extending the two functions.
