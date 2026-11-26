# ADR-0140: The control plane brokers each organization's sign-in and issues its own short-lived tokens; the tenant comes from the connection, never from a claim

**Status:** Proposed
**Decided by:** agent proposal (design 013, 2026-11-24), for acceptance under Owner decision 2026-11-24-4 once the review of design 013 is complete
**Refines / supersedes:** refines Architecture decision 0016 (the Studio is a same-origin application; its OIDC flow now talks to the platform's own issuer when federation is on; a stream still ends at its token's expiry, now at most five minutes); Architecture decision 0019 decisions 2 and 3 (the CLI gains a key-store source after `--token-file` and `SAGAT_TOKEN`; the development identity provider gains browser flows on separate paths, decision 8 below); Architecture decision 0062 decisions 5 and 8 (an embedded app's sign-in goes to the platform's issuer and is redeemed in process; the platform-authored sign-in pages are allowed on the Studio origin); and Architecture decision 0050 decision 7 (design 013 adds to slot 4a the tenant's identity-settings row and the connection row before memberships, puts the authorization code in slot 2, and takes a per-identity advisory lock between them; recorded in Architecture decision 0050 as an addition), Architecture decision 0050 decision 9 and Architecture decision 0082 (fleet capability activation through the instance registry: federation is activated that way, decision 9 below); supersedes none. The deployment issuer of feature 001 (`crates/api-http/src/auth`) is kept beside it
**Enforced by:** planned in design 013 and not existing yet: the module `openbao::identity_ring` (design 013 Technical Plan 12, with `openbao::identity_mount`, whose paths cannot reach a ring, `an_identity_path_cannot_reach_a_ring`), which alone holds the rings' material, from its own transport that reads and rewrites the stored document to deriving purpose keys, with the architecture rule that keeps the label literals `"cli-refresh"` and `"signin-attempt"` there and allows that module to expose only its listed types, operations and trait implementations (only `RingKey`, `PurposeKey` and `RingDocument` bear material, all fully private; the operations take only an `IdentityOpenBao`, built only by `IdentityOpenBao::from_lookup` from the identity settings, `identity_openbao_never_reads_the_secrets_mount_settings`, `an_identity_client_refuses_a_token_without_only_the_identity_policy`, `a_secrets_client_refuses_a_token_with_the_identity_policy`), and `an_identity_key_ring_never_prints_key_material`, `a_cli_refresh_key_rotation_keeps_a_session_refreshing_across_it` and `a_forged_lower_generation_does_not_revoke_the_session` and `a_ring_loads_only_from_its_own_path` (decision 11); `a_reenabled_connection_revives_no_session` and `a_revoked_key_is_refused_on_every_replica_at_once` (decisions 6 and 7); the signer test that a sign-in through a connection can only produce a token naming that connection's tenant; `tests/contract/federated-login/` (an identity provider's token and an ID token are never accepted as a platform access token; the authorize endpoint refuses an unregistered redirect URI and a request without S256 PKCE; no route returns a platform token to the apps origin; a sign-in started by another site establishes no session); `tests/integration/federated-login/revocation.rs` (a suspended member's next request is refused on every caller family of the membership read functions; a revoked session's token is refused on its next request, with the revocation committed while a request is held by a barrier); the older-replica activation test of `identity-federation-v1`; the architecture rules that `VerifiedClaims` is consumed only by `authenticate_bearer`, that `IdentityService::authenticate` is the only caller of `Membership::principal` outside the domain, and that no SQL reads `memberships` outside `crates/infra-postgres/src/identity.rs` and the migrations

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

1. When federation is enabled, the control plane MUST act as an OpenID provider for its own clients (the Studio and the
   CLI; feature 027 registers its apps origin as a third client when it is built) at `<public origin>/v1/auth`, and as a relying party (OIDC) or service
   provider (SAML) toward each tenant's configured identity connections.
2. Each identity connection MUST belong to exactly one tenant. A sign-in through it MUST produce a token for that
   tenant only; no claim, attribute, parameter or path segment from the identity provider or the browser MAY choose
   the tenant.
3. The platform MUST issue its own access tokens: ES256 JSON web tokens of at most 300 seconds, with `typ`
   `at+jwt`, `aud` the API's resource identifier, `client_id`, `iss` the platform issuer, `tenant`, `sub` (the platform
   subject the connection's identity is linked to), `sid` (the sign-in session) and `jti`; the platform verifier MUST
   require each and enforce the lifetime itself. The signing key MUST live in a separate OpenBao mount and policy for
   identity material, readable by the control plane only (OpenBao Transit signing is deferred), never nameable by a workflow secret
   reference, and be rotated with an overlap; the public keys MUST be served at the issuer's key set, disjoint from
   the connections' client keys. An identity provider's own token, an ID token, or a token signed by a connection key
   MUST NOT be accepted by the API as a platform access token. A deployment issuer equal to the platform issuer MUST be
   refused at start-up, and a deployment-issuer token naming a `federated:` subject MUST be refused.
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
6. Revoking a session (sign-out, sign-out everywhere, suspension, removal, a SCIM deactivation, a disabled connection, a raised authentication context) MUST stop renewal at once and MUST make the session's access tokens
   refused on their next request: the per-request membership read, which every request already makes, joins the
   session by (`tenant`, `sid`) in the same statement, requires the session's subject and client to equal the token's,
   and refuses a revoked or expired one. A suspended or removed
   membership MUST be refused on its next request on every path that reads a membership for authority, which the
   schema enforces (the membership read functions return `ACTIVE` memberships only).
7. Signing keys MUST be created and rotated only by an audited operator command, never by a serving process; the active
   key version MUST be recorded in PostgreSQL; a new public key MUST be in every replica's key set (refreshed from that
   record) for at least two refresh intervals before any replica signs with it, and a retired key stays verifiable
   until its successor's activation plus two refresh intervals, the longest token lifetime and the leeway. A leaked key
   is revoked by an audited operator command and refused on every replica at once by the per-request statement. An OpenBao outage stops issuance and renewal, never verification, and is alerted.
8. The development identity provider keeps its JSON-only token route for `sagat dev token`; its browser routes for the
   development OIDC provider live on separate paths (SAML runs against a pinned Keycloak container, design 013
   Decision 013-010), keep its development-only, loopback, known-host and
   bounded-body protections, and issue tokens only for the configured test people; its one claims override, for the
   refusal tests, sets only the claims those tests break, and works only with the test-only setting that the test
   support sets.
9. Federation MUST be activated through the instance-capability registry (Architecture decision 0082): a control plane lists
   `identity-federation-v1` only when its configuration enables federation, and an execution engine of a release
   that knows federation lists it always (the engine needs no configuration for federation), the operator records the requirement, and
   until it is recorded connections, SCIM, suspension and federated-only are refused; a binary rollback below the
   requirement is unsupported.
10. The deployment issuer and the operator's recovery command MUST stay available as the break-glass path for a tenant
   that requires federated sign-in, for human members named by the operator, with every use audited and alerted; the
   execution engine keeps verifying the deployment issuer only, so workers stay machine identities.
11. The CLI's refresh token MUST be `<sid>.<generation>.<key id>.<mac>`, an HMAC-SHA256 under a `cli-refresh` purpose
   key of a dedicated ring in the identity mount, never `SAGAT_SECRETS_KEYS` (held by every execution engine) and never
   the access-token key, over the session id, the generation and a random value kept on the session, the purpose key derived by HKDF-SHA256
   (no salt, the label `cli-refresh` as `info`) from the ring key the token names; the ring has its
   own type, rotates in two steps keeping the old key for the CLI session's life, writes by check-and-set, and has a
   `revoke` that replaces every key and rotates every random value. Rejected: a token signed with the access-token key;
   an opaque token with a hash per generation (unbounded storage); a key from the secrets ring. Consequences: a revoke
   or a database restore invalidates every CLI refresh token, so CLI users sign in again.

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
4. **Opaque session tokens checked in the database on every request.** Instant revocation. Rejected: workers and the
   deployment issuer would still use JSON web tokens, so the API would verify two formats; the session join of
   decision 6 gives the same instant revocation inside the read every request already makes.
5. **Five-minute tokens with a per-process poll of a revocation table** (the first draft). Rejected after the
   architecture review of 2026-11-25: a poller, a fail-closed mode and a window argument for a bound of seconds, where
   the session join gives revocation on the next request at no extra round trip.

## Consequences

- The platform holds signing keys of its own; their rotation is an operational duty with a runbook.
- The Studio's sign-in starts at the platform's issuer when federation is on; nothing changes for a deployment that
  does not enable it.
- A revoked session or a suspended membership acts on no request after the revocation commits.
- Every live stream reconnects at least every five minutes (its token's lifetime) and resumes with `Last-Event-ID`.
- When feature 032 is built, sessions and platform tokens must carry or check the region epoch, so that a failover that
  may have lost a revocation invalidates the sessions issued before it.
- A federated developer in a federated-only tenant runs workers with a machine identity an administrator binds.
- An embedded app (feature 027) signs in through the platform's issuer, so the apps origin's redirect is registered by
  the platform itself (closes Gap 1364 of feature 027 for federated tenants).
- Rollback: federation is turned off only after every tenant's federated-only setting is released and the requirement
  removed; federated members cannot sign in while it is off; a binary rollback below the requirement is unsupported.

## Evidence

The current verification path is read from `crates/api-http/src/auth/verifier.rs`, `config.rs` and
`crates/application/src/identity.rs` on `develop` at 2026-11-24. The planned tests are listed under **Enforced by**;
none exists yet.
