# Targeted re-read 3 of design 051: correctness (2026-11-26)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: a3e766c2458eacbf64abd6cd049e105e4625d563
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation read of
the fixes of `reread-2-correctness.md` (seven resolved; Blockers 1 to 3 resolved in the prose but replaced by the new
findings below) and of the security changes since; the arithmetic rechecked holds. The reviewer read the design, this
platform's code and migrations and the sqlx-postgres 0.9.1 source, and ran nothing. Fixed at the next commit, the
startup findings (Blockers 2 and 3, Major 1, Nits 2 and 4) by replacing the static search of option sources with a
startup guard that reads what each client sends.

## Blocker

1. This platform's `proconfig` list names only `search_path`, but
   `20270208000200_lease_withdrawal_floor.up.sql:46` sets `lock_timeout` on `platform_requirement_remove`, so every
   turn fails. Fix: admit the settings found, minus the durability names.
2. The `.options(` rule cannot read `pool.rs:112-124`, whose names sit in a local vector, so it refuses this
   platform's own pool and cannot generate its published options; a line match cannot tell receivers by type.
3. Scanning every workspace literal in three option forms refuses unrelated literals (a test server's `-c ssl=on`,
   CLI test arguments, `rerun-if-changed=`), so the rule refuses this platform's commit.
4. (Plausible) After the kill, `stats_reset` may precede `pg_postmaster_start_time()`, outside the admitted window.
   Fix: admit a reset between the injector's recorded fault time and the first scan after the restart.

## Major

1. SQLx turns a URL's `options[name]=value` keys into `-c` settings, which the URL check does not decode.
2. (Plausible) Only the `crash` subcommand reads the driver's directory and copies uploads, but every job kind needs
   the boundaries, the acknowledgement and the upload copy. Fix: a host subcommand in every job kind.
3. (Plausible) Refusing `synchronous_commit` in this platform's `set_config` list conflicts with feature 006's planned
   acknowledgement statement, which raises it to `local`.

## Nit

1. The Technical Plan's rule paragraph still says `options` and `-c` are refused outright and omits the new checks.
2. The startup allowlist omits `user` and `database`, which every startup message carries, and should compare names
   without case.
3. The window-start snapshot's timing is undefined; post planned boundaries before the rung or bound the lag.
4. The `.options(` scan also finds `crates/infra-database/src/executor.rs:116`, which connects to users' databases.

Counts: Blocker 4, Major 3, Nit 4
