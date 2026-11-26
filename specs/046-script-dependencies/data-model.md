# Data model (design 046)

Additive migrations after `20270119000000` (timestamps allocated by the coordinating session). Every table has
`tenant_id` first in its key and foreign keys to its tenant and namespace; grants for `sagat_app` are listed; every
variable-size column is classified in `crates/infra-postgres/storage-classification.json`. New checks on existing
tables are added `NOT VALID` and validated by a second migration (design 043's rule). Each down migration refuses while a
row it would drop exists.

## Changes to existing tables

| Table | Change |
|---|---|
| `scripts`, `script_versions`, `script_test_runs` | `manifest text NULL CHECK (octet_length(manifest) <= 65536)`; `script_versions_guard` protects it |
| `script_versions` | `bundle_version smallint NULL` (5 for a version with packages) |
| `serving_instances`, `platform_requirements`, `platform_requirement_history` | the checks `serving_instances_capabilities_known`, `platform_requirements_capability_known` and `platform_requirement_history_capability_known` restate their full set with `script-dependencies-v1` added (`schedules-v1`, `access-narrowing-v1`, `payload-encoding-v1`, `run-input-validation-v1`, and those other features add first) |
| the `script_jobs` quota query (`crates/infra-postgres/src/quotas.rs`) | counts `script_dependency_jobs` (phases `RESOLVE` and `INSTALL`) and `script_build_jobs` in `QUEUED` and `RUNNING` only when their version is `BUILDING`, with test runs |

`script_artifacts` is not changed: it stays immutable (`script_artifacts_immutable`).

## New tables

| Table | Key | Columns | Notes; grants |
|---|---|---|---|
| `script_runtime_policies` | `(tenant_id, namespace_id)` | `revision bigint`, `document jsonb` (at most 65,536 bytes; keys `languages` of feature 023 and `packages`), `updated_by`, `updated_at` | feature 023's specification, created here; compare-and-swap on `revision`; writers take the row `FOR UPDATE`; SELECT, INSERT, UPDATE |
| `script_package_rules` | `(tenant_id, namespace_id, id)` | `kind` (`ALLOW`, `DENY`), `ecosystem`, `name_glob`, `versions`, `created_by`, `created_at` | 500 per kind per namespace, counted under the policy row's lock, whose revision a rule change increases; SELECT, INSERT, DELETE |
| `supply_chain_waivers` | `(tenant_id, namespace_id, id)` | `advisory_id`, `ecosystem`, `package`, `version`, `reason` (at most 1 KiB), `expires_at` (at most 90 days after `created_at`), `created_by`, `created_at`, `expired_at` | 200 per namespace; a change increases the policy revision; a trigger refuses a malicious advisory; SELECT, INSERT, UPDATE (`expired_at`), DELETE |
| `script_dependency_jobs` | `(tenant_id, id)` | `namespace_id`, `version_id`, `phase` (`RESOLVE`, `INSTALL`, `BACKFILL`), `target`, `state` (`QUEUED`, `RUNNING`, `SUCCEEDED`, `FAILED`, `CANCELLED`), `artifact_digest` (set for `BACKFILL` only), `attempt`, `lease_token_hash`, `lease_expires_at`, `not_before`, `charged_day`, `charged_bytes` (the fetch-budget charge of the current resolve attempt), `failure_kind`, `failure_code`, `log` (at most 64 KiB, redacted), `created_at`, `finished_at` | `UNIQUE NULLS NOT DISTINCT (tenant_id, version_id, phase, target)` as a partial index `WHERE phase <> 'BACKFILL'`; for `BACKFILL` (whose `version_id` is null until its claim) a partial unique index on `(tenant_id, artifact_digest, target)` over `QUEUED` and `RUNNING` rows only, so a later pass can queue a failed pair again (the version is picked at claim, among the artifact's `READY` versions); claim index on `(state, not_before, target)`; SELECT, INSERT, UPDATE |
| `script_version_dependencies` | `(tenant_id, version_id)` | `ecosystem`, `runtime`, `manifest_digest`, `lock bytea` (at most 262,144), `lock_digest`, `metadata jsonb` (at most 4 MiB), `report jsonb` (at most 1 MiB), `snapshot_ids jsonb`, `policy_revision`, `reservation_revision`, `earliest_waiver_expiry`, `layer_digests jsonb` (target to digest, size, files, until the records are signed) | written in the resolve and install transactions and the finish; guarded once `READY` (per-target digests then live only in `script_layer_records`); SELECT, INSERT, UPDATE |
| `script_locked_packages` | `(tenant_id, version_id, ecosystem, name, version)` | `locked_seq bigint GENERATED ALWAYS AS IDENTITY (CACHE 1)`, checked by a migration test (the advisory import's mark), `artifact_digest` (set at finish), `file_sha256`, `source` (`PUBLIC`, `PRIVATE`) | index `(ecosystem, name, version)` for the advisory switch and index `(locked_seq)` for the rows above a mark; every inserting transaction holds the table's `ROW EXCLUSIVE` lock from its insert to its commit, which the advisory barrier's `SHARE` lock waits for; SELECT, INSERT, UPDATE (`artifact_digest`) |
| `script_layer_materializations` | `(tenant_id, id)` | `namespace_id`, `artifact_digest`, `target`, `activity_id` (the ready activity it was asked for), `runner_principal`, `state` (`RUNNING`, `FAILED`, `DONE`), `failure_code`, `created_at` | three `FAILED` rows of distinct `runner_principal` for one (tenant, artifact, target) fail its ready steps; SELECT, INSERT, UPDATE |
| `script_version_sboms` | `(tenant_id, version_id)` | `sbom bytea` (at most 4 MiB, CycloneDX 1.5) | SELECT, INSERT |
| `script_layer_records` | `(tenant_id, artifact_digest, target)` | `layer_digest`, `unpacker`, `signature`, `key_id`, `created_at` | immutable; SELECT, INSERT |
| `script_build_provenance` | as feature 023's data model | as feature 023's data model; the envelope names the lock and SBOM digests (at most 65,536 bytes) | created here (feature 023, Decision 1628) |
| `script_artifact_blocks` | `(tenant_id, id)`, `UNIQUE NULLS NOT DISTINCT (tenant_id, artifact_digest, namespace_id, reason, source)` | `artifact_digest`, `namespace_id` (null for every namespace; foreign key otherwise), `reason`, `source` (`ADVISORY_IMPORT`, `REEVALUATION`, `RESERVATION`, `OPERATOR`), `policy_revision`, `reservation_revision` (the revisions it was evaluated at), `steps_failed_at` (set by the block pass), `detail jsonb` (advisory identifiers, package; at most 8 KiB), `created_at` | read by the lease statement, the materialization route and the runnable route; a trigger refuses deleting an `ADVISORY_IMPORT` row unless its advisory is withdrawn; SELECT, INSERT, DELETE, and UPDATE of `steps_failed_at` only, with a trigger that refuses an update changing any other column |
| `script_policy_reevaluations` | `(tenant_id, id)` | `namespace_id` (null for the tenant), `reason` (`POLICY`, `WAIVER`, `SNAPSHOT`, `RESERVATION`, `UPLOAD`), `cursor uuid`, `lease_token_hash`, `lease_expires_at`, `created_at`, `finished_at` | inserted by the causing transaction; worked per tenant in creation order (a tenant-wide row before the namespace rows created after it); SELECT, INSERT, UPDATE |
| `package_reservations` | `(tenant_id, ecosystem, normalised_name)` | `kind` (`NAME`, `NPM_SCOPE`), `created_by`, `created_at` | 1,000 per tenant; SELECT, INSERT, DELETE |
| `package_reservation_revisions` | `(tenant_id)` | `revision bigint` | increased under `FOR UPDATE` by every reservation change; read `FOR SHARE` by resolve completion and finish |
| `private_packages` | `(tenant_id, ecosystem, normalised_name, version)` | `namespace_id` (where it is stored), `file_sha256`, `filename`, `size`, `blob_key`, `uploaded_by`, `uploaded_at`, `released_by`, `released_at` (an early release by another `package:manage` holder; a check refuses `released_by = uploaded_by`) | counted in `tenant_content_bytes` by trigger (Architecture decision 0027); SELECT, INSERT, DELETE, and UPDATE of `released_by` and `released_at` only |
| `package_advisories` | `(ecosystem, snapshot_id, id)` | `aliases text[]`, `malicious boolean`, `severity`, `affected jsonb` (at most 64 KiB), `matched_pairs jsonb` (the exact name and version pairs computed at staging), `modified_at`, `withdrawn_at` | platform-wide; staged per snapshot; rows of old snapshots deleted after the switch; index `(ecosystem, snapshot_id, malicious)`; SELECT for `sagat_app`; INSERT, UPDATE, DELETE for the role `sagat_advisory_import`, which also holds INSERT on `script_artifact_blocks` and `script_policy_reevaluations` and UPDATE on `package_advisory_snapshots`, and SELECT on `script_locked_packages` (for the barrier, staging and the switch); the control plane's import pass connects with it (`SAGAT_ADVISORY_IMPORT_DATABASE_URL`); the trigger on block deletion guards against defects, not against a compromised control plane, which holds both roles |
| `package_advisory_snapshots` | `(ecosystem)` | `current_snapshot_id`, `source`, `record_count`, `max_modified_at`, `switched_at`, `forced`, `held_snapshot_id` | read under the snapshot lock (Lock order) and updated by the switch under it; the migration seeds one row per ecosystem (`npm`, `pypi`) so that it always exists |
| `package_store_pins` | `(id)` | `file_sha256`, `holder_kind` (`JOB`, `TEST_RUN`, `MATERIALIZATION`, `OFFLINE_BUNDLE`), `holder_id`, `lease_until` (null for bundles) | SELECT, INSERT, DELETE |
| `package_store_marks` | `(file_sha256)` | `marked_at` | the collection pass's marks; SELECT, INSERT, DELETE |
| `package_fetch_usage` | `(tenant_id, day)` | `bytes bigint` | the resolve claim charges the 1 GiB ceiling under this row's lock and refuses past the budget; every completion of the attempt, failures included, settles it, on the job's `charged_day` row only and never below zero, to the forwarded bytes; an expired lease keeps the ceiling; SELECT, INSERT, UPDATE |

## Lock order

- **The snapshot lock:** a transaction-level advisory lock on a fixed key per ecosystem. Decisions take it shared
  through `sagat_lock_advisory_snapshot_share(ecosystem)`, which also returns the snapshot row as it stands; the switch
  takes it exclusively, then the row `FOR UPDATE`. A heavyweight lock request that conflicts with a waiting one queues
  behind it, so shared takers cannot starve the switch (a row lock gives no such order).
- **Resolve completion:** the job row, the version row, then the snapshot lock shared, then `FOR SHARE` on the
  namespace's `script_runtime_policies` row and `package_reservation_revisions`, then inserts.
- **Install completion:** the job row, then the version row.
- **Finish:** the version row, then the snapshot lock shared and the same two rows `FOR SHARE`, then the artifact, the
  records and the blocks.
- **Advisory barrier:** `script_locked_packages` in `SHARE` mode (through `sagat_advisory_barrier()`), read the highest
  `locked_seq`, commit.
- **Advisory switch:** the snapshot lock exclusively, then `package_advisory_snapshots` `FOR UPDATE`, then block inserts
  (no version row is locked, so it cannot deadlock with a decision, which takes the snapshot lock after its own rows).
- **Author-lock publication and any other writer of `script_locked_packages`:** the snapshot lock shared (ordering the
  decision against a switch), then its inserts, whose table lock the advisory barrier waits for.
- **Policy, rule and waiver writes:** the policy row `FOR UPDATE`. **Reservations:** the revision row `FOR UPDATE`.
- **Re-evaluation page:** its job row, then the snapshot lock shared, then the policy and reservation-revision rows
  `FOR SHARE`, then the
  page's blocks in artifact digest order.
- **Block pass (`fail_ready_step`):** one step per transaction, in the engine's order: the run's chain, the run row, then
  the step row `FOR UPDATE`, rechecked; then the block row's `steps_failed_at` when the batch is done.
- **Install refusal:** the job row, the version row, then the build job and the `QUEUED` sibling jobs with
  `FOR UPDATE SKIP LOCKED`.
- **Advisory barrier:** `sagat_advisory_barrier()`, `SECURITY DEFINER` (fixed `search_path`, trusted owner), takes
  `script_locked_packages` in `SHARE` mode and returns the highest `locked_seq`; `EXECUTE` revoked from `PUBLIC` and
  granted to `sagat_advisory_import` only.
- **Shared locks of the runtime role:** `sagat_app` takes the snapshot lock only through the `SECURITY DEFINER` function
  `sagat_lock_advisory_snapshot_share(ecosystem)` (fixed `search_path`, trusted owner, `EXECUTE` revoked from `PUBLIC` in
  the migration, as PostgreSQL grants it by default, and granted to `sagat_app` only), so that one function names the key (advisory lock functions are executable by every role, so the key is a convention
  of trusted code, not a privilege) and the runtime role needs no privilege on `package_advisory_snapshots` beyond SELECT;
  tested as the runtime role (`SAGAT_TEST_AS_RUNTIME_ROLE=1`).
- **Collection:** in one transaction per object, re-read that no lock, upload or unexpired pin references the hash and
  that its mark is at least 7 days old, delete the mark, commit; then delete the object through the gateway. A reference
  gained after the commit finds the object still present (the deletion is after) or absent (the pin's holder fetches it
  again from upstream, or, offline, fails retryable).
