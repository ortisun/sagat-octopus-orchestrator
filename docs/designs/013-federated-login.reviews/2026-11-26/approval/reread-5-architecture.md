# Targeted architecture re-read of design 013 for approval, round 5 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 977125fce50a262003f3a4b87086b02b5259cd13
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: a confirmation of round 4 and a re-read of the changes since (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. All seven round-4 findings are closed.
Counts: Blocker 0, Major 2, Nit 5

1. **Major. Technical Plan 12, *Transport*; Settings.** `IdentityOpenBao::from_lookup` reads the shared `SAGAT_OPENBAO_ADDRESS`, which in today's code turns on the OpenBao secrets provider and makes the control plane require its token and intent key, so federation would force feature 015's managed secrets. Fix: the identity client's own address and CA settings in the Settings table, and a case with federation on and no `SAGAT_OPENBAO_*` set.
2. **Major. Technical Plan 9; Technical Plan 12, *Transport*; Task 013-8.** The control plane needs a second token, but the chart renders one agent with one role and one sink per component, and the design names neither the second agent, its sink and environment, the `openbao.roles` value and validation, the Kubernetes role nor the `sagat-identity/` mount; federation cannot be turned on in production. Fix: list them and add a chart-render test of two agents with distinct roles and sinks.
3. **Nit. Technical Plan 12, *The rule*.** The sealing supertrait and the seal-output type are both named `Sealed`, which a name-based allowlist cannot tell apart. Fix: rename one.
4. **Nit. Technical Plan 2, port row.** The signatures name `MacVerifier`/`MacMinter` while the prose says trait objects, and the key id and MAC are untyped; `infra-secrets` types would make the application depend on an adapter. Fix: type aliases of the trait objects in the application, with the application's own key-id and MAC types converted by the adapter.
5. **Nit. Technical Plan 9.** The development bootstrap's `identity/token` already holds the database runner's development token. Fix: another directory.
6. **Nit. Technical Plan 12, *Transport*.** Role names are configurable while both clients check a policy name. Fix: state that `sagat-identity` is fixed, in the runbook and the policies README.
7. **Nit. Technical Plan 12, *Transport* and `identity_mount`.** Only the ring transport has `measured` names, so identity-mount traffic would mix into the secrets mount's series. Fix: closed names for the four operations.
