# Targeted re-read 1 of design 053: egm-design (critic) (2026-11-26)

Doc: docs/designs/053-rolling-upgrades.md
Reviewed-commit: 34c5479e38c86871c61ead3c4eaafbcb111d170b
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A re-read of the fixes of
`critic_report.md` (at 212e4262, the first draft), not a full review.

Full round: the Blocker and Majors 1, 2, 4, 5, 7, 8, 9 and 10 resolved; Majors 3 and 6 moot (contracts deferred, the
version gate removed); Nits 1 to 7, 10 and 12 resolved, 8, 9, 11 and 13 moot.

## Major

1. The partitioned-index file (a non-concurrent `CREATE INDEX` on a hot parent, without a transaction) is refused by
   `no-transaction` and `hot-lock`, and the loop's start check would read the parent's flag, which can stay false for
   good, so every later `migrate` would exit. Fix: exempt such files; judge them per partition at start.
2. Phase 2 takes the row `FOR UPDATE` then `runs IN SHARE MODE`, the reverse of a REPEATABLE READ form (b) writer (the
   table, then the row through the fence function): a deadlock that can fail a run's start. Fix: the table first;
   retry `40P01`; a barrier test with such a writer.

## Nit

1. Under REPEATABLE READ the fence fails with `40001`, not its own name, so the retry with the capability inactive never
   fires.
2. Classifying `57014` by the attempt's whole duration misreads an operator's cancel of a long earlier statement.
3. ADR-0131 decision 3 says `migrate` confirms, though `migrate()` does not; decision 7 requires a rollback the test
   may record as not applicable.

Counts: Blocker 0, Major 2, Nit 3

All five were fixed at the next commit.
