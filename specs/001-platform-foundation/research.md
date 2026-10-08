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
| G2 | `openapi.yaml` had no `health`, `auth context`, `workspace` or `namespace` operations, which FR-001-002 requires. | Extend the OpenAPI contract together with the API implementation. | Resolved on the HTTP API branch |
| G3 | `schema.sql` had no `workspaces` or `memberships` tables and no composite tenant-scoped foreign keys (its own footer says real migrations MUST add them). | Add the tables and composite `(tenant_id, id)` foreign keys in the real migrations and mirror them in `schema.sql`. | Resolved on the persistence branch |
| G4 | `receiveSignedWebhook` declared its `202` response with an unquoted comma inside a YAML flow mapping, producing a phantom `event durably admitted` property. | Fixed in `openapi.yaml`; OpenAPI lint now reports 0 errors. | Resolved |
| G5 | The `X-DF-*` header prefix was an abbreviation of the former project name. | Rename together with the OpenAPI extension. | Resolved on the HTTP API branch |
| G6 | OTLP trace export and metrics endpoint are required by NFR-001-OBS, but no collector is part of the local environment yet. | Structured logs with `trace_id` first; OTLP export and metrics stay **pending** (T001-011 stays unchecked until proven). | Open |

## Decisions taken in this feature

- **D1 — Namespace uniqueness is per tenant, not per workspace.** The OpenAPI header and `schema.sql` (`UNIQUE(tenant_id, slug)`) address a namespace by name alone, so the name must resolve unambiguously inside a tenant. A namespace still belongs to exactly one workspace.
- **D2 — Built-in roles only.** `admin`, `developer`, `operator`, `viewer`, with a fixed permission table in the domain. Custom roles and advanced RBAC belong to feature 009. The permission table is deny-by-default.
- **D3 — One membership per subject per tenant.** A subject holds a single role in a tenant, which keeps authorization deterministic. A subject that must act in two tenants has two memberships, resolved by the tenant claim of the token.
- **D4 — The tenant is never an input.** Every tenant-scoped constructor takes an authenticated `Principal` and inherits its tenant; the domain exposes no constructor that accepts a client-supplied tenant id. A cross-tenant reference is reported to callers as "not found" so tenant existence is not disclosed.
- **D5 — Tenant creation is not a tenant API.** Tenants are provisioned by operators or the development seed (`Tenant::provision`, `Membership::bootstrap_admin`).
- **D6 — Slugs are DNS-label-like** (3–63 characters, `[a-z0-9-]`, start with a letter, end with a letter or digit, no `--`) so they are safe in hostnames, paths and subjects.
