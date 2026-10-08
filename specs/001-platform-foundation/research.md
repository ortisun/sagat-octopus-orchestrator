# Research & Decisions: Platform foundation and contracts

## Main decision

Rust workspace with domain/application/infra-postgres/api-http/api-grpc, Axum+tonic+SQLx. Protobuf and JSON schema are canonical contracts. RLS as an optional additional defense, never replacing the tenancy filter in the application layer.

## Rationale

Provide a foundation of tenants, namespaces, HTTP/gRPC contracts and a reproducible environment without coupling the domain to infrastructure. The decision respects the recovery, auditability, security and versioning properties defined in the Constitution.

## Alternatives considered

1. **Generic ad hoc queue-based solution:** rejected as the sole durability mechanism, since message delivery is not equivalent to a transaction over workflow state.
2. **Independent services per feature from day 1:** postponed to avoid operational and consistency complexity without a benchmark.
3. **SDK-specific logic:** rejected for core invariants, to avoid behavior differences between languages.

## Internal references

- `docs/adr/` (architectural decisions)
- `docs/architecture/overview.md`
- `docs/test-strategy.md`
- `specs/001-platform-foundation/contracts/`

## Risks and mitigation

- Complexity/scope risk: deliver P1 user stories separately, running contract tests.
- Semantic ambiguity risk: encode reducers and golden fixtures, do not decide via UI alone.
- Crash/redelivery risk: test harness with real failures at checkpoints.
- Security risk: specific pentest and tenant isolation from the first milestone.

## Questions for spike validation

- Latency and overhead benchmarks on the critical path defined for this feature.
- Detail compliance and retention requirements according to the deployment environment.

## Gaps found while implementing (T001-001)

Recorded on 2026-10-08 after checking FR-001-001..007, the success criteria and the ADRs against the contracts.

| # | Gap | Resolution | Status |
|---|---|---|---|
| G1 | `.specify/memory/constitution.md` is referenced by `README.md`, `AGENTS.md` and `SPECIFICATION-INDEX.md` but is not in the repository. The plan's Constitution Check (`I`–`VIII`) is the only trace of its content. | Pending decision: the maintainers must supply the constitution. It was **not** invented. | Open |
| G2 | `openapi.yaml` had no `health`, `auth context`, `workspace` or `namespace` operations, which FR-001-002 requires. | Extend the OpenAPI contract together with the API implementation. | Resolved |
| G3 | `schema.sql` had no `workspaces` or `memberships` tables and no composite tenant-scoped foreign keys (its own footer says real migrations MUST add them). | Add the tables and composite `(tenant_id, id)` foreign keys in the real migrations and mirror them in `schema.sql`. | Resolved |
| G4 | `receiveSignedWebhook` declared its `202` response with an unquoted comma inside a YAML flow mapping, producing a phantom `event durably admitted` property. | Fixed in `openapi.yaml`; OpenAPI lint now reports 0 errors. | Resolved |
| G5 | The `X-DF-*` header prefix was an abbreviation of the former project name. | Renamed to `X-Octopus-*` together with the OpenAPI extension. | Resolved |
| G6 | OTLP trace export and metrics endpoint are required by NFR-001-OBS, but no collector is part of the local environment yet. | Structured JSON logs and the `X-Trace-Id` / `traceId` correlation (also stored in audit records) are done and tested. OTLP export and metrics stay **pending**; T001-011 stays unchecked until proven. | Open |
| G7 | Access tokens are verified offline against the JWKS; there is no revocation list or token introspection (plan risk: "JWT expired/revoked"). | The exposure window is bounded by `max_token_lifetime` (1 h by default, enforced from `iat`/`exp`). A removed membership takes effect on the very next request because roles are read from the database on every request. Introspection or a `jti` denylist is a pending decision. | Open |
| G8 | Create operations are not idempotent through `Idempotency-Key`, although the OpenAPI declares the header for the other mutating operations. | A retry after an ambiguous response returns `409`; stated in the OpenAPI description and pinned by a fault test. Implement the key store (`command_dedupe` in `schema.sql`) in a follow-up. | Open |
| G9 | Permission-denied attempts (`403`) are not audited; only cross-tenant access attempts are. | Auditing every `403` could flood the trail and be abused to fill it. Decide an aggregation or sampling policy first. | Open |
| G13 | There is no rate limiting, per-tenant fairness or load shedding, and `/v1/ready` (unauthenticated) takes a database connection per call. A tenant that floods the API can exhaust the shared pool for others; a user who knows many foreign identifiers can still produce one denial record per identifier per window. | Pending: a concurrency/rate limit per tenant and per client, a cached readiness result, and aggregation of denial events. The connection cap and header timeout (see D11) bound the damage of the worst abuse in the meantime. | Open |
| G10 | There is no development identity provider in the repository, so a developer who has no OIDC provider cannot obtain a token for the local environment (the test suites use a fake provider that is not shipped). | Pending: a dev-only token issuer (loopback, never in the production image). Belongs with feature 012 (developer experience). | Open |
| G11 | The application connected with the database owner role, so a compromised process could disable the audit triggers or drop the audit table. | **Largely resolved.** The API serves requests as `sagat_app`, which owns nothing and can only read tenants and memberships, create and read workspaces and namespaces, and append to and read the audit trail (migration `runtime_role_grants`, proven by `tests/integration/001-platform-foundation/runtime_role.rs` and by `scripts/smoke-compose.sh`). The audit triggers are `ENABLE ALWAYS`. The owner role is used only by `migrate` and the seed. Still open: row-level security as an additional defence, and shipping the audit trail to an external, immutable store. | Partly open |
| G12 | `docker compose` publishes dev credentials from `.env.example`. | Acceptable for the local environment only; the compose file refuses to start without `POSTGRES_PASSWORD`, binds to `127.0.0.1` and is documented as not for production. | Accepted |
| G14 | Access-token `typ` is not enforced (RFC 9068 `at+jwt`), so an ID token from the same provider whose `aud` matches this API would be accepted. | Low in practice: ID tokens carry the client's audience, not the API's. Pending: an optional `SAGAT_OIDC_REQUIRED_TYP`, off by default because providers disagree on the value. | Open |
| G15 | A client-supplied valid `traceparent` becomes the request's trace id and is stored in audit records, so correlation ids can be chosen (spoofed) by the caller. | By design of W3C trace propagation. Treat the trace id as a correlation hint, never as proof of origin. | Accepted |
| G16 | `cargo audit` reports RUSTSEC-2023-0071 (`rsa` 0.9, Marvin timing attack) through `jsonwebtoken`'s `rust_crypto` backend. | Not applicable: the attack targets RSA *private-key* operations and this service only verifies signatures with public keys. Recorded as an explicit, justified ignore in `.cargo/audit.toml`; the CI job `dependencies` fails on any other advisory or warning. Revisit when `rsa` ships a fix or the JWT backend changes. | Accepted |

