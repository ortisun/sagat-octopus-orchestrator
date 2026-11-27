# Correctness review: digest key ring, capability listing, admission rule and key-listing fixes (`f330ff66..295b158b`)

Reviewer: `correctness-reviewer` (read-only), 2026-11-20, given the diff and the spec only. Sources: the diff, the three
keys reviews, FR-022-018..022/053/057/059, SC-022-003/004/027, `data-model.md`, D022-055..060, T022-026..029, and the
code the diff relies on. It ran nothing. (Report as returned, lightly condensed; its file list is omitted.)

## Summary

Blocker: none. Major: 1 (a sealed cursor can be longer than `open_cursor` accepts). minor: 9. nit: 7.

Checked and sound: the trigger under multi-row statements (every order of delete and insert row events of one key in one
statement: each intermediate row satisfies the CHECK; the last leaves `waiters = rows`, `oldest = min(rows)`; the
`COALESCE` fallback runs only when no row survives, so a later trigger brings it to 0 and NULL); the snapshot under READ
COMMITTED after the `PERFORM ... FOR UPDATE`; the stronger lock adds no wait (only foreign-key `FOR KEY SHARE` checks,
which run after `file_keys` holds the row); backfill and down; the keyset SQL (total order, same expressions, sentinels
only on the first component, the group guard); 57014 maps to 503; the reveal check matches feature 009's rule without
its locks; the refusal order; the store wiring of both binaries; and the new tests fail for the reasons they state
(`against_at` cannot pass vacuously; round 3 holds the run before the key; the 20-round T14 loop; the seeded checks; the
renamed column; the one-statement trigger test).

## Major

### M1. A sealed cursor can exceed `MAX_SEALED_BYTES`: paging stops with 400 at a key with a long derived group

`keyed_digest.rs:341` (`MAX_SEALED_BYTES = 2 + 12 + 16 + 256`), checked at `:313`; `seal_cursor` has no bound. The
position text is `k1.` + instant + pool + scope + 64 hex + the group in hex: 99 bytes for a waited ORDINARY/STEP key with
a 16-digit instant, plus twice the group's length, and a derived step group can be 130 characters (260 hex), so up to
about 361 bytes. Scenario: a 40-character slug and a 40-character node (an 82-character group); a page ends on that key;
the 263-byte position seals to 586 hex characters, above 572: `400`, and no later page can be reached. Fix: derive the
bound (3 + 20 + 1 + 10 + 1 + 4 + 1 + 64 + 1 + 260 = 365, rounded to 384), refuse to seal above it, and test a 130-character
group with `DRAFT_TEST`, `STEP` and the largest instant.

## minor

- **m1.** The plan test asserts `Actual Rows == 51` on the scan, which counts rows returned, not read: a row comparison
  turned into a Filter would still pass. Assert no `Rows Removed by Filter` and that `Index Cond` holds the comparison;
  seed waited keys and plan a cursor on a waited key.
- **m2.** The startup key-version check (`check_flow_control_keys`, `missing_key_versions`) and `process_env` have no
  caller in any test: add a test (active 2, ring 1: refused; then accepted; then a retiring version missing: refused),
  and use `process_env` with a real engine or remove it.
- **m3.** The role half of the reveal rule is untested: demote the granted developer to viewer and assert `null`.
- **m4.** Cursors are sealed under the highest version in the file, not the active one: during a rolling addition of a
  new version, a cursor from an updated replica is a 400 on another. Seal under the lowest held version or the active one.
- **m5.** A key file with every switch off can still stop a process (the check runs whenever a ring exists): run it only
  when `served` is not empty.
- **m6.** `tasks.md` and `data-model.md` were not updated in this delivery (T022-026's cursor and labels, T022-028's four
  orders and 20 T14 rounds, T022-029's admission test and counter rows; the new column, CHECK, indexes and trigger lock).
- **m7.** Keys-review correctness m5 (the takeover test races a raw UPDATE) and m8 (five same-key starts, `limit: 1`) are
  neither fixed nor recorded.
- **m8.** Invariant 12 is checked for `concurrency_waiters` only, and a RUN-scope waiter is only checked "not terminal",
  not `WAITING`.
- **m9.** `key_consistency` still probes `oldest_wait_at` and `runs.flow_control_gates` by name and drops the checks when
  absent: decide from `_sqlx_migrations` which columns the schema must have and fail when one is missing.

## nit

- n1. Nothing guards the trigger's `PERFORM ... FOR UPDATE` (defensive: every path files keys first).
- n2. Round 3 of `queue.rs` uses the unbounded `lock_waiters` loop; wrap it in `bounded::within`.
- n3. Round 3's +2 s deadline can be too short on a loaded runner (it fails, never passes falsely); consider +5 s.
- n4. `queued_runs_count_errors` is evaded by an alias (`w.run_id`) or a `concat!` split.
- n5. Paths other than `get` and `list` that render a run show `keyLabel: null` even to a granted caller (fails closed).
- n6. A storage error of the reveal check fails the whole run read; hiding the label on error would keep it available.
- n7. `KeyPosition::encode`'s capacity and the bound's comment understate the size (see M1).
