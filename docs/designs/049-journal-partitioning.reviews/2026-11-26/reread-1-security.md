# Targeted re-read 1 of design 049: security (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: df04d3a34d993a5de8f823ffd6cde0d445c43937
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A targeted re-read of the
fixes of the full round (`security-full.md`, at 66634fb9), not a full review. The reviewer read files only and ran
nothing.

Full round: Majors 1, 2, 3, 5 resolved; Major 6 resolved in the text (its enforcement is Nit 6 below); Major 4 partly
(Major 2 below); Nits 1 to 7, 10 and 11 resolved; Nit 8 partly (Major 1 below); Nit 9 partly (the no-values rule covers
only the triggers of Requirement 049-002).

## Major

1. A direct call of `journal_retire_start` on a future or current month with no runs inserts a row, after which
   `ensure_journal_buckets()` skips that month for good: every new root of every tenant then fails in that month. Fix:
   the function itself refuses the legacy bucket and any month that ended less than 24 h ago; `ensure_journal_buckets`
   skips only months whose retirement dropped or is done.
2. (Full-round Major 4, partly.) Erasure of a tree must check the permission and the eligibility of every run of the
   tree before purging any, and say what the report holds when one run is refused (feature 020's requirement 020-037).

## Nit

1. `EXECUTE` grants per role are not listed, and the audit checks `proacl` only against `PUBLIC`.
2. `journal_retire_drop(bucket, table)` takes any table; limit it to the two journal tables and confirm the partition
   through `pg_partition_tree`.
3. The pre-drop sessions report needs `pg_read_all_stats` for other roles' sessions; use an owner-owned definer function
   returning only the four fields.
4. The authorization of a `run_payloads` delete is not defined for root-keyed rows, and the abandon sweep becomes a second
   writer of authorization rows. Fix: allow it only for the authorized run's tree once every run of it is purged; audit
   the sweep per tenant.
5. Tenant audit records would carry the operator's identity; name a platform actor there and keep the operator in
   `journal_retirements`.
6. Nothing enforces the purged check on per-run reads of `run_events`; route them through one checked function and
   refuse other paths in the static check.
7. (Full-round Nit 9, partly.) The no-values rule for messages must cover every trigger of the design.

Counts: Blocker 0, Major 2, Nit 8

Every finding was fixed at `78155363`: the two refusals in `journal_retire_start` and the skip rule (Major 1); erasure
checks every run of the tree first, erases nothing if one is refused, and reports the blocking run and the surviving
content (Major 2); per-role `EXECUTE` audited against `proacl` (Nit 1); the table argument limited and confirmed (Nit 2);
the sessions function (Nit 3); the tree rule and the per-tenant audit of the sweep (Nit 4); the platform actor (Nit 5);
`journal.rs` and its static rule (Nit 6); the no-values rule for every trigger and function (Nit 7).