## Decisions taken in this feature

- **D1 — Namespace uniqueness is per tenant, not per workspace.** The OpenAPI header and `schema.sql` (`UNIQUE(tenant_id, slug)`) address a namespace by name alone, so the name must resolve unambiguously inside a tenant. A namespace still belongs to exactly one workspace.
- **D2 — Built-in roles only.** `admin`, `developer`, `operator`, `viewer`, with a fixed permission table in the domain. Custom roles and advanced RBAC belong to feature 009. The permission table is deny-by-default.
- **D3 — One membership per subject per tenant.** A subject holds a single role in a tenant, which keeps authorization deterministic. A subject that must act in two tenants has two memberships, resolved by the tenant claim of the token.
- **D4 — The tenant is never an input.** Every tenant-scoped constructor takes an authenticated `Principal` and inherits its tenant; the domain exposes no constructor that accepts a client-supplied tenant id. A cross-tenant reference is reported to callers as "not found" so tenant existence is not disclosed.
- **D5 — Tenant creation is not a tenant API.** Tenants are provisioned by operators or the development seed (`Tenant::provision`, `Membership::bootstrap_admin`).
- **D6 — Slugs are DNS-label-like** (3–63 characters, `[a-z0-9-]`, start with a letter, end with a letter or digit, no `--`) so they are safe in hostnames, paths and subjects.
- **D7 — Fail closed with the right signal.** When the signing keys cannot be obtained (provider down and cached keys older than `jwks_ttl + jwks_max_stale`) or the database is unavailable, the API answers a retryable `503` with `Retry-After`, never `401` (which would make clients discard valid credentials) and never access.
- **D8 — One answer for every authentication failure.** Unknown tenant, no membership, bad signature, expired, wrong audience and malformed tokens all produce the same `401` body, so a caller cannot probe which tenants or subjects exist.
- **D9 — Cross-tenant access is a `404` plus an audit record.** The record is stored in the *actor's* tenant and carries only the identifier the caller sent and the request's trace id. It is written best effort and **off the response path** (so response time does not reveal that the identifier exists elsewhere), identical events from the same actor are collapsed within a 5-minute window (atomically, with a per-event lock), and at most 64 writes may be pending (further events are dropped and logged). The denial never depends on the audit write.
- **D10 — The JWKS location is configuration, never token content.** `jku`, `x5u` and embedded `jwk` headers are ignored, the fetch is HTTPS-only (relaxable for local development), follows no redirects, is size- and time-bounded, and refreshes are rate limited.
- **D11 — Connection-level protection is part of the server.** `axum::serve` gives hyper no timer, which silently disables its header-read timeout. The server therefore runs its own HTTP/1 loop with a header-read timeout (also reclaiming idle keep-alive connections), a connection cap that applies back-pressure instead of exhausting descriptors, and a bounded shutdown grace.
- **D12 — Expired signing keys are served while refreshing.** Keys past `jwks_ttl` (and inside `jwks_max_stale`) keep verifying while a background refresh runs, so a slow provider never stalls the API. A key the provider withdraws therefore keeps working until `jwks_ttl` plus one refresh. A key set without a single usable key is treated as a provider incident and never replaces a good cache.
- **D13 — The runtime role is provisioned, the grants are migrated.** The role `sagat_app` is created by whoever provisions the database (it carries a password, so it does not belong in a migration; the compose stack does it in `postgres-init/`). The migration grants it its fixed privilege set if the role exists when it runs, so create the role *before* the first migration.
- **D14 — Privileged writes outside the tenant API are audited too.** Provisioning a tenant and granting its first administrator (and any membership added by an operator) write `tenant.provisioned` and `membership.granted` records in the same transaction as the change. The actor is the non-authenticatable subject `system:provisioning`; the operator's own identity is not captured yet (the control plane CLI has none).
