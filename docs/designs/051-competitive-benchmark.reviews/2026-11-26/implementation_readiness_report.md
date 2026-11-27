# egm-implementation-readiness on design 051, full round of 2026-11-26

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: 831873036bbf0a4a0e5ec51d988892a4d1f8331c
Note: the reviewer's output, condensed by the author; every finding and severity is kept. The reviewer read files only
and ran nothing; it did not repeat the reports beside this one.

## Blocker

1. This platform cannot declare an activity "safe to retry": `end_attempt` sends a declared activity whose attempt ends
   with `LEASE_EXPIRED` or `ATTEMPT_TIMEOUT` to review unless the node has a reconciliation (the safe-repeat
   qualification is only proposed, ADR-0076, and design 045's Non-Goal), so fault (b) sends every activity in flight to a
   person. Fix: each activity of this platform declares `reconcileWith` a reconciler registered with
   `worker.Host.RegisterReconciler` whose verdict is `NotApplied()` (the step re-runs, a repeat); reconciliations write
   records but are not transitions; the constraint and Success criterion 051-003 reworded.

## Major

1. The script containers cannot be put under the product's control group without platform code (`docker.rs` sets no
   `cgroup_parent` or `cpuset_cpus`, and a test asserts it). Fix: the daemon's default `cgroup-parent` set to a slice of
   the product's set during this platform's turn, the daemon restarted between turns, or another design's setting.
2. `SHOW` sees only its own session, so the durability check cannot fail; `fsync` and `full_page_writes` are not session
   settings. Fix: `pg_settings` and `pg_db_role_setting` at each turn's start; the products' connection options
   published; a scan of `pg_stat_statements` for `SET` or `set_config` of the names after every rung.
3. `pg_walinspect` may find the window's WAL recycled. Fix: a physical replication slot reserved at the window's start,
   dropped after the scan; its WAL in the disk budget, the scan's time in the rung.
4. This platform has no published images: its compose builds local images, refused by the digest rule; a cold release
   build in every job does not fit. Fix: build once in the history job, save as a pinned artifact, load later; exempt
   `sagat/` as "built at the commit under test"; the minutes and disk in the plan check.
5. The driver's unit tests and the rule's fixtures never run in CI (the selector classes the directory as no suite).
   Fix: a `cargo test` step for the driver's workspace, selected for that path, with a selector test.
6. The split between `run.sh` and the driver is undefined. Fix: the driver owns the ladder, rotation and plan check;
   `run.sh` prepares and calls subcommands; their arguments are the matrix fields.
7. The execution-record file's format is unfixed (fields, a product-neutral workflow identifier, markers, clock, write
   discipline, path). Fix: its grammar in `shapes.md`.
8. No task produces the campaign-level report (medians, ratios, the common rate, charts). Fix: an aggregation command
   after all jobs; the chart's format.
9. The no-op ceiling check can only be evaluated after the campaign. Fix: measured at each throughput job's start
   against the highest rate its plan can reach.
10. The architecture rule cannot be written from the text (no YAML parsing in `scripts/`; no root flag; the crate's
    name; the workflow absent until Task 051-8).

## Nit

1. The release procedure is `docs/runbooks/release.md`.
2. The reader's guide has no path; `docs/roadmap.md`; a standing rule is not a task.
3. `SAGAT_DB_REQUIRE_SYNCHRONOUS_STANDBY` does not exist yet (design 048 adds it).
4. The Rust SDK's defaults (30 s, four attempts) against Requirement 051-005; the adapter sets 5 s and one attempt.
5. Fault (a) depends on `SAGAT_SHARD_LEASE_SECS` too.
6. Tests that could not fail: an accepted 350-minute plan; an independent oracle for the injector's 100 ms.
7. The version rule's day comes after the pinning.
8. Task 051-2 cannot audit the sidecar's modules (Task 051-4); the competitors' readers belong to Tasks 051-4 and
   051-5; `govulncheck` for Go.
9. `pg_stat_statements` needs `shared_preload_libraries`; `pg_walinspect` an extension.
10. The seeds, the five orders of the throughput repetitions, the bisection's midpoint.
11. Pin `ubuntu-24.04` and record the image version.
12. Fault (b)'s trigger has no wait limit.
13. The workflow lives in `.github/workflows/`, not under `tests/performance/competitive/`.
14. Whether the variants are versions of one workflow or one workflow each.

Counts: Blocker 1, Major 10, Nit 14
