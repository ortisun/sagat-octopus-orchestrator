# Approval confirmation of design 051: architecture (2026-11-26)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: c91af7ac2be73ff9eb4a2163d5d124a15f9bbac8
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The last of the targeted
re-reads of the approval run under Owner decision 2026-11-25-2: one full round (`architecture-full.md`), then
`reread-1-architecture.md`, whose six Majors and five Nits are all confirmed resolved here. Not a full review. The
reviewer read files only and ran nothing; it read constitution 5.1.0 from its unmerged worktree (Owner decision
2026-11-25-3). The C extension needs neither an ADR (easy to remove, its trade-off recorded under Alternatives
Considered) nor a constitution rule (a method constraint, not a platform guarantee), only a fitness row (Nit 4). No
ADR is diverged from; isolation holds; designs 048, 052 and 053 are consistent but for the items below. Outside this
count: design 052 still calls design 051's campaign "paid".

No Blocker.

## Major

1. Design 049's Success criterion 049-002 needs a maximum statement latency, while design 051 keeps only `calls` and
   `total_exec_time` differences, a mean; `load --endpoint` also leaves its database side unstated (role, the module
   with `track = all`, who takes the snapshots). Fix: `max_exec_time` per key in the output and its golden schema,
   labelled "since the last reset" (the whole turn in a campaign); `load` given a database URL for a role with
   `pg_read_all_stats` and an option to reset at its start, refusing to start without the module or with `track` not
   `all`.
2. The driver reads this platform's internal schema (the journal, the run row's commit time) and no check catches a
   migration that breaks it, so design 053's gate in `release.yml` could break unseen until a release. Fix: a test of
   both readers against a disposable server migrated at the commit, and the driver's step selected on changes of
   `crates/infra-postgres/migrations/`.

## Nit

1. The freeze digests leave out the `postgres/` configuration and each product's pilot-chosen configuration.
2. "Driving N-1 with the SDK of N-1's tree" has no mechanism under a `--locked` path dependency, and design 053 no
   longer asks for it; remove it.
3. "Run only by hand" clashes with design 053's `mixed-writers` job in `release.yml`; limit it to the competitor parts
   and the campaign workflow, `release.yml` consumers building the default features only under the release cache rule.
4. Fitness rows for `check_competitive_benchmark` and `sagat_commit_check`; the rule refuses a reference to the
   extension or its image outside `tests/performance/competitive/`; the C built with warnings as errors.
5. Task 051-6 lacks a space in "unlogged form,a commit".

Counts: Blocker 0, Major 2, Nit 5
