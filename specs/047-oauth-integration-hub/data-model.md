# Data model (design 047)

Additive migrations after `20270119000000` (timestamps allocated by the coordinating session). Every table has
`tenant_id` first in its key (except the deployment-scope rows, where it is null and `scope = 'DEPLOYMENT'`), foreign
keys to its tenant and namespace, grants for `sagat_app` as listed, and every variable-size column classified in
`crates/infra-postgres/storage-classification.json`. New checks on existing tables are added `NOT VALID` and validated by
a second migration. Each down migration refuses while a row it would drop exists.

## Tables

| Table | Key | Columns | Notes; grants |
|---|---|---|---|
| `oauth_providers` | `(scope, tenant_id, id)` | `slug`, `created_by`, `created_at` | `scope` `TENANT` or `DEPLOYMENT` (hub providers only); SELECT, INSERT |
| `oauth_provider_versions` | `(provider_id, version)` | `state` (`DRAFT`, `PUBLISHED`, `BLOCKED`), `source` (`AUTHORED`, `HUB`), `provenance jsonb` (index, item, version, digest, signer; at most 4 KiB), `definition jsonb` (at most 32 KiB), `published_by`, `published_at` | immutable once `PUBLISHED` except `BLOCKED`; SELECT, INSERT, UPDATE (`state` only) |
| `oauth_client_registrations` | `(scope, tenant_id, id)` | `provider_version`, `client_id` (at most 512 bytes), `method`, `secret_revision` (vault reference), `redirect_path`, `state` (`ACTIVE`, `BROKEN`, `RETIRED`), `created_by`, `created_at` | 50 per tenant; SELECT, INSERT, UPDATE |
| `oauth_client_offers` | `(registration_id, tenant_id)` | `state` (`OFFERED`, `OPTED_IN`, `OPTED_OUT`, `WITHDRAWN`), `changed_by`, `changed_at`, `refresh_stops_at` | SELECT, INSERT, UPDATE |
| `oauth_credentials` | `(tenant_id, id)` | `namespace_id`, `registration_id`, `grant` (`AUTHORIZATION_CODE`, `CLIENT_CREDENTIALS`), `state` (`PENDING`, `ACTIVE`, `NEEDS_REAUTHORIZATION`, `DISCONNECTING`, `DISCONNECTED`), `fence bigint`, `lease_until`, `granted_scopes text[]` (at most 200), `subject_hmac bytea`, `subject_label text` (classified `pii`, at most 256), `bound_host` (at most 255), `access_revision`, `access_expires_at`, `refresh_revision`, `refresh_used_at`, `revocation_outcome`, `last_error_code`, `last_used_at`, `created_by`, `created_at` | 200 per namespace; the unique row the fence protects; SELECT, INSERT, UPDATE |
| `oauth_token_revisions` | `(tenant_id, credential_id, revision)` | `kind` (`ACCESS`, `REFRESH`), `fence`, `state` (`PENDING`, `ACTIVE`, `RETIRED`, `DESTROYED`), `written_at`, `retired_at` | at most 3 not `DESTROYED` per credential and kind; SELECT, INSERT, UPDATE |
| `oauth_authorization_requests` | `(tenant_id, id)` | `state_hash bytea` (unique), `sealed_verifier bytea`, `principal`, `registration_id`, `credential_id`, `scopes text[]`, `return_path` (a path inside the Studio, at most 512), `expires_at`, `used_at` | 10 open per person; deleted after `expires_at` by the hourly pass; SELECT, INSERT, UPDATE, DELETE |
| `oauth_delivery_asks` | `(tenant_id, attempt_id, resource)` | `asks smallint`, `last_ask_at` | deleted with the attempt's retention; SELECT, INSERT, UPDATE |
| `oauth_passes` | `(name)` | `holder`, `lease_until` | the lease rows of the keep-alive, request-cleanup and revocation passes |
| `hub_resource_type_versions` | `(tenant_id, type_key, version)` | `state` (`DRAFT`, `PUBLISHED`, `BLOCKED`), `definition jsonb` (at most 256 KiB), `provenance jsonb`, `published_by`, `published_at` | the `hub` source of feature 021's catalogue (Decision 021-004); `type_key` under `hub.`; SELECT, INSERT, UPDATE (`state` only) |
| `hub_item_blocks` | `(item_digest)` | `reason_class` (`security`, `other`), `source` (`INDEX`, `FEED`, `OPERATOR`), `blocked_by`, `created_at`, `lifted_by`, `lifted_at` | deployment-wide; SELECT for `sagat_app`, writes by the operator surface |
| `hub_item_findings` | `(tenant_id, item_digest)` | `kind`, `reason`, `created_at` | SELECT, INSERT |

## Changes to feature 028's tables

| Table | Change |
|---|---|
| `template_entries` | `kind` check adds `OAUTH_PROVIDER` and `INTEGRATION`; `revoked_class` and `revoked_reason` nullable columns |
| `template_imports` | `namespace_id` nullable (tenant-wide imports of providers and types); `target_kind` check adds `OAUTH_PROVIDER` and `RESOURCE_TYPE`; `request_digest` (the body's SHA-256, for `IDEMPOTENCY_KEY_REUSED`) |

## Lock order

Credential writes: the credential row (fence compared), then its revision rows. A disconnect takes the credential row
then the resource revision of feature 021 in the deletion's order (feature 021's resource row first when it deletes).
Import: the import record (unique key), then the drafts. Publication: the version row. Passes take their `oauth_passes`
row, then credential rows one at a time in identifier order.
