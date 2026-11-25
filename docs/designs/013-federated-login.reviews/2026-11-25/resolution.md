# Resolution of the reviews of design 013 (2026-11-25)

The author's disposition of each finding, under Owner decision 2026-11-24-4 (findings settled by the delegate). A
finding marked fixed is answered in the design or its records at the commit that follows the report; the re-read of
the round checks the fixes.

## Security review, round 1 (`security.md`, at `6a49c922`)

All 25 findings are accepted and fixed; none is contested.

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Blocker | Fixed: the session and its cookie are created only at code redemption with the PKCE verifier; a live session of another tenant is never replaced silently; the Studio refuses a renewed token of another tenant and shows the tenant; an unsolicited SAML response answers a page with a button | Requirements 013-008, 013-009, 013-018; Technical Plan 4; Decision 013-012; Architecture decision 0140 decision 5 |
| 2 | Blocker | Fixed: `apps` codes are redeemed only in process by feature 027's session creation; renewal is Studio-only; no CORS on `/v1/auth/`; codes bound to their client | Requirement 013-005; Decision 013-004; Technical Plan 4; Architecture decision 0140 decision 5 |
| 3 | Blocker | Fixed: the poll re-reads a window, never a watermark; the set is loaded before the first platform token; fail closed after 15 seconds; the bound is 15 seconds everywhere; the platform verifier enforces 300 seconds itself | Requirement 013-019; Technical Plan 4; Decision 013-003; Gap 013-003; Architecture decision 0140 decision 6 |
| 4 | Major | Fixed: disabling, deleting or resetting a connection and raising its context revoke its sessions; renewal, refresh and silent issuance re-check four conditions | Requirements 013-001, 013-010, 013-019 |
| 5 | Major | Fixed: one store method that never returns a suspended membership, on every path that acts for a subject (bearer, app sessions, schedules, links, stream and blob deadlines); a deactivation revokes app sessions too | Requirement 013-016; Technical Plan 2 (architecture rule) |
| 6 | Major | Fixed: refused subject formats; a closed list of OIDC subject claims (`oid` with `tid` for Entra ID); an immutable SCIM matching attribute | Requirements 013-003, 013-004, 013-015; Architecture decision 0142 decision 1 |
| 7 | Major | Fixed: template issuers refused; shared issuers need a tenant restriction checked on every ID token; the domain limit reads only a verified e-mail, never Entra ID's `email` | Requirements 013-003, 013-007, 013-012 |
| 8 | Major | Fixed: the issuer and the entity identifier are immutable | Requirements 013-003, 013-004; Decision 013-014 |
| 9 | Major | Fixed: one owning connection; rules match only that connection's groups; a role mapped from assertion groups expires with them | Requirement 013-013; Technical Plan 6; Decision 013-013; Architecture decision 0142 decision 5 |
| 10 | Major | Fixed: linking to an existing subject only inside that subject's sign-in; the member routes never attach; recent sign-in for enabling federated-only and creating SCIM tokens | Requirements 013-011, 013-020, 013-021; Architecture decision 0142 decision 2 |
| 11 | Major | Fixed: `at+jwt`, the API audience and `client_id` required; no other token kind with the access-token key; equal issuers refused; `user:` subjects refused from the deployment issuer | Requirement 013-018; Technical Plan 4 (Verify); Architecture decision 0140 decision 3 |
| 12 | Major | Fixed: a new identity egress guard specified (global addresses, allowlisted hosts never to loopback, link-local or metadata, port 443, no redirects); per-connection caches with a minimum refresh interval | Requirement 013-024; Technical Plan 2 |
| 13 | Major | Fixed: a separate OpenBao mount and policy for identity material; Transit for the signing key where available; disjoint key sets | Requirement 013-025; Decision 013-015; Architecture decision 0140 decision 3 |
| 14 | Major | Fixed: a challenge always succeeds; the conflict is refused only at verification | Requirement 013-022; Technical Plan 8 |
| 15 | Major | Fixed: the profile names the unsigned Response values it compares and adds every structural and semantic check listed; the certificate check replaced by the signature algorithm and key size | Technical Plan 5; Architecture decision 0141 decisions 3 and 4 |
| 16 | Major | Fixed: limits per address and global, a per-tenant limit that admits live attempts and sessions first, a capped attempts table purged at expiry | Technical Plans 3 and 8 |
| 17 to 25 | Nit | Fixed: machine members exempt by role, break-glass audited once per token and refused on a failed audit; codes stored as hashes and bound; the loopback redirect and no CLI cookie; no provider error text forwarded; `startLogin` and the content security policy described as they are; the tenant check restated as a signer test; PS256 not mapped to RS256; key rotation keeps the state; SCIM and callback scoping; the test sign-in provisions nobody; enumeration recorded as Gap 013-007 | Requirements 013-004, 013-005, 013-006, 013-015, 013-017, 013-021, 013-030; Technical Plans 3, 4, 7, 10; Architecture decision 0140 "Enforced by" |

