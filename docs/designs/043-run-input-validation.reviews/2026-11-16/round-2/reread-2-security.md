# Targeted re-read 2 of design 043: security (2026-11-16)

Doc: docs/designs/043-run-input-validation.md
Reviewed-commit: d890b9263cd8b84a7156edf66c3f08224b0d1617
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A second targeted re-read of
the changes since a763d9d0 (OD-2026-11-16-3), not a full review. The reviewer read files only and ran nothing.

Re-read 1 Nits 1 to 4: closed (Nit 1 as the record G043-057). Verified with no finding: the tenant boundary of the
schedule clause, the runtime role's grants, the order of authorization before each busy answer, the caught panic and what
it logs, the counting labels.

## Nit

1. The privilege text of the inventory's module doc and `stored_versions_page` is stale; the runbook's cost line leaves out
   the schedules.
2. The inventory tests connect as the owner, so no test would catch a missing grant.
3. A panicking schema stops the inventory with "run it again" and names no version.

Counts: Blocker 0, Major 0, Nit 3
