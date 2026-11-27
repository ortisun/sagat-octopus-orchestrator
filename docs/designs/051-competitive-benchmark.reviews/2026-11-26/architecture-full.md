# Architecture review of design 051, full round of 2026-11-26

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: 831873036bbf0a4a0e5ec51d988892a4d1f8331c
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: the reviewer's output, condensed by the author; every finding and severity is kept. The reviewer read files only,
ran nothing, could not list branches, and did not repeat the reports beside this one. No ADR is needed: the choices are
reversible test tooling.

## Major

1. Designs 049 and 053 need a driver interface this design never defines (053's `mixed-writers` job drives
   `infra/docker/compose.yaml` at half the measured capacity with a year of monthly history; the rule forbids a
   workspace dependency on the driver; the loader cannot make monthly buckets). Fix: consumer subcommands (`capacity`,
   `load --rate --duration --endpoint`), a JSON output with the two latencies, Cargo features so this platform's adapter
   builds alone, the SDK version that drives an N-1 server, dated histories as design 049's fixture.
2. The driver adds a fifth Rust lock coupled to `sdks/rust` (and Go modules to `sdks/go`) that nothing rebuilds when the
   SDK changes. Fix: the dependency form stated; a CI step building and testing the driver and the Go modules, selected
   for the driver, `sdks/rust/` and `sdks/go/`, and on full runs; a selector test.
3. A dispatch can run code no check has seen (the rules run only on full runs; dispatch from any branch), and a campaign
   is not tied to a reviewed commit. Fix: `run.sh` runs the competitive rule first; the workflow refuses refs other than
   `develop`, `main`, `release/**` and `v*`; the report records the ref and commit; Requirement 051-017 counts only those.
4. This platform's configuration under test is not fixed (image profile, authentication, TLS, audit, telemetry, log
   level); a pilot could switch off audit or run the development issuer. Fix: every product's documented production
   profile; this platform's release image, no development issuer, audit on, telemetry exported within its budget,
   info logs; TLS on or off for every product alike; no pilot turns off telemetry or audit.
5. The published raw data expire (artifacts are kept 90 days). Fix: commit the per-job summaries under
   `tests/performance/reports/raw/competitive-benchmark/<date>-<commit>/` with a manifest of SHA-256s; only the
   regenerable history archives expire.
6. A campaign can starve CI for a day (the free plan's 20 concurrent jobs per account; 12 for a day), and two dispatches
   overlap. Fix: a `concurrency` group; the rule checks it and that all matrices' `max-parallel` sum to 12 at most; the
   release-candidate CI run ordered apart from the campaign.
7. Two MUST rules without a failing check: Requirement 051-017 (a checker refusing a comparative line without a dated
   section, and a release note citing another commit, run where checkers run and before a release); the ceilings fixed
   before any pilot (a method version in `shapes.md`, recorded, the aggregation refusing a change without a new version).

## Nit

1. The workflow's rule beside `scripts/tests/test_ci_workflow.py`'s precedent, with every `uses:` pinned to a commit.
2. Keep the matrix in a file the workflow reads, so tuning never edits `.github/workflows/`.
3. The driver's lock and Go modules bumped by hand per method version; competitor digests frozen per method version.
4. An internal network publishes no port; the driver joins each product's network; Windmill's egress at start.
5. Design 052's "maximum sustained rate" is its own; never place its figures in this report.
6. A step timeout below the job's so that the partial raw data upload still runs.

Counts: Blocker 0, Major 7, Nit 6
