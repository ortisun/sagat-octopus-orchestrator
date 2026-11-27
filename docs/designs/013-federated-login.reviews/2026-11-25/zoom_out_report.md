# egm-review, Stage 5 (Initial Zoom Out) of design 013 (2026-11-25)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: d7cca86d
Reviewer: egm-review (Stage 5, egm-zoom-out)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: the reviewer's output, condensed by the author to one paragraph per finding; every finding, severity, location and fix is kept. The reviewer found that context sufficiency and the root-problem focus pass, and that the broker and platform-token decision, revocation in the membership read and the single process are the right structural answers; it confirmed the reuse of the egress `Guard` and `NameResolver`, `classify`, the instance registry, the scheduler's leased loops, the Studio's PKCE client, `sagat_lock_memberships` and the RustCrypto crates already locked.

Counts: Blocker 0, Major 8, Nit 6

1. **Major. Overview, the plan, Decision 013-017, Technical Plan 3, Task 013-5, Success criterion 013-001.** About six separable capabilities (OIDC brokering, the SAML verifier, SCIM users and groups, mapping, domains, federated-only with break-glass) ship with no step between, and Tasks 013-5 and 013-11 and the journey depend on feature 029's `access_groups`, which does not exist, without a gate. Fix: milestones with their own acceptance and gates: A (OIDC, broker, tokens, sessions, just in time, member routes, CLI, slug sign-in, context, SCIM users), B (SAML), C gated on 029 (SCIM groups, external groups, mapping), D (federated-only and break-glass); move the `access_groups` columns and their test to C; split the success criterion.

2. **Major. Requirement 013-013, Technical Plans 4, 6 and 13, Decision 013-007.** Groups from the sign-in assertion are the largest source of complexity (a mapped role with its `sid`, an effective role computed from live sessions in a function shared by twelve caller families, a union across sessions, a 24-hour bound, an expiry audit, an amendment of FR-029-024, an argued fence deviation), for the result SCIM already delivers durably. Fix: in 0.3.0 take external groups and mapping rules from SCIM only; a connection without SCIM gets its default role and administrator-assigned roles; a gap naming the providers that lose group sync; assertion groups, if essential, in a follow-up design.

3. **Major. Requirements 013-006, 013-012, 013-022, Technical Plans 3, 6 and 8, Decision 013-011, Gap 013-007, Requirement 013-031.** DNS-proven domains are speculative: slug sign-in works, the just-in-time domain limit adds nothing for a single-tenant provider, and shared issuers already need `hd` or `tid`; domains bring a resolver, an engine re-check job needing DNS, dispute rules, reasons, a ceiling, a screen, a battery entry and more enumeration. Fix: defer domains and discovery by e-mail; sign-in by slug; `login_hint` passed to the provider; remove `verified_domains`, `DomainResolver`, the re-check and the resolver task.

4. **Major. Overview, Requirement 013-033, Technical Plan 9, Task 013-3, Goals.** A development SAML provider next to a pinned Keycloak (which speaks both protocols) and the refusal corpus proves nothing more, needs a second XML signature implementation or reuses the verifier's canonicalization (agreeing by construction), and its break switch duplicates the corpus. Fix: drop it; run SAML against the pinned Keycloak; keep the development OIDC provider; most OIDC refusals as domain unit tests over the pure claim rules.

5. **Major. Requirement 013-005, Decisions 013-004 and 013-019, Technical Plan 2, Task 013-17.** Feature 027 is not built, yet the design registers an `apps` client, a token-endpoint refusal for it and an in-process redemption port with no caller. Fix: register only `studio` and `cli`; state the contract 027's delivery must meet (sign-in through the platform issuer, redeemed in process, no platform token to the apps origin); move `redeem_for_app`, the `apps` redirect and their tests to 027's tasks.

6. **Major. Requirement 013-025, Decision 013-015, Technical Plans 3 and 4, the Verification Plan.** Two signing-key backends: Transit "where configured" (a new integration nothing uses today, not in the development overlay, untested, with its own versioning overlapping the design's lifecycle, and an OpenBao round trip per renewal) and the identity-mount key. Fix: one backend for 0.3.0, either the identity-mount key read by the control plane only with the design's lifecycle, or Transit only, enabled in the overlay and owning the versioning; list the chosen path in the Verification Plan.

7. **Major. Task 013-4 against Technical Plan 3 and the settled correctness Blocker 1.** The task still says "the reserved `federated:` prefix in `identity.rs`" with the test `a_federated_subject_cannot_be_parsed_from_a_deployment_token`, which would reintroduce the Blocker. Fix: remove it; move the refusal and a test such as `a_deployment_token_naming_a_federated_subject_is_refused` to Task 013-10.

8. **Major. Task 013-13 against Requirement 013-013 and Technical Plan 13.** The test `the_pass_stores_the_default_role_with_its_audit` asserts what the requirement forbids (an expiry is never stored as a role change). Fix: `an_expired_mapped_role_is_audited_once_and_never_stored`, or nothing if finding 2 is accepted.

9. **Nit. Technical Plans 2 and 4.** The platform verifier's key set restates `JwksCache` (time to live, single-flight refresh, minimum interval, staleness bound), which already reads through a `JwksFetcher`. Fix: the platform key set is a `JwksCache` over a fetcher of `identity_signing_keys`.

10. **Nit. Technical Plan 2.** Moving `JwksCache` into `infra-federation` makes `api-http`, which depends on no `infra-*` crate today, depend on the crate holding the SAML parser. Fix: put `JwksCache` and `JwksFetcher` beside the guarded client in `crates/infra-egress`, or keep it in `api-http` and name the edge.

11. **Nit. Requirements 013-013, 013-016, 013-019, 013-021, Technical Plan 4.** Requirements carry SQL, lock slots, function names, a list of twelve stores, paths and test names. Fix: behaviour and guarantee in requirements; mechanics in Technical Plans 4 and 13.

12. **Nit. Technical Plan 1 against Technical Plans 2 and 6.** "Nothing runs in the engine" while the maintenance passes run in the engine's leased loop. Fix: reword.

13. **Nit. Requirement 013-024, Technical Plans 11 and 13.** The egress guard is described in Technical Plan 2, not 4; `IDENTITY_OWNER_DISABLED` is missing from the closed alert list that requires a rule test. Fix: correct the reference; add the alert with its test or drop it.

14. **Nit. Technical Plans 3 and 13.** `identity_connection_revisions` keeps an immutable history whose only use is binding `TESTED` and `ACTIVE` to a revision, which a counter does. Fix: a `settings_revision` counter; the history table when something reads it.
