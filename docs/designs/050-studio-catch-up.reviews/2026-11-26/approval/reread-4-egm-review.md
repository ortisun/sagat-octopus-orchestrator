# egm-review re-read of design 050 for approval, round 4 (2026-11-26)

Doc: docs/designs/050-studio-catch-up.md
Reviewed-commit: 34a282d5e36e93ae99d939852ddb1eb51617fd7d
Reviewer: egm-review (re-read for approval, egm-design)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: a design-critic re-read of the text changed since round 3 (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found its round-3 findings closed, the new cursor-sealing text reading as one plan with the refusals, metrics, Studio paragraph, tasks, risks and complexity rows, and the `migrate`, `MigrateError`, `getrandom`, `aes-gcm`, `sha2`, `handover.rs`, `changed_db_role`, `protected_system` and request-budget claims accurate. This round's four reports are the approval set: security `reread-18-security.md`, architecture `reread-6-architecture.md`, correctness `reread-4-correctness.md` and this one, all at this commit; their nits are settled in `docs/owner-decisions.md` (2026-11-26) with follow-up tasks.
Counts: Blocker 0, Major 0, Nit 3

1. **Nit. Technical Plan 6, "Cursor sealing"; Task 050-4.** The clock the `rotate` refusal reads is unstated and only the adapter's clock is movable, so the rotation tests would wait minutes; and under the refusal the concurrent-rotation test can only end in one insert and one refusal. Fix: the refusal reads the database's `now() - created_at`, tests backdate `created_at` as the schema owner; rename the test `two_concurrent_rotations_insert_one_key_and_refuse_the_other`.
2. **Nit. Task 050-4.** `runtime_privileges` checks the SQL of every non-test source, so the `INSERT`, `DELETE` and `LOCK TABLE` on `inbox_cursor_keys` need `ALLOWED` entries with reasons, as the trigger handover has. Fix: add them to the task and name `the_runtime_role_cannot_write_the_cursor_key` as the proof.
3. **Nit. Technical Plan 6, "Studio".** The `400` clause was inserted inside the parenthesis, leaving a clause attached to nothing. Fix: move the `400` sentence after the parenthesis.
