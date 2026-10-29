# Data model — feature 019 (proposed)

Every table is tenant-keyed: the primary key starts with `tenant_id`, every foreign key is composite with the
tenant (and the namespace where the parent has one), and every query filters by both. Migration timestamps are
allocated at implementation (D1074); slugs below. Grants are to the runtime role `sagat_app` in the same file. The
runtime role takes no row locks; fences are transaction-scoped advisory locks (`pg_advisory_xact_lock[_shared]`)
or existing `SECURITY DEFINER` functions. Storage classes refer to `crates/infra-postgres/storage-classification.json`
(`content`, `log`, `protected_system`).

## States and transitions

### Deployment version

```text
 first registration ──► INACTIVE ── set ramping ──► RAMPING ── set current ──► CURRENT
                           │                          │                          │
                           │ retire (never routed)    │ ramp removed or          │ replaced as current
                           ▼                          │ replaced                 │
                        RETIRED ◄── retire (exact,    ▼                          ▼
                        (tombstone)  fenced) ── DRAINED ◄── no open pins, ── DRAINING ◄──┘
                                                            no active or
                                                            detached leases
```

- A version that has ever been routed (`first_routed_at` set) never returns to `INACTIVE`: when its ramp is removed
  or it stops being current it becomes `DRAINING`, so runs pinned to it during a ramp keep being served.
- Retiring an `INACTIVE` version is allowed only when `first_routed_at` is null (nothing can be pinned to it); such a
  version is deleted rather than kept as a tombstone, and its build id may register again.
- `DRAINING`/`DRAINED` versions may become `RAMPING` or `CURRENT` again (a rollback); `RETIRED` is final and its
  tombstone is never purged while the deployment exists (D1079).
- `DRAINING` versus `DRAINED` is a sampled display state; retirement recomputes pins and leases exactly.
- Every deployment has one `UNVERSIONED` pseudo-version row (`build_id = ''`), which starts `CURRENT` and counts as
  routed.

### Step lease holder (additive to the existing step states)

```text
READY ──claim (pull)──► LEASED/RUNNING [holder=session]
READY ──claim (push)──► LEASED/RUNNING [holder=push endpoint]
LEASED/RUNNING [session] ──Detach──► RUNNING [holder=detached]  (expiry = detached heartbeat timeout)
RUNNING [push] ──202 (async mode)──► RUNNING [holder=detached]
RUNNING [detached] ──external complete/fail──► SUCCEEDED | RETRY_WAIT | FAILED (as a worker report)
RUNNING [detached] ──timeout/max duration──► RETRY_WAIT | FAILED (DETACHED_TIMEOUT, generation+1)
```

### Push endpoint health

`HEALTHY → FAILING` (consecutive failures ≥ breaker threshold; breaker open for its window) `→ HEALTHY` (a success
after the window); `→ DENIED` (egress refusal; claims held until a configuration or policy change);
`PAUSED` by an operator; `DELETED` is a tombstone.

## Tables (new)

### `worker_deployments`

| Column | Type | Notes |
|---|---|---|
| `tenant_id`, `namespace_id` | uuid | FK `namespaces (tenant_id, id)` |
| `id` | uuid | PK `(tenant_id, id)` |
| `name` | text | `^[a-z][a-z0-9-]{2,63}$`; unique `(tenant_id, namespace_id, name)` among non-deleted |
| `routing_revision` | bigint | CAS; starts 1 |
| `current_version_id` | uuid | FK to versions; never null after creation (the `UNVERSIONED` row) |
| `ramping_version_id` | uuid null | ≠ current (CHECK) |
| `ramp_basis_points` | integer | 0–10,000; 0 when no ramping version |
| `default_behavior` | text null | `pinned` / `autoUpgrade`; null ⇒ namespace default |
| `created_by`, `created_at`, `updated_at`, `deleted_at` | | tombstone on delete |

Classification: `name` `protected_system` (bounded identifier). Grants: SELECT, INSERT, UPDATE.

### `worker_deployment_patterns`

`(tenant_id, namespace_id, pattern)` PK, `deployment_id` FK. One row per pattern (≤256 per deployment). The PK makes
identical patterns unique per namespace; prefix overlap (`payments.*` versus `payments.refund`) and overlap with push
endpoint activities are checked by the application under the exclusive namespace dispatch-catalog fence (D1075),
which every claim takes shared. Grants: SELECT, INSERT, DELETE.

### `worker_deployment_versions`

