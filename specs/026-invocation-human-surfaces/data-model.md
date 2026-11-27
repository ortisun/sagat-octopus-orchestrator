# Data model — feature 026

Status: Proposed — not started. Revised on 2026-10-26 after the architecture and security reviews
(inline stream data, stream budgets, public-link capacity, Slack app bindings, single-use identity
codes, derived receipt tokens, infrastructure-time link validity). Re-baselined on 2026-11-24 against
`develop` (research.md, Decisions 026-004, 026-006, 026-007 and 026-008): `runs.public_link_id` and
database-kept counters instead of `runs.origin` and counting queries; an attempt fence row and no lock on
`steps`; no foreign key from stream tables to `runs`; a storage class of its own for stream bytes; the
purge instant stamped by the purge pass; the `NOT VALID` rule for CHECKs.

Every table is tenant-keyed with composite foreign keys to the tenant's own rows
(`(tenant_id, …) REFERENCES … (tenant_id, …)`), every variable-size column is entered in
`crates/infra-postgres/storage-classification.json` (ADR-0027) and every statement is executable by the
least-privilege runtime role `sagat_app` (checked by `SAGAT_TEST_AS_RUNTIME_ROLE=1`). Migrations are
additive; a down migration refuses while rows it would discard exist (the ADR-0025 pattern). Revised
2026-11-24: migration timestamps are allocated at merge after the newest on `develop`
(`20270119000000_batch_purge` on 2026-11-24); every CHECK added or widened (`destination_kind` gains
`SLACK`, `TEAMS`, `EMAIL` and keeps `REDIS` of feature 033; `kinds` gains the approval kinds) is added
`NOT VALID` and validated by a later migration, the rule design 043 set and
`a_constraint_added_not_valid_is_validated_by_a_later_migration` enforces; no table on the append or
admission path has a foreign key to `runs` or `steps` except `runs.public_link_id` itself. Instants
are classified as in the ADR-0063 draft of feature 028: token validity, retention and rate windows are
infrastructure time; run history is workflow time; no predicate compares the two. Names below are
proposals for the migration review.

## Derived, not stored

- **Form model** — computed per request from the pinned version's carrier (`inputSchema`, an approval
  `form.schema`) by `crates/infra-schema`; cached per process by `(version id, carrier pointer)` with a
  bounded size, like the feature-016 validator cache.
- **Workflow OpenAPI document** — computed per request; `ETag` = SHA-256 of
  `(version digest, generator version)`.
- **Waiter** — an in-process registration `(tenant, run, subject, deadline)`; lost with the process by
  design; the client retries with its idempotency key.
- **Link and receipt status** — `EXPIRED`, `EXHAUSTED` and `AT_CAPACITY` are computed under the link
  row lock; the receipt's coarse status is computed from the run.
- **Receipt token** — recomputed as an HMAC (research D1358); only its hash is stored.
- **Stream control events** — `attempts`, `superseded`, `settled` are computed from committed attempt
  state at read time (FR-026-016); nothing is stored for them.

## Public links

### `public_links`

| Column | Type | Notes |
|---|---|---|
| `tenant_id`, `link_id` | uuid | PK `(tenant_id, link_id)` |
| `namespace_id` | uuid | FK `(tenant_id, namespace_id)` → `namespaces` |
| `name` | text | slug, unique `(tenant_id, namespace_id, name)`, protected (pattern-bounded) |
| `embed_id` | text | 22-character base64url of 128 random bits, **globally unique**, non-secret, in the page path; selects the page headers; never rotated |
| `title`, `description` | text | ≤ 120 / ≤ 1,000 UTF-8 bytes, **content** (shown publicly) |
| `workflow_id`, `version_id` | uuid | FK to the tenant's published version; the pinned version |
| `owner_subject` | text | accountable owner (ADR-0061); protected, bounded like actors |
| `status` | text | `ACTIVE`, `PAUSED`, `REVOKED` |
| `token_hash` | bytea | SHA-256 of the current token, **globally unique** (the lookup key of an anonymous request; the row supplies tenant and namespace) |
| `token_version` | integer | increments on rotation |
| `revision` | integer | optimistic concurrency, equals the head of `public_link_revisions` |
| `exposed_fields` | jsonb | array of ≤ 128 JSON pointers, closed schema, protected |
| `presets` | jsonb | nullable, ≤ 16 KiB canonical, **content**, inline so it is deleted with the link |
| `result_disclosure` | text | `NONE`, `PUBLIC_FIELDS_V1` |
| `allowed_origins` | text[] | ≤ 8 `https://` origins, **content** |
| `expires_at` | timestamptz | required, infrastructure time, ≤ creation + ceiling |
| `max_submissions` | integer | nullable; `submissions_count` integer ≥ 0 |
| `max_concurrent_runs` | integer | required, 1..ceiling; live runs counted, not stored |
| `created_by`, `created_at`, `updated_at` | | audit-safe metadata |

