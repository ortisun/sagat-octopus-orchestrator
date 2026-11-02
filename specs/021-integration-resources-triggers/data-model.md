# Data model — feature 021

Proposed; nothing exists. Revised 2026-10-27 after the architecture and security reviews. Locking
follows ADR-0050 decision 7 (variable fence 4a; resource and variable heads 4b; secrets 5; pins 6;
[plan](plan.md#lock-order-adr-0050-decision-7), D1159). Migration timestamps are allocated at merge (D1151); file names below
are provisional. Every table carries `tenant_id` and, where namespace-scoped, `namespace_id`, with
composite foreign keys to `namespaces(tenant_id, id)` as in 017. The runtime role `sagat_app`
receives only the grants listed. Every new variable-size column is classified in
`crates/infra-postgres/storage-classification.json`.

## Entities and states

### Resource type (code, not a table)

Shipped in the server: `key` (`http_api`, `graphql`, `s3_bucket`, `smtp`, `slack_bot`), `version`
(positive integer, immutable), settings schema (closed, bounded JSON Schema document), slots
(`name`, `shape`: `token`, `username_password`, `client_secret`, `access_key_pair`, `api_key`),
auth schemes, operations, probe, egress profile (`public`; `private` only inside operator-approved
deployment ranges). Older versions remain readable while any revision references them. A type
version is never removed while referenced.

### Resource

`ACTIVE ⇄ PAUSED → DELETED` (terminal tombstone; never revived). Every transition writes a new
immutable revision. `PAUSED` and `DELETED` refuse publication, admission, probe and capability
use; an in-flight request is not recalled.

### Variable

`ACTIVE → DELETED` with revisions for every value change. Levels `public | internal`.

### Run bindings

Created once in the admission transaction; immutable; deleted only with the run (`ON DELETE CASCADE`
from `runs`).

### Step credential pins

One row per resolved slot per attempt and generation; immutable; metadata only. Retained while the
attempt is live or in manual review, and for 24 hours after the run is terminal (017 cleanup rule).

### Connector stream lease (every managed kind)

`unclaimed → held(holder, role, generation, credential version, expires_at) → expired | released |
ended`. A claim increments `generation` and pins the credential version; renewal and every batch
compare holder and generation and recheck connector status, secret ACL and credential version; a
refusal moves the lease to `ended` and the holder must close the source connection (D1138).

## Tables

### `resources`

| Column | Type | Notes |
|---|---|---|
| `id` | uuid PK | |
| `tenant_id`, `namespace_id` | uuid | FK `namespaces(tenant_id, id)` |
| `name` | text | `^[a-z][a-z0-9-]{1,61}[a-z0-9]$`, no `--`; unique `(tenant_id, namespace_id, name)` |
| `type_key` | text | closed set; never changes |
| `current_revision` | integer > 0 | deferred FK to the revision row (017 pattern) |
| `status` | text | `ACTIVE`, `PAUSED`, `DELETED` |
| `created_at`, `updated_at` | timestamptz | DB clock |

Constraints: unique `(tenant_id, namespace_id, id)`; ALWAYS trigger refuses identity/type changes,
revival of `DELETED` and revision jumps other than +1. Index: `(tenant_id, namespace_id, status)`.
Grants: SELECT, INSERT, UPDATE, DELETE. Classification: `name` internal metadata.

Folder placement (029, when present): resources join 029's `item_placements` as an additive
`item_kind = resource` with a `resource_id` column, FK `(tenant_id, namespace_id, resource_id)` →
`resources (tenant_id, namespace_id, id)` (unique exists) `ON DELETE CASCADE`, and a partial unique
index `(tenant_id, resource_id)`; resources are tombstoned and keep their row. The migration is
delivered with 029 or with 021, whichever comes second (T021-078). Variables are not placed.

### `resource_revisions`

| Column | Type | Notes |
|---|---|---|
| `tenant_id`, `namespace_id`, `resource_id` | uuid | FK to `resources` |
| `revision` | integer > 0 | PK `(tenant_id, namespace_id, resource_id, revision)` |
| `type_version` | integer > 0 | pinned type version |
| `settings` | jsonb | object, ≤ 65,536 bytes; never secret |
| `slots` | jsonb | `{slot: secret logical name}`; names only |
| `slot_destinations` | jsonb | `{slot: normalized host[:port]}` derived from the host-bearing settings; compared with the previous revision to decide whether `secret:manage` is required (D1158) |
| `status` | text | requested status of this revision |
| `actor` | text | 1–255 bytes, no control characters |
| `request_id` | uuid | unique `(tenant_id, namespace_id, actor, request_id)` |
| `created_at` | timestamptz | |

ALWAYS trigger refuses UPDATE. Retained-byte triggers charge `settings` and `slots`
(`sagat_quota_row_delta`, as in `20261123000000_database_connections.up.sql`). Grants: SELECT,
INSERT, DELETE. Classification: `settings`, `slots` and `slot_destinations` internal.

### `run_resource_bindings`

PK `(tenant_id, run_id, node_id, role)` where `role` ∈ `forward`, `compensate`, `reconcile`;
columns `namespace_id`, `resource_id`, `revision`. FK `(tenant_id, namespace_id, run_id)` →
`runs` ON DELETE CASCADE; FK to `resource_revisions`. ALWAYS trigger refuses UPDATE. Grants:
SELECT, INSERT, DELETE.

### `step_resource_credentials`

PK `(tenant_id, run_id, path, node_id, attempt, generation, slot)`; `path` is the scoped instance
path array (empty for root, D1126); columns `namespace_id`, `provider_operation` (uuid),
`secret_name`, `secret_version`, `created_at`. FK to the run and step; FK to the provider operation;
no FK to the logical secret (017 rule: logical deletion revokes immediately, evidence stays). No
credential bytes. Grants: SELECT, INSERT, DELETE; ALWAYS trigger refuses UPDATE.

### `variables` and `variable_revisions`

`variables`: `id` PK, `tenant_id`, `namespace_id`, `name` (pattern `^[a-z][a-zA-Z0-9_]{0,62}$` so
that `vars.<name>` is an identifier), `value_type` (`string`, `integer`, `decimal`, `boolean`,
`json`), `level` (`public`, `internal`), `current_revision`, `status` (`ACTIVE`, `DELETED`),
timestamps; unique `(tenant_id, namespace_id, name)`; type and level immutable (a change is a new
variable).
`variable_revisions`: PK `(tenant_id, namespace_id, variable_id, revision)`, `value` jsonb
(≤ 65,536 bytes; `integer`/`decimal` stored as strings), `actor`, `request_id` (unique per actor),
`created_at`; immutable. Classification: `value` follows `level` (`public` or `internal`); never
`pii` or `secret`. Retained-byte triggers on `value`.

### `run_variable_bindings`

PK `(tenant_id, run_id, name)`; `namespace_id`, `variable_id`, `revision`; FKs as above; immutable.

### Changes to existing tables

| Table | Change |
|---|---|
| `runs` | `origin` CHECK gains `RESOURCE_PROBE` |
| `event_endpoints` | `kind` (`WEBHOOK` default, `ROUTE`, `EMAIL`); `route_method`, `route_path_normalized`, `route_shape_key` (path with parameters replaced by `{}`), `route_captures` jsonb (declared headers/query, bounded), `idempotency_mode` (`HEADER`, `GENERATE`), `idempotency_header`; `email_provider` (`MAILGUN`, `GENERIC_RELAY`); `profile` CHECK gains `MAILGUN`. Unique `(tenant_id, namespace_id, route_method, route_shape_key)` where `kind = 'ROUTE'`. Existing rows become `WEBHOOK` with NULLs. |
| `namespace_route_prefixes` (new) | PK `(tenant_id, namespace_id)`; `prefix` text unique (26 base32 characters, random); `previous_prefix`, `previous_valid_until` for bounded rotation overlap. |
| `event_connectors` | `kind` CHECK gains `POSTGRES_CDC`, `MQTT`, `GCP_PUBSUB`, `AZURE_SERVICE_BUS`, `WEBSOCKET`; `placement` (`ENGINE` for NATS/Kafka/SQS, `MANAGED` for every new kind; CHECK ties placement to kind); `filter` text (bounded expression); `filtered_total` bigint; `lag_bytes` bigint NULL; `health_state` (`OK`, `ERROR`, `LAGGING`). |
| `connector_leases` | `generation` bigint NOT NULL DEFAULT 0, `holder_role` (`engine`, `database-runner`, `integration-runner`; CHECK against the connector kind), `credential_version` text NULL, `state` (`HELD`, `ENDED`) for managed stream holders. |
| `connector_offsets` | reused: CDC `stream` = slot name, `next_offset` = end LSN (signed 64-bit; larger refused, D1139). |
| `connector_dead_letters` | `reason` CHECK gains `UNSUPPORTED_TYPE` (CDC column type outside the cell vocabulary). A filter that cannot be evaluated counts as not matching, as trigger filters do today, and is counted as `filter_error`; it is not a dead letter. |
| `inbox_events` | `envelope_extension` jsonb NULL (≤ 16 KiB): `message` (broker key, topic, partition, offset, timestamp) or `request` (route method, path parameters, captured query and headers). For `ROUTE` endpoints `payload_digest` covers body and extension; existing kinds keep a body-only digest. Classification: `pii`, as `payload`. `stream_position` bigint NULL (CDC commit LSN) with partial index `(tenant_id, namespace_id, source, stream_position) WHERE stream_position IS NOT NULL`; the purge loop skips a row whose `stream_position` is above its connector's `connector_offsets.next_offset` (D1139). `correlation_data_derived` boolean NOT NULL DEFAULT false: the correlation key is returned only under the FR-016-026 reveal when true. |
| `quota_limits` | resource CHECK gains `resources`, `variables`, `integration_concurrency`, `tenant_integration_concurrency`, `cdc_connectors`. |

Down migrations refuse while any `resources`, `variables`, bindings, pins, `ROUTE`/`EMAIL` endpoints
or new-kind connectors exist; they never delete customer rows.

## CDC event data (canonical shape)

`data = {op, schema, table, commitLsn, commitTime, xid, new: {column: cell}, old: {column: cell}}`;
`cell` is the 017 typed cell (`null`, `boolean`, `integer` as decimal string, `decimal` string,
`float`, `text`, `timestamp`, `date`, `binary` base64, `json`) or `{"unchanged": true}` for an
unchanged TOAST value. `old` holds the replica-identity columns only. A row whose encoded data
exceeds 240 KiB is a dead letter `TOO_LARGE` holding metadata only (schema, table, LSN).

## E-mail event data (canonical shape)

Event id `doc-` + SHA-256 of this document (provider envelope fields excluded); the event `subject`
and correlation key are not set from it.
`data = {messageId, from, to[], cc[], subject, date, text, html, headers{bounded selection},
attachments[{filename, contentType, sizeBytes, contentBase64?}], provider}`; total within the
inbox bound; attachments without content when they do not fit, with `truncated: true`.

## Reset (018)

`run_resource_bindings` and `run_variable_bindings` are `REMATERIALIZE` side tables in 018's reset-rule
registry: the reset transaction copies them at the same revisions after rechecking current authority;
`step_resource_credentials` is not copied. 021 adds no journal event type
([spec](spec.md#reset-of-runs-that-use-resources-018)).

## Retention

- Revisions referenced by a binding are retained while the run exists; unreferenced revisions follow
  the identity's lifetime (tombstones keep history, 017 rule). Counts are bounded by the ceilings.
- Credential pins: live and manual-review attempts indefinitely; terminal runs 24 hours (017 cleanup).
- Inbox rows and extensions: the existing window and purge (ADR-0012).
- Variable revisions: retained with the variable; tombstoned variables keep revisions referenced by
  runs. Retention of exported or archived copies belongs to 020.
