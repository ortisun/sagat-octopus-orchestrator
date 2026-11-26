# Targeted re-read of design 054 for approval: security (2026-11-26)

Doc: docs/designs/054-buffered-signals.md
Reviewed-commit: b83781f7c0dfea6d880837aa2c439d4a384a0e5a
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and its severity is kept. The third targeted re-read
(Owner decision 2026-11-25-2, step 4), not a full review. The reviewer read files only and ran nothing.

The history: the full review `../security.md` (at `9ba3ed85`: Blocker 0, Major 7, Nit 2), all fixed; re-read 1
`../reread-1/security.md` (at `ca46330f`: Nit 3), fixed; re-read 2 `../reread-1/security-2.md` (at `9f803d8c`: Nit 1), fixed;
this re-read, at `b83781f7`. No security finding was accepted instead of fixed.

The Nit of re-read 2 is resolved in all four parts: the tenant aggregate's default is one value (the smaller of 1 GiB and a tenth
of `tenant_content_bytes`), Task 054-4 builds both resources and the trigger charges both, Task 054-8 tests two namespaces over the
tenant's ceiling, and the metric label and the refusal log name the scope as a closed value. Checked with no finding: a tenant-scope
refusal reveals only the caller's own tenant's total; the label stays closed; the tasks add no operation, grant or dynamic SQL.

Remark of the reviewer, not counted: the "smaller of" default does not say how an unlimited `tenant_content_bytes` is read; a
reading as zero would fail closed. Task 054-4's tests pin it.

Counts: Blocker 0, Major 0, Nit 0
