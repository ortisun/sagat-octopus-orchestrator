# Approval confirmation of design 049: security (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: c621f25b4f50038411fd38bc746a4d92867dc9d5
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The last of the targeted
re-reads of the approval run under Owner decision 2026-11-25-2: one full round (`security-full.md`, at 66634fb9, 6
Majors and 11 Nits), then the fixes re-read in `reread-1-security.md` to `reread-12-security.md`, every finding of each
fixed before the next. Not a full review. The reviewer read files only and ran nothing.

Re-read 12's Nit is resolved: the audit compares `is_grantable`, false for every grantee but the owner, with a negative
fixture for a grant `WITH GRANT OPTION`; neither `sagat_app` nor `sagat_retirer` can pass `EXECUTE` on. The edit only
makes the audit stricter and leaves the effective-grants read, the `PUBLIC` comparison, the owner check and the
null-`proacl` fixture as they were.

Every finding of every security report of this approval run is fixed; none is accepted.

No Blocker, no Major, no Nit.

Counts: Blocker 0, Major 0, Nit 0
