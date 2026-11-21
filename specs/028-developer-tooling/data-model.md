# Data model — feature 028

Proposed; nothing below exists yet. Every tenant-owned table has `tenant_id` in its primary key and composite foreign
keys that include the tenant (the rule of features 001 and 009). Migration timestamps are allocated at implementation
after the latest on `develop`. Grants name the runtime role `sagat_app`; row locks the runtime role cannot take are
taken by narrow `SECURITY DEFINER` functions with a fixed `search_path`, as feature 017 does.

## 1. Time classes (production schema, no new table)

`crates/infra-postgres/time-classification.json` lists every `timestamp`/`timestamptz` column of the migrated schema
with one class and the modules that write and read it. A catalog test fails when a column is missing or a listed one
no longer exists, when a workflow-class column defaults to `now()` or `clock_timestamp()`, or when a function body in
`pg_proc` compares a workflow-class column with a built-in clock without a listed reason. The human-readable catalog,
including the instants of the parity drafts, is [contracts/time-classes.md](contracts/time-classes.md).

| Class | Meaning | Examples from the baseline schema |
|---|---|---|
| `workflow_due` | an instant compared with workflow now to decide that something is due | `steps.due_at`, `steps.next_attempt_at`, `steps.attempt_deadline_at`, `runs.deadline_at`, `workflow_triggers.next_fire_at`, `workflow_triggers.retry_at`, `approval_timers.due_at`, `loop_timers.due_at`, `loop_scopes.deadline_at`, `loop_scopes.delay_due_at`, `execution_chains.original_deadline_at` |
| `workflow_record` | an instant of a run's history | `run_events.occurred_at`, run and step lifecycle instants, approval decision instants |
| `infrastructure` | liveness, windows, credentials, operations | `steps.lease_expires_at`, `steps.event_since` (compared with `inbox_events.received_at`), `worker_sessions.expires_at`, `scheduler_shards.expires_at`, `inbox_events.received_at`/`expires_at`, cleanup and receipt windows, `audit_records.created_at`, every column added by this feature for jobs, receipts and the catalog |

One function derives workflow time from an infrastructure sample; it is owned by the migration owner, `EXECUTE`
granted to `sagat_app`, with a `COMMENT` that names its canonical body:

```sql
CREATE FUNCTION sagat_workflow_from(ts timestamptz) RETURNS timestamptz
  LANGUAGE sql STABLE PARALLEL SAFE AS $$ SELECT $1 $$;
-- selection (index-range bound):   ... WHERE due_at <= sagat_workflow_from(now())
-- post-lock sample (one reading):  WITH tick AS MATERIALIZED (SELECT clock_timestamp() AS infra)
--                                  SELECT infra, sagat_workflow_from(infra) AS workflow FROM tick
-- lag over a workflow column:      sagat_workflow_from(clock_timestamp()) - min(due_at)
```

A writer of a `workflow_*` column takes its value from `sagat_workflow_from(...)` of the one sample; a writer of an
`infrastructure` column keeps `now()` or `clock_timestamp()`; a comparison with now uses the now of the column's
class. In production the function inlines to its argument, so both instants are the same value (an equality test at
offset zero proves it).

## 2. Test deployment (test database only — `crates/infra-testing` migration)

| Table / function | One row is | Notes |
|---|---|---|
| `sagat_deployment_profile` | the marker: `profile = 'TEST_VIRTUAL_CLOCK'`, `created_at`, `test_server_version` | singleton (`CHECK (id = 1)`); its existence makes production composition roots refuse the database |
| `sagat_test_clock` | the clock: `offset_us bigint CHECK (offset_us >= 0 AND offset_us <= 3162240000000000)`, `epoch bigint`, `updated_at` (infrastructure), `updated_by` | singleton; `sagat_app` has `SELECT` only |
| `sagat_test_clock_advances` | one committed change: `epoch` (PK), `from_offset_us`, `to_offset_us` (`> from`), `mode` (`STEP_STOP`, `STEP_TARGET`, `JUMP`), `actor`, `trace_id`, `at` | append-only (trigger refuses `UPDATE`/`DELETE`, `ENABLE ALWAYS`); bounded at 100,000 rows |
| `sagat_test_clock_advance(expected_epoch bigint, to_offset_us bigint, mode text, actor text, trace_id text)` | `SECURITY DEFINER`; locks the clock row, checks `expected_epoch` (when not null), monotonicity and ceilings, updates the clock, inserts the advance row; returns the new epoch | the only write path |
| replaced body | `sagat_workflow_from(ts)` = `ts + offset` read from `sagat_test_clock` | `STABLE` kept; the argument is still the one infrastructure sample |

The test server's audit record of an advance is an ordinary `audit_records` row of its single tenant.

