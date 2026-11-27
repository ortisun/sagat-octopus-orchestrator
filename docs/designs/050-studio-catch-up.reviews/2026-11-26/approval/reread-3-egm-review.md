# egm-review re-read of design 050 for approval, round 3 (2026-11-26)

Doc: docs/designs/050-studio-catch-up.md
Reviewed-commit: 4d9b589e
Reviewer: egm-review (re-read for approval, egm-design)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: a design-critic re-read of the text changed since round 2 (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found its six round-2 findings closed and the `migrate`, `run_migrations`, test harness, `aes-gcm`, `hkdf` and `protected_system` claims accurate.
Counts: Blocker 0, Major 0, Nit 4

1. **Nit. Technical Plan 6, "Cursor sealing".** The nonce arithmetic cites a per-process limiter that an operator can change, so with N replicas the random-nonce limit comes after 1.36/N years. Fix: state the bound per replica and tie the rotation to it, or put a random salt in the wire form and derive a per-cursor key so the rotation period no longer matters.
2. **Nit. Technical Plan 6, "Cursor sealing".** "Recorded in the deployment's operator history" names a history that does not exist; each deployment-wide command records on its own row. Fix: add `created_by` (the bounded, printable `--operator` value) and `created_db_role` (`session_user`) to `inbox_cursor_keys`.
3. **Nit. Technical Plan 6; Task 050-4.** The first key's guard is undefined, `migrate`'s error type has no variant for a failed random source, `infra-postgres` has no random-source dependency, and the new `bytea` column needs a storage-classification entry. Fix: create the first key in the migration's own SQL, or keep the Rust insert as `INSERT … WHERE NOT EXISTS` with an error variant and a named random source; either way add the manifest entry to Task 050-4.
4. **Nit. Technical Plan 6, "Cursor sealing" and "Studio".** The port is not stated asynchronous; the `503`'s code, reason, retryability and `Retry-After` are unstated and not in the contract task; the Studio's `503` behaviour is unstated; the answer to an unknown key id with no re-read left is unstated. Fix: `async` `seal` and `open`; `UNAVAILABLE`, retryable, a named reason, in Task 050-2; the Studio keeps the last list, shows "temporarily unavailable" and backs off; an unknown id with no re-read left gets the same `400`, with a test.
