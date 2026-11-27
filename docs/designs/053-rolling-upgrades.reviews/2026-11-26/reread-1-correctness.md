# Targeted re-read 1 of design 053: correctness (2026-11-26)

Doc: docs/designs/053-rolling-upgrades.md
Reviewed-commit: d90539779808af7cc0e27d19fcadaab28c7e6ea8
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A re-read of the fixes of
`correctness-full.md`, not a full review. The reviewer read files only and ran nothing; PostgreSQL behaviour behind the
items marked plausible is from its knowledge.

Full round: the Blocker and Majors 1, 2, 3, 5 to 11 resolved (some with new defects below); Major 4 partly (the hot list
kept by hand; deadlocks not retried); Nits 1 to 5 resolved.

## Major

1. Form (b) refuses run starts for 12 to 24 s after phase 1 on a process that has not yet learned of it; the
   mixed-writer test would not see refused starts. Fix: a dedicated error, the start retried once with a legacy root;
   a test; a condition "no start refused".
2. (Plausible) Form (b) leaks past phase 2 under REPEATABLE READ or SERIALIZABLE, where the trigger reads the
   transaction's snapshot. Fix: refuse or lock under those levels; a test.
3. (Plausible) The journal-buckets drain check proves a negative over all of `runs` under the table lock, past the 2 s
   timeout, and each `NOT_APPLIED` retry takes the lock again. Fix: an index for the check; check `NOT_DRAINING` and
   `NOT_APPLIED` before the table lock.
4. The `order` rule refuses a hotfix merged forward (Requirement 053-009). Fix: exempt a file byte-identical to one of
   a release tag's tree.
5. (Plausible) A deadlock (`40P01`) ends `migrate`; the widening's lock order is unstated. Fix: retry it; fix the order.
6. (Plausible) The 60 s session lock timeout of the migration lock still applies to concurrent builds; the deadline
   leaves the server's build running. Fix: reset the lock timeout; `statement_timeout` from the time left; a test.
7. (Plausible) The attach step is unbounded, and child index names are unspecified. Fix: lock timeout and retry;
   deterministic names reused after a crash.

## Nit

1. `lease_withdrawal_required()`'s `FOR SHARE` makes phase 1 wait while holding the epoch lock; use `FOR KEY SHARE`.
2. `uses()` stays true after a failed read; answer false then.
3. Phase 2's own `55P03` and `57014` have no outcome.
4. Matching `57014` by message depends on the server's language; classify by elapsed time.
5. The validity check by name accepts a same-named index elsewhere; check `indrelid` and the definition.
6. `instance_register` changes but is not in the replaced list or the ADR; design 049's `permanent` text.
7. Completion of the partitioned procedure must be judged per partition (the parent's flag is not revalidated on a
   detach).
8. The mixed-writer fixture migration has no path into the loop; the rollback cannot register when only some of N's
   capabilities were removed.

Counts: Blocker 0, Major 7, Nit 8

All fifteen were fixed at the next commit.
