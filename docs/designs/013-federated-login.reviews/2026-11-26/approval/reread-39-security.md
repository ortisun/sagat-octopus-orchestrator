# Targeted security re-read of design 013 for approval, round 39 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 284dfae6318d2817d7314e412f6fb2458a426534
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the fix of the sixth architecture re-read (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept.
Counts: Blocker 0, Major 1, Nit 1

1. **Major. Technical Plan 9; Task 013-8.** Under federation alone the egress rule to OpenBao is rendered, but `openbao.network.port` and `openbao.network.to` are checked only under `openbao.enabled`, and the default empty `to` allows every destination from the control plane on that port; the render test asserts only that the rule is present. Fix: the network checks whenever either value is true; a render case with federation on, `openbao.enabled=false` and an empty `to` that must fail.
2. **Nit. Technical Plan 9.** The chart's secret-transport check (native TLS or `allowPlaintextSecrets`) fires only on `secrets.existingSecret` or `openbao.enabled`, so a federation-only deployment carrying connection secrets and refresh tokens skips it. Fix: add `identity.federation.enabled` to its condition and cover it in the render test.
