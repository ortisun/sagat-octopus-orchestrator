# Data Model: Security, isolation and multi-tenant governance

## Context

Protect tenant workloads, secrets and untrusted code execution before offering inline scripts.

Data ownership follows the bounded contexts of `docs/architecture/overview.md`, with the reference physical schema in `specs/001-platform-foundation/data-model.md` and `specs/002-durable-execution/data-model.md`. The physical schema of this feature is the six additive migrations `20261010000000_quotas`, `20261010000100_worker_bindings`, `20261010000200_secrets`, `20261010000300_egress_policies`, `20261010000400_quota_ceilings` and `20261010000500_steps_leased_by_index` in `crates/infra-postgres/migrations/`, each with a tested `down`.

## Entities and observable state

| Entity | Table | Notes |
|---|---|---|
| Quota setting | `quota_limits` | `(tenant, namespace or NULL, resource) -> max_value` (resource-specific ranges in `sagat_domain::quota`). `NULL` namespace is the tenant-wide setting; a resource counted per tenant (`tenant_active_runs`, `worker_sessions`) has no namespace level (check constraint). A missing row means the provisional platform default (`sagat_domain::quota`). Composite foreign key to `namespaces`. |
| Quota ceiling | `quota_ceilings` | `(tenant, resource) -> max_value` (resource-specific ranges in `sagat_domain::quota`): the most a tenant's administrators may set for that resource, and the most that can be in force. No row means the platform default (`sagat_domain::quota`). Written only by the operator's command (`control-plane quota-ceiling`); no API route writes it. |
| Active-run index | `runs_active_idx` | Partial index `(tenant_id, namespace_id) WHERE finished_at IS NULL`: what the exact count of a start reads. |
| Worker binding | `worker_bindings` | One row per `(tenant, namespace, subject, pattern)`; a pattern is `*`, `prefix.*` or an exact activity name (check constraint mirrors the grammar of `sagat_domain::binding`). No row, no registration. |
| Secret | `secrets` | `(tenant, namespace, name)` unique; `current_version`; no value. Target of the composite foreign keys below. |
| Secret version | `secret_versions` | `id` (part of the associated data), `version`, `key_id`, 12-byte `nonce`, `ciphertext` (value plus 16-byte tag, at most 16 400 bytes). The last five versions are kept. Deleted with its secret. |
| Secret access | `secret_access` | `(secret, subject)`: who, besides administrators, may reference the secret from a workflow they publish. Deleted with its secret. |
| Secret pool | `secret_pools` | `(secret, pool)`: a worker pool allowed to receive the secret (G48). No row means no restriction. At most 16 per secret, name `^[A-Za-z0-9._:+-]{1,64}$`. Deleted with its secret. |
| Worker pool grant | `worker_binding_pools` | `(tenant, namespace, subject, pool)`: the pools an administrator granted to a worker identity in a namespace (G48). Replaced with the binding call that names `pools`, removed with the binding. Composite key on `(tenant, namespace)`. |
| Egress policy | `egress_policies` | At most one row per namespace: `rules` (canonical text), `ports`, `allow_http`. No row means everything is denied. |
| Live-lease index | `steps_leased_by_idx` | Partial index `(tenant_id, leased_by) WHERE status IN ('LEASED', 'RUNNING')`: what releasing the attempts of closed sessions reads. |
| Audit | `audit_records` (existing) | New actions: `quota.changed`, `secret.stored`, `secret.deleted`, `secret.access_granted`, `secret.access_revoked`, `secret.resolved`, `secret.denied`, `secret.pools_changed`, `secret.delivery_refused`, `egress.policy_changed`, `egress.denied`, `worker.bound`, `worker.unbound`, `worker.denied`; new resource types `secret`, `quota`, `egress_policy`, `worker_binding`. None carries a value. |

The workflow IR gains an optional `secrets` array on `activity` nodes (names only, at most 16, unique, `^[a-z][a-z0-9._-]{0,63}$`); it is part of the canonical document and therefore of the version's digest.

## Invariants