| Column | Type | Notes |
|---|---|---|
| `tenant_id`, `namespace_id`, `deployment_id` | uuid | FK deployment |
| `id` | uuid | PK `(tenant_id, id)` |
| `build_id` | text | `^[A-Za-z0-9._:+-]{1,128}$` or `''` for `UNVERSIONED`; unique `(tenant_id, deployment_id, build_id)`; rows of versions that were ever routed are never deleted while the deployment exists, so such a retired build cannot register again (never-routed versions are deleted on retirement) |
| `state` | text | CHECK in the six states |
| `manifest_activities` | text[] | frozen at first registration (≤256) |
| `manifest_languages` | text[] | ≤16 |
| `manifest_protocol_minor` | integer | |
| `first_registered_at`, `first_routed_at`, `retired_at` | timestamptz | database clock |
| `retired_by`, `retire_reason` | text | reason 8–2,000 characters |

An `ENABLE ALWAYS` trigger refuses updates of `build_id` and `manifest_*`, and any change of a `RETIRED` row.
Classification: `build_id`, `manifest_*` `protected_system`; `retire_reason` `log` (bounded, redacted on output).
Grants: SELECT, INSERT, UPDATE (state columns only, enforced by the trigger).

### `run_deployment_pins`

`(tenant_id, run_id, deployment_id)` PK; `namespace_id`, `version_id` (FK, same tenant/namespace), `source`
(`claim`, `inherited`, `chain`, `repin`), `pinned_at`, `closed_at` (set, in the same transaction, by an `ENABLE
ALWAYS` trigger when the run reaches a terminal status). Composite FK to `runs (tenant_id, id)`. Inserted in the
claim, admission or handoff transaction; updated only by a repin (journaled) or the closing trigger. Index
`(tenant_id, version_id) WHERE closed_at IS NULL` supports drainage counts and `PATTERN_IN_USE`. Classification: no variable-size columns besides `source` (closed enum).
Retention: deleted with the run. Grants: SELECT, INSERT, UPDATE.

### `step_checkpoints`

| Column | Type | Notes |
|---|---|---|
| `tenant_id`, `namespace_id`, `run_id` | uuid | FK run |
| `step_id` | uuid | the stable step identity of the instance (scoped path included) |
| `attempt` | integer | attempt that recorded it |
| `holder_epoch` | smallint | 0 while a worker or the push invoker holds the lease; 1 after a detach |
| `sequence` | bigint | monotonic within one `(attempt, holder_epoch)`; final details use the maximum value |
| `details` | bytea | canonical JSON, ≤64 KiB (CHECK) |
| `digest` | bytea | SHA-256, 32 bytes |
| `recorded_at` | timestamptz | coalescing reference |

PK `(tenant_id, run_id, step_id)`; an upsert replaces the row only when `(attempt, holder_epoch, sequence)` is
lexicographically greater than the stored triple (D1076), so a later attempt always wins and stale or out-of-order
beats are ignored. Classification: `details` **`content`** (charged to
`tenant_content_bytes` by the ADR-0027 byte-accounting trigger, delta on overwrite). Retention: purged with the run's
payloads. Grants: SELECT, INSERT, UPDATE, DELETE.

### `step_ended_leases`

`(tenant_id, run_id, step_id, attempt, generation)` PK; `lease_token_hash` bytea (32 bytes, the SHA-256 already
stored for the lease), `ended_at`, `ended_by` (closed enum: `report`, `external_report`, `expiry`, `timeout`,
`cancel`, `reclaim`). Inserted in the transaction that ends a lease that was ever detached or push-held (the only
leases a task token can name), so a later use of that token is answered `STALE_ATTEMPT` only after its secret
matches, and every mismatch is `NOT_FOUND` (D1057). At most one row per ended attempt (≤ the step's `maxAttempts`).
Retention: deleted with the run. Grants: SELECT, INSERT, DELETE.

### `push_endpoints` and `push_endpoint_revisions`

`push_endpoints`: `(tenant_id, id)` PK, `namespace_id`, `name` (unique per namespace, `^[a-z][a-z0-9-]{2,63}$`),
`current_revision`, `state` (`ACTIVE`, `PAUSED`, `DELETED`), `health`, `health_reason` (closed enum),
`consecutive_failures`, `breaker_open_until`, `updated_at`. Grants: SELECT, INSERT, UPDATE.

`push_endpoint_revisions` (immutable, `ENABLE ALWAYS` trigger refuses UPDATE/DELETE): `(tenant_id, endpoint_id,
revision)` PK, `kind` (`https`, `aws-lambda`), `target` (HTTPS URL ≤2,048 bytes, or Lambda function ARN/name with
qualifier and region), `activities` text[] (≤256, exact names, none in a deployment, no reserved prefix),
`auth_mode` (`signature-only`, `aws-sigv4`, `google-oidc`), `auth_secret` (logical secret name, never a value),
`bound_by` (the principal whose `secret:use` bound the secret; the engine rechecks the secret's access list against it
at each invocation, ADR-0025 precedent), `region` (`aws-lambda` only, `^[a-z]{2}(-[a-z]+)+-\d$`, equal to the ARN's
region), `mode` (`sync`, `async`), `request_timeout_ms`, `max_concurrency`, `max_envelope_bytes`, `created_by`,
`request_id` (idempotent create/update receipt), `created_at`. The Google audience is not stored: it is derived from
the target's canonical origin. At most 1,000 revisions per endpoint; revisions are never pruned (the trigger refuses
DELETE), and an update beyond the bound is refused. Classification: `target`, `region` `protected_system` (validated,
bounded); `activities` `protected_system`. Activities are unique per namespace across
active endpoints and disjoint from deployment patterns (application check under the same exclusive namespace
dispatch-catalog fence).

