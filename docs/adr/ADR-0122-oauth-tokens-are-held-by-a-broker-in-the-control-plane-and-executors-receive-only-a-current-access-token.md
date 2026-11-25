# ADR-0122: OAuth tokens are held by a broker in the control plane, with values in the secret authority, and executors receive only a current access token

**Status:** Proposed
**Decided by:** agent proposal (design 047, 2026-11-24, revised after review round 1), to be accepted or rejected under Owner decision 2026-11-24-4 when design 047 completes its review
**Refines / supersedes:** refines ADR-0055 (feature 021's `oauth` scheme names a credential of this design; clause 11(e) places the token requests here), ADR-0035, ADR-0074 (operator registrations and offers wait for it) and ADR-0082 (the capability `oauth-broker-v1`); supersedes none
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
   component that talks to a token, authorization or revocation endpoint. It runs no tenant code. Its calls use the
   guarded egress client under the egress policy of the credential's namespace; redirects are not followed; at most 4
   requests are in flight and 10 per second per registration.
2. **Values live in an OAuth vault:** an OpenBao mount of its own, under a policy only the broker's token holds, reached
   through an application port only the broker uses, with a reference type ordinary secret delivery cannot build. Each
   revision is written once. PostgreSQL holds only metadata: state, granted scopes, expiry, revision rows, the fence.
3. **The refresh token, the client secret and the client's private key never leave the broker.** An executor (feature
   021's integration runner, a sandbox runner for a script, an SDK worker for an activity) receives, for a resource its
   task definition declares, the resource's non-secret settings and a deliverable access token, through delivery that
   follows Principle V's rules for a declared set.
4. **Every writer of a credential compares and increases its fence.** A refresh acquires a lease with a conditional
   update that also compares the access revision the caller saw, calls the provider within 15 s of acquiring with a 10 s
   deadline and no transaction open, writes `PENDING` revision rows and then the vault, and activates only if the fence,
   the tenant and the state `ACTIVE` still match. A later holder adopts a complete pending refresh revision before it
   calls the provider. Expiry is counted from the acquire's database time.
5. **A lost rotation is stated, not hidden.** A refresh refused with `invalid_grant` moves the credential to
   `NEEDS_REAUTHORIZATION`, audited and reported; a step that needs it fails retryable with
   `RESOURCE_REAUTHORIZATION_REQUIRED` until someone reconnects the same account.
6. **The authorization-code flow** uses PKCE with S256, a single-use `state` bound to the person and the registration,
   one redirect URI per registration (`/studio/oauth/callback/<registration>`), and the RFC 9207 `iss` check, required
   when the provider definition says the provider sends it. The callback's check and mark are one statement. A reconnect
   that yields another account is refused.
7. **Registrations are per tenant**, or owned by the platform operator for a published hub provider and offered to
   tenants that opt in (after ADR-0074 is accepted); both keep their secret in the OAuth vault.
8. **The delivery of a declared resource is a declared set of Principle V** whose wording design 047 proposes
   (Technical Plan 12); this decision is not accepted before that wording is in the constitution.
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
6. **A separate token-broker service.** It needs the same database, secret authority and egress; it adds a process and a
   credential for little isolation. Rejected for 0.3.0; the ports keep the split possible after measurement.

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
