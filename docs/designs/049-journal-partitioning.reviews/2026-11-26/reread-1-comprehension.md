# Targeted re-read 1 of design 049: egm-comprehension (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 23c03d12bf28c770043e7dc330dc077e5221e5c9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A targeted re-read of the
fixes of the full round (`comprehension_report.md`, at 66634fb9), not a full review.

Full round: Blockers 1 to 3 resolved (2 with Major 1 below); Majors 1 to 12 resolved (7 with Major 2 below; 9 with Nit 1
below); Nits 1 to 3 and 7 resolved; Nit 4 partly (the step 1 trigger names `run_payloads` and the registry before they
exist); Nit 5 partly (run-purpose blobs, sinks and holds, the active region, design 052's queue undefined; held-run
reasons unmapped); Nit 6 open (other decision records not linked); Nit 8 partly (advisory lock keys not given); Nit 9 open
(the last sentence of Requirement 049-014 and the savepoint re-check need feature 020 to be read).

## Major

1. The root's `created_at` is said to be bound before the first write, yet the Problem section has it read at the run
   insert, and the 24-hour rule needs the database clock before any statement. Fix: name the clock and when it is read.
2. "The two shared columns" means two different pairs in two places, and the Layout and the test use it unnamed.
3. The Overview, CUJ 1 and the Goals say one transaction detaches, drops and releases; Requirement 049-009 has three.
4. Requirement 049-016 has the retirement call feature 020's purge function in the mark-only form, which `sagat_app` may
   execute, while Requirement 049-014 says that function has neither form. Fix: a separate owner-only function.

## Nit

1. The SQLSTATE, constraint name and mapping of the `runs` trigger's refusals are not given.
2. CUJ 2 says retirement stops before recording anything, while the Retirement section records held runs in a row that
   does not exist yet; "as one" is garbled.
3. Step 1 creates partitions before `ensure_journal_buckets` (Task 049-9) exists, without the previous month, and Task
   049-9 is not in the merge order.

Counts: Blocker 0, Major 4, Nit 3

Fixed at `dff2ebed`: the creating transaction's `clock_timestamp()` or the command's time, and the process's clock for the
24-hour rule checked by the trigger with `clock_timestamp()` (Major 1); the columns named everywhere (Major 2); three
transactions in the Overview, CUJ 1 and the Goals (Major 3); `purge_run_content_mark_only` (Major 4); SQLSTATE 23514 and
`runs_journal_bucket`, an internal error (Nit 1); held runs printed only, waits recorded once the row exists (Nit 2); step
1 calls `ensure_journal_buckets()` and Task 049-9 merges first (Nit 3); the step 1 trigger has its legacy branch only and
is regenerated in step 3 (full-round Nit 4); the terms and reasons defined (full-round Nit 5); the lock keys given
(full-round Nit 8). Full-round Nits 6 and 9 are settled, not fixed (the egm-review summary).
