# Targeted re-read of design 044 for approval: security, fifth pass (2026-11-26)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: c01bd4322725d29a2fbc296fae2bd42e53ab4538
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Scope: the text changed
since 334676bc. All five previous findings are closed. Every finding below was fixed in the next commit.

## Major

1. The run's own limit of 32 answers with the tenant limit's reason, so a run can keep its own failed attempts from
   counting: its own reason, counted through `uncounted_expiries`; the lease's 16 checked first.

## Nit

2. The refusal's reason is not stored, so the expiry pass cannot tell the reasons apart.
3. A deferral that finds its blob taken by the sweep has no stated outcome.
4. `verifying_token_hash` needs a size check, a classification and the never-returned test.
5. No test for the run's 32 or a lease's 17th upload at the run's limit.

Counts: Blocker 0, Major 1, Nit 4
