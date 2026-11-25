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