## Architecture review, round 1 (`architecture.md`, at `77f6a49b`)

All 26 findings are accepted and fixed; none is contested. Finding 4 also replaces the revocation poll that the
security review's Blocker 3 fix had specified: revocation now happens through the membership read every request already
makes, so the watermark, window and fail-closed questions of that Blocker no longer arise.

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Blocker | Fixed: suspension enforced in the schema (the redefined `sagat_lock_memberships` and the three plain reads), an architecture rule on reads of `memberships`, a test per caller family | Requirement 013-016, Technical Plans 2 and 3, Task 013-5 |
| 2 | Blocker | Fixed: `identity-federation-v1` in the instance registry, operator activation, refusals until recorded, design 050's names derived from it, rollback below it unsupported, an older-replica test | Requirement 013-032, Rollout, Architecture decision 0140 decision 9, Task 013-10 |
| 3 | Major | Fixed: the slot sequence of every write transaction; no app-session writes; memberships before the fence; the assertion write's deviation argued | Technical Plan 4 (Lock order), the group-source contract |
| 4 | Major | Fixed: the session joined into the per-request membership read; poll, table and fail-closed mode removed | Requirement 013-019, Decision 013-003, Technical Plan 4, Architecture decision 0140 decision 6 and option 5 |
| 5 | Major | Fixed: the key lifecycle (operator commands, the active version in PostgreSQL, publish before signing), the OpenBao dependency and its alert, `TokenSigner` and `IdentitySecrets` in `infra-secrets`, the intent, write, verify, activate protocol, the development overlay's identity mount | Requirement 013-025, Technical Plans 2, 4 and 9, Architecture decision 0140 decision 7 |
| 6 | Major | Fixed: Decision 013-019 and Owner decision row OD-2026-11-24-717, cited by `Owner-gate:` on Tasks 013-11 and 013-17; feature 015's roadmap row in Task 013-1; the 026 and 027 tests gated | Decision 013-019, Tasks 013-1, 013-11, 013-17 |
| 7 | Major | Fixed: the engine verifies the deployment issuer only; workers stay machine identities; `Core::authenticate` widened | Decision 013-016, Gap 013-008, Technical Plans 2 and 4 |
| 8 | Major | Fixed: external groups are rows of 029's `access_groups` with new columns; 029's migration first; the connection-delete rule; recorded in 029's data model and contract | Decision 013-017, Technical Plan 3, specs/029 data-model and group-source |
| 9 | Major | Fixed: `PlatformSignIn::redeem_for_app`; app sessions bound to the sign-in session; Requirement 027-014 amended | Technical Plan 2, Task 013-17, specs/027 spec |
| 10 | Major | Fixed: Architecture decision 0062 refined with the sign-in pages' exception and the origin equalities; start-up test | Technical Plan 7 (Origins), Architecture decision 0062 acceptance refinement 4 |
| 11 | Major | Fixed: the `config.json` shape with both issuers, the deployment issuer kept in `connect-src`, `credentials: 'same-origin'` | Technical Plan 7, Task 013-14 |
| 12 | Major | Fixed: the prefix is `federated:`, reserved by `Subject::parse`, and the migration refuses while a membership uses it | Requirement 013-032, Technical Plan 3 |
| 13 | Major | Fixed: `down` refuses while a member is suspended or any federation row exists; the rollback path stated | Requirement 013-032, Technical Plan 3, Rollout |
| 14 | Major | Fixed: the effective role computed in the per-request read; the maintenance passes placed with their lease and capability | Technical Plan 6, Technical Plan 2, Task 013-13 |
| 15 | Major | Fixed: separate paths for the browser flows, the JSON route kept, Architecture decision 0140 decision 8 | Technical Plan 9, Task 013-3 |
| 16 | Major | Fixed: the development providers moved to the server lane first; Task 013-5 split in three; 026 and 027 tests gated; audit, metrics and ceiling tests named | Detailed Implementation Plan |
| 17 | Major | Fixed: the rules on `VerifiedClaims` and `Membership::principal`, with negative fixtures | Technical Plan 2, Architecture decision 0140 "Enforced by" |
| 18 | Major | Fixed: break-glass, tenant settings, signing keys and break-glass uses tables; tenant keys everywhere; the federated-only flag on `tenants`; delete rules | Technical Plan 3 |
| 19 to 26 | Nit | Fixed: the identity egress policy in the domain, the resolver in `infra-egress`; the OIDC rules in the domain and `JwksCache` shared; the process split argued with revisit criteria; the contradictions removed; `x-access` on the new operations; the CLI's source order; streams reconnecting with `Last-Event-ID`; the region epoch note | Technical Plans 2, 4, 11; Decision 013-018; Requirements 013-019, 013-026; Gap 013-009; Architecture decision 0140 |