Constraints: `status` closed set; `submissions_count <= max_submissions` when set;
`cardinality(allowed_origins) <= 8`; a `REVOKED` row cannot return to another status (trigger).
Indexes: `(tenant_id, namespace_id, status)` for listing; unique `(token_hash)` and `(embed_id)`.

### `public_link_revisions`

PK `(tenant_id, link_id, revision)`; the complete settings snapshot of each edit (version, exposed
fields, presets, disclosure, origins, expiry, caps, status), `actor`, `request_id`, `created_at`.
Immutable: an `ALWAYS` trigger refuses `UPDATE` and `DELETE` (pattern of
`approval_decision_receipts_immutable`), except the link purge function. A unique
`(tenant_id, namespace_id, actor, request_id)` receipt makes management retries idempotent.

### `public_link_submissions`

| Column | Notes |
|---|---|
| `tenant_id`, `link_id`, `submission_key_hash` | PK; the key (≥ 128 random bits) is generated by the page per opened form |
| `namespace_id`, `link_revision` | the revision the submission was judged against |
| `run_id` | the admitted run; FK `(tenant_id, run_id)` → `runs` `DEFERRABLE INITIALLY DEFERRED`, since the receipt is written in slot 2 and the run in slot 6 of the same transaction (revised 2026-11-24: the live-run count no longer reads this table, Decision 026-004) |
| `body_sha256` | canonical body fingerprint; same key + different body is `CONFLICT` |
| `receipt_token_hash`, `receipt_key_version` | SHA-256 of the derived receipt token (globally unique) and the key version used |
| `created_at`, `purge_after` | infrastructure time; receipt retention (default 30 days, never beyond the run) |

No source address, user agent or value column exists (ADR-0061). Values live only in the run input.

### Runs started through a link

Revised on 2026-11-24 (research.md, Decision 026-004). The first draft gave `runs.origin` a value
`PUBLIC_LINK` and counted live runs by exact queries. Both are wrong on `develop`: `runs.origin` is feature
017's private-run marker (`runs_private_definition_guard` refuses a non-null origin on a workflow that is
not internal), and active runs are read from counters the database keeps with the runs
(`run_active_counts`, `20261017000100_active_run_counts.up.sql`), because counting cost a start one index
entry per active run.

- `runs.public_link_id` uuid null: the link that admitted the run, written in the admission transaction
  and immutable afterwards (a trigger of its own refuses an `UPDATE` that changes it, as the admission
  identity is guarded). A composite foreign key `(tenant_id, public_link_id)` → `public_links`; the
  submission already holds that link row `FOR UPDATE`, so the key check waits on nothing.
- `public_link_active_counts`: PK `(tenant_id, namespace_id, link_id, slot)`, `active` bigint, sixteen
  slots per link, kept by `AFTER INSERT`, `AFTER UPDATE OF finished_at` and `AFTER DELETE` triggers on
  `runs` with `WHEN (NEW.public_link_id IS NOT NULL)` (or `OLD`), so a run without a link pays nothing,
  exactly the shape of `run_active_counts_track()`. A run that ends simply stops being counted.
