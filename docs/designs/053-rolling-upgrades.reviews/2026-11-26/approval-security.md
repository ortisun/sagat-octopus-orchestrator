# Approval confirmation of design 053: security (2026-11-26)

Doc: docs/designs/053-rolling-upgrades.md
Reviewed-commit: 3cf5ee9737c4611e9126403c7f5dbfecc0b4f861
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The last of the targeted
re-reads of the approval run under Owner decision 2026-11-25-2: one full round (`security-full.md`, at 7771688e, 6
Majors and 5 Nits), then the fixes re-read in `reread-1-security.md` to `reread-5-security.md`, every finding of each
fixed before the next. Not a full review. The reviewer read files only and ran nothing.

Re-read 5's Nit is resolved: the catalog test checks `sagat_retirer` over every relation with
`has_any_column_privilege` (table-level grants included) and every sequence, allowing only `SELECT` on
`registry_first_confirmation`; requires the role attributes `rolsuper`, `rolbypassrls`, `rolcreaterole`, `rolcreatedb`
and `rolreplication` false; allows no `CREATE` on any schema and only `CONNECT` on the database; and keeps the checks of
no role membership and of `EXECUTE` exactly on the functions Requirement 053-005 names. A remark for the build, not a
finding: the `EXECUTE` check counts grants to `sagat_retirer` itself, not `PUBLIC`'s defaults on `pg_catalog`.

Every finding of every security report of this approval run is fixed; none is accepted.

No Blocker, no Major, no Nit.

Counts: Blocker 0, Major 0, Nit 0
