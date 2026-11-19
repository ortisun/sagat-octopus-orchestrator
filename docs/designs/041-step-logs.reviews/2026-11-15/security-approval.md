# Security review of design 041, the approval run

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 882e56fcd513fa4d0de4205b95532113522f223a
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a fresh `security-reviewer`, given the committed design, its companions, the ADRs, the constitution and its own `security-final.md`; no shell. Recorded condensed.

Findings of `security-final.md`: A, B, C answered; D open (G041-015); E withdrawn (the author's contest holds).

## Blocker

None.

## Major

None.

## Nit

1. `tenant_step_log_bytes` usage is readable by viewers (`quota:read`, `identity.rs:682`), against ADR-0085 decision 6; omit `used` without `run:logs:read`, or have the owner accept G041-015.
2. The design's "no `SECURITY DEFINER` function" contradicts data-model §4 and the Amendment.
3. A script log returned by `getScriptExecution` under FR-041-022 is not audited as `run.logs_read`, and `may_read` needs a transaction holding the grant-set lock.

Counts: Blocker 0, Major 0, Nit 3