- Per link: the sum of its slots, read under the link row lock (slot 1). Per namespace
  (`public_link_active_runs`): the sum over the namespace's links, read under the feature-009 quota
  advisory lock of `active_runs` taken with the active-run admission of slot 6 of ADR-0050 decision 7, in
  the existing namespace-then-tenant order.

### Transitions

`ACTIVE ⇄ PAUSED` (`link:manage`), `ACTIVE|PAUSED → REVOKED` (terminal), rotation keeps the status and
the embed id and replaces `token_hash`. Each transition writes the link row, a revision, an audit record
and, for creation, the `public_links` quota count in one transaction. A submission locks the link row
`FOR UPDATE` (counter, state, expiry, live-run count) in slot 1 of ADR-0050 decision 7, before the
receipt (slot 2) and the owner's membership and namespace row (slot 4a), so a concurrent edit either
commits first and is seen, or waits.

## Workflow Streams (ADR-0068)

### `run_stream_heads`

PK `(tenant_id, run_id)`; `namespace_id`; `next_offset` bigint (starts at 1); `author_events`,
`author_bytes`, `reserved_events`, `reserved_bytes` (pool usage); `last_appended_at`;
`terminal_seen_at` timestamptz null (infrastructure time, stamped by the purge pass the first time it
sees the run terminal; rows are purged after it plus the effective stream retention). Created by the
first append of a run (`INSERT … ON CONFLICT DO NOTHING`, then `SELECT … FOR UPDATE`).

