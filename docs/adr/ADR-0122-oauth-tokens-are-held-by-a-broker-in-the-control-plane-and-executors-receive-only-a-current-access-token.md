# ADR-0122: OAuth tokens are held by a broker in the control plane, with values in the secret authority, and executors receive only a current access token

**Status:** Proposed
**Decided by:** agent proposal (design 047, 2026-11-24), to be accepted or rejected under Owner decision 2026-11-24-4 when design 047 completes its review
**Refines / supersedes:** refines ADR-0055 (the `oauth2_client_credentials` scheme of feature 021 no longer gives the client secret to the integration runner), ADR-0035 and the managed-secret provisioning of feature 017 (intent, provider write, activation); supersedes none
**Enforced by:** planned tests of design 047 (`docs/designs/047-oauth-integration-hub.md`, Verification Plan): the refresh race test with two control planes held at a barrier, the crash tests between the token response and the secret write, the disclosure test that no executor response ever contains a refresh token or client secret (canary values), the state and PKCE tests of the authorization flow, the egress tests of the token client, and `scripts/check-architecture.py` (only `crates/infra-oauth` speaks to a token endpoint). Until they exist: review only.

## Context

Windmill connects a resource to a provider account by OAuth and refreshes its token on the server; the script receives
the token in its resource. Feature 021 offers OAuth 2.0 client credentials only, with the client secret handed to the
integration runner, which requests a token on every attempt, and leaves authorization-code flows out. Design 047 adds
authorization-code credentials, a curated hub and resources usable by scripts and activities.

The forces:

1. A refresh token is a long-lived credential to a person's account. If an executor (a script, an activity) receives
   it, a compromised step keeps access after the run.
2. Many providers rotate the refresh token on each use and revoke the whole family when an old one is reused. Two
   concurrent refreshes of one credential lose it.
3. A crash after the provider answered and before the new token is stored can lose the rotated refresh token.
4. The authorization-code callback is a browser redirect; it must be bound to the person who started it (cross-site
   request forgery, mix-up of providers, code injection).
5. Principle V: secret values never reach workflow state, the journal, logs or responses, and an executor receives only
   the secrets its immutable task definition declares, all or nothing, with a value-free audit before disclosure.

## Decision

1. **A token broker in the control plane** (`crates/application/src/oauth`, adapter `crates/infra-oauth`) is the only
   component that talks to a token, authorization or revocation endpoint. It runs no tenant code. Its outbound calls use
   the guarded egress client (`crates/infra-egress`) under the namespace's egress policy intersected with the
   deployment's; redirects are not followed.
2. **Values live in the secret authority** of feature 017 (OpenBao, or the sealed application vault where a deployment
   uses it), each as a revision written with the recoverable provisioning of feature 017 (intent, provider write,
   activation). PostgreSQL holds only metadata: state, granted scopes, expiry, revision pointers, the refresh fence.
3. **The refresh token, the client secret and the client's private key never leave the broker.** An executor (the
   integration runner of feature 021, a sandbox runner for a script, an SDK worker for an activity) receives, for a
   resource its task definition declares, the resource's non-secret settings and a current access token with its
   expiry, through the ordinary declared-secret delivery of Principle V.
4. **One refresh at a time per credential.** A refresh takes a fenced lease on the credential row with a conditional
   update (no transaction open during the provider call), calls the token endpoint within a deadline shorter than the
   lease, writes the new revisions, and activates them with a compare on the fence. A caller that finds a refresh in
   progress returns the current access token if it is still valid, and otherwise waits a bounded time for the new one.
5. **A lost rotation is stated, not hidden.** If the provider rejects a refresh with `invalid_grant`, the credential
   becomes `NEEDS_REAUTHORIZATION`, is audited and is reported to its owners; a step that needs it fails retryable with
   `OAUTH_REAUTHORIZATION_REQUIRED`, so its retry policy carries it until someone connects the account again.
6. **The authorization-code flow** uses PKCE with S256, a single-use `state` of 32 random bytes stored as a hash with
   the starting principal, the registration and an expiry of 10 minutes, an exact redirect URI on the Studio's origin,
   and the RFC 9207 `iss` check when the provider sends it. The Studio's callback page posts `code` and `state` to the API
   with the person's own credentials; the API refuses a `state` that another principal started.
7. **Client registrations are per tenant** (the tenant's own client at the provider), or offered by the operator for the
   deployment and opted into by a tenant; both keep their secret in the secret authority.

## Considered options

1. **A broker in the control plane, values in the secret authority, access token only to executors (chosen).** The
   long-lived credentials have one holder that runs no tenant code; refresh is single-flight; disclosure follows the
   existing rules.
2. **Each executor refreshes (feature 021's current client-credentials design).** Every executor holds the client
   secret and, for authorization code, the refresh token; concurrent attempts race the rotation. Rejected.
3. **An OAuth plugin in OpenBao** (a community secrets engine that exchanges and refreshes tokens). Moves refresh into
   the secret store, but adds a third-party plugin to its trust base, ties the feature to OpenBao (the sealed vault would
   lose it) and gives the platform no control of egress policy or audit. Rejected.
4. **Store tokens sealed in PostgreSQL with the application key ring.** Simpler, but feature 017 made OpenBao the
   authority for new credentials. Rejected; the sealed vault remains available where a deployment chose it.
5. **Give the refresh token to the executor with the access token** (as some products do). Rejected: a compromised step
   would keep access after the run.

## Consequences

- The control plane makes outbound calls to token endpoints; their latency is on the path of a disclosure only when the
  access token is about to expire (a proactive pass refreshes ahead for credentials in use).
- One more state machine (the credential) with a fence; crash tests are required.
- Feature 021's `oauth2_client_credentials` scheme changes its runner contract: the runner asks the broker for a token
  instead of receiving the client secret. Until design 047's capability is recorded, 021 refuses that scheme.
- Compatibility: additive tables, endpoints and an additive delivery shape; nothing existing changes.

## Evidence

None yet: this is a proposal. The tests named under "Enforced by" are required before acceptance, with real OpenBao and
a local OAuth test server, and real-provider runs recorded as gaps until credentials are supplied.
