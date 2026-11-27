# Targeted re-read of design 044 for approval: security, third pass (2026-11-26)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 08580114df8610e8056248a054372caba90cd3c0
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Scope: the text changed
since 70d71adf. Previous findings 1, 2, 3, 5 and 6 are closed, 4 partly closed, 7 accepted as documented before.
Every finding below was fixed in the next commit.

## Major

1. A refusal at the tenant's command-value ceiling is counted, so one worker can push other namespaces' runs under
   review for 24 hours: answer it `UNAVAILABLE`, not counted; cap one run's uncommitted blobs across attempts.

## Nit

2. The verifier's session is not recorded, so a revoke-and-register-again passes the re-read.
3. The step-row lock clause attaches to the wrong sentence; name the advisory lock; the conditional commit.
4. The "16 MiB stays free" refusal has no reason label.

Counts: Blocker 0, Major 1, Nit 3
