# Targeted re-read 2 of design 048: egm-implementation-readiness (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: b7cf600bb2032a30eeb4ff602b5a6d9f440a7984
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A second targeted re-read
of the fixes of re-read 1, not a full review.

Re-read 1: Majors 2 to 6 and Nits 1, 2, 4, 5 resolved; Major 1 partly (Major 1 below); Nits 3 and 6 partly.

## Major

1. Helpers shared by snapshot reads and writes (about 219 typed `&mut PgConnection`) and the savepoints of `events.rs`
   have no interface under `WriteTx`.
2. The change of about 617 store constructors to `DbHandle` has no owner, and `DbHandle` (Task 048-7) needs the port
   that Task 048-10 creates; the order of Tasks 048-7 and 048-8 is not stated.
3. `input-schema-inventory`'s code 1 already means "non-conforming versions found".

## Nit

1. Task 048-13's rules have no expressions and are not wired into `prometheus.yml`, the `promtool` steps or the contract
   test.
2. The Technical Plan's table still names `crates/scheduler`.
3. There are eleven unknown-outcome tests in seven files, and `capacity_transfer.rs` detects one too.
4. No task reads `SAGAT_DB_REQUIRE_SYNCHRONOUS_STANDBY` or exports its metric.
5. Task 048-6's other branch needs Task 048-7; "the jobs" of the budget; the harness's path.

Counts: Blocker 0, Major 3, Nit 5

All eight were fixed at `e50e6886`.
