# Targeted architecture re-read of design 013 for approval, round 1 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: dabbc4bc
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: a re-read of the text changed since the reviewer's full review of 2026-11-25 (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept.
Counts: Blocker 0, Major 3, Nit 5

1. **Major. Technical Plan 4 "Redeem" step 6, Requirement 013-026, "Signing keys".** Signing happens inside the write transaction, where a stale signer reads OpenBao, holding the advisory lock, the settings and connection rows, the membership and perhaps the fence. Fix: a signing handle obtained before `BEGIN` (or a refusal before any lock), CPU-only signing inside; the same for refresh; a test.
2. **Major. Technical Plans 2 and 4.** Nothing in the placement table reads `identity_signing_keys`; a fetcher in `infra-postgres` or SQL in `infra-secrets` adds unaccounted edges. Fix: a `FederationStore` method, its adapter in `infra-postgres`, a small fetcher in `api-http` mapping to a `JwkSet`, the signer given the active version by the use case.
3. **Major. Requirement 013-032, Technical Plans 7 and 10, Task 013-10.** The in-memory registry view that `config.json` and `identity.federation` follow does not exist (the member keeps no requirement set), and no task builds it; a replica started after activation never offers the platform issuer. Fix: Task 013-10 builds it; two tests.
4. **Nit. Requirement 013-032, Architecture decision 0140 decision 9.** "Federation turns on nothing in the engine" is false (the maintenance loop). Fix: "needs no configuration".
5. **Nit. Architecture decision 0140 decision 6 and "Enforced by".** "Reset" is stale; two tests are missing. Fix: correct.
6. **Nit. Decision 013-017.** A stale "freshness" column. Fix: remove.
7. **Nit. Technical Plan 13 "Activation window".** The in-transaction check and the in-memory view are mixed. Fix: separate them; scope the runbook's wait to the Studio's offer.
8. **Nit. Architecture decision 0050's recorded addition.** It omits the registry epoch first, `Slot::IdentityAdvisory` for SCIM `POST` and pre-provisioning, and the demotion restart rule. Fix: add them.
