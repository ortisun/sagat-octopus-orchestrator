# Security review of design 053, full round of 2026-11-26

Doc: docs/designs/053-rolling-upgrades.md
Reviewed-commit: 7771688e6ab65562395c6c2faa8c548e66a50bdd
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output, condensed by the author; every finding and severity is kept. The reviewer read files only
and ran nothing.

## Major

1. A `draining` row still reads as active in the database fences and operator paths:
   `schedule_trigger_handover_complete` and `lease_withdrawal_required()` check only that the row exists;
   `platform_requirement_activate` answers `ALREADY_RECORDED`; `remove`, `requirements()` and `requirement_recorded()`
   ignore `state`; whether registration and `missing_for` bind a draining row is unsaid; undrain runs no census.
   Scenario: drain `schedules-v1` before the handover, then hand over: no cron trigger fires and phase 2 is refused for
   ever. Fix: `state = 'active'` in those functions; a refusing `DRAINING` outcome for activation; a draining row still
   binds registration and `missing_for`; undrain runs activation's census under the epoch lock; every reader listed
   with a test.
2. `applied_epoch` is self-reported and unbounded (one beat with 2^62 passes every later drain), and which client epoch
   is sent is unsaid (`epoch()` against `kept_epoch()`; a failed `refresh_kept` is swallowed). Fix: store
   `LEAST(sent, current)`, refuse negative or null, null is not applied; send `min(epoch(), kept_epoch())`; two tests.
3. The lint's security rules can be self-exempted (any waiver line, any rule name; the toml boundaries movable in the
   same pull request), and a statement parse misses `EXECUTE format(...)`, `ALTER FUNCTION ... SECURITY DEFINER`,
   `RESET search_path`, later grants to `PUBLIC`, default privileges, and a `search_path` without a trailing `pg_temp`;
   no catalog check covers design 053's functions. Fix: security rules not waivable; waivers in the leading block from
   a closed list with a reason; the merge-base check refuses moving the boundaries or shrinking the hot list; owners for
   the migrations and the toml; fail closed on a parse error; one catalog test over every definer function in `public`
   (pinned `search_path=pg_catalog, pg_temp`, effective grants, no `PUBLIC`, no grant option, an allowlist).
4. `sagat_retirer` has no table privilege and no grant of `command_await_durability`, yet every command must check the
   schema, `missing_for` and the durability wait; design 049 grants `sagat_schema_handshake()`. Fix: state the
   retirement commands' profile and `sagat_retirer`'s exact execute set; align the name.
5. The schema check does not fail closed: `42883`, `42501` (a grant skipped by `IF EXISTS role`) or a timeout must not
   read as empty. Fix: any error closes the gate with `SCHEMA_TOO_OLD`; bound the array argument.
6. Any live row can hold phase 2 for ever (a compromised process registers rows and never advances; a benign process
   whose requirements read keeps failing), and the refusal names no instance. Fix: phase 2 returns the count and up to
   16 instance ids; an owner-only `instance_evict(uuid, actor)` with history and a runbook step.

## Nit

1. A `SCHEMA_TOO_OLD` 503 is design 048's constant answer before authentication; design 048 says two reasons.
2. `pglast` supply chain: `pip install --require-hashes --no-deps --only-binary=:all:`, `pip-audit`, a `libpg_query`
   of PostgreSQL 16, `contents: read`; a fixture of a definer created inside a `DO` block.
3. The guard admits only `state` and `draining_since`, only between `active` and `draining`; `DRAINED` and `UNDRAINED`.
4. The partitioned-index procedure quotes identifiers from `pg_inherits` and confirms the parent by
   `pg_partition_tree` and the hot list; never splices the migration's text.
5. The forward-merge check takes only tags matching `^v\d+\.\d+\.\d+$`, protected, quoted in shell.

Tenant boundary: no finding.

Counts: Blocker 0, Major 6, Nit 5
