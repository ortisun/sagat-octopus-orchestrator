# Data model (design 047)

Additive migrations after `20270119000000` (timestamps allocated by the coordinating session). Tenant tables have
`tenant_id` first in every key and composite foreign keys that carry it; deployment-scope rows live in tables of their own
with no tenant column. Grants for `sagat_app` are listed; every variable-size column is classified in
`crates/infra-postgres/storage-classification.json`. New checks on existing tables are added `NOT VALID` and validated by a
second migration. Each down migration refuses while a row it would drop exists.

## Tenant tables

| Table | Key | Columns | Notes; grants |
|---|---|---|---|
| `oauth_providers` | `(tenant_id, id)` | `slug`, `created_by`, `created_at` | SELECT, INSERT |
| `oauth_provider_versions` | `(tenant_id, provider_id, version)` | `state` (`DRAFT`, `PUBLISHED`, `BLOCKED`), `source` (`AUTHORED`, `HUB`), `provenance jsonb` (at most 4 KiB), `definition jsonb` (at most 32 KiB, including `apiHosts`), `published_by`, `published_at` | immutable once `PUBLISHED` except `BLOCKED`; SELECT, INSERT, UPDATE (`state`) |
| `oauth_client_registrations` | `(tenant_id, id)` | `provider_id`, `provider_version`, `client_id` (at most 512), `method`, `secret_revision`, `redirect_path`, `directory` (the bound Microsoft directory or other `hostBindings` value, at most 255), `state` (`ACTIVE`, `SUSPECT` after the first client error within 10 minutes, `BROKEN` after the third, `RETIRED`), `invalid_client_count`, `invalid_client_since`, `created_by`, `created_at` | composite foreign key to the provider version; 50 per tenant; SELECT, INSERT, UPDATE |
| `oauth_credentials` | `(tenant_id, id)` | `namespace_id`, `tenant_registration_id` (foreign key with `tenant_id` to `oauth_client_registrations`) or `deployment_registration_id` (foreign key `(deployment_registration_id, tenant_id)` to `oauth_deployment_offers`), exactly one set (a check), `grant_type` (`AUTHORIZATION_CODE`, `CLIENT_CREDENTIALS`; not `grant`, a reserved word), `state` (`PENDING`, `ACTIVE`, `NEEDS_REAUTHORIZATION`, `DISCONNECTING`, `DISCONNECTED`), `fence bigint`, `grant_epoch bigint`, `lease_until`, `granted_scopes text[]` (at most 200), `subject_hmac bytea`, `subject_label text` (classified `pii`, at most 256), `bound_host` (at most 255), `access_revision`, `access_expires_at`, `access_stale boolean`, `force_refresh_revision` (the access revision a `forceRefresh` was last honoured for, so it is honoured once per revision), `refresh_revision`, `refresh_used_at`, `revocation_outcome`, `last_error_code`, `last_used_at`, `created_by`, `created_at` | `CHECK (NOT (deployment_registration_id IS NOT NULL AND grant_type = 'CLIENT_CREDENTIALS'))`; 200 per namespace; SELECT, INSERT, UPDATE |
| `oauth_token_revisions` | `(tenant_id, credential_id, revision)` | `kind` (`ACCESS`, `REFRESH`), `origin` (`EXCHANGE`, `REFRESH`), `fence`, `grant_epoch`, `expires_at`, `scopes text[]`, `value_hmac bytea` (an HMAC of the token value under the tenant, compared before an abandoned revision is revoked), `revocation_attempts smallint`, `revocation_next_at`, `revocation_outcome`, `state` (`PENDING`, `ACTIVE`, `RETIRED`, `ABANDONED`, `DESTROYED`), `vault_written_at`, `created_at`, `retired_at` | at most 8 not `DESTROYED` per credential; SELECT, INSERT, UPDATE |
| `oauth_authorization_requests` | `(tenant_id, id)` | `state_hash bytea` (unique per tenant), `sealed_verifier bytea`, `principal`, `tenant_registration_id` or `deployment_registration_id` (as `oauth_credentials`, exactly one set), `credential_id`, `scopes text[]`, `return_path` (at most 512, a checked Studio path), `expires_at`, `used_at`, `completed_at` | 10 open per person (counted under a per-person advisory lock); deleted after expiry by the hourly pass; SELECT, INSERT, UPDATE, DELETE |
| `oauth_registration_windows` | `(registration_kind, registration_id, window_start)` | `count_this_second smallint` | the registration total, without a tenant (renamed from `window`, a reserved word); requests in flight are not stored here: they are the unexpired leases of the registration's credentials and the unexpired `oauth_broker_calls` rows, counted by the acquire under this row's lock; SELECT, INSERT, UPDATE |
| `oauth_tenant_windows` | `(tenant_id, registration_id, window_start)` | `count_this_second smallint` | a tenant's share on a deployment registration, locked after the registration row; SELECT, INSERT, UPDATE |
| `oauth_broker_calls` | `(tenant_id, id)` | `registration_kind`, `registration_id`, `kind` (`EXCHANGE`, `USERINFO`, `REVOCATION`), `lease_until` | a slot of a call that is not a refresh, deleted when the call ends or by the hourly pass once expired; SELECT, INSERT, DELETE |
| `oauth_delivery_attempts` | `(tenant_id, attempt_id)` | `full_calls smallint`, `reasks smallint`, `hour_started timestamptz`, `per_resource jsonb` (name to last ask, at most 8 entries) | deleted 7 days after the attempt ends; SELECT, INSERT, UPDATE, DELETE |
| `hub_resource_type_versions` | `(tenant_id, type_key, version)` | `state` (`DRAFT`, `PUBLISHED`, `BLOCKED`), `definition jsonb` (at most 256 KiB), `provenance jsonb`, `published_by`, `published_at` | the `hub` source of feature 021's catalogue (Decision 021-004); SELECT, INSERT, UPDATE (`state`) |
| `hub_import_items` | `(tenant_id, kind, item, version)` | `digest`, `import_id`, `created_at` | the digest conflict; SELECT, INSERT |
| `hub_item_findings` | `(tenant_id, item_digest)` | `kind`, `reason`, `created_at` | SELECT, INSERT |

