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

## Decisions taken in this feature

- **D1 — Namespace uniqueness is per tenant, not per workspace.** The OpenAPI header and `schema.sql` (`UNIQUE(tenant_id, slug)`) address a namespace by name alone, so the name must resolve unambiguously inside a tenant. A namespace still belongs to exactly one workspace.
- **D2 — Built-in roles only.** `admin`, `developer`, `operator`, `viewer`, with a fixed permission table in the domain. Custom roles and advanced RBAC belong to feature 009. The permission table is deny-by-default.
- **D3 — One membership per subject per tenant.** A subject holds a single role in a tenant, which keeps authorization deterministic. A subject that must act in two tenants has two memberships, resolved by the tenant claim of the token.
- **D4 — The tenant is never an input.** Every tenant-scoped constructor takes an authenticated `Principal` and inherits its tenant; the domain exposes no constructor that accepts a client-supplied tenant id. A cross-tenant reference is reported to callers as "not found" so tenant existence is not disclosed.
- **D5 — Tenant creation is not a tenant API.** Tenants are provisioned by operators or the development seed (`Tenant::provision`, `Membership::bootstrap_admin`).
- **D6 — Slugs are DNS-label-like** (3–63 characters, `[a-z0-9-]`, start with a letter, end with a letter or digit, no `--`) so they are safe in hostnames, paths and subjects.
- **D7 — Fail closed with the right signal.** When the signing keys cannot be obtained (provider down and cached keys older than `jwks_ttl + jwks_max_stale`) or the database is unavailable, the API answers a retryable `503` with `Retry-After`, never `401` (which would make clients discard valid credentials) and never access.
- **D8 — One answer for every authentication failure.** Unknown tenant, no membership, bad signature, expired, wrong audience and malformed tokens all produce the same `401` body, so a caller cannot probe which tenants or subjects exist.
- **D9 — Cross-tenant access is a `404` plus an audit record.** The record is stored in the *actor's* tenant, carries only the identifier the caller sent and the request's trace id, and is written best-effort: the denial never depends on the audit write.
- **D10 — The JWKS location is configuration, never token content.** `jku`, `x5u` and embedded `jwk` headers are ignored, the fetch is HTTPS-only (relaxable for local development), follows no redirects, is size- and time-bounded, and refreshes are rate limited.
