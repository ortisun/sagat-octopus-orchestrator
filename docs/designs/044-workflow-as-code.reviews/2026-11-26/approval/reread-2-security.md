# Targeted re-read of design 044 for approval: security, second pass (2026-11-26)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 70d71adfdc35a31e28890f897102c1e977603405
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Scope: the text changed
since ed293ea9. All nine first-pass findings are closed. Every finding below was fixed in the next commit.

## Major

1. Command-value blobs leave the tenant's blob ceiling with no ceiling of their own.
2. The chain receipt route's key and lookup omit the namespace.

## Nit

3. Expiries after a `semaphore_full` refusal, which other tenants cause, should never count.
4. The verifier test names a revoked session; the rule re-reads only the binding.
5. The disclosure of the gated entry names is not stated.
6. A reused blob may race its deletion.
7. Four principals can still fill a run's 64 open updates (documented, acceptable).

Counts: Blocker 0, Major 2, Nit 5
