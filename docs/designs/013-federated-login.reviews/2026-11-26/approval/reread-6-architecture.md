# Targeted architecture re-read of design 013 for approval, round 6 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 8f4174139c8f2fb390051bd38e4daf1cd1adec72
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: a confirmation of round 5 and a re-read of the changes since (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. Round-5 Major 1 and the five Nits are closed; Major 2 is closed in shape, with the gap below; the security-round changes add no layering or visibility defect.
Counts: Blocker 0, Major 1, Nit 1

1. **Major. Technical Plan 9, the production chart; Technical Plan 12, *Transport*.** Every piece the identity agent needs (image, CA, auth mount, service account, egress rule) renders only under `openbao.enabled`, which also turns on feature 015's provider on every component, so a Helm deployment cannot have federation without it, against the stated decoupling; and `SAGAT_IDENTITY_OPENBAO_CA_FILE` is not wired, so the identity client fails TLS. Fix: gate those pieces on `openbao.enabled` or federation, keep the provider's environment, intent key and roles on `openbao.enabled`, validate the identity pieces whenever federation is on, add the CA variable, and extend the render test with federation on and `openbao.enabled=false`.
2. **Nit. Technical Plan 9.** Reusing the agent helper gives a duplicate container name, the management config or a probe on the wrong sink, and the new sink volume needs a name apart from `openbao-identity`. Fix: its own container name, config volume, sink volume and startup probe; the render test asserts distinct container names.