## Deployment tables

| Table | Key | Columns | Notes |
|---|---|---|---|
| `oauth_deployment_provider_versions` | `(provider_slug, version)` | as `oauth_provider_versions`, hub source only | written by operator commands |
| `oauth_deployment_registrations` | `(id)` | as `oauth_client_registrations`; the grant is the authorization code only | written by operator commands; SELECT for `sagat_app` |
| `oauth_deployment_offers` | `(registration_id, tenant_id)` | `state` (`OFFERED`, `OPTED_IN`, `OPTED_OUT`, `WITHDRAWN`), `changed_by`, `changed_at`, `refresh_stops_at` | SELECT, and UPDATE of `state`, `changed_by`, `changed_at` and `refresh_stops_at` only (an opt-out sets `refresh_stops_at` to 7 days later), for `sagat_app`; opt-in is `WHERE state IN ('OFFERED', 'OPTED_OUT')` and opt-out `WHERE state IN ('OFFERED', 'OPTED_IN')`, and a `WITHDRAWN` offer answers as a missing one (`404`); a trigger refuses any change out of `WITHDRAWN` by the runtime role |
| `hub_item_blocks` | `(item_digest)` | `reason_class`, `source` (`INDEX`, `OPERATOR`), `created_at`, `lifted_by`, `lifted_at` | written by the definer function `hub_record_revocation` (pinned `search_path`, `EXECUTE` revoked from `PUBLIC` and granted to `sagat_app`), which only inserts blocks of source `INDEX`, never touches `lifted_by` or `lifted_at` and never un-lifts a block, so a misuse can only block items (a denial of service the operator lifts); lifted only by operator commands |

## Changes to feature 028's tables

| Table | Change |
|---|---|
| `template_entries` | `kind` check adds `OAUTH_PROVIDER` and `INTEGRATION`; nullable `revoked_class` and `revoked_reason` |
| `template_imports` | `namespace_id` nullable; the unique key becomes `UNIQUE NULLS NOT DISTINCT (tenant_id, namespace_id, actor, request_key)`; `target_kind` adds `OAUTH_PROVIDER`, `RESOURCE_TYPE` and `INTEGRATION`; `request_digest` |

## Statements

```text
acquire:   UPDATE oauth_credentials SET fence = fence + 1, lease_until = now() + interval '30 s'
           WHERE tenant_id = $t AND id = $c AND state = 'ACTIVE'
             AND access_revision IS NOT DISTINCT FROM $seen
             AND (lease_until IS NULL OR lease_until < now())
             AND <the registration is usable (its state is ACTIVE or SUSPECT, and an offered one is OPTED_IN) and room: the count of
               unexpired leases of its credentials plus unexpired oauth_broker_calls rows is below 4 (below 1 for the
               tenant on a deployment registration), and count_this_second is below the per-second limit>
           RETURNING fence, grant_epoch, refresh_revision, now()
           (one transaction: the credential row first, then the registration's window row and, on a deployment
           registration, the tenant's window row FOR UPDATE, so that two acquires on different credentials of one
           registration count the unexpired leases and calls one after the other; "room" is fewer unexpired leases
           of the registration's credentials plus unexpired oauth_broker_calls rows than the limit, and a second
           count under the limit)
activate:  UPDATE oauth_credentials SET access_revision = $a, access_expires_at = $x, refresh_revision = $r,
             access_stale = false, lease_until = NULL, fence = fence + 1
           WHERE tenant_id = $t AND id = $c AND fence = $held AND state = 'ACTIVE'
           (then, in the same transaction, the old and lower-fence pending revisions become RETIRED)
callback:  UPDATE oauth_authorization_requests SET used_at = now()
           WHERE tenant_id = $t AND state_hash = $h
             AND (tenant_registration_id, deployment_registration_id) IS NOT DISTINCT FROM ($g, $d) AND principal = $p
             AND used_at IS NULL AND expires_at > now()
           RETURNING ...
writers:   every other credential write is UPDATE ... SET fence = fence + 1, ... WHERE tenant_id = $t AND id = $c
           AND state IN (<the states that transition allows>)
```

## Vault references of token revisions

Each revision has one write-once reference on the OAuth mount (KV version 2). Every write to it is a check-and-set write
with `cas=0`, so it succeeds only where nothing was ever written: the holder's token write, and the tombstone with which
the adopt step and the sweeper claim an unmarked row's reference before abandoning or closing it (Decision 047-063). A
tombstone is a fixed payload (`{"sagat_tombstone": true}`) with the custom metadata `sagat-tombstone=true`; a probe reads
it as claimed, never as a token, so a claim interrupted before its row is marked abandoned is finished by the next probe,
which abandons the row. Destroying a revision destroys the versions' data and keeps the key's metadata, so the reference
can never be written again. Tests: `a_claim_interrupted_before_the_abandon_mark_is_finished_by_the_next_probe`,
`a_destroyed_reference_cannot_be_written_again`.

## Lock order

The credential row first, always; then its revision rows; then any of feature 021's resource rows. A deletion of resources
that disconnects locks the credential row before it counts the resources naming it. Passes claim credentials in batches
with `FOR UPDATE SKIP LOCKED`, in identifier order.
