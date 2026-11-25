# Research and registers: OAuth resources and a curated integration hub (design 047)

The design is [docs/designs/047-oauth-integration-hub.md](../../docs/designs/047-oauth-integration-hub.md). This file
holds its registers, append-only. Every product or design choice below was taken under the delegation of Owner
decisions 2026-11-20-6 and 2026-11-24-4, weighing every option for the most architectural and business advantage; each
says "delegate (OD-2026-11-24-4)" and why.

## Sources read

- Features 017 (OpenBao authority, recoverable provisioning in `crates/application/src/managed_secrets.rs`, the KV
  version 2 adapter in `crates/infra-secrets/src/openbao.rs` that writes each revision once), 021 (resource types,
  slots, the integration runner, `contracts/resource-types.md` with its `oauth2_client_credentials` scheme), 028 (the
  signed template index, Requirements 028-039 to 028-042, `data-model.md` section 4), 005 (the shim protocol, Decision 111
  on redaction), 009 (egress), 016 (classification), 026 (channel credentials, ADR-0055 rule 11b).
- Constitution 5.0.0 as approved (Owner decision 2026-11-24-2), "Admitted scope" and "Out of scope".
- The Studio's sign-in (OIDC in the browser, `frontend/apps/studio/src/auth`), which makes the API a bearer API.
- Standards: RFC 6749 (OAuth 2.0), RFC 7636 (PKCE), RFC 7009 (revocation), RFC 7523 (JWT client authentication),
  RFC 8414 (server metadata), RFC 9207 (the `iss` parameter), RFC 9700 (OAuth 2.0 security best current practice),
  RFC 8693, RFC 9449, RFC 8705 and RFC 7591 (out of scope, gaps), RFC 8785 (canonical JSON, feature 028).
- Windmill's documented behaviour: OAuth resources connected from its UI and refreshed by the server, tokens given to
  scripts in the resource object, instance-level OAuth client configuration, an open hub of scripts, flows and resource
  types.

## Decisions

