# ADR-0140: The control plane brokers each organization's sign-in and issues its own short-lived tokens; the tenant comes from the connection, never from a claim

**Status:** Proposed
**Decided by:** agent proposal (design 013, 2026-11-24), for acceptance under Owner decision 2026-11-24-4 once the review of design 013 is complete
**Refines / supersedes:** refines Architecture decision 0016 (the Studio is a same-origin application; its OIDC flow now talks to the platform's own issuer when federation is on) and Architecture decision 0019 decision 3 (the development identity provider stays development-only and gains the flows a federated sign-in needs); refines Architecture decision 0062 decision 5 (an embedded app's sign-in popup goes to the platform's issuer); supersedes none. The deployment issuer of feature 001 (`crates/api-http/src/auth`) is kept unchanged beside it
**Enforced by:** planned in design 013 and not existing yet: the signer test that a sign-in through a connection can only produce a token naming that connection's tenant; `tests/contract/federated-login/` (an identity provider's token and an ID token are never accepted as a platform access token; the authorize endpoint refuses an unregistered redirect URI and a request without S256 PKCE; no route returns a platform token to the apps origin; a sign-in started by another site establishes no session); `tests/integration/federated-login/revocation.rs` (a suspended member's next request is refused on every path that acts for a subject; a revoked session's token is refused within 15 seconds, including a revocation committed behind a poll); the architecture rule that only `crates/api-http/src/auth` builds a `Principal` from a token

## Context

Today one deployment-wide OIDC issuer signs every person in. The API verifies its tokens (`crates/api-http/src/auth`),
trusts the `tenant` claim to name the tenant and the `sub` claim to name the subject, and reads the built-in role from
`memberships (tenant_id, subject)` on every request (`crates/application/src/identity.rs`, `authenticate`). That
works for one organization that runs its own provider, or for a deployment whose operator federates everything
upstream. It does not let each tenant bring its own provider: a tenant's administrator cannot connect the company's
Okta, Entra ID or Google Workspace, or a SAML provider, without the operator changing the deployment.

Owner decision 2026-11-24-1 asks for "Studio sign-in through each organization's OIDC or SAML provider". Three forces:

1. **Trust of the tenant.** If an organization's provider could put any `tenant` in a token the API accepts, one
   tenant's administrator could sign people into another tenant. The binding of a provider to a tenant must be the
   platform's, not the provider's.
2. **Two protocols.** OIDC providers issue JSON web tokens; SAML providers issue signed XML assertions to a browser
   form post. The API should verify one token format, not two.
3. **Revocation.** Offboarding must take effect quickly, without a revocation list per provider.

## Decision

1. When federation is enabled, the control plane MUST act as an OpenID provider for its own clients (the Studio, the
   CLI, the apps origin of feature 027) at `<public origin>/v1/auth`, and as a relying party (OIDC) or service
   provider (SAML) toward each tenant's configured identity connections.
2. Each identity connection MUST belong to exactly one tenant. A sign-in through it MUST produce a token for that
   tenant only; no claim, attribute, parameter or path segment from the identity provider or the browser MAY choose
   the tenant.
3. The platform MUST issue its own access tokens: ES256 JSON web tokens of at most 300 seconds, with `typ`
   `at+jwt`, `aud` the API's resource identifier, `client_id`, `iss` the platform issuer, `tenant`, `sub` (the platform
   subject the connection's identity is linked to), `sid` (the sign-in session) and `jti`; the platform verifier MUST
   require each and enforce the lifetime itself. The signing key MUST live in a separate OpenBao mount and policy for
   identity material (a Transit key that never leaves OpenBao where available), never nameable by a workflow secret
   reference, and be rotated with an overlap; the public keys MUST be served at the issuer's key set, disjoint from
   the connections' client keys. An identity provider's own token, an ID token, or a token signed by a connection key
   MUST NOT be accepted by the API as a platform access token. A deployment issuer equal to the platform issuer MUST be
   refused at start-up, and a deployment-issuer token naming a `user:` subject MUST be refused.
4. The API's verifier MUST accept tokens from the deployment issuer exactly as today and from the platform issuer by
   its in-process key set; it MUST select the verifier by `iss` and refuse any other issuer. The built-in role MUST
   still come from the membership on every request, which MUST now also be active (not suspended).
5. A sign-in session MUST be a database row with an absolute and an idle expiry, created only when the client
   redeems its authorization code with the PKCE verifier (never at the provider's callback), and, for the Studio,
   bound to the browser by an `HttpOnly`, `Secure`, `SameSite=Lax`, host-only cookie whose value is stored only as a
   hash. The Studio MUST obtain new access tokens from the session through a same-origin renewal call that the browser
   cannot send cross-site (cookie, exact `Origin`, a custom request header, no CORS on any sign-in route); tokens stay
   in memory, as today. The apps origin of feature 027 MUST never receive a platform token: its codes are redeemed in
   process by its session creation.
6. Revoking a session (sign-out, sign-out everywhere, suspension, removal, a SCIM deactivation, a disabled or reset
   connection, a raised authentication context) MUST stop renewal at once and MUST make every serving process refuse
   that session's tokens within 15 seconds of the revocation's commit: each process re-reads, every 10 seconds, the
   window of revocations younger than the token lifetime plus leeway plus one interval (a window, never a watermark),
   and refuses platform tokens when its last read is older than 15 seconds. A suspended or removed membership MUST be
   refused on its next request on every path that acts for that subject.
7. The deployment issuer and the operator's recovery command MUST stay available as the break-glass path for a tenant
   that requires federated sign-in, with every use audited and alerted.

## Considered options

1. **Broker in the control plane, platform tokens (chosen).** One token format for the API; the tenant binding is the
   platform's; OIDC and SAML end in the same session; revocation through the session and the membership; the Studio's
   existing OIDC code talks to a same-origin issuer.
2. **Accept each tenant's OIDC tokens directly (a multi-issuer resource server).** No token minting. Rejected: SAML
   produces no bearer token, so SAML would still need a broker; the API would trust each provider's claims for the
   subject; revocation would depend on each provider's token lifetime; the Studio would need a connect-src entry per
   provider in its content security policy.
3. **An external broker (Keycloak, Dex, Zitadel) run beside the platform.** Least code here. Rejected: the
   per-tenant configuration, SCIM provisioning, group mapping and audit would live in another product with its own
   database, outside the tenant boundary and the audit trail of this one, and every deployment would operate a second
   identity system; the repository's infrastructure rule would require shipping and upgrading it.
4. **Opaque session tokens checked in the database on every request.** Instant revocation. Rejected: a read per API
   request on every route for a guarantee the membership check and a ten-second revocation poll already give; workers
   and the deployment issuer would still use JSON web tokens, so the API would verify two formats anyway.

## Consequences

- The platform holds signing keys of its own; their rotation is an operational duty with a runbook.
- The Studio's sign-in starts at the platform's issuer when federation is on; nothing changes for a deployment that
  does not enable it.
- A revoked session can act for at most the poll interval; a suspended membership not at all.
- An embedded app (feature 027) signs in through the platform's issuer, so the apps origin's redirect is registered by
  the platform itself (closes Gap 1364 of feature 027 for federated tenants).
- Rollback: disable federation; the deployment issuer keeps working; sessions and connections stay stored.

## Evidence

The current verification path is read from `crates/api-http/src/auth/verifier.rs`, `config.rs` and
`crates/application/src/identity.rs` on `develop` at 2026-11-24. The planned tests are listed under **Enforced by**;
none exists yet.