Revised on 2026-11-24 (Decisions 026-006 and 026-008): no foreign key to `runs` (an insert would take
`KEY SHARE` on a row the engine locks `FOR UPDATE`, the reason feature 041's tables have none); integrity
is kept by the purge and erasure paths, which delete stream rows with or before the run's, and by a
consistency test. The purge instant is no longer written by the run's terminal transaction.

### `run_stream_attempts` (added 2026-11-24, Decision 026-006)

PK `(tenant_id, run_id, node_id, instance_path, attempt, generation)`; `namespace_id`; `closed_at`
timestamptz null; `closed_by` text (`FINAL_BATCH`, `SETTLED`, `SUPERSEDED`, closed set). Inserted by the
first append of an attempt; locked `FOR UPDATE` by every append of that attempt; closed by a batch with
`final: true`, or by a feed read or the maintenance pass once the committed step state shows the attempt
settled or superseded (an `UPDATE … WHERE closed_at IS NULL`, which waits for an append that holds the
row). A feed sends `settled` or `superseded` for an attempt only after its row is closed. No foreign key to
`runs` or `steps`; leasing, heartbeat and completion never read or write it. Deleted with the head.

### `run_stream_channel_usage`

PK `(tenant_id, run_id, channel)`; `events`, `bytes`. One row per channel used by the run (≤ 32 author
channels plus the reserved channels of the version), updated in the append transaction under the head
lock; deleted with the head.

### `run_stream_events`

| Column | Notes |
|---|---|
| `tenant_id`, `run_id`, `stream_offset` | PK; offsets are gap-free per run |
| `namespace_id` | |
| `channel` | declared channel name, pattern-bounded, protected |
| `node_id`, `instance_path` | the step instance (the existing canonical scoped identity encoding of feature 014; root = empty) |
| `attempt`, `generation` | of the publishing lease |
| `publisher_seq` | bigint ≥ 0, chosen by the publisher per attempt |
| `data` | jsonb, canonical, ≤ 64 KiB, inline: charged by the ADR-0027 triggers on insert and released on delete, to the storage class **`workflow_stream`** and the tenant quota `tenant_stream_bytes` (revised 2026-11-24, Decision 026-007; it was `content` under `tenant_content_bytes`) |
| `data_sha256` | bytea, for replay comparison without reading `data` |
| `published_at` | infrastructure time at append |

Unique `(tenant_id, run_id, node_id, instance_path, attempt, generation, publisher_seq)` is the
deduplication key (FR-026-015). Index `(tenant_id, run_id, channel, stream_offset)` serves channel
filtering. Immutable except for the purge: an `ALWAYS` trigger refuses `UPDATE`; `DELETE` is allowed
only to the purge function, which also deletes the head and the channel usage rows.

### Transactions

Append (one transaction per batch; revised 2026-11-24, Decision 026-006): validation by the design-043
validator happens before the transaction; then lock the attempt row of `run_stream_attempts` `FOR UPDATE`
(refuse when closed); read the step's lease **without a lock** and check token hash, generation, attempt,
session and expiry against the database clock sampled after that read; check the node's `emits`; lock
`run_stream_heads` `FOR UPDATE`; for each event, look up the deduplication key (replay returns the stored
offset; a different `data_sha256` is a conflict), else check the channel budget, its pool and the per-run
ceiling, insert the event, update the channel usage and pool counters and advance the head; with `final`,
close the attempt row; commit. The deferred ADR-0027 accounting charges `workflow_stream` at commit and
never takes the tenant usage row before it. Neither the run row nor the step row is locked; completion
and failure do not touch the stream tables. An append that read the lease before a completion committed
either commits before the feed closes the attempt (and its events precede `settled`) or is refused after.

Purge (one run per transaction): the pass first stamps `terminal_seen_at` on heads of terminal runs that
have none (one statement per batch of heads), and closes their open attempt rows; then it locks a head,
deletes events, attempt rows, channel usage and head when `terminal_seen_at` plus the retention passed
(infrastructure time), and lets the accounting triggers release exactly the bytes
charged on insert; audit nothing (retention is configuration, not an action). The integration test
compares the tenant's content bytes before the first append with the value after the purge.

## Approvals over channels

### Additive columns

- `approval_requests.summary_digest` bytea null, FK → `payloads` (run content, retained and removed
  with the run's data like every other step value); `summary_status` text (`NONE`, `EVALUATED`,
  `FAILED`); `summary_failure` text null (closed code). Written in the opening transaction.
- `approval_decision_receipts.input_digest` bytea null, FK → `payloads`; `via_channel` text null
  (`SLACK`, `TEAMS`, `EMAIL`, `API`, derived by the server from a verified link, never from the
  request) and `via_link_id` uuid null. The `request_hash` covers the input only when one is present:
  a request without input hashes exactly as before the feature (compatibility test). Receipts stay
  immutable.

### `approval_decision_links`

| Column | Notes |
|---|---|
| `tenant_id`, `link_id` | PK |
| `namespace_id`, `request_id` | FK → `approval_requests` (tenant-scoped) |
| `delivery_id` | FK → `webhook_deliveries` (tenant-scoped); fixes the channel the link was sent through |
| `token_hash` | SHA-256 of the full token, globally unique |
| `mac_key_version` | integer; verification tries the current and the previous key |
| `status` | `ACTIVE`, `SETTLED`, `EXPIRED`, `REVOKED` |
| `valid_until` | infrastructure time: creation + min(approval timeout, 30 days) |
| `created_at`, `closed_at`, `purge_after` | closed + 7 days |

A link is usable only while `status = ACTIVE`, `valid_until` has not passed and its request is
`PENDING`. Opening inserts one row per delivery; settlement (decision, expiry, cancellation) closes every
link of the request in the settlement transaction. No recipient address is stored here.

### Approval lock order

Unchanged from feature 014 (applicable chain → run → request); the new rows are written after the
request row in the same transaction. Notification fan-out reads the channel rows `FOR KEY SHARE` and
never waits on a delivery row.

## Notification channels

Physical rows stay in the historical outbound tables (ADR-0025 decision 8):

- `webhook_subscriptions.destination_kind` gains `SLACK`, `TEAMS`, `EMAIL`; `destination_settings`
  holds the typed non-secret settings (e-mail recipient list ≤ 50, classified **content**, never
  logged, returned only to `event:manage` holders); `secret_name` holds the Teams Workflows URL or the
  optional SMTP credential binding; the existing `name` is the channel name referenced by `notify`.
- New columns: `max_classification` text not null default `'public'` (`public`, `internal`, `pii`;
  `secret` refused by CHECK); `classification_set_by` text null (the administrator whose reveal grant
  backs a `pii` ceiling; rechecked at each delivery carrying a `pii` field, and the channel is paused
  with `disabled_reason = PII_GRANT_REVOKED` when it no longer holds, the uniform egress rule of
  feature 020's O1081); `run_filter` jsonb null (≤ 32 workflow names and a subset of terminal
  statuses, closed schema); for `SLACK`: `slack_app_binding_id` uuid (FK, tenant-scoped),
  `slack_team_id` text (copied from the binding, never from the request), `slack_channel_id` text,
  `interactive` boolean.
- A **global** unique partial index `(slack_team_id, slack_channel_id) WHERE interactive` makes an
  interactive Slack channel belong to one tenant. It is the one deliberate cross-tenant constraint; a
  violation answers `CONFLICT` (`CHANNEL_ALREADY_BOUND`) with no information about the other tenant.
- `kinds` CHECK grows to `run.finished`, `ops.alert.fired`, `ops.alert.resolved`,
  `approval.requested`, `approval.settled`.
- `webhook_deliveries.provider_ref` text null (≤ 128 bytes, protected: the Slack `ts`), written in the
  attempt transaction that recorded the acknowledgement. The delivery `payload` stays the closed
  identifier envelope (ADR-0027 protected class); rendered message text is never stored.

### `slack_app_bindings`

PK `(tenant_id, binding_id)`; `namespace_id`; `name` (unique per namespace); `bot_token_secret` and
`signing_secret_secret` (secret names, ADR-0035 authority); `team_id` text (from `auth.test` at
registration and on each re-verification); `status` (`ACTIVE`, `PAUSED`, `INVALID` when `auth.test`
fails or returns another team); `verified_at`; `created_by`, `created_at`. The binding id is in the
interaction URL. Management writes the row, its audit and the quota count in one transaction after the
`auth.test` call (an external read outside the transaction; a changed team id refuses the update).

## Chat identity links (P3)

- `chat_link_codes`: PK `(tenant_id, code_hash)`; `namespace_id`; `subject`; `binding_id` (the binding
  the code may be used through); `expires_at` (infrastructure time, 10 minutes); `consumed_at` null.
  The code carries ≥ 128 random bits and is shown once. Consumption is a `SECURITY DEFINER` function
  that sets `consumed_at` only where it is null and unexpired and the binding's team equals the
  request's team, returning the row; `sagat_app` has no direct `UPDATE` (single use is a database
  property, not a convention).
