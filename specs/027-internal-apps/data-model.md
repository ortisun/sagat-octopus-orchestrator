# Data model — feature 027 internal apps

Proposed; nothing exists yet. All tables are tenant-scoped: the primary key starts with `tenant_id`,
every foreign key to a tenant row is composite `(tenant_id, …)`, and every query joins on the
authenticated tenant. Namespaced tables also carry `namespace_id` with a composite FK to
`namespaces`. Timestamps come from the database clock. The migration is
`crates/infra-postgres/migrations/<timestamp>_internal_apps.{up,down}.sql`; the timestamp is chosen
after rebasing on the other parity features' migrations.

## Entities and states

### App

`DRAFT_ONLY` (no active version) → `ACTIVE` (pointer set) → `DELETED` (tombstone). A tombstone keeps
its name reserved (it counts towards `apps` until purged by a future retention policy, G1368),
refuses sessions, drafts and publication, and keeps versions, activations and pinned components.

### App version

Immutable. Has a number (1, 2, …), canonical document, digest, release notes, publisher, origin
(`PUBLISH` or `PROMOTE` with source), resolved targets and pinned components.

### App publication request

`PENDING` → `APPROVED` → `PUBLISHED`; from either open state `REJECTED`, `WITHDRAWN`, `SUPERSEDED`
(ADR-0018 §4, as `publication_requests` of feature 011).

### App session

`ACTIVE` → `ENDED` (viewer) | `REVOKED` (activation, audience change, administrator) | `EXPIRED`
(database clock passes `expires_at`; no write needed) → purged 24 h after expiry.

## Tables

| Table | Primary key | Columns (essential) | Constraints and indexes |
|---|---|---|---|
| `apps` | `(tenant_id, id)` | `namespace_id`, `name` (slug 3–63), `title` (≤ 120), `description` (≤ 1,000), `active_version_id` null, `state` (`ACTIVE`, `DELETED`), `created_by`, `created_at`, `deleted_at` | unique `(tenant_id, namespace_id, name)`; deferred composite FK of `active_version_id` to `app_versions` |
| `app_drafts` | `(tenant_id, app_id)` | `revision` bigint ≥ 0, `document` jsonb (≤ 1 MiB canonical), `edited_by`, `edited_at` | CAS on `revision` |
| `app_versions` | `(tenant_id, id)` | `app_id`, `number` int ≥ 1, `document` jsonb, `digest` bytea(32), `release_notes` (≤ 2,048), `origin`, `promoted_from_version_id` null, `published_by`, `published_at` | unique `(tenant_id, app_id, number)`, `(tenant_id, app_id, digest)` non-unique index; ALWAYS trigger refuses UPDATE/DELETE/TRUNCATE; ≤ 1,000 per app checked under the app lock |
| `app_version_targets` | `(tenant_id, version_id, binding_id)` | `kind` (`query`, `update`, `start`, `list_runs`, `run_result`), `workflow_id`, `pinned_workflow_version_id` null, `operation_name` null, `observed_contract_digest` bytea(32) | composite FKs to `app_versions` and `workflows`; immutable (ALWAYS trigger) |
| `app_version_components` | `(tenant_id, version_id, component_version_id)` | `allow_pii_paths` text[] (≤ 64), `frame_level` (`public`, `internal`, `pii`: the highest level the version can pass to this component, computed at publication) | FK to `app_component_versions` **ON DELETE RESTRICT** (ADR-0018 §6 pattern); immutable |
| `app_activations` | `(tenant_id, id)` | `app_id`, `kind` (`PUBLISH`, `PROMOTE`, `ROLLBACK`, `ACTIVATE`), `version_id`, `previous_version_id`, `actor`, `reason` (8–2,000), `request_id` null, `sessions_revoked` int, `created_at` | append-only (ALWAYS trigger); index `(tenant_id, app_id, created_at DESC, id DESC)`; last row is the pointer |
| `app_publication_requests` | `(tenant_id, id)` | `app_id`, `draft_revision`, `digest`, `required_approvals` 0–3, `state`, `requested_by`, `draft_edited_by`, `published_version_id` null, timestamps | partial unique index: one `PENDING`/`APPROVED` request per app |
| `app_publication_decisions` | `(tenant_id, request_id, approver)` | `verdict`, `reason` (8–2,000), `decided_at` | approver ≠ requester and ≠ `draft_edited_by` (checked in the decision transaction) |
| `app_access` | `(tenant_id, app_id, entry_kind, entry)` | `entry_kind` (`SUBJECT`, `ROLE`), `entry` (subject ≤ 255 or built-in role name), `granted_by`, `granted_at` | ≤ 256 per app |
| `app_embed_origins` | `(tenant_id, app_id, origin)` | `origin` exact `https://host[:port]`, ≤ 255 | ≤ 16 per app |
| `app_namespace_policies` | `(tenant_id, namespace_id)` | `custom_components_enabled` bool default false, `embed_origins` text[] (≤ 64), `changed_by`, `changed_at` | no row = defaults |
| `app_components` | `(tenant_id, id)` | `namespace_id`, `name` (slug), `state`, `created_by`, `created_at` | unique `(tenant_id, namespace_id, name)` |
| `app_component_versions` | `(tenant_id, id)` | `component_id`, `number`, `digest` bytea(32), `script` bytea, `style` bytea null, `props_schema` jsonb, `events_schema` jsonb, `byte_length`, `uploaded_by` (the subject whose rights D1399 checks), `uploaded_at` | unique `(tenant_id, component_id, number)` and `(tenant_id, component_id, digest)`; immutable; script+style ≤ 1 MiB |
| `app_sessions` (lock slot 1 of ADR-0050's merged order, D1646) | `(tenant_id, id)` | `namespace_id`, `app_id`, `version_id` null, `draft_revision` null, `subject`, `mode` (`STUDIO`, `EMBEDDED`, `PREVIEW`), `embed_origin` null, `token_hash` bytea(32), `created_at`, `expires_at`, `ended_at` null, `revoked_at` null | unique `token_hash`; exactly one of `version_id`/`draft_revision`; `PREVIEW` ⇔ `draft_revision`; index `(tenant_id, app_id) WHERE revoked_at IS NULL AND ended_at IS NULL`; index `(expires_at)` for purge |
| `app_session_frames` | `(tenant_id, id)` | `session_id`, `component_version_id`, `frame_token_hash` bytea(32), `created_at`, `expires_at` (≤ created + 60 s and ≤ session expiry), `consumed_at` null | unique `frame_token_hash`; single use (consumed by one conditional `UPDATE`); ≤ 256 per session; deleted with the session |

The session-token and frame-token columns hold SHA-256 of 32 random bytes; the raw values exist only
in the response that creates them and in the viewer's memory (a frame URL also in the iframe's
navigation, where it is spent on first use). Lookup by hash is constant-time
relative to the token (index on a digest).

