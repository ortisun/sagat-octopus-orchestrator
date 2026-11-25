# Data model (design 046)

Additive migrations after `20270119000000` (timestamps allocated by the coordinating session). Every table has
`tenant_id` first in its key and a foreign key to its tenant (and namespace where it has one); grants for `sagat_app`
are listed per table; every variable-size column is classified in `crates/infra-postgres/storage-classification.json`.
New CHECK constraints on existing tables are added `NOT VALID` and validated by a second migration (design 043's rule).
Each down migration refuses while a row it would drop exists.

## Changes to existing tables

| Table | Change |
|---|---|
| `scripts`, `script_versions`, `script_test_runs` | `manifest text NULL CHECK (octet_length(manifest) <= 65536)` (the TypeScript manifest; Python's is in the source). `script_versions_guard` adds `manifest` to the columns it protects |
| `script_versions` | `bundle_version smallint NULL` (5 for a version with packages); state `BUILDING` covers the dependency phases |
| `serving_instances` (`serving_instances_capabilities_closed`) and `platform_requirements` (`platform_requirements_capability_valid`) | the CHECKs add `script-dependencies-v1` (restating the full set, since features 022 and 023 widen them too) |

`script_artifacts` is not changed: it stays immutable (`script_artifacts_immutable`).

## New tables

| Table | Key | Columns | Notes; grants |
|---|---|---|---|
| `script_runtime_policies` | `(tenant_id, namespace_id)` | `revision bigint`, `document jsonb` (at most 65,536 bytes; keys `languages` (feature 023) and `packages`), `updated_by`, `updated_at` | as specified by feature 023's data model, created here; compare-and-swap on `revision`; SELECT, INSERT, UPDATE |
| `script_package_rules` | `(tenant_id, namespace_id, id)` | `kind` (`ALLOW`, `DENY`), `ecosystem`, `name_glob`, `versions`, `created_by`, `created_at` | at most 500 per kind per namespace (counted under the policy row's lock); SELECT, INSERT, DELETE |
| `supply_chain_waivers` | `(tenant_id, namespace_id, id)` | `advisory_id`, `ecosystem`, `package`, `version`, `reason` (at most 1 KiB), `expires_at` (at most 90 days after `created_at`), `created_by`, `created_at`, `expired_at` | 200 per namespace; SELECT, INSERT, UPDATE (`expired_at` only) |
| `script_dependency_jobs` | `(tenant_id, id)` | `version_id`, `phase` (`RESOLVE`, `INSTALL`), `target` (null for resolve), `state` (`PENDING`, `LEASED`, `DONE`, `FAILED`), `attempt`, `lease_token_hash`, `lease_expires_at`, `not_before`, `failure_kind`, `failure_code`, `log` (at most 64 KiB, redacted), `created_at`, `finished_at` | `UNIQUE (tenant_id, version_id, phase, target)`; claim index on `(state, not_before)`; SELECT, INSERT, UPDATE |
| `script_version_dependencies` | `(tenant_id, version_id)` | `ecosystem`, `runtime`, `manifest_digest`, `lock bytea` (at most 262,144), `lock_digest`, `metadata jsonb` (the gateway's answer for every entry, at most 4 MiB), `report jsonb` (at most 1 MiB), `policy_revision`, `policy_digest`, `snapshot_ids jsonb`, `layers jsonb` (target to digest, size, files) | written in the resolve and install transactions; SELECT, INSERT, UPDATE until the version is `READY`, then guarded |
| `script_locked_packages` | `(tenant_id, version_id, ecosystem, name, version)` | `file_sha256`, `target_set` | the index the advisory import and the collection pass use; secondary index `(ecosystem, name, version)`; SELECT, INSERT |
| `script_version_sboms` | `(tenant_id, version_id)` | `sbom bytea` (at most 4 MiB, CycloneDX 1.5) | SELECT, INSERT |
| `script_build_provenance` | as feature 023's data model | as feature 023's data model | created here; feature 023 adds its build types |
| `script_artifact_blocks` | `(tenant_id, artifact_digest, scope, reason)` with `scope` the namespace id or the literal `*` | `source` (`ADVISORY_IMPORT`, `REEVALUATION`, `RESERVATION`, `OPERATOR`), `detail jsonb` (advisory ids, package; at most 8 KiB), `created_at` | read in the lease statement; removal audited; SELECT, INSERT, DELETE |
| `script_policy_reevaluations` | `(tenant_id, id)` | `namespace_id` (null for the tenant), `reason` (`POLICY`, `WAIVER`, `SNAPSHOT`, `RESERVATION`), `cursor uuid`, `lease_token_hash`, `lease_expires_at`, `created_at`, `finished_at` | inserted in the causing transaction; SELECT, INSERT, UPDATE |
| `package_reservations` | `(tenant_id, ecosystem, normalised_name)` | `kind` (`NAME`, `NPM_SCOPE`), `created_by`, `created_at` | 1,000 per tenant; SELECT, INSERT, DELETE |
| `private_packages` | `(tenant_id, ecosystem, normalised_name, version)` | `file_sha256`, `filename`, `size`, `uploaded_by`, `uploaded_at` | counted in `tenant_content_bytes` by trigger (Architecture decision 0027); SELECT, INSERT, DELETE |
| `package_advisories` | `(ecosystem, id)` | `aliases text[]`, `malicious boolean`, `severity` (`CRITICAL`, `HIGH`, `MODERATE`, `LOW`, `UNRATED`), `affected jsonb` (at most 64 KiB), `modified_at`, `withdrawn_at` | platform-wide, no tenant; index on `(ecosystem, malicious)`; SELECT for `sagat_app`, writes by the import pass's role |
| `package_advisory_snapshots` | `(ecosystem)` | `snapshot_id`, `source`, `record_count`, `max_modified_at`, `imported_at`, `forced boolean` | the row the import takes `FOR UPDATE` and the finishing transaction `FOR SHARE` |
| `package_fetch_usage` | `(tenant_id, day)` | `bytes bigint` | the daily fetch budget; SELECT, INSERT, UPDATE |

## Lock order

A publication's finishing transaction: the version row, then `package_advisory_snapshots` (`FOR SHARE`), then the
artifact insert. An advisory import: `package_advisory_snapshots` (`FOR UPDATE`), then `package_advisories`, then the
inserted blocks (no row of versions is locked). A re-evaluation page: the reevaluation row, then the blocks of that
page's artifacts, in artifact digest order. The collection pass locks no version row; it deletes a store object only after
re-reading, in its own transaction, that no `script_locked_packages` or `private_packages` row references the hash.