- `chat_identity_links`: PK `(tenant_id, link_id)`; `subject`; `binding_id`; `provider` (`SLACK`);
  `team_id`, `external_user_id` (content, bounded); `status` (`PENDING`, `ACTIVE`, `REVOKED`);
  `created_at`, `confirmed_at`, `revoked_at`. Unique `(tenant_id, provider, team_id, external_user_id)`
  where not revoked; at most the ceiling per subject. `PENDING → ACTIVE` only by the subject in a
  signed-in session (`confirmed_at`); revoked automatically when the subject's membership is removed
  (same transaction as the membership change, like reveal grants in feature 016).

## Registry entries (coordinator decisions C1, C7, C8)

- **Feature 018 reset-rule registry** (`specs/018-schedules-bulk-operations/contracts/journal-and-events.md`):
  this feature adds no journal event type. Stream events are not journal events and are **not copied**
  by a reset: the new run starts with an empty log. The optional decision input carried by the existing
  `approval.decided` event is `COPY`. The side rows of `approval.opened` (summary, deliveries, decision
  links) are `REMATERIALIZE`: they belong to the source run's request; if a reset reopens an approval,
  the opening path of this feature creates new ones (and, for a draft test run of feature 022, none).
- **Feature 020 content-store registry**: `run_stream_events` (+ `run_stream_channel_usage`,
  `run_stream_attempts`, `run_stream_heads`) — purged at run end (as the purge pass first sees it) +
  `stream_retention_seconds`, never after the run's content;
  erasure of the run deletes them at once. `public_links.presets` and `public_link_revisions` — deleted
  with the link. `public_link_submissions` holds no values. Summary and decision-input payloads are run
  content purged with the run under feature 020.
