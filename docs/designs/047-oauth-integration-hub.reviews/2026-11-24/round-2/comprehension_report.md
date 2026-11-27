Doc: docs/designs/047-oauth-integration-hub.md
Reviewed-commit: e1d438b0ff4e17994feb29517a4d0cd2582b1349
Reviewer: egm-comprehension (egm-review stage 6, round 2)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Counts: Blocker 0, Major 6, Nit 7

**What this proposes**

OAuth 2.0 credentials for feature 021's resources (authorization code with PKCE, client credentials). A token broker in the control plane is the only component calling provider endpoints; it keeps refresh tokens and client secrets in its own OpenBao mount and refreshes each credential once at a time under a fenced lease row. Executors (integration runner, script sandbox, SDK activities) get a resource's non-secret settings and a current access token through one all-or-nothing, value-free-audited call, `ResolveDeclaredResources`. A curated hub under `hub/` holds integrations as signed data; importing creates drafts, publishing is separate, revocations can block an item.

**How the existing system works, per this document**

Feature 021 provides typed resources, secret slots and an isolated integration runner over a closed set of operation kinds, and refuses its `oauth` scheme until a broker exists; scripts and activities cannot use resources. Feature 028 designs a signed template index imported as drafts. Feature 017 provides the OpenBao client and recoverable writes. Executors receive declared secrets through `ResolveSecrets` under Principle V.

**Findings**

1. **Major.** The builder table and Technical Plan 14 disagree on phases; Task T047-1 has no step.
2. **Major.** Adoption needs a completed vault write but nothing marks it; the access revision on adoption.
3. **Major.** A client-credentials credential's lifecycle (creation, first token, `ACTIVE`) and recovery from a `BROKEN` registration are not described.
4. **Major.** `RESOURCE_ASK_LIMIT` is step-facing and missing from the codes this design adds.
5. **Major.** Which permission publishes an imported provider (`oauth:client:manage` or `hub:publish`).
6. **Major.** A dead control plane cannot write `oauth.authorization.incomplete`; which component detects the case.
7. **Nit.** Whether the initiator's permission is rechecked at delivery.
8. **Nit.** Undefined terms (generation, capability token, worker binding, pool-delivery policy, "Both paths").
9. **Nit.** Which column the keep-alive reads.
10. **Nit.** Journey 6 gives a security revocation a finding; the requirement does not.
11. **Nit.** Opaque references without a gloss.
12. **Nit.** Paths without a builder.
13. **Nit.** What a caller sees past the rate limits.
