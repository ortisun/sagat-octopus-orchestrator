# Targeted correctness re-read of design 050 for approval, round 4 (2026-11-26)

Doc: docs/designs/050-studio-catch-up.md
Reviewed-commit: 34a282d5e36e93ae99d939852ddb1eb51617fd7d
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: a re-read of the changes since round 3 (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. Nothing in the changed text reopens an earlier finding.
Counts: Blocker 0, Major 0, Nit 3

1. **Nit. Technical Plan 6, "Cursor sealing"; Task 050-4.** Where `rotate` checks "the newest key is younger than two minutes" is unstated; checked before the lock, two concurrent rotations both pass, the second drops the key replicas still seal with and leaves a zero-second key sealing, while `two_concurrent_rotations_leave_the_two_newest_keys` still passes. Fix: the age check after the lock, in READ COMMITTED; the test asserts one success and one refusal, keys {K1, K2}.
2. **Nit. Technical Plan 6; the rotation tests.** The tests can move only the replica's clock while the refusal uses the database's `now() - created_at`, so tests needing several rotations must wait real minutes, and `created_at` counts time spent waiting on the lock. Fix: the tests backdate `created_at` as the schema-owning role, no sleeps; `created_at = clock_timestamp()` after the lock.
3. **Nit. Task 050-9.** The verification filter `iteration` misses four of the task's seven red tests. Fix: name the module file, or drop the filter.
