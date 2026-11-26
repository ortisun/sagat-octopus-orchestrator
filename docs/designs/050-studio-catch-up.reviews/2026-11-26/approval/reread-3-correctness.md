# Targeted correctness re-read of design 050 for approval, round 3 (2026-11-26)

Doc: docs/designs/050-studio-catch-up.md
Reviewed-commit: 4d9b589e
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: a confirmation of round 2 and a re-read of the changes since (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found its four round-2 findings closed, no new transaction or lock, and the nonce arithmetic right (2^32 cursors at 100 a second is about 1.36 years).
Counts: Blocker 0, Major 0, Nit 3

1. **Nit. Technical Plan 6, "Cursor sealing"; Task 050-4.** The first-key guard is a "singleton constraint on the first key", but `rotate` deletes keys, so after two rotations `migrate` inserts an unplanned key; `rotate`'s insert and delete are not serialised, and "newest" is not defined. Fix: `migrate` inserts only into an empty table; `migrate` and `rotate` each run in one transaction that takes `LOCK TABLE inbox_cursor_keys IN SHARE ROW EXCLUSIVE MODE`; key ids come from an identity column, are never reused, and the newest is the highest; a test runs `migrate` after two rotations and expects no new key.
2. **Nit. Technical Plan 6; Task 050-4 tests.** Both rotation tests depend on reload timing the test cannot control, and the answer when the on-demand read is suppressed or fails is unstated. Fix: an injected clock or a test-only `reload_now`; the start-up and timer reads do not use the on-demand budget; a suppressed read means `400`, a failed read `503`; add `an_unknown_key_id_with_a_failed_read_is_unavailable`.
3. **Nit. Technical Plan 6, "Studio"; Task 050-10.** The Studio's handling of the inbox `503` and that answer's code and reason are not specified, and an error never backs off. Fix: name the code (`UNAVAILABLE`, retryable); the Studio keeps the last list with "could not refresh" and doubles the interval on errors; a test `an_unavailable_inbox_keeps_the_last_list_and_backs_off`.