| ID | Decision | Reasoning |
|---|---|---|
| D047-001 | A token broker in the control plane is the only component that talks to OAuth endpoints. delegate (OD-2026-11-24-4) | One holder for long-lived credentials that runs no tenant code and already reaches the secret authority; executors would otherwise each hold refresh tokens and race rotations (ADR-0122) |
| D047-002 | Refresh tokens, client secrets and private keys never leave the broker; executors receive the access token only. delegate (OD-2026-11-24-4) | A compromised step loses access when its token expires; Windmill also refreshes server side, but this makes it a rule with canary tests |
| D047-003 | Values in the secret authority of feature 017, one revision per value written once; metadata and the fence in PostgreSQL. delegate (OD-2026-11-24-4) | Feature 017 made OpenBao the authority for new credentials; immutable revisions match its adapter; the fence must be in the database that decides leases |
| D047-004 | Refresh is single-flight by a fenced lease on the credential row, never with a transaction open during the provider call. delegate (OD-2026-11-24-4) | Providers that rotate and detect reuse revoke the family on a race; Principle V forbids holding a transaction over provider input or output |
| D047-005 | A lost rotation moves the credential to `NEEDS_REAUTHORIZATION`; steps fail retryable with `OAUTH_REAUTHORIZATION_REQUIRED`. delegate (OD-2026-11-24-4) | A durable workflow should survive a person reconnecting later; failing permanently (Windmill's behaviour) loses the run, and waiting silently hides the cause |
| D047-006 | PKCE S256 always, even for confidential clients; `state` bound to the starting principal, single use, 10 minutes; `iss` checked when sent. delegate (OD-2026-11-24-4) | RFC 9700 recommends PKCE for every client and defends code injection and mix-up; binding to the principal defeats a forged callback |
| D047-007 | The callback is a Studio page that posts `code` and `state` to the API with the person's bearer token. delegate (OD-2026-11-24-4) | The platform has no cookie session; a cookie only for this would be a second session model to secure. The bearer post binds the callback to the person with the existing model |
| D047-008 | Client registrations per tenant by default; the operator may offer deployment registrations that tenants opt into. delegate (OD-2026-11-24-4) | Per tenant keeps isolation (a tenant's revocation, rate limits and consent screen are its own); Windmill configures OAuth per instance, which a SaaS operator needs for one-click onboarding, so both are offered and the shared one is opt-in |
| D047-009 | Client authentication: `client_secret_basic`, `client_secret_post` and `private_key_jwt`. delegate (OD-2026-11-24-4) | The first two are what providers support; `private_key_jwt` lets a tenant avoid a shared secret where the provider supports it. Mutual TLS is a gap |
| D047-010 | Scripts and activities may declare resources; delivery is the ordinary declared-secret delivery of Principle V with a new gRPC method and a new host call. delegate (OD-2026-11-24-4) | Windmill parity requires scripts to use connected accounts; reusing the declared-set rules keeps one disclosure model. A separate method keeps resource authorization distinct from secret access |
| D047-011 | Access tokens are refreshed on demand within 120 s of expiry, and ahead of time by a pass for credentials used in the last 24 hours; rotating refresh tokens get a keep-alive refresh. delegate (OD-2026-11-24-4) | On-demand alone puts a provider round trip on the path of a step; refreshing every credential wastes requests; some providers expire refresh tokens unused for months |
| D047-012 | Disconnect revokes at the provider at least once, then retires revisions regardless of the outcome. delegate (OD-2026-11-24-4) | Revocation is an external effect that may fail; keeping the revisions after the person asked to disconnect would be worse than an unknown revocation outcome, which is recorded |
| D047-013 | The hub's items live in `hub/` in this repository, with two named reviewers each, validated by the platform's codecs and signed into the release index (feature 028's format). delegate (OD-2026-11-24-4) | Constitution 5.0.0: signed sources only; review and CI in the repository give the same evidence trail as code |
| D047-014 | Hub resource types specialise `http_api` and `graphql` only, with fixed hosts and request presets evaluated by the existing operations. delegate (OD-2026-11-24-4) | Integrations as data add no code path to the integration runner; richer integrations are script templates. Executable connectors per item would each be code in the runner's trust base |
| D047-015 | Imports create immutable tenant-scoped versions with provenance; adoption of a newer version is an explicit resource revision. delegate (OD-2026-11-24-4) | Principle IV: a resource pins what it uses; a hub update must not change running work |
| D047-016 | A revoked hub item cannot be imported; imported versions get a finding; the operator may block new admissions. delegate (OD-2026-11-24-4) | Blocking automatically could stop a tenant's production for a cosmetic revocation; never telling would hide a dangerous one. The operator decides with the index's reason |
| D047-017 | The first curated set: GitHub, Google, Microsoft Graph, Slack, Atlassian, Salesforce, HubSpot, Notion, Zoom and a generic OAuth 2.0 API template. delegate (OD-2026-11-24-4) | The providers Windmill users connect most often across development, office, chat, CRM and project tools; each is a common OAuth 2.0 authorization-code provider with documented endpoints |
| D047-018 | Tenants may author provider definitions (not hub items), with https endpoints under the egress policy. delegate (OD-2026-11-24-4) | Internal identity providers and niche services must be usable without waiting for a release; this is authored content, not a hub source |
| D047-019 | The feature is switched on by the capability `oauth-broker` in the instance registry. delegate (OD-2026-11-24-4) | Every replica must deliver resources the same way before a version declares one (ADR-0082) |
| D047-020 | Notification channels of feature 026 keep their own credentials in 0.3.0. delegate (OD-2026-11-24-4) | Moving them now would change an approved draft's security model during the release; a later move is Gap 047-004 |

## Gaps

| ID | Gap | State |
|---|---|---|
| G047-001 | Token exchange (RFC 8693) to narrow a token per step is not offered; a step receives the credential's whole granted scope | Open (later release) |
| G047-002 | Sender-constrained tokens (DPoP, RFC 9449; mutual-TLS-bound tokens, RFC 8705) and mutual-TLS client authentication are not offered | Open |
| G047-003 | Dynamic client registration (RFC 7591) is not offered; registrations are made by hand at the provider | Open |
| G047-004 | Notification channels of feature 026 could hold their credentials as OAuth credentials of this design (Slack, Microsoft Teams) | Open (after 0.3.0) |
| G047-005 | The proactive window (5 minutes), the use window (24 hours) and the keep-alive interval per provider are estimates | Open (measure) |
| G047-006 | Runs against the real providers of the curated set need accounts and credentials that only the owner can create; until then the recorded-response tests are the evidence | Open (owner's credentials) |
| G047-007 | The window between a provider's rotation of a refresh token and its write to the secret authority cannot be closed by the platform; a crash there may need a reconnect (CUJ 5 of the design) | Accepted, stated |

## Owner questions

None open. ADR-0122 and ADR-0123 are accepted or rejected under Owner decision 2026-11-24-4 when the design completes
its review.
