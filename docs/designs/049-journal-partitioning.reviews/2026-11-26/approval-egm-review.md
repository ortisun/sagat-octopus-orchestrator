# egm-review for the approval of design 049 (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: c621f25b4f50038411fd38bc746a4d92867dc9d5
Reviewer: egm-review
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: the author's summary. The approval follows Owner decision 2026-11-25-2: one full round by every required reviewer,
then the fixes re-read, then the remaining findings settled in writing. The full round of the EGM lenses ran on
66634fb9 (`zoom_out_report.md`, `comprehension_report.md`, `critic_report.md`, `implementation_readiness_report.md`
beside this file), with the repository's architecture, security and correctness reviewers (`*-full.md`). The fixes were
re-read by egm-comprehension (`reread-1-comprehension.md`, then a confirmation at this commit with no Blocker or Major)
and egm-implementation-readiness (`reread-1-` to `reread-3-implementation-readiness.md`, the last at 75f01e1f with no
Blocker or Major; the text changed after it only to answer the security and correctness re-reads). Every Blocker of
every lens was fixed. The Majors below are the full round's that were answered by keeping the design's choice, settled
by delegate rows; the Nits below are the minors left open at this commit, settled the same way.

## Major (full round, settled)

1. egm-zoom-out, full round, Major 3: retirement is overbuilt for an opportunistic path; defer it past 0.3.0. Settled:
   the owner approved the retirement of a whole month in constitution 5.1.0 (Owner decision 2026-11-25-3, point 1); the
   design dropped the scheduled job, the owner credential and every detached-but-not-dropped state the reviewer pointed
   at, so what remains is the smallest mechanism that deletes a month safely.
2. egm-zoom-out, full round, Major 1: partitioning pays only through retirement; ship run-owned payloads unpartitioned
   now and partition later. Settled: 0.3.0's upgrade from 0.2.x stops the writers, so the conversion now needs no online
   machinery, while a later release would need design 053's online rules for a table that by then holds more rows; the
   reasoning is in the Overview ("Why release 0.3.0").

## Nit (settled)

3. egm-comprehension, full round, Nit 6: the decision records other than 0132 are cited by number without links.
4. egm-comprehension, full round, Nit 9: the last sentence of Requirement 049-014 and the savepoint re-check of
   Requirement 049-007 need feature 020 to be read.
5. egm-implementation-readiness, `reread-3-implementation-readiness.md`, Nits 1 to 5 (the runtime-SQL baseline entry of
   `telemetry.rs`, the readiness-check exception in the requirements, the active values of `journal_bucket_retiring`,
   the scope of `ensure_journal_buckets()`, `mark_projected`'s bucket).
6. egm-comprehension, approval confirmation at this commit, Nits 1 to 3 (step 2 of the Retirement section names feature
   020's purge function where it means `purge_run_content_mark_only`; an unclosed parenthesis in Requirement 049-006;
   "a retirement is active" not mapped to the states).
7. The Nits of the repository reviewers' approval confirmations at this commit (`approval-correctness.md` Nits 1 to 5,
   `approval-architecture.md` Nits 2 to 6).

Counts: Blocker 0, Major 2, Nit 5

Findings 3 to 7 are recorded as design 049's follow-up Task 049-19, added after the approval, so that each is fixed in
the build, test first; findings 1 and 2 stay as the design chose. Owner decisions 2026-11-24-516 to 2026-11-24-518 record
the settlements.
