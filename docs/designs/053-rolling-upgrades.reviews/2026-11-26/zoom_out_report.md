# egm-zoom-out on design 053, full round of 2026-11-26

Doc: docs/designs/053-rolling-upgrades.md
Reviewed-commit: 212e4262a9a4834529a22e88bba39d184e93b191
Note: the reviewer's output, condensed by the author; every finding and severity is kept. The reviewer read files only
and ran nothing.

## Major

1. The release-version layer (`schema_compatibility`, `releases.toml`, `develop`'s pre-release version, version order in
   SQL, `introduced_in`, `sagat_oldest_compatible()`, `BINARY_TOO_OLD`) repeats the registry: every new-state write is
   behind a capability (053-007), every contract names an active capability (053-001), and the registry already refuses
   a process lacking an active capability (`instances.rs`, `activate` and `register`); sqlx already refuses an older
   `migrate` (`VersionMissing`). Fix: drop the layer; a fencing capability is activated for every kind; one-run commands
   check the requirements with `missing_for`; keep the handshake for `SCHEMA_TOO_OLD` only.
2. If the layer stays, moving `develop`'s version is under-built: `release-version.sh` is a read-only check; the SDK and
   Studio versions must move too; setuptools writes `0.4.0.dev0`, which `package-sdks.sh` does not expect; text order in
   SQL puts `0.10.0` below `0.9.0`.
3. The lint's cut-off forces waivers on every 0.3.0 migration (all run in the stopped upgrade) and puts the lint on the
   critical path of designs 048, 049 and 052. Fix: cut-off at 0.3.0's last migration, only the security rules before
   it, no `stopped-upgrade` waiver.
4. The contract machinery is out of scope for 0.3.0 (the first contract can ship in 0.4.0), and the phase-2 condition
   "no contract naming the capability is applied" cannot be built (`_sqlx_migrations` keeps no headers). Fix: the lint
   refuses `-- phase: contract` in 0.3.0; defer the contract rules to the first contract.
5. The `permanent` column, its rewritten check and the guard's refusals repeat `remove`'s feature-owned verify step;
   design 049's drain check can never pass after first use. Fix: "permanent" is a drain check that cannot pass, run by
   `remove` and phase 2; drop the column.
6. The registry reads the same thing three ways: the gate already re-reads the set when the epoch moves
   (`refresh_kept`); the handshake on every beat returns every applied migration every 10 s. Fix: keep the heartbeat;
   the handshake at start, at each confirmation, and on beats only while `SCHEMA_TOO_OLD` is set.
7. The functions a process calls before its migrations (registration, heartbeat, confirmation, handshake) can never be
   renamed, against Requirement 053-002's "a changed function gets a new name", and `_v2` versions break the cold start.
   Fix: stable names and signatures across releases, or N-1's names until N's migrations apply; a test starting N on
   N-1's schema.
8. The lint rebuilds a SQL parser (dollar quotes, `DO` blocks) and duplicates the `NOT VALID` rule already tested over
   `MIGRATOR` (`a_constraint_added_not_valid_is_validated_by_a_later_migration`). Fix: a pinned Squawk or a libpg_query
   binding for the generic rules; custom code for the headers, the hot-table lock and the definer rules.

## Nit

1. The earlier `DROP INDEX CONCURRENTLY IF EXISTS` migration never runs again after a failed build; the loop's
   `indisvalid` drop-and-retry is what recovers. Delete the rule.
2. The requirements carry code-level numbers and test names; move them into the tasks.
3. `migration-checksums.txt` repeats the merge-base check and sqlx's checksums; derive earlier releases' migrations from
   the release tags.
4. The mixed-writer test does not reuse `scripts/test-release-upgrade-profile.py`'s released-tree build, and names no
   commit pair before 0.3.0.
5. The Constitution Check cites 5.1.0, not merged (`docs/constitution.md` is 5.0.0; OD-2026-11-25-3 "Decided; not
   built").

Counts: Blocker 0, Major 8, Nit 5