### Clock states

```
epoch 0, offset 0 ──advance(step stop | target | jump)──▶ epoch n+1, offset' > offset ──▶ …   (never decreases)
```

## 3. Git sync (production schema)

| Table | Primary key | Columns (abridged) | Constraints and indexes |
|---|---|---|---|
| `git_repository_bindings` | `(tenant_id, id)` | `namespace_id`, `head_revision`, `state` (`ACTIVE`, `PAUSED`, `DELETED`), `pause_reason` (closed code), `consecutive_dead`, `last_imported_commit`, `last_reconciled_at`, `last_probe_at`, `last_probe_outcome` (closed), `review_required` (boolean or null: what the latest probe read from the provider), `created_by`, `created_at`, `updated_at` | `UNIQUE (tenant_id, namespace_id) WHERE state <> 'DELETED'` (one per namespace); FK `(tenant_id, namespace_id)`; deferred FK from `(tenant_id, id, head_revision)` to a committed revision |
| `git_binding_revisions` | `(tenant_id, binding_id, revision)` | `settings jsonb` (closed schema: provider, apiBaseUrl, repository, branch, manifestDir, auth `{kind, appId?, installationId?, privateKeySecret?, tokenSecret?}`, webhookSecret, outboundPolicy, inboundPolicy, reconcileIntervalSeconds — secret **names** only), `actor`, `request_id`, `created_at` | immutable (`ENABLE ALWAYS` trigger); `UNIQUE (tenant_id, actor, request_id)` receipt; settings size `CHECK` ≤ 8 KiB |
| `git_sync_documents` | `(tenant_id, binding_id, workflow_id)` | `path`, `status` (closed set of FR-028-036), `base_digest bytea` (32), `base_commit text`, `last_export_id`, `last_import_id`, `draft_revision_written`, `updated_at` | FK workflow `(tenant_id, workflow_id)` |
| `git_sync_exports` | `(tenant_id, id)` | `binding_id`, `binding_revision`, `kind` (`WORKFLOW`, `RESYNC`), `workflow_id` (null for `RESYNC`), `activation_id`, `state` (`PENDING`, `LEASED`, `RECORDED`, `SUPERSEDED`, `DEAD`), `attempt`, `lease_token_hash`, `lease_expires_at`, `next_attempt_at`, `rendered_digest`, `branch`, `commit_sha`, `change_request` (number), `error_code` (closed), `created_at`, `finished_at` | `UNIQUE (tenant_id, binding_id, activation_id)`; partial index `(tenant_id, binding_id, workflow_id) WHERE state IN ('PENDING','LEASED')`; claim index `(next_attempt_at) WHERE state = 'PENDING'`; FK to `workflow_activations` |
| `git_sync_imports` | `(tenant_id, id)` | `binding_id`, `binding_revision`, `trigger` (`WEBHOOK`, `RECONCILE`, `MANUAL`), `head_commit`, `state` (`PENDING`, `LEASED`, `APPLIED`, `PARTIAL`, `DEAD`), lease/attempt columns as above, `error_code`, `created_at`, `finished_at` | `UNIQUE (tenant_id, binding_id) WHERE state IN ('PENDING','LEASED')` (coalescing) |
| `git_sync_import_entries` | `(tenant_id, import_id, name)` | `workflow_id`, `outcome` (`SKIPPED`, `CREATED`, `REPLACED`, `CONFLICT`, `REMOVED_IN_REPOSITORY`, `INVALID`, `IN_SYNC`), `draft_revision`, `conflict_id`, `findings_count` | ≤ 500 per import |
| `git_webhook_receipts` | `(tenant_id, binding_id, delivery_id)` | `received_at`, `body_sha256`, `event` (closed), `outcome` (`ENQUEUED`, `IGNORED`), `expires_at` | purge after `expires_at`; `delivery_id` ≤ 128 printable ASCII |
| `git_sync_conflicts` | `(tenant_id, id)` | `binding_id`, `workflow_id`, `base_digest`, `draft_revision`, `draft_digest`, `repository_commit`, `repository_digest`, `repository_document jsonb`, `state` (`OPEN`, `RESOLVED`, `SUPERSEDED`), `resolution` (`TAKE_REPOSITORY`, `KEEP_DRAFT`), `resolved_by`, `resolved_at`, `created_at` | `UNIQUE (tenant_id, binding_id, workflow_id) WHERE state = 'OPEN'`; document ≤ 1 MiB |
| `draft_provenance` | `(tenant_id, workflow_id, draft_revision)` | `origin` (`GIT_COMMIT`, `TEMPLATE`), `binding_id`, `commit_sha`, `path`, `commit_author` and `committer` (unverified, ≤ 256 each), `review_verified`, `template_import_id`, `created_at` | immutable; FK workflow |