### `script_warm_policies`

`(tenant_id, namespace_id, artifact_digest, secret_set_digest)` PK; `revision`, `enabled`, `min_warm`, `max_warm`,
`max_uses`, `max_idle_ms`, `max_lifetime_ms`, `residue_acknowledged_by`, `residue_acknowledged_at`,
`acknowledgement_text_version` (the version of the risk statement accepted), `updated_by`, `updated_at`. FK to the
tenant's artifact (`script_artifacts`). `secret_set_digest` is SHA-256 of the sorted declared secret names (empty set
allowed). Grants: SELECT, INSERT, UPDATE, DELETE.

### `script_warm_slots`

`(tenant_id, slot_id)` PK; `namespace_id`, `artifact_digest`, `secret_set_digest` (FK to the policy), `runner_subject`,
`acquired_at`, `expires_at` (database clock, TTL ≤300 s, renewed by the runner). Inserted under the namespace and
tenant warm advisory locks after counting live slots against `script_warm_containers`,
`tenant_script_warm_containers` and the policy's `max_warm` (exact across runners, D1066). A runner serves a run in a
warm container only while that container's slot lease is live. Renewal returns `secret_version_digest`, the
digest of the current versions of the declared secret set, which every warm-eligible lease also carries; the runner
evicts a container on a mismatch before starting a process. Grants: SELECT, INSERT, UPDATE, DELETE.

### `worker_session_activity`

`(tenant_id, session_id)` PK, FK `worker_sessions (tenant_id, id)` ON DELETE CASCADE; `last_poll_at`,
`last_heartbeat_at`. Updated conditionally (≥5 s apart). Grants: SELECT, INSERT, UPDATE, DELETE.

### `namespace_dispatch_insights`

`(tenant_id, namespace_id, subject_kind, subject)` PK; `subject_kind` in (`activity`, `version`, `endpoint`,
`pool`); `subject` text ≤160 (activity name, version id, endpoint id, advertised pool label); `ready`,
`oldest_ready_seconds`, `leased`, `detached`, `pollers`, `last_poll_seconds`, `last_heartbeat_seconds`,
`pinned_open_runs`, `flags` (closed set: `UNVERSIONED_IDLE`, `NO_POLLERS`, `PUSH_INELIGIBLE_BACKLOG`),
`sampled_at`. At most 500 rows per namespace (monitor truncates by backlog). Written only by the leased monitor;
the runtime role writes it with a `DELETE` + `INSERT` per namespace per round. Classification: `subject`
`protected_system`. Retention: overwritten each round.

## Columns added to existing tables (nullable, additive)

- `runs`: `priority smallint` (1–5), `fairness_key text` (CHECK pattern, ≤64), `fairness_key_digest bytea`
  (per-tenant HMAC), `fairness_key_level text` (`public`, `internal`), `fairness_weight smallint` (1–100),
  `versioning_behavior text` (`pinned`, `autoUpgrade`, null).
- `steps`: `priority smallint`, `fairness_key_digest bytea` (the claim groups by digest; the raw key stays on the
  run), `fairness_weight smallint`, `versioning text`,
  `lease_holder_kind text` (`session`, `push`, `detached`; null ⇒ session for legacy rows),
  `push_endpoint_id uuid`, `push_revision integer`, `deployment_version_id uuid`, `detached_at timestamptz`,
  `detached_heartbeat_timeout_ms integer`, `detached_deadline timestamptz`.
- `worker_sessions`: `deployment_id uuid`, `deployment_version_id uuid` (FK, same tenant/namespace).
- `script_executions`: `warm_slot_id uuid` (null for a cold run) and `temperature text` (`cold`, `warm`), checked
  against a live matching slot when the runner reports the execution.
