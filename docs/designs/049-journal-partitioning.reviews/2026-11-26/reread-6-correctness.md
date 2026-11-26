# Targeted re-read 6 of design 049: correctness (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 44d6a0b3cd3afe687d5144167c8d18ef73870e19
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A targeted re-read of the approval run under Owner decision 2026-11-25-2 (one full round, `correctness-full.md` at 66634fb9; the
fixes re-read in `reread-1-correctness.md` to `reread-5-correctness.md`, each Major fixed before the next), not a full
review. The reviewer read files only and ran nothing.

Re-read 5's seven Nits are resolved. Checked with no Blocker or Major: the abandon's own authorization (owner-only, under
the row lock, with the bytes cleared on reactivation), the `EXECUTE` list, the gate's requirement set (the epoch is
compared on the client, so a lost response is retried), the permanent capability, the merge order.

No Blocker, no Major.

## Nit

1. `journal_retire_start` checks the bucket's runs before it takes the exclusive lock, so a creation it waits for can
   commit a live dated root it never saw; step 2's re-check and step 3 catch it and the retirement stalls until
   abandoned. Reachable only with a process clock a day slow. Fix: lock first, check in a later statement.
2. Task 049-5's test `a_process_registered_after_the_switch_creates_dated_roots` cannot fail in Task 049-5, where the
   capability does not exist yet. Fix: test `is_active` there on a test-only capability; run the named test in Task
   049-13.
3. The merge order lacks Task 053-3 before Task 049-5 and Task 053-2 before Task 049-9's migration.
4. The lock key casts a date to text under the session's `DateStyle`, and the trigger's month checks depend on its
   `TimeZone`. Fix: pin both on the functions, or key on `to_char(bucket, 'YYYY-MM-DD')`.
5. The definer trigger functions have no expected `proacl` in the `EXECUTE` list.
6. Design 053: "the epoch has moved" should be decided by the process, which sends its gate's epoch, as today.

Counts: Blocker 0, Major 0, Nit 6

All six were fixed at `2245250b`: the lock before the check (Nit 1); `is_active` tested on a test-only capability in Task
049-5 and the named test in Task 049-13 (Nit 2); the merge order (Nit 3); `TimeZone`, `DateStyle` and `to_char` (Nit 4);
the trigger functions owner-only (Nit 5); the process's epoch in design 053 (Nit 6).