- Every row of every table above carries `tenant_id`, and every foreign key into `namespaces` or `secrets` is the composite `(tenant_id, id)`: the database refuses a quota, binding, secret, version, access entry or policy that points into another tenant.
- A quota is judged by an exact count taken under `pg_advisory_xact_lock` on the counted scope, in the transaction that inserts the thing counted. Scopes lock namespace before tenant, so they cannot deadlock with each other, and the child-run tree lock of feature 002 is always taken before them. Both locks are held until the transaction commits, so starts in different namespaces of one tenant wait for each other for the length of an insert (research G46).
- The limit in force is `min(setting, ceiling)`: a setting above the ceiling (because the operator lowered it afterwards) does not apply, and a tenant cannot store a setting above it. The ceiling is the platform default unless `quota_ceilings` has a row for the tenant and resource.
- A secret's ciphertext opens only with the associated data `(tenant, namespace, name, version row, key id)` it was sealed with; a copy in another row is noise. The value is never stored anywhere else and never leaves the process except in the answer to `ResolveSecrets` and in the sealed row.
- A worker session exists only while its identity's bindings cover the activities it advertises; changing a binding deletes the sessions it no longer covers in the same transaction and releases the attempts they hold. Storing a session takes the same lock and reads the binding on its own connection, so a session is either stored before a change (which then closes it if it is no longer covered) or judged against the new binding.
- A secret is resolved only to the holder of the current, unexpired lease of a step whose pinned definition declares it, for an identity still bound to that activity, and only while the publisher of that version still holds `secret:use` and is on the secret's list (or is an administrator). The resolution and its audit record commit together.
- A transaction that holds a connection never asks the pool for another.

## Lock order

Run row, then the child-run tree (feature 002), then the quota scope of the namespace, then the quota scope of the tenant. The secrets of a namespace and the creators of one secret share the namespace's `secrets` scope lock, taken before the secret is looked up. The binding lock of an identity in a namespace (`binding:{tenant}:{namespace}:{subject}`) comes before the session-quota lock of the tenant, in `replace_binding` and in `create_session` alike.

## Migrations

Additive, with a tested `down` (checked by the rollback test of feature 001: no table, function or trigger is left behind). The runtime role `sagat_app` receives only what it needs: `SELECT, INSERT, UPDATE, DELETE` on `quota_limits`, `quota_ceilings`, `worker_bindings`, `egress_policies`, `secrets` and `secret_versions`, and `SELECT, INSERT, DELETE` on `secret_access`. Constraints that widen an earlier table (`quota_limits_resource_valid` gains `secrets`) are replaced, never edited, and restored by the `down`.


## Durable quota accounting (ADR-0027)

| Entity | Durable representation | Invariant |
|---|---|---|
| Retained usage | `tenant_retained_usage`: tenant, content bytes, reserved content bytes, log bytes | Exact nonnegative committed byte totals; catalog triggers fold transaction-local deltas into the tenant row. Runtime cannot directly mutate totals. |
| Transaction delta | `quota_transaction_deltas`: transaction, tenant, signed deltas | Owned by security-definer accounting functions; checked at transaction completion, rolled back with the owning mutation. |
| Admission bucket | `quota_rate_buckets`: tenant, namespace, resource/source kind and stored source ID, microtokens, DB update time | One request costs 1,000,000 microtokens; capacity is two seconds of rate. Database clock is read after the bucket lock. |
| Blob reservation | `blob_objects.reservation_bytes`, `uploaded_at`, size/status | Unknown uploads keep reservation; known uploads charge actual measured bytes, including zero. Committing a reference never charges a second time. |
| Accounting switches | `quota_accounting_control` | Owner-only rollout control, never tenant-settable. |
| Storage classification | `infra-postgres/storage-classification.json` | Every variable-size column classified as charged content, charged logs, or independently bounded protected operational metadata; schema coverage is tested against PostgreSQL. |

Byte resources allow 0..2^50; API/event rates 0..1,000,000 per second; event retention is
zero or 60..2,592,000 seconds; script retention 0..365 days. Settings and operator ceilings
share the same fences as admission. Retention and rate resources omit misleading live-object
`used` counts; byte resources report authoritative counters.

Build versions add `failure_code` (closed enum) and `build_log_truncated`; event and connector
last-error fields have explicit truncation flags. Script trace IDs are null or nonzero lowercase
32-hex values. Script lease journal details capture a versioned retention policy plus forward,
compensation and reconciliation retention pins when applicable. These are admission evidence;
stored record expiry is not rewritten after a policy decrease.

The coordinated upgrade, legacy trace preflight and guarded rollback are documented in
[the governance runbook](../../docs/runbooks/security-governance.md#upgrade-and-rollback).