### Export states

```
PENDING ──claim──▶ LEASED ──record (fenced)──▶ RECORDED
   │                 │ lease expires ──▶ PENDING (attempt + 1, backoff)   attempts = 12 ──▶ DEAD ──redrive──▶ PENDING
   └── newer activation of the same workflow, or binding revision changed ──▶ SUPERSEDED
```

Imports follow the same lease/attempt/`DEAD` rules and end `APPLIED` or `PARTIAL` (some entries `INVALID` or
`CONFLICT`). A conflict is `OPEN` until resolved or superseded by a newer repository commit for the same workflow.

### Per-workflow status (derived, stored in `git_sync_documents.status`)

`IN_SYNC` (repository digest = active digest = base) · `EXPORT_PENDING` · `PULL_REQUEST_OPEN` ·
`DRAFT_FROM_REPOSITORY` (draft replaced by import, not yet published) · `CONFLICT` · `DRIFT` (repository differs from
active and from base, e.g. a closed unmerged request or a `KEEP_DRAFT`) · `REMOVED_IN_REPOSITORY` · `ERROR`.

Draft provenance of an import also records `review_verified` (boolean, from the binding's latest probe) and is shown
on any publication request for that draft revision; the publication of a draft whose last saver is `system:git-sync`
counts one extra required approval (research D1432).

### Grants

`sagat_app`: `SELECT, INSERT` on revisions, provenance, receipts, import entries and conflicts; `SELECT, INSERT,
UPDATE` on bindings, documents, exports, imports and conflicts (state columns only, enforced by triggers that refuse
changes to identity columns); `DELETE` only on expired receipts through the purge function. Claims use
`SECURITY DEFINER` functions that take the row lock and check tenant, state and fence.

## 4. Template catalog

Deployment-level tables (no tenant: the catalog is the operator's), bounded by the ceilings and classified
`protected_system` with explicit size checks:

| Table | Primary key | Columns | Notes |
|---|---|---|---|
| `template_indexes` | `id` (slug) | `kind` (`BUNDLED`, `REMOTE`), `url`, `pinned_key_ids text[]`, `highest_version bigint`, `signed_at`, `expires_at`, `signer_key_id`, `state` (`AVAILABLE`, `EXPIRED`, `INVALID`, `UNREACHABLE`), `last_checked_at` | `highest_version` never decreases (trigger) |
| `template_entries` | `(index_id, template_id, version)` | `kind` (`WORKFLOW`, `SCRIPT`, `RESOURCE_TYPE`), `metadata jsonb` (title, summary, tags, license, maintainer, requirements; ≤ 16 KiB), `entry_digest`, `files jsonb` (name → sha256, size) | replaced per refresh under the index row lock |
| `template_files` | `sha256` | `bytes bytea CHECK (octet_length(bytes) <= 1048576)` | content-addressed; unreferenced rows purged after a refresh |
| `template_index_events` | `(index_id, seq)` | `event` (`REFRESHED`, `REFUSED_SIGNATURE`, `REFUSED_ROLLBACK`, `REFUSED_EXPIRED`, `REFUSED_DIGEST`, `UNREACHABLE`), `version`, `at` | append-only; the deployment-level record (audit is tenant-scoped) |

Tenant-owned:

| Table | Primary key | Columns | Notes |
|---|---|---|---|
| `template_imports` | `(tenant_id, id)` | `namespace_id`, `actor`, `request_key`, `index_id`, `template_id`, `version`, `entry_digest`, `signer_key_id`, `target_kind` (`WORKFLOW`, `SCRIPT`), `target_id`, `created_at` | `UNIQUE (tenant_id, namespace_id, actor, request_key)`; immutable |

`sagat_app`: `SELECT` on the catalog tables (the control plane's refresh path uses a `SECURITY DEFINER` function to
write them); `SELECT, INSERT` on `template_imports`.

## 5. Storage classification and retention

- Content (ADR-0027, charged to the tenant): `git_binding_revisions.settings`, `git_sync_conflicts.repository_document`,
  `draft_provenance.path` and `commit_author`, imported drafts (existing draft columns).
- Protected system with checked bounds: closed state/code columns, digests, commit identifiers, receipt envelopes,
  catalog tables, the test clock tables.
- Retention: exports and imports in terminal states are kept 30 days, then purged in batches (provenance and audit
  remain); receipts until `expires_at`; conflicts until resolved and 30 days after; revisions while the binding exists
  and with its history afterwards (tombstone). Catalog files follow their index. Test databases are disposable.
- Nothing in these tables holds a secret value: credentials are names of namespace secrets; the webhook's raw body is
  discarded after verification.