- Partial index `steps_detached_idx ON steps (tenant_id, namespace_id) WHERE lease_holder_kind = 'detached'`, built
  concurrently; Detach and the push `202` path count detached attempts through it under the namespace task-token
  advisory lock (exact `task_tokens` quota).
- Expression index `steps_ready_priority_idx ON steps (tenant_id, namespace_id, activity, (coalesce(priority, 3)),
  ready_at) WHERE status = 'READY'`, built `CONCURRENTLY` in its own migration; claim queries use the same
  expression verbatim. No stored generated column (it would rewrite `steps` under an exclusive lock, D1078).
- Partial index `steps_push_leases_idx ON steps (tenant_id, namespace_id, push_endpoint_id) WHERE lease_holder_kind =
  'push' AND status IN ('LEASED', 'RUNNING')`, built concurrently; the push claim counts active push leases through it
  under the tenant → namespace → endpoint push fences (bounded by the ceilings: at most 1,024 rows per tenant).

Classification: `runs.fairness_key` is **`content`** (retained content, charged and purged with the run's payloads),
with the ADR-0033 level recorded in `fairness_key_level`: an `{expr}` key takes the level of the input field it reads
(`pii` when undeclared, which publication refuses), literal and start-option keys are `internal`. The raw key is
shown only to `payload:read-internal` holders; everyone else, logs, the journal and insights see the digest, an
HMAC-SHA-256 under a per-tenant key derived from the deployment's digest key (G1049, G1057).

## Journal events (additive)

`run.started` gains optional `dispatch {priority, fairness_key_digest, fairness_weight, versioning_behavior}` (the raw
key stays on the run row, G1049); `run.deployment_pinned {deployment, build_id, source}`; `run.deployment_repinned
{deployment, from, to, actor, reason}`; `step.leased` gains optional `dispatch_mode`, `deployment_build_id`,
`push_endpoint`, `checkpoint {attempt, digest, size}`; `step.detached {attempt, generation, expires_at, max_until}`;
ending events (`step.failed`, lease expiry, timeout) gain optional `checkpoint {digest, size}`. Detail keys are
snake_case. Old readers ignore unknown fields; replay of old journals yields null dispatch attributes and no pins. The
reset rule of each event type (018's registry, coordinator decision C1) is listed in
[contracts/errors-and-events.md](contracts/errors-and-events.md#journal-events-additive): `run.deployment_pinned`
and `run.deployment_repinned` are `REMATERIALIZE`, the others `COPY`.

## Content-store registration (020)

`step_checkpoints.details` and `runs.fairness_key` are tenant content. They are registered as content stores in
020's content-store registry (coordinator decision C8): purged with the run's content under 020's retention and
erasure rules, charged to `tenant_content_bytes` (ADR-0027), never exported by 020 sinks or archives except as
structure.

## Time classes (028)

Every new time column takes a class of 028's time-class catalog (`specs/028-developer-tooling/data-model.md` §1,
manifest `crates/infra-postgres/time-classification.json`, coordinator decision C13): `steps.detached_deadline` (the
maximum detached duration, bounded by the step timeout and run deadline) is `workflow_due`; detached-lease expiry
(and therefore task-token validity, which ends with the lease), `step_checkpoints.recorded_at`,
`script_warm_slots.expires_at`, `push_endpoints.breaker_open_until`, `worker_session_activity.*`,
`namespace_dispatch_insights.sampled_at`, version timestamps and `step_ended_leases.ended_at` are `infrastructure`;
`run_deployment_pins.pinned_at`/`closed_at` and `steps.detached_at` are `workflow_record`.

## Quota resources (added to `QuotaResource`)

`worker_deployments`, `push_endpoints`, `push_concurrency`, `tenant_push_concurrency`, `task_tokens`
(detached attempts), `heartbeat_detail_bytes` (per-operation policy), `script_warm_containers`,
`tenant_script_warm_containers`. Count resources allow zero (refuse); policy resources must be positive; defaults and
ceilings in [plan.md](plan.md#finite-ceilings).

## Retention

Pins, checkpoints, ended-lease hashes and detached-attempt columns live and die with their run. Version rows that
were ever routed, including their `RETIRED` tombstones, are kept until the deployment is deleted and never purged
before (D1079); a version that was never routed is deleted (not tombstoned) when it is retired, so a credential that
registers many build ids cannot exhaust the bound, and version creation is rate-limited per deployment. At 1,000
records a deployment refuses new versions with `QUOTA_EXCEEDED`. Warm slot leases are deleted when they expire or are
released. Endpoint revisions are never pruned (at most 1,000 per endpoint) and go with the endpoint's namespace. Warm policies are deleted explicitly or with their namespace. Insights rows are
overwritten every monitor round.
