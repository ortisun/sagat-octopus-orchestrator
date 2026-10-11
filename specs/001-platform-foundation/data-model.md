# Data Model: Platform foundation and contracts

## Context

Provide a foundation of tenants, namespaces, HTTP/gRPC contracts and a reproducible environment without coupling the domain to infrastructure.

Data ownership follows the bounded contexts of `docs/architecture/overview.md`, with the reference physical schema in `specs/001-platform-foundation/data-model.md` and illustrative SQL in `contracts/schema.sql`.

## Entities and observable state

- **Definition/Version/Artifact:** immutable after publication; tenant and namespace required.
- **Run/Step/Attempt:** stable identifiers, version, status, timestamps, deadlines, parent/correlation.
- **JournalEvent:** append-only seq per Run, event_type, payload_ref, causation/correlation and actor.
- **Inbox/Outbox:** uniqueness for dedupe and delivery; next_attempt_at; last_error without PII.
- **Lease/Subscription/Timer:** fencing generation, validity, deadline and transition status.
- **SagaEntry/Compensation:** state, dependency order, idempotency key, reason and outcome.

## Invariants

- Every tenant-scoped record validates the security context.
- No in-place update of a published version.
- Every operational state change records a sequenced event and information for reconstruction.
- Large payloads go to the blob store with digest and non-sensitive references.

## Identity and tenancy model (feature 001)

Implemented in `crates/domain/src/identity.rs`; the persistence mapping lives in `crates/infra-postgres`.

| Entity | Fields | Uniqueness | Notes |
|---|---|---|---|
| Tenant | `id`, `slug`, `created_at` | `slug` globally | Provisioned by operators or the dev seed, never by a tenant principal |
| Workspace | `id`, `tenant_id`, `slug`, `created_at` | `(tenant_id, slug)` | Groups namespaces |
| Namespace | `id`, `tenant_id`, `workspace_id`, `slug`, `environment`, `created_at` | `(tenant_id, slug)` | `environment` is `dev`, `staging` or `prod`; the same slug may exist in different tenants |
| Membership | `tenant_id`, `subject`, `role`, `created_at` | `(tenant_id, subject)` | `subject` is the OIDC `sub`; one role per subject per tenant |

Identity entities are immutable once created in this feature; there are no state transitions. Renaming and deletion are out of scope.

### Roles and permissions (deny by default)

| Permission | admin | developer | operator | viewer |
|---|:-:|:-:|:-:|:-:|
| `workspace:read`, `namespace:read`, `definition:read` | ✓ | ✓ | ✓ | ✓ |
| `definition:write`, `definition:publish` | ✓ | ✓ | | |
| `workspace:create`, `namespace:create`, `membership:manage`, `audit:read` | ✓ | | | |

### Invariants

- The tenant of any tenant-scoped entity equals the tenant of the principal that created it.
- A namespace's workspace belongs to the same tenant as the namespace. Composite foreign keys `(tenant_id, workspace_id) → workspaces(tenant_id, id)` make the database reject a mismatch even if the application is bypassed.
- Authorization is evaluated before tenant ownership, so a caller without the permission learns nothing about other tenants.
- Error messages never contain identifiers of other tenants.

## Migrations

Additive migrations first; expand/contract for existing schemas; `CONCURRENTLY` indexes where applicable on large databases; idempotent and measured backfill. See the feature plan and ADRs.