## Correctness review, round 1 (`correctness.md`, at `2368d9ab`)

All 33 findings are accepted and fixed; none is contested. Most fixes are in the new Technical Plan 13 ("Concurrency,
grants and edge rules").

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Blocker | Fixed: `Subject::parse` does not reserve `federated:`; the refusal lives in `authenticate_bearer` for deployment tokens, case-insensitive; an end-to-end red test with a schedule | Technical Plan 3, Task 013-5 |
| 2 | Major | Fixed: one effective-membership expression used by every read, the plain reads moved into the identity store; the pass never stores a role | Technical Plans 6 and 13, Requirement 013-013 |
| 3 | Major | Fixed: assertion rows carry their `sid`; live sessions joined and unioned | Technical Plan 13, Architecture decision 0142 decision 4, specs/029 contract and data model |
| 4 | Major | Fixed: FR-029-024 amended to keep external memberships; a computed fallback is not a stored change; the survival test | specs/029 spec, Technical Plan 13, Requirement 013-013 |
| 5 | Major | Fixed: a rule or default change recomputes owned members; the fallback is the lower of mapped and default | Requirement 013-013, Technical Plan 6 |
| 6 | Major | Fixed: the code is consumed first by a conditional update; the token signed before the commit; a barrier test | Technical Plan 4 (Redeem), Technical Plan 3 |
| 7 | Major | Fixed: definer functions for every write to `memberships`; the runtime role keeps `SELECT`; a privilege test | Technical Plan 13, Requirement 013-016, Architecture decision 0142 decision 6 |
| 8 | Major | Fixed: `FOR NO KEY UPDATE` in the functions, re-checks after the lock, ceilings counted under the writer's lock | Technical Plan 13 |
| 9 | Major | Fixed: the connection row taken `FOR SHARE` by sign-in paths and `FOR UPDATE` by a disable; a barrier test | Technical Plan 13 |
| 10 | Major | Fixed: the tenant's identity-settings row serializes federated-only, connection states and the last administrator | Requirement 013-021, Technical Plan 13 |
| 11 | Major | Fixed: generation re-read and retry for SCIM group writes | Technical Plan 13, Task 013-11 |
| 12 | Major | Fixed: an advisory lock per identity; adoption; re-provisioning to a new subject; typed conflicts | Technical Plan 13 |
| 13 | Major | Fixed: unique names among local groups only; external rows unique by connection and value | Technical Plan 3, specs/029 data model |
| 14 | Major | Fixed: compare-and-set rotation, signing before the commit, a grace window for the previous generation, a CLI file lock | Requirement 013-026 |
| 15 | Major | Fixed: no app-session write; app sessions check the membership and the sign-in session | Requirement 013-016, Architecture decision 0142 decision 6 |
| 16 | Major | Fixed: streams carry `sid` and re-check at most every 10 seconds; the bound restated and measured | Requirement 013-019, Non-functional requirement 013-001 |
| 17 | Major | Fixed: retired keys kept until the next activation plus lifetime, refresh and leeway; refresh on an unknown `kid`; signing stops on a stale set; a controlled clock | Technical Plan 4 (Signing keys) |
| 18 | Major | Fixed: `TESTED` and `ACTIVE` bound to the tested revision | Technical Plan 13, Technical Plan 3 |
| 19 | Major | Fixed: a silent code renews the existing session and is never recent | Technical Plan 4 (Begin), Requirement 013-021 |
| 20 | Major | Fixed: the link flow specified, with the test sign-in inside it, and a positive test | Technical Plan 13 |
| 21 | Major | Fixed: suspension revokes the role-change grants, reveal grants and secret access entries; reactivation restores nothing | Requirement 013-016 |
| 22 | Major | Fixed: `ON DELETE SET NULL`, no-owner rule, the disabled-owner alert, recompute on ownership change | Technical Plans 3 and 13 |
| 23 | Major | Fixed: the last-administrator guard under the settings row lock | Requirement 013-021, Technical Plan 13 |
| 24 to 33 | Nit | Fixed: renewal only after activity; the code's expiry bounded by its attempt and attempts purged after codes; constraints named in `classify`; break-glass `ON CONFLICT DO NOTHING`; the plain reads moved; "merges two reads into one"; the reasons returned by the statement; the barrier oracle stated and the key test on a controlled clock; the writer's check of the requirement under a shared epoch lock and the activation window; case-insensitive prefix checks | Requirements 013-018, 013-019; Technical Plans 3, 4 and 13 |

## egm-review Stage 5, Initial Zoom Out (`zoom_out_report.md`, at `d7cca86d`)

All 14 findings are accepted and fixed; none is contested. Findings 2 to 6 remove scope: groups from the sign-in
assertion, domains, the development SAML provider, the `apps` client and Transit are deferred, each with a gap, which
also removes the parts of the correctness fixes that existed only for them (the session-bound effective role, the
union of assertion sets, the expiry audit).

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Fixed: milestones A to D with their own acceptance; the 029-dependent parts gated in milestone C | Decision 013-020, Success criterion 013-001, the plan's introduction, Tasks 013-5, 013-11, 013-12, 013-17 |
| 2 | Major | Fixed: groups and mapping from SCIM only; Gap 013-011 names the providers that lose group sync; 029's contract, data model and FR-029-024 reverted to the SCIM kind | Decision 013-007, Requirements 013-013, 013-014, Technical Plans 3, 6, 13, Architecture decision 0142 decisions 4 and 5, specs/029 |
| 3 | Major | Fixed: slug sign-in with `login_hint` passed through; domains deferred | Decision 013-011, Requirements 013-006, 013-012, 013-022, Technical Plans 3 and 8, Gap 013-010 |
| 4 | Major | Fixed: no development SAML provider; SAML against the pinned Keycloak in tests and the development overlay; OIDC refusals as domain unit tests | Decision 013-010, Requirement 013-033, Technical Plan 9, Task 013-3 |
| 5 | Major | Fixed: only `studio` and `cli`; the contract feature 027 meets; 027 builds its redemption | Requirement 013-005, Decision 013-019, Architecture decision 0140 decision 1, specs/027 spec, Task 013-17 |
| 6 | Major | Fixed: one backend, the identity-mount key read by the control plane only; Transit deferred | Decision 013-015, Requirement 013-025, Gap 013-012, Architecture decision 0140 decision 3 |
| 7 | Major | Fixed: Task 013-4 no longer reserves the prefix | Task 013-4 |
| 8 | Major | Fixed: the pass stores and audits no expired role (none exists now); its tests replaced | Task 013-13 |
| 9 | Nit | Fixed: the platform key set is a `JwksCache` over a fetcher of `identity_signing_keys` | Technical Plan 4 |
| 10 | Nit | Fixed: `JwksCache` and `JwksFetcher` move into `crates/infra-egress` | Technical Plan 2, Task 013-6 |
| 11 | Nit | Fixed in part: Requirement 013-016 states the behaviour and its guarantee, the mechanics in Technical Plan 13; the other requirements keep their acceptance test names, which the template asks for | Requirement 013-016 |
| 12 | Nit | Fixed: the control plane and the engine's leased loop named | Technical Plan 1 |
| 13 | Nit | Fixed: the reference corrected; `IDENTITY_OWNER_DISABLED` in the alert list with its rule test | Requirement 013-024, Technical Plan 11 |
| 14 | Nit | Fixed: a `settings_revision` counter instead of a revisions table | Technical Plan 3, Task 013-4 |

## egm-review Stage 6, Comprehension Test (`comprehension_report.md`, at `26d4d65b`)

28 of the 29 findings are accepted and fixed. Two are settled in part. While fixing finding 12 the author also found
that the sign-in service's routes share `/v1/auth/` with the existing REST route `GET /v1/auth/context`. Technical Plans
7 and 11 now scope the sign-in service's rules to its own listed routes. The API answers no route with CORS headers.

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Blocker | Fixed: a numbered "Test sign-in and link" flow. A bearer-authenticated `POST /v1/auth/links` returns a one-use start address; a small window binds the attempt to the browser; the callback issues no code; the opening window confirms with the same bearer token and the attempt cookie. No session row is created. CUJ 1 and Requirements 013-001, 013-011 and 013-012 cite it | Technical Plan 4, Technical Plan 13, CUJ 1, Task 013-12 |
| 2 | Major | Fixed in the Overview and in Architecture decision 0140 decision 8 | Overview, Architecture decision 0140 |
| 3 | Major | Fixed: organization, with an optional `login_hint` | Requirement 013-027 |
| 4 | Major | Fixed: no group is read; the attempt gains a purpose and the starting subject | Technical Plans 3 and 4 |
| 5 | Major | Fixed by removal: authorize always goes to the provider. Reload, a new tab or a deep link renews instead. "Recent" means a provider round trip; the test is now `a_renewal_is_not_a_recent_sign_in` | Requirements 013-019, 013-021, Technical Plan 4 |
| 6 | Major | Fixed: one platform session per browser, stated. The server refuses with `409 SESSION_CONFLICT` unless `replace_session=true`, and the old session is revoked in the transaction | Requirement 013-018, Technical Plans 4 and 11, Task 013-9 |
| 7 | Major | Fixed: redemption is one numbered transaction. The order is code (slot 2), advisory lock, then the settings row, connection, membership and fence (slot 4a), then the session, the attempt and the signature. The code is marked once. Slot is glossed and linked | Technical Plans 4 and 13, Terms |
| 8 | Major | Fixed: "at least once every 10 seconds (an interval of at most 10 seconds)" | Requirement 013-019 |
| 9 | Major | Fixed (option a): within 30 seconds, while the successor has never been used, a reuse issues a new successor and retires the unused one. A reuse after the successor was used revokes the session (new test). The CLI re-reads the key store under its lock | Requirement 013-026, Technical Plan 4, Task 013-15 |
| 10 | Major | Fixed: `federated_only` moves onto `tenant_identity_settings`. There is exactly one row per tenant, inserted by the migration and by tenant creation, and the membership read's join refuses a tenant without it | Technical Plans 3, 4 and 13, Task 013-5 |
| 11 | Major | Fixed: no conversion path exists. A tenant leaving federation gives each person a new membership under the deployment issuer, and the old subject's grants stay with the old subject | Technical Plan 12 |
| 12 | Major | Fixed: two route tables with full prefixes, authentication and permissions. `membership:manage` exists today and is held by `admin` only. `/auth/sessions` is an ordinary REST route | Technical Plan 11, Requirement 013-020, Technical Plan 7 |
| 13 | Major | Fixed: the interval is 30 seconds by default and the leeway 30 seconds. Every statement says two intervals. The command waits and resumes. A retired key is kept until its successor's activation plus 300 seconds, one interval and the leeway | Requirement 013-025, Technical Plan 4, Architecture decision 0140 decision 7 |
| 14 | Major | Fixed: Terms gain slot, access fence, instance registry, intent-write-verify-activate, reveal grants and the other entries, and the test terms, with links. The beat is corrected to 10 seconds (`HEARTBEAT` in `crates/application/src/instances.rs`). The capability names cite design 050 and Architecture decision 0143 | Terms, Requirements 013-032, Technical Plan 13 |
| 15 to 23 | Nit | Fixed. The domain leftovers are removed and the test renamed. The two plain reads are named. The nine tests are placed in their tasks. Abbreviations are written in full: "CUJ" stays as the template's heading term, and `OD-…` stays in register strings. The residual of a stolen token is stated, and the OpenBao consequence is made conditional in both places. The federated-only re-check on renewal and the break-glass context clause are removed. The reset rule is moved into Requirement 013-021. The terms are added | Throughout |
| 24 | Nit | Settled in part. Architecture decision 0050 decision 9 is the fleet capability activation that 0082 refines, so the citation was right. Architecture decision 0082 is added beside it for clarity | Architecture decision 0140 |
| 25 to 29 | Nit | Fixed. The pre-provisioning workflow is stated, with the provider guides. The deployment issuer is a second choice on the sign-in page. Tasks 013-16 and 013-18 go to the server builder after both lanes. The OIDC leeway is 30 seconds. CUJ 5's Given is stated | Requirement 013-020, Technical Plan 10, the plan, Requirement 013-007, CUJ 5 |
