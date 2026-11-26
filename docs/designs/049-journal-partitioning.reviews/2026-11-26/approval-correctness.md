# Approval confirmation of design 049: correctness (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 3c82957ecde6e5dd24a4138fca874b3cf9e29df8
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The last of the targeted
re-reads of the approval run under Owner decision 2026-11-25-2: one full round (`correctness-full.md`, at 66634fb9, 2
Blockers and 9 Majors), then the fixes re-read in `reread-1-correctness.md` to `reread-6-correctness.md`, each Major
fixed before the next. Not a full review. The reviewer read files only and ran nothing.

Re-read 6's six Nits are resolved: the lock before the check and the `READ COMMITTED` refusal, tested; `is_active` tested
on a test-only capability and the registration test in Task 049-13; the merge order matching design 053; `TimeZone`,
`DateStyle` and `to_char` (the key is the same text in every session); the trigger functions owner-only (PostgreSQL does
not check `EXECUTE` when a trigger fires); the process's epoch in design 053. ADR-0132's "Enforced by" matches the tasks.

No Blocker, no Major.

## Nit

1. The Constitution Check's row II still says the heartbeat returns only the epoch, against Requirement 053-005 and Task
   049-5. Fix: "the heartbeat returns the set only when the epoch the process sends differs".
2. Requirement 049-016 has every function set `TimeZone`, `DateStyle` and `search_path`, but Requirement 049-013's audit
   compares only `proacl` and `prosecdef`. Fix: Task 049-7 also compares `proconfig`.
3. The `runs` trigger's late-dated-root branch does not say that `journal_bucket_retiring` is checked in a separate
   statement after the shared lock, or that the trigger function and `journal_retire_start` are `VOLATILE`; with one
   snapshot, a creation that waited on the start could commit a dated root after it, and the retirement would stall at
   step 3 until abandoned (only when both 24-hour checks fall in the same instant). Fix: state both.

Counts: Blocker 0, Major 0, Nit 3

Settled: the three are recorded as design 049's follow-up Task 049-19, added after the approval as the approval rule
allows (a ticked or follow-up task may change an approved design's body), so that they are fixed in the build, test
first; the egm-review summary of this approval lists them.
