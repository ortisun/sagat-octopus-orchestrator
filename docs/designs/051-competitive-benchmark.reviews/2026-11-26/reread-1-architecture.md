# Targeted re-read 1 of design 051: architecture (2026-11-26)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: 528ffae3ce81536de5f410a5d008432829788c38
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A re-read of the fixes of
`architecture-full.md`, not a full review. The reviewer read files only and ran nothing.

Full round: Majors 2, 3 and 5 and all Nits resolved; Majors 1, 4, 6 and 7 partly (below as A to E).

## Major

1. (A) The driver's interface lacks the worker and record-volume contract, a schema version of its JSON output read
   across releases, the per-second completions, refused starts and run identifiers design 053 needs, one definition of
   transition latency with design 053, and a bound of `capacity`.
2. (B) The production profile is stated but unchecked; the release Dockerfile and profile unstated.
3. (C) The `concurrency` group, `max-parallel` in `campaign.json` and the stages' order have no check; the
   release-candidate ordering is prose.
4. (D) The release-note rule cannot be implemented (the tagged commit is never the campaign's), and its gate is manual.
   Fix: an ancestor whose diff touches only `docs/` and `tests/performance/reports/`; the checker in `release.yml`'s
   `version` job.
5. (E) The method version's freeze has no check. Fix: digests of `shapes.md`, `campaign.json`, `images.lock` and the
   driver's lock per job; the aggregation refuses a mismatch without a new version.
6. (F) Task 051-0 leaves `setup-go`'s default cache in the release workflow's `sdks` job. Fix: `cache: false` on every
   setup action; the workflow test and the competitive rule check that no release job restores a cache.

## Nit

1. "Bumped by hand per method version" against a `--locked` path dependency on `sdks/rust`.
2. The claims checker would refuse this design's own goal line and the roadmap assessment's vendor figures; statements
   of fact only, with exemptions and fixtures.
3. No size limit on committed raw data.
4. A third dispatch cancels a pending one under the `concurrency` group.
5. Name the command `run.sh` runs first.

Counts: Blocker 0, Major 6, Nit 5

All eleven were fixed at the next commit; the release-candidate ordering stays a manual step of the release procedure,
stated.
