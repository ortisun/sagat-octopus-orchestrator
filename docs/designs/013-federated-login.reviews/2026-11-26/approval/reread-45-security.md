# Targeted security re-read of design 013 for approval, round 45 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: a8820014ea23e0d9a2b5305437956597d29e197a
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the last change, the CA mount limited to the control-plane component (Owner decision 2026-11-25-2, step 4). The reviewer found only the public CA certificate moving, the token and intent-key mounts staying on `openbao.enabled`, the engine and secret cleanup unchanged, and no duplicate mount. Every security finding of the approval rounds was fixed, none accepted.
Counts: Blocker 0, Major 0, Nit 0

No findings.
