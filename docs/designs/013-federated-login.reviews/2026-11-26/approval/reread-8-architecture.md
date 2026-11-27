# Targeted architecture re-read of design 013 for approval, round 8 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: f03379e1833d7900a8d4d5b180bc1de95c2aa51b
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: a confirmation of round 7 and a re-read of the changes since (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. All three round-7 findings are closed.
Counts: Blocker 0, Major 1, Nit 1

1. **Major. Technical Plan 9, the production chart and its render test.** The control-plane container's own mounts are unstated and `SAGAT_IDENTITY_OPENBAO_TOKEN_FILE` has no value; the CA mount comes only from `sagat.openbao.mounts` under `openbao.enabled`, so a federation-only control plane cannot read its CA or token and crash-loops, or a second CA mount duplicates a `mountPath`; the test checks neither. Fix: the container mounts the sink read-only and the CA exactly once under either value, the token-file variable names the sink; the test asserts every `SAGAT_*_FILE` path lies under a mount and no two mounts share a path.
2. **Nit. Technical Plan 9.** "The ConfigMap holding exactly `control-plane-identity.hcl`" sits under "in both modes", which fails when `openbao.enabled` adds the three existing entries. Fix: state it for the federation-only mode and the four entries for the other.
