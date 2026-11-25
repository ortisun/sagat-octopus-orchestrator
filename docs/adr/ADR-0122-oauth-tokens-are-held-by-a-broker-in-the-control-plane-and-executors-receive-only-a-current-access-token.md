# ADR-0122: OAuth tokens are held by a broker in the control plane, with values in a vault of their own, and executors receive only a current access token

**Status:** Proposed
**Decided by:** agent proposal (design 047, 2026-11-24, revised after review rounds 1 and 2), to be accepted or rejected under Owner decision 2026-11-24-4 when design 047 completes its review; not acceptable before Principle V carries the declared set of design 047's Technical Plan 12
**Refines / supersedes:** refines ADR-0055 (feature 021's `oauth` scheme names a credential of this design; clause 11(e) is widened to admit RFC 7009 revocation, OIDC userinfo calls and the delivery method `ResolveDeclaredResources`), ADR-0035, ADR-0074 (deployment registrations and offers wait for it) and ADR-0082 (the capability `oauth-broker-v1`); supersedes none
**Enforced by:** planned tests and checks of design 047 (Verification Plan): `two_control_planes_refresh_once`, `a_holder_paused_past_its_lease_activates_nothing`, `a_disconnect_between_answer_and_store_revokes_the_new_token`, `a_pending_revision_is_adopted_after_a_crash`, `no_transaction_is_open_during_a_token_request` (a barrier in the OAuth test server and `pg_stat_activity`), `canaries_never_leave_the_broker`, `ordinary_token_is_refused_on_the_oauth_mount`, the callback refusal tests, `a_credential_cannot_be_bound_to_a_foreign_host`, `one_tenant_cannot_starve_a_shared_registration`; and two rules of `scripts/check-architecture.py` with negative tests: the `"grant_type"` string literal only in `crates/infra-oauth` (one recorded exemption), the `OAuthVault` port only in the broker, its implementation and the composition root. Until they exist: review only.

## Context

Windmill connects a resource to a provider account by OAuth and refreshes its token on the server; the script receives
the token in its resource. Feature 021 offers an `oauth` scheme whose slot names a credential of design 047 and is refused
until a broker exists; it leaves authorization-code flows, resources for scripts and activities, and a hub to design 047.

The forces:

1. A refresh token is a long-lived credential to a person's account. If an executor receives it, a compromised step keeps
   access after the run.
2. Many providers rotate the refresh token on each use and revoke the whole family when an old one is reused. Two
   concurrent refreshes of one credential, or a refresh racing a disconnect, lose it.
3. A crash after the provider answered and before the answer is stored can lose the rotated refresh token.
4. The authorization-code callback is a browser redirect; it must be bound to the person and the registration that started
   it (forgery, mix-up of providers, code injection).
5. Principle V: secret values never reach workflow state, the journal, logs or responses, and an executor receives only
   what its immutable task definition declares, all or nothing, with a value-free audit before disclosure.

## Decision

1. **A token broker in the control plane** (`crates/application/src/oauth`, adapter `crates/infra-oauth`) is the only
   component that talks to a token, authorization, revocation or userinfo endpoint. It runs no tenant code. Its calls use
   the guarded egress client under the egress policy of the credential's namespace; redirects are not followed; request
   windows in PostgreSQL bound each registration and each tenant's share of a deployment registration.
2. **Values live in an OAuth vault:** an OpenBao mount of its own, under a token and a policy only the broker holds,
   reached through an application port only the broker uses, with a reference type ordinary delivery cannot build. Each
   revision is written once. PostgreSQL holds the metadata: state, scopes, expiry, revision rows, fence, grant epoch.
3. **The refresh token, the client secret and the private key never leave the broker.** An executor receives, for a
   resource its task definition declares, the resource's settings, its slots' secrets and a deliverable access token,
   after feature 021's resolution checks (publisher's and initiator's use and effect, slot secret access), bound like
   ordinary delivery for workers and sandbox runners and like managed delivery for the integration runner.
4. **Every writer of a credential increases its fence with a state predicate; only a refresh's activation compares the
   fence it acquired.** A refresh acquires a lease that also compares the access revision the caller saw, calls the
   provider within 15 s of acquiring (on a clock that counts during suspension) with a 10 s deadline and no transaction
   open, stores whatever the provider answered before anything else, and activates only if the fence, the tenant and the
   state `ACTIVE` still match. A later holder adopts a stored pending refresh revision of the same grant epoch before it
   calls the provider. A holder whose activation loses to a disconnect revokes the new refresh token itself.
5. **A lost rotation is stated, not hidden.** A refresh refused with `invalid_grant` moves the credential to
   `NEEDS_REAUTHORIZATION`, audited and reported; a step that needs it fails retryable with
   `RESOURCE_REAUTHORIZATION_REQUIRED` until the same account is connected again. The window between a provider's answer
   and its store, and a timed-out call, remain stated residual risks.
6. **The authorization-code flow** uses PKCE with S256, a single-use `state` bound to the tenant, the person and the
   registration, one redirect URI per registration, and the RFC 9207 `iss` check, required when the provider sends it; the
   callback's check and mark are one statement; a reconnect that changes the account is refused.
7. **Registrations are per tenant**, or owned by the platform operator for a published hub provider, authorization code
   only, and offered to tenants that opt in, after ADR-0074 is accepted.
8. **The delivery of a declared resource is a declared set of Principle V** whose wording design 047 proposes; this
   decision is not accepted before that wording is in the constitution.

## Considered options

1. **A broker in the control plane, a vault of its own, the access token only to executors (chosen).** The long-lived
   credentials have one holder that runs no tenant code; refresh is single-flight; disclosure follows the declared-set
   rules.
2. **Each executor refreshes.** Every executor holds the client secret and the refresh token, and concurrent attempts race
   the rotation. Rejected.
3. **An OAuth plugin in OpenBao.** A third-party plugin in the secret store's trust base, the feature tied to OpenBao, no
   control of egress or audit. Rejected.
4. **Tokens sealed in PostgreSQL with the application key ring.** Feature 017 made OpenBao the authority for new
   credentials. Rejected; the sealed vault stays where a deployment chose it.
5. **The refresh token given to the executor with the access token.** A compromised step would keep access. Rejected.
6. **A separate token-broker service.** The same database, egress and audit needs; a process and a credential more for
   little isolation. Rejected for 0.3.0; the ports keep the split possible after measurement.
7. **A broker-side proxy that injects the token into each request.** Strongest custody, but every script request would
   pass through the control plane. Later (design 047, Gap 047-008).

## Consequences

- The control plane makes outbound calls to token endpoints; their latency is on a delivery's path only when the token is
  due.
- A credential state machine with a fence, a grant epoch and pending revisions; crash and race tests are required.
- Rollback once declarations exist requires removing the recorded requirement first (ADR-0082); old control planes then
  answer `UNIMPLEMENTED`, which executors treat as retryable.
- Compatibility: additive tables, endpoints, a gRPC method under a new protocol minor and an IR field.

## Evidence

None yet: this is a proposal. The tests and checks named under "Enforced by" are required before acceptance, with real
OpenBao and a local OAuth test server, and real-provider runs recorded as gaps until credentials are supplied.
