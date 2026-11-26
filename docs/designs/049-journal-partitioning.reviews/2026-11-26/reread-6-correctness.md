# Approval confirmation of design 049: correctness (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 44d6a0b3cd3afe687d5144167c8d18ef73870e19
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The last of the targeted
re-reads of the approval run under Owner decision 2026-11-25-2 (one full round, `correctness-full.md` at 66634fb9; the
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

Settled (see the egm-review summary of this approval): Nits 1 to 5 are recorded as design 049's follow-up Task 049-19,
added after the approval as the approval rule allows; Nit 6 is fixed in design 053.
