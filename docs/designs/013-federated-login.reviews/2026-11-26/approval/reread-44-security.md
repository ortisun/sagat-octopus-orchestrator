# Targeted security re-read of design 013 for approval, round 44 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: e17389bc68ff95cd160067af6db55a36798874a1
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the fix of the eighth architecture re-read (Owner decision 2026-11-25-2, step 4). The reviewer found the control-plane container receiving only the identity sink, read-only, never the projected service-account token, a wrong mount failing closed, and the CA mount exposing only the public certificate. It noted, outside security, that the CA line's new condition should be limited to the control-plane component; the author did so.
Counts: Blocker 0, Major 0, Nit 0

No findings.