## Invariants

- A version is never updated; its targets, components and document are immutable with it.
- `apps.active_version_id` equals the version of the latest `app_activations` row of the app; both
  are written under the app row lock in one transaction, the row strictly after the previous one.
- A session is usable only if `revoked_at` and `ended_at` are null and `expires_at` is after the
  database clock; its namespace and app match the route; its version still exists.
- A component version referenced by any app version cannot be removed; a tombstoned component
  cannot be added to a new version.
- `custom` components appear in a version only if the namespace policy enabled them at publication;
  disabling the policy refuses new sessions' frames for that namespace (existing published versions
  render the component as unavailable with the reason).
- No column stores a secret, a credential, a bearer token, an authorization code, a binding argument
  or a result.
- A published version's `frame_level` per component never exceeds what its publisher and the
  component version's `uploaded_by` could read when it was published; session creation re-checks
  `uploaded_by` against `frame_level`.
- A frame capability is consumed at most once; a consumed, expired or orphaned capability is never
  served.

## Grants for the runtime role `sagat_app`

| Table | Grants |
|---|---|
| `apps` | SELECT, INSERT, UPDATE |
| `app_drafts` | SELECT, INSERT, UPDATE |
| `app_versions`, `app_version_targets`, `app_version_components`, `app_activations`, `app_component_versions` | SELECT, INSERT (append-only triggers refuse the rest) |
| `app_publication_requests` | SELECT, INSERT, UPDATE |
| `app_publication_decisions` | SELECT, INSERT |
| `app_access`, `app_embed_origins` | SELECT, INSERT, DELETE |
| `app_namespace_policies` | SELECT, INSERT, UPDATE |
| `app_components` | SELECT, INSERT, UPDATE |
| `app_sessions` | SELECT, INSERT, UPDATE, DELETE (purge) |
| `app_session_frames` | SELECT, INSERT, UPDATE (`consumed_at`), DELETE |

The runtime role owns none of them, so it cannot disable the triggers
(`tests/integration/platform-foundation/runtime_privileges` covers every new statement).

## Storage classification (`crates/infra-postgres/storage-classification.json`)

| Class | Columns |
|---|---|
| `content` | `apps.title`, `apps.description`, `app_drafts.document`, `app_versions.document`, `app_versions.release_notes`, `app_activations.reason`, `app_publication_decisions.reason`, `app_access.entry`, `app_embed_origins.origin`, `app_namespace_policies.embed_origins`, `app_component_versions.{script,style,props_schema,events_schema}`, `app_version_components.allow_pii_paths` |
| `protected_system` | names (bounded slugs), digests, token hashes, enums, `app_sessions.subject` (bounded identity), `app_sessions.embed_origin` (validated, ≤ 255) |

Content bytes are charged to `tenant_content_bytes` by the generated triggers of ADR-0027.

## Retention

- Versions, activations, requests, decisions and audit live while the app exists and after its
  tombstone (G1368); the per-app version ceiling bounds growth.
- Sessions and frames: deleted 24 h after `expires_at`, `ended_at` or `revoked_at`, by the leased
  purge loop in batches of at most 1,000 rows per transaction.
- Down migration: refuses while any `apps` or `app_components` row exists.

## Mapping to existing records

- Binding invocations add no table: the Update receipt, the start idempotency record and the run
  journal remain the durable outcome; `app.binding_invoked` audit rows link them to the app.
- The underlying audit row and `app.binding_invoked` share `trace_id`; `resource_id` of the app row is
  `<app-id>@<version>/<binding-id>` (bounded: binding ids are names of ≤ 64 characters).