- **Feature 031 encoded-field registry**: read positions — the derived start form and public-link
  exposure of `inputSchema`; write path — public submissions into the run input; carriers — stream
  channel schemas and the approval form schema; expression positions — approval summary `value`s.
  Codes are 031's (`ENCODING_READ_BY_FORM`).
- **Feature 032**: the stream cursor carries the cell epoch, which is read from the cell state and not
  stored per run; `SAGAT_LINK_KEYS` is an entry of the cell key inventory.

## Quota and policy resources

| Resource | Scope / kind | Default | Notes |
|---|---|---:|---|
| `public_links` | namespace count | 0 | zero refuses creation (enables the feature, D1336) |
| `public_link_active_runs` | namespace count (sub-pool of `active_runs`) | 20 | sum of `public_link_active_counts` of the namespace (revised 2026-11-24) |
| `tenant_stream_bytes` | tenant storage quota of the class `workflow_stream` | the operator's default, as `tenant_step_log_bytes` | added 2026-11-24, Decision 026-007 |
| `public_link_submissions_per_minute` | namespace durable token bucket | 60 | ADR-0027 rate pattern |
| `notification_channels` | namespace count | 20 | chat/e-mail destinations only |
| `slack_app_bindings` | namespace count | 2 | |
| `run_wait_ms` | namespace policy | 30,000 | clamp of `waitMs` |
| `stream_retention_seconds` | namespace policy | 604,800 | after run end |

All resolve namespace → tenant default → operator ceiling and are audited through the existing quota
administration.

## Storage classification additions

`workflow_stream` (a class of its own, added 2026-11-24 with an ADR-0027 amendment, as feature 041 added
`step_log`): `run_stream_events.data`.
`content`: `public_links.title`, `.description`, `.allowed_origins`, `.presets`;
`public_link_revisions` snapshot; `chat_identity_links.team_id`,
`.external_user_id`; `webhook_subscriptions.destination_settings` for e-mail recipients (already
content); the payloads referenced by `summary_digest` and `input_digest` (the `payloads` table is
already classified). `protected_system`: names, statuses, embed ids, pointer arrays (closed schema),
hashes, `provider_ref`, `channel`, `via_channel`, `run_filter`, Slack team and channel ids of bindings
and subscriptions (bounded identifiers). The manifest test fails on any unlisted variable-size column.

## Grants for `sagat_app`

`SELECT, INSERT, UPDATE` on `public_links`, `run_stream_heads`, `run_stream_attempts`, `run_stream_channel_usage`,
`approval_decision_links`, `slack_app_bindings`, `chat_identity_links`; `SELECT, INSERT` on
`public_link_revisions`, `public_link_submissions`, `run_stream_events`, `chat_link_codes`;
`EXECUTE` on the `SECURITY DEFINER` functions for code consumption and for the purges of stream rows,
submissions, link revisions and decision links, each with a fixed `search_path`, tenant and identity
filters and `PUBLIC` execution revoked (feature 017 pattern). Privilege tests run every new statement
as `sagat_app`.

## Retention

| Data | Kept until |
|---|---|
| Stream events, channel usage and head | run terminal + `stream_retention_seconds` (default 7 days, ceiling 30 days), never beyond the run; data deleted with the row |
| Submission receipts | 30 days after creation, never beyond the run |
| Link revisions and presets | as long as the link plus the audit retention |
| Decision links | closed + 7 days |
| Summary and decision input payloads | with the run (run content) |
| Chat link codes | consumption or expiry (10 minutes), then purged |
| Messages already sent to chat or e-mail | outside the platform (G1330) |
