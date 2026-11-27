Doc: docs/designs/047-oauth-integration-hub.md
Reviewed-commit: f6fef1d6c9277d7f88b5b159177b0cbd7af5decd
Reviewer: architecture (repository agent `.claude/agents/architecture-reviewer.md`)
Agent-blob: architecture-reviewer
Counts: Blocker 2, Major 9, Nit 3

Condensed by the author from the reviewer's output; every finding and severity is kept.

1. **Blocker.** At the reviewed commit the constitution is 4.7.0, with "a connector marketplace" out of scope; Principle V allows exactly two declared sets, and an access token derived from a declared resource (and 021's integration runner) is in neither. Fix: do not approve until 5.0.0 has merged; write the exact Principle V clause needed and get it into an amendment before approval.
2. **Blocker.** Importing a provider or resource type creates an immutable usable version with no draft and no publication, against the admitted wording ("imported as drafts under the same publication rules") and Requirement 028-041. Fix: draft versions with an audited publication, or an ADR and amendments.
3. **Major.** 021 withdrew `oauth2_client_credentials` for an `oauth` scheme whose `oauth_credential` slot names a credential, and asks for a manage permission 047 does not define; 047 ties one credential to one resource. Fix: adopt 021's model; an independent credential; `oauth:credential:manage`.
4. **Major.** `oauth-broker` breaks the `-v1` naming; capabilities are switched on by the audited activation of Requirement 018-052; runners do not register; journey 1 treats it per namespace; refusal code unspecified. Fix: `oauth-broker-v1`, control plane and engine only, 021's `OAUTH_BROKER_UNAVAILABLE`.
5. **Major.** Step-failure codes conflict with 021's `CAPABILITY_UNAVAILABLE` and `RESOURCE_UNAVAILABLE`. Fix: one mapping table per executor path.
6. **Major.** "Only `infra-oauth` builds token requests" is already broken by 021's Pub/Sub and Service Bus token calls; custody is a module boundary in the process that serves `ResolveSecrets`. Fix: a defined check with a recorded exemption list; an `OAuthVault` port only `application::oauth` may use.
7. **Major.** Import verifies with codecs in the adapter `infra-schema`, which the application may not depend on without a port; 028 places index and import code in `crates/domain/src/templates.rs` and `crates/application/src/templates.rs`. Fix: a `HubItemCodec` port; one module and one owner.
8. **Major.** Nothing leases an activity that declares resources only to sessions at the new minor; rollback names `SCRIPT_DENIED` for activities. Fix: lease gating by minor with a conformance case; a typed rollback code.
9. **Major.** Neither ADR records refining ADR-0074 (Proposed, operator permissions only for scaling) or ADR-0082; a tenant-authored token endpoint is admitted only by ADR-0055 rule 11(e). Fix: add both, state the dependency on ADR-0074, define the operator permissions.
10. **Major.** No permission for a tenant-wide import; 028's `template_imports` is namespace-scoped with `target_kind` workflow or script. Fix: `hub:import` at tenant scope; widened checks in an additive migration.
11. **Major.** Requirement 047-001 checks hosts against the deployment policy only while ADR-0122 says the intersection; tenant-scoped providers have no namespace; secret paths for tenant and deployment registrations undefined.
12. **Nit.** Hub types limited to `http_api` and `graphql` while 021 admits more; auth schemes other than OAuth left out.
13. **Nit.** "Same limit class" as `ResolveSecrets` does not fit 30 calls per attempt; state the class in `sdks/SPEC.md`.
14. **Nit.** Confirm no other branch builds this or 028's hub code before the build starts.
