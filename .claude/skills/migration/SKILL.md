---
name: migration
description: Checklist for a PostgreSQL migration in crates/infra-postgres (additive, grants for the runtime role, down that refuses data loss, storage classification, privilege audit). Use when adding or changing a table, column, index, trigger or function.
when_to_use: "new migration; add a column; add a table; alter table; index"
---

1. File: `crates/infra-postgres/migrations/<YYYYMMDDHHMMSS>_<slug>.up.sql` and `.down.sql`. Take a timestamp later than every file in the directory and on other branches (`git ls-tree -r origin/develop --name-only | grep migrations | tail`).
2. Additive: an older engine or worker must keep working against the new schema (constitution IV). New columns are nullable or have a default; no rename or type change in place.
3. Grants: give `sagat_app` exactly the privileges the code needs, in this file. Remember a row lock (`FOR UPDATE`/`FOR SHARE`) needs `UPDATE` on the table, which the runtime role usually lacks: use a plain read or an advisory lock.
4. Append-only or immutable tables keep their triggers (`ENABLE ALWAYS`); a `SECURITY DEFINER` function pins `search_path`.
5. Every new variable-size column is classified in `crates/infra-postgres/storage-classification.json` (a test fails otherwise).
6. `down`: refuses to destroy data with a clear `RAISE EXCEPTION` and `ERRCODE = 'object_not_in_prerequisite_state'` when rows exist that older code cannot read; otherwise restores the previous schema. Tests that roll back across it must drain the rows as an operator would.
7. Test it against real PostgreSQL: migrate from the previous schema with data present, roll back, migrate again (`integration_platform_foundation` migrations tests), run `runtime_privileges`, and rerun the feature's suites with `SAGAT_TEST_AS_RUNTIME_ROLE=1`.
8. New store code uses `begin_safely()`. Document operational impact (lock time on large tables, index build) in the plan or the runbook.
