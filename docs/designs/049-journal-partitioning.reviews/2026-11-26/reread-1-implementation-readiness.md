# Targeted re-read 1 of design 049: egm-implementation-readiness (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: dff2ebed1561d6623a772893fa2d7c6dae8afca7
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A targeted re-read of the
fixes of the full round (`implementation_readiness_report.md`, at 66634fb9), not a full review.

Full round: Blockers 1 to 3 resolved; Majors 1, 2, 5 to 9 resolved; Major 3 partly (the `journal.rs` and `payloads.rs`
rules cannot be emptied by Task 049-8: 17 files read `run_events`, 16 read and 8 write `payloads`); Major 4 partly (the
probe's commit sites, the count of round trips and the exclusion of the probe's own statements unnamed); Nits 1 to 6
resolved; Nit 7 partly (the fixture unnamed, Task 049-12 not in the merge order).

## Blocker

1. `ensure_journal_buckets()` (Task 049-9, merged first, called in step 1) and the `RUN_PURGED` mapping of Task 049-8 read
   `journal_retirements` and `journal_bucket_retiring`, which only the gated Task 049-14 creates. Fix: create them in Task
   049-9.

## Major

1. No task makes a creation command carry "the switch is active"; `ServingGate` exposes no active capabilities. Fix: a
   gate accessor and the flag in Task 049-5, false until Task 049-13.
2. The per-run `run_events` read function of `journal.rs` and the payload write helper belong to no task. Fix: name both
   and their tasks; a baseline per rule.
3. (Full-round Major 3, partly.) As above.
4. (Full-round Major 4, partly.) As above.

## Nit

1. Task 049-7 adds `run_payloads` to the append-only set before Task 049-10 creates it.
2. Step 1 calls `ensure_journal_buckets()` twice, the first before the parent exists.
3. Task 049-15 says T020-046 uses Task 049-11's indexes, though Task 049-11 comes after it, and omits the purged branch
   that T020-045 adds.

Counts: Blocker 1, Major 4, Nit 3

Fixed at `7da51b79`: the two objects created in Task 049-9 (Blocker 1); `ServingGate::is_active` and the creation flag in
Task 049-5 (Major 1); `read_run_events` in `journal.rs` (Task 049-8) and `write_run_content` in `payloads.rs` (Task 049-10),
with a baseline per rule emptied in the merge order (Majors 2 and 3); the probe's seam in `dispatch.rs` and `execution.rs`,
the round trips from `pg_stat_statements` plus `BEGIN` and `COMMIT`, the `/* budget-probe */` exclusion (Major 4); Task
049-7 after Task 049-10, one call in step 1, the T020-045 and T020-046 amendments, the fixture named and Task 049-12 in the
order (Nits 1 to 3 and full-round Nit 7).
