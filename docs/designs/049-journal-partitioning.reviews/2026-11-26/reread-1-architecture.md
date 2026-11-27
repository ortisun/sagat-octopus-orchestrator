# Targeted re-read 1 of design 049: architecture (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: acd8c828a0955e187596fc4f71c4894612662d29
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A targeted re-read of the
fixes of the full round (`architecture-full.md`, at 66634fb9), not a full review. The reviewer read files only and ran
nothing.

Full round: Blockers 1 and 2 resolved; Majors 1, 2, 3, 5, 8, 9 resolved; Major 4 partly (the waivers leave out the
non-concurrent index builds, the several insert-blocking locks, the unique index on an existing table, and
`stopped-upgrade` is not a rule of design 053's lint); Major 6 partly (the legacy reference check is built by both
T020-046 and T049-11); Major 7 partly (T020-061 still opens its own amendment); Nits 1 to 10 resolved (Nit 7 with new
Major 1); Nit 11 open until Task 049-1; Nit 12 open (no shell, so no duplication check).

## Major

1. Design 053 still lets `journal-buckets-v1` be deactivated and its mixed-writer test and rollback journey deactivate
   N's capabilities; an N-1 binary cannot read a dated run's content. Fix: 053 refuses to deactivate this capability and
   leaves it out of the test.
2. A migration that grants `pg_read_all_stats` to the owner fails on PostgreSQL 16 and on managed services, blocking the
   upgrade. Fix: no grant in a migration; the runbook names it; the function reports an unknown start.
3. Retirement accepts `archive: NONE`, against Owner decision 2026-11-25-3 point 1 ("only after a verified archive") and
   the Constitution Check. Fix: drop the exception.
4. (Full-round Major 4, partly.) As above.
5. (Full-round Major 6, partly.) As above.
6. (Full-round Major 7, partly.) As above.

## Nit

1. The ADR's Refines line and the Risks table say the bytes are released in the drop's transaction.
2. "Enforced by" leaves out the rule that only `journal.rs` reads `run_events` for one run.
3. How the switch state reaches the store from the application's registry is not named.
4. The retirement command's metrics are never scraped; say whether they are printed and recorded.

Counts: Blocker 0, Major 6, Nit 4

All were fixed at `23c03d12`: design 053 declares `journal-buckets-v1` permanent, refuses its deactivation and leaves it
out of the rollback (Major 1); no migration grants a predefined role and the function reports an unknown start (Major 2);
a verified archive is required for every run (Major 3); the `stopped-upgrade` rule is defined in design 053 and the
design lists every rule it waives (Major 4); Task 049-11 builds only the indexes and the check stays T020-046's (Major 5);
T049-15 points T020-061 to 5.1.0 (Major 6); the ADR's wording, its "Enforced by", the creation command's flag and the
printed and recorded results (Nits 1 to 4).
