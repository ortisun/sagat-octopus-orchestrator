# Feature 020 — Research: baseline, sources, decisions, gaps and owner questions

Baseline investigated on 2026-10-26 in the worktree `sagat-octopus-parity-specs` (branch
`feature/018-032-parity-specs`, cut from `origin/develop` at `70f09435`, release 0.2.0). Nothing in this
document is implemented. Identifiers: decisions D1080–D1119, gaps G1080–G1119, owner questions
O1080–O1119 (this feature's allocation).

## 1. What exists today (verified paths)

| Area | Evidence | What it means for 020 |
|---|---|---|
| Run listing | `crates/api-http/src/routes/execution.rs::list_runs`, `crates/application/src/runs.rs::RunFilter` (`workflow`, `status`, `origin`), OpenAPI `listRuns` (`GET /runs`, cursor, limit 1..200) | The only filters are workflow, status and origin. No search attribute, no query language, no count. 018 must keep working on `RunFilter` when 020 is absent. |
| Run row | `crates/infra-postgres/migrations/20261009000100_execution.up.sql` (`runs`, `run_events`, `steps`, `payloads`) | `runs.input_digest` is `NOT NULL` with a foreign key to `payloads`; `run_events`, `steps` reference `runs` without cascade; payloads are content-addressed per tenant and shared between runs. Run ids are UUIDv7 (`crates/domain/src/macros.rs`). |
| Telemetry note | `crates/infra-postgres/migrations/20261015000100_telemetry_projection.up.sql` (`telemetry_pending`, `engine_leases`), writers in `crates/infra-postgres/src/{execution.rs,execution/path.rs,chain_progress.rs,loop_progress.rs,approval_progress.rs,primitives.rs}`, store `crates/infra-postgres/src/telemetry.rs`, port `crates/application/src/telemetry.rs::TelemetryStore`, loop `crates/scheduler/src/telemetry.rs` | One upsert per run-changing transaction (ADR-0017). One projector at a time (`engine_leases` row `telemetry-projector`, 30 s). Notes of finished, projected runs are purged after 24 h (`apps/execution-engine/src/lib.rs`, `purge_projected`). `derive`/`attempts` are pure functions of `RunFacts` + journal. |
| Shards | `crates/infra-postgres/migrations/20261017000000_scheduler_shards.up.sql` (`run_shard(uuid)`, `scheduler_shards`), `crates/domain/src/sharding.rs` | 64 fixed shards, owner/generation/expiry on the database clock, fence read `FOR SHARE` inside each action (ADR-0020). |
| Classification | `specs/016-payload-classification/spec.md` FR-016-001..029, `crates/application/src/{classification.rs,payload_disclosure.rs,reveal_grants.rs}`, migration `20261201000000_payload_classification` (`workflow_versions.classification_rules IN (0,1)`) | Annotation `x-sagat-classification`; carriers are exactly workflow `inputSchema`, `outputSchema` and node `outputSchema` (FR-016-002); annotation elsewhere is `CLASSIFICATION_NOT_APPLICABLE`. `payload:read-internal` (developer, operator, admin) and the per-namespace `payload:reveal-pii` grant list. |
| Declared disclosure | ADR-0031, `$defs/resultSchema` and `$defs/query` in `specs/001-platform-foundation/contracts/workflow.schema.json` | Closed bounded result schemas and an explicit `disclosure` constant are the precedent for "values leave only by declaration". The evaluator has no object literal (ADR-0032 decision 7). |
| Outbound pipeline | ADR-0025, `crates/application/src/outbound.rs`, `webhook_deliveries` (payload ≤ 65,536 bytes, CHECK) | Typed destinations, `credentials_secret`, one worker for rate/lease/retry/DEAD/redrive. |
| Blob store | ADR-0023, `crates/infra-blobstore` (`object_store` 0.14.2, S3/R2/fs/memory) | Claim-check descriptors; "committed blobs are never deleted … retention … not decided (G69)". |
| Retention gaps | `specs/002-durable-execution/research.md` G28 (journal/payload grow without bound, Open), `specs/010-scalability-ha/research.md` G241 (journal not partitioned or archived), G251 (object-store backup not covered) | No run, payload or blob is ever deleted today. |
| Quotas | ADR-0027, `crates/domain/src/quota.rs`, `crates/infra-postgres/storage-classification.json` | Byte resources, policy resolution namespace → tenant → operator ceiling, manifest of every variable-size column. |
| Egress | `crates/infra-egress/src/lib.rs` (`EgressClient`, guarded resolver, no redirects/proxies) | Native sinks must use it; `object_store` brings its own `reqwest` client (see G1083). |
| Permissions | `crates/domain/src/identity.rs` (`Permission::ALL` = 48, roles admin, developer, operator, viewer, worker, runner, database-runner) | New permissions and one machine role are additive. |
| Alerts | `crates/domain/src/operations.rs::AlertKind` (5 kinds), monitor `crates/scheduler/src/monitor.rs` | New kinds are additive; conditions are found by looking (ADR-0017 §6). |
| Studio | `frontend/apps/studio/src/features/runs/RunsPage.tsx` | Runs page has the three filters only. |
| CLI | `apps/cli/src/args.rs` (`workflow runs`, `workflow inspect`) | Query and history export are additive flags/commands. |
| Worker protocol | `specs/001-platform-foundation/contracts/worker.proto` (latest additive minor 1.6, `ResolveDatabase`) | A pull exporter needs a new service in the next free minor. |
| Database | `infra/docker/compose.yaml` (`postgres:16-alpine`) | `xid8`, `pg_current_xact_id()`, `pg_current_snapshot()` and `pg_snapshot_xmin()` exist (PostgreSQL ≥ 13). |

## 2. External semantics consulted (2026-10-26)

- Temporal search attributes, <https://docs.temporal.io/search-attribute>: typed (Keyword, KeywordList,
  Text, Int, Double, Bool, Datetime), reserved default attributes, set at start and upserted from
  workflow code; per-namespace count limits for SQL visibility and Temporal Cloud; a 2 KB value and
  40 KB per-execution default; values are **not** processed by a payload codec and the page warns not
  to put PII in them; the Visibility store updates after the state change.
- Temporal list filter, <https://docs.temporal.io/list-filter>: `=, !=, >, >=, <, <=, AND, OR, BETWEEN,
  IN, STARTS_WITH, IS NULL`; Text is word-level with OR token matching; `ORDER BY` not supported in
  Temporal Cloud; `CountWorkflow` for counts; paginate with a token.
- Temporal archival, <https://docs.temporal.io/temporal-service/archival>: histories and visibility
  records of closed executions copied to a blob store asynchronously; experimental; data can exist in
  both stores until retention cleanup.
- Temporal Cloud history export, <https://docs.temporal.io/cloud/export>: hourly, closed workflows
  only, protobuf files, **at least once**, consumers must expect duplicates.
- Windmill jobs, <https://www.windmill.dev/docs/core_concepts/jobs>: job run details retained a
  maximum of 30 days on the open-source edition, configurable retention; job labels as JSON.
- PostgreSQL 16 snapshot functions, <https://www.postgresql.org/docs/16/functions-info.html>: every
  transaction id below a snapshot's `xmin` is committed or rolled back; `xid8` does not wrap.
- ClickHouse `ReplacingMergeTree`,
  <https://clickhouse.com/docs/engines/table-engines/mergetree-family/replacingmergetree>:
  deduplication only at merge time, `FINAL` for exact reads; insert deduplication on retries,
  <https://clickhouse.com/docs/guides/developer/deduplicating-inserts-on-retries>:
  `insert_deduplication_token`, finite deduplication window, `non_replicated_deduplication_window`
  defaults to 0.

What does not transfer: Temporal's visibility is a separate, eventually consistent store fed by
history tasks; this platform keeps PostgreSQL as the authority, so 020 indexes search attributes in
the same transaction as the run (D1081). Temporal concepts tied to code replay (search attribute
upserts from deterministic workflow code) become declarative IR expressions here (D1082).

## 3. Decisions

### Search attributes and queries

- **D1080 — Namespace registry, version declarations.** A name has one type and one classification
  level per namespace. A workflow version declares the names it uses; publication registers a new name
  in the publication transaction (within the namespace ceiling). A declaration whose type or level
  differs from the registry is refused (`SEARCH_ATTRIBUTE_TYPE_CONFLICT`, `…_CLASSIFICATION`). The
  registry level may be tightened (applies at once to every run) and never loosened, so no edit reveals
  values of runs that already ran (ADR-0033 decision 4). `secret` cannot be declared.
- **D1081 — Values are transactional run state.** The index rows and a journaled
  `search_attributes.upserted` event commit in the transaction of the transition that sets them; the
  rows equal a fold of those events (`verify_run`). Rejected: an asynchronous visibility projection
  (Temporal): a dry-run count of 018 would disagree with the batch it selects.
- **D1082 — Sources.** At start: explicit caller values and declared `from` expressions over the run
  input. Afterwards: a node-level `upsertSearchAttributes` map of expressions evaluated by the engine
  over the node's committed output in its completion transaction. No worker-protocol change. Values are
  per run: a continuation successor computes its own from its input (no implicit carry-over).
- **D1083 — Invalid derived values do not fail the run.** A derived value of the wrong type or size is
  not indexed; `search_attributes.rejected` (closed reason, no value) is journaled and counted.
  Explicit caller values that do not validate refuse the start with `VALIDATION_FAILED`.
- **D1084 — Storage.** One row per (run, attribute, element) in `run_search_attributes`, one typed
  value column, partial b-tree per type on `(tenant, namespace, attribute, value, run)`, a `simple`
  `tsvector` GIN for `text`. Rejected: a wide slot table (every update rewrites every slot index) and a
  `jsonb` GIN (no range or ordered scan).
- **D1085 — System attributes** are PascalCase reserved names mapped to `runs`/`workflows` columns;
  custom names match `^[a-z][a-zA-Z0-9_]{0,63}$`.
- **D1086 — A closed query grammar** (contracts/list-query.md) parsed into a typed AST in
  `crates/domain`, compiled to parameterized SQL in `crates/infra-postgres`; never string interpolation;
  sent in a POST body, never in a URL (ADR-0031 decision 6).
- **D1087 — Classification gates predicates, sort and display.** Using an attribute in a predicate or
  sort requires the right to see its level: `internal` needs `payload:read-internal`; `pii` needs the
  reveal grant plus an explicit `reveal: ["pii"]` and is audited per request, never collapsed.
  A predicate is an existence oracle, so it is treated as a disclosure.
- **D1088 — Keyset cursors** over (sort value, run id), opaque and bound to namespace, query digest and
  sort; reuse with another query is `BAD_REQUEST`/`CURSOR_MISMATCH`.
- **D1089 — Bounded count and a selector port for 018** (revised 2026-10-27, review S9).
  `countRuns` is exact up to a ceiling and says when it is not. `application::run_search::RunSelector`
  (contracts/list-query.md, co-owned with 018) selects keyset pages per round under the job creator's
  current rights, previews a count bounded by the caller's cap (018: `maxItems + 1`, at most 100,001),
  has no total ceiling of its own, and audits `pii` predicates once per round. `BusinessId` and
  `ScheduleId` are system attributes once 018 exists. 018 keeps `RunFilter` when 020 is absent.
- **D1090 — Saved views run with the viewer's rights.** A view the viewer cannot fully evaluate is
  refused (`VIEW_NOT_EVALUABLE`), never silently broadened by dropping a predicate.
- **D1091 — Two new classification carriers.** Search attribute declarations (a `level` field) and
  `analytics[].schema` (`x-sagat-classification`). A value derived by an expression takes at least the
  strictest level of every carrier path the expression reads; an undeclared or unresolvable path counts
  as `pii`. Revised 2026-10-27 (coordinator C6): 020 only states its own two carriers in the spec's
  "Amendment to feature 016" subsection; the single amendment of FR-016-002 and of a new classification
  rules version (rules 1 plus the consolidated carriers of every draft, kept in
  `SPECIFICATION-INDEX.md`; FR-016-005, D661) is opened by the first of those features to be
  implemented. 020 does not claim that rules version on the others' behalf; rules 0 and 1 keep their
  meaning.

### Analytics projection (ADR-0053)

- **D1092 — Generalize the note, keep its name** (revised 2026-10-27, review S1, S3).
  `telemetry_pending` gains `namespace_id`, `moved_xid xid8` and `moved_at`, set by a trigger when
  `head_seq` moves, with an index led by `(tenant_id, namespace_id, shard)`. No writer changes and no
  new application statement — but the trigger and the index entry *are* write-path cost, so they are
  installed only when an operator enables analytics (`control-plane analytics enable`, migration role),
  and the fitness check is WAL bytes, index-tuple inserts and commit latency per transition against a
  baseline, not a statement count. The physical name stays (precedent: ADR-0025 decision 8).
- **D1093 — Discovery by transaction horizon.** A sink shard reads notes with
  `moved_xid ∈ [cursor, pg_snapshot_xmin(pg_current_snapshot()))` in keyset order
  `(moved_xid, run)` inside its own `(tenant, namespace, shard)` prefix, so a sink never reads another
  tenant's notes (review S3). Below `xmin` every writer has finished, so no committed note can appear
  behind the cursor. Rejected: sequence or timestamp watermarks (commit order differs from allocation
  order: silent loss); per-sink dirty rows written by the transition (a write-path fan-out); a derived
  feed table in PostgreSQL (a second copy of declared values in the authority).
- **D1094 — Shards reuse scheduler ownership, behind a gate** (revised 2026-10-27, review S2). The
  replica that holds scheduler shard *k* projects shard *k*. The fence is read `FOR SHARE` in its own
  short statement as the last statement before commit of the plan and ack transactions, never across
  the note scan or sink I/O, so analytics holds the shard row for microseconds. A renewal-skip counter
  and a contention test (16 sinks × 64 shards) gate enabling analytics; if renewals are skipped, a
  separate `analytics_shards` lease table is mandatory (G1100).
- **D1095 — Frozen batch plans.** The window, the per-run sequence ranges and the sink revision commit
  before delivery; delivery re-derives the same records (immutable journal, content-addressed payloads,
  pinned declarations). A retry produces identical bytes, object names and deduplication tokens. One
  open batch per (sink, shard) keeps order.
- **D1096 — Canonical model `sagat.analytics.v1`.** Kinds `run_snapshot`, `step_attempt`,
  `journal_event`, `inbound_event`, `delivery`, `declared_record`, `run_tombstone`; every record is
  immutable under a deterministic key; "latest" is a sink-side view. The model grows only (the
  `docs/event-schema-evolution.md` rule); a breaking change is `v2` beside `v1`.
- **D1097 — Reuse the pure derivations.** `step_attempt` comes from `application::telemetry::attempts`;
  a new pure `application::analytics::derive` produces the other kinds.
- **D1098 — Envelopes, opt-in** (revised 2026-10-27, review S1). `inbox_events` and
  `webhook_deliveries` get `analytics_rev` and `moved_xid` maintained by row triggers and indexes that
  are installed only by `control-plane analytics enable --envelopes`; until then a sink revision asking
  for `inbound_event`/`delivery` is refused (`ENVELOPES_NOT_ENABLED`). Envelope projection runs under one
  `engine_leases` row (not sharded, G1082). Inbound `data` leaves only through an endpoint declaration.
- **D1099 — Declared records.** IR `analytics` on `activity`, `script`, `waitEvent` and `child`
  nodes; `project` is a field→expression map because the evaluator has no object literal. Evaluated
  in the projector from immutable payloads under the pinned declaration and validated against the
  closed record schema; blobs stay descriptors; encrypted (031) or secret-suspect values are withheld
  with a marker record, never dropped silently.
- **D1100 — Sink disclosure** (revised 2026-10-27, review B1, S6). Search attribute values leave only
  when the pinned version's declaration opts in per attribute (`export:
  "declared-analytics-export-v1"`); otherwise the name is exported with a `withheld` marker
  (`NOT_DECLARED`). A sink revision carries `maxLevel` ∈ {`internal`, `pii`}. `pii` needs a
  configuring principal (the disclosure owner) holding `payload:read-internal` and the reveal grant;
  both are rechecked at every batch plan and a revocation auto-pauses the sink. Canonical metadata
  has a fixed level table (data-model §4): `correlationId`, `startedBy`, journal `actor`, envelope
  `subject` and `correlationKey`, and 018's business id are `pii`; the rest is `internal`. The table is
  stricter than `getRun` because the data leaves the platform. Audit refines FR-016-012 explicitly: one
  `analytics.batch_exported` per `PII` batch whose retained run manifest is the per-run evidence
  (recorded in ADR-0053).
- **D1101 — Where sinks run** (revised 2026-10-27, review S5 and the coordinator's ClickHouse
  decision). Object storage and stream deliveries run in the execution engine (ADR-0055 rule 11(b);
  constitution II: no new process without measurement), adapters in `crates/infra-analytics`, through the
  egress guard: public destinations only, platform state endpoints denied. Database sinks — PostgreSQL
  (D1104) and ClickHouse (D1103) — run in `apps/database-runner`, because no engine or control-plane
  process opens a connection to a customer database, whatever the protocol (rule 11(c), ADR-0035). Other
  private destinations use the pull exporter inside the customer network.
- **D1102 — Object storage.** NDJSON+gzip is the baseline open format; Parquet behind a feature.
  Deterministic names `<prefix>/sagat.analytics.v1/<kind>/dt=<UTC plan date>/shard=<k>/<sink>-<shard>-<batch>.<ext>`
  plus a manifest object per batch; PUT, then HEAD verification of size and checksum before ack.
- **D1103 — ClickHouse through the managed database executor** (revised 2026-10-27). A bound
  `database-runner` leases the batches, resolves the sink's settings and credential through
  `ResolveSinkConnection` (017's two-phase path, fenced by the batch lease) and sends fixed,
  platform-generated statements over ClickHouse's HTTP interface: `INSERT … FORMAT JSONEachRow` with
  `insert_deduplication_token = <sink>:<epoch>:<shard>:<batch>:<kind>`, and a `DESCRIBE TABLE` probe as a
  zero-record `PROBE` batch. The destination is checked against the managed executor's destination
  policy. Documented `ReplacingMergeTree(version)` DDL ordered by the record key; exact reads need
  `FINAL` (G1085).
- **D1104 — PostgreSQL sink through the managed database executor** (revised 2026-10-27, review S5;
  the earlier engine-native design diverged from ADR-0035 decisions 3–4). The sink names a 017
  connection (`connectionRef`, revision pinned in the sink revision). Its frozen batches are leased by a
  bound `database-runner` identity over `AnalyticsExportService`, which resolves the connection's
  credential through the 017 two-phase path (`ResolveSinkConnection`, fenced by the batch lease) and
  writes with fixed platform templates `INSERT … ON CONFLICT (tenant_id, key, version) DO NOTHING` in one
  explicit transaction under 017's destination policy. The write is idempotent by key, which is the
  declared effect contract ADR-0035 decision 11 requires for retry. No PostgreSQL client in the engine;
  private networks work through 017's operator-approved destinations; the platform never runs DDL.
- **D1105 — Stream sink reuses ADR-0025.** The ack transaction inserts delivery rows (event id
  `<batch>:<part>`, parts within the existing 64 KiB delivery bound) for a subscription dedicated to the
  sink (it receives no run events of its own, so analytics backlog cannot trip ADR-0025's switch-off for
  operational events); the cursor moves when those rows commit; a `DEAD` analytics delivery auto-pauses
  the sink (`STREAM_DELIVERY_DEAD`) because it would otherwise be a silent skip. Rows of kind
  `analytics.batch` are measured as content in the storage manifest.
- **D1106 — Pull exporter.** A gRPC service `AnalyticsExportService` in `worker.proto` at the next free
  additive minor; machine role `analytics-exporter` (`PULL` sinks) or `database-runner` (`POSTGRESQL` and
  `CLICKHOUSE` sinks); per-sink exporter bindings; batch lease =
  token hash + generation + database-clock expiry; finite record/byte credit (ADR-0029 pattern).
- **D1107 — Failures pause, never skip.** A permanent rejection marks the batch `REJECTED` and the sink
  `AUTO_PAUSED` with a typed reason; transient failures retry with capped backoff and set `FAILING`
  after a threshold. `skip` and `rewind` are explicit, audited operator actions. A sink edit supersedes
  an unacknowledged batch.
- **D1108 — Lag** per (sink, shard) is now − the oldest `moved_at` beyond the cursor; the metric
  `analytics_sink_lag_seconds{sink_kind}` is the maximum per kind; per-sink lag is in the API; the
  monitor raises `ANALYTICS_SINK_STUCK` per namespace.
- **D1109 — Export bytes quota.** Tenant resource `analytics_export_bytes_per_day`; exhaustion makes
  sinks `THROTTLED` (planning stops, nothing is lost).
- **D1110 — Backfill and rewind** scan `runs` in UUIDv7 order per shard from the journal (the
  authority); overlap with live discovery is absorbed by keys and per-run progress. Purged runs are
  out of reach (G1090).
- **D1111 — Export ledger and tombstones.** Acknowledged batches and their run manifests are kept
  (default 30 days) so "which sinks received run X" has an answer; purge and erasure emit
  `run_tombstone` records. Exported data is outside platform deletion.

### Retention, archival and history export (ADR-0054)

- **D1112 — Optional namespace retention policy** (`hotRetentionDays` 1..3650 clamped to an operator
  ceiling, `archive` `REQUIRED|NONE`, `archiveValues` `METADATA_ONLY|UP_TO_INTERNAL|UP_TO_PII`,
  destination `PLATFORM|OBJECT_STORAGE`). Unset means today's behavior: nothing is deleted (O1080).
  Revised 2026-10-27 (review B2): retention changes an obligation of constitution principle I (the journal
  is append-only; no component other than PostgreSQL may hold the only copy of a transition), so it
  needs a MINOR amendment
  of principle I, proposed in ADR-0054 and not made here; `archive = NONE` means deletion with no copy.
  US08/US09 are gated on the accepted amendment PR and O1080/O1082. The purge also requires the policy
  revision it applies to be an audited change by a human principal (review, minor).
- **D1113 — Archive format `sagat-run-archive-v1`**: one gzip NDJSON object per run plus a manifest,
  sharing the history export document model; JSON Schema in contracts; blob bytes are not copied.
- **D1114 — Delete only after verification.** Per-run job `ELIGIBLE → EXPORTING → EXPORTED → VERIFIED →
  PURGING → PURGED`; verification re-reads size and SHA-256; purge runs in bounded chunk transactions per run (the first
  marks the stub, the last records `PURGED`); payload
  rows are deleted only when no reference remains (a concurrent reuse of the same digest receives one
  retryable failure, tested). Revised 2026-10-27 (review S8, 024 B3): references are found from the
  database catalog (every foreign key to `payloads` and to `blobs`, whichever feature added it) plus a
  registry of non-FK copies that carry run provenance (022 cache entries), each with an erasure hook; a
  test fails on an unregistered referencing column. Purge deletes only run-purpose blobs (`STEP_OUTPUT`,
  `RUN_INPUT`); `DATA_SNAPSHOT_PART` blobs (024) are excluded, their retention is ADR-0058's. The stub
  keeps `input_digest` non-null by pointing to a per-tenant tombstone payload (review S7).
- **D1115 — Eligibility.** Terminal; `finished_at + hotRetentionDays` passed on the database clock; the
  whole run tree terminal; a chain segment only after the chain is final; each non-`DELETED` sink
  (paused and auto-paused included, review S4) has applied the run or its hold expired (counted as a
  retention gap on that sink). `COMPENSATION_FAILED`, `PARTIAL`
  and runs with an unresolved unknown outcome or manual review wait for an audited resolution: retention
  must not erase the evidence of an effect nobody has settled (constitution III).
- **D1116 — History export `sagat-run-history-v1`**: bounded synchronous document, modes `STRUCTURE`
  and `DISCLOSED`, `complete` flag, from the hot store or a verified archive; `run:history:export` plus
  the classification rules; consumed by 028 replay tests, whose required fields
  (`specs/028-developer-tooling/contracts/history-replay.md`) are adopted as part of the format.
- **D1117 — Explicit erasure** of a terminal run (`retention:manage`) uses the same pipeline with
  eligibility now; content-addressed payloads still referenced by retained runs survive (stated).
  Revised 2026-10-27 (review S11): erasure does not bypass the unresolved-effect exclusion; a run ended
  `COMPENSATION_FAILED`/`PARTIAL` or with an unresolved outcome is refused (`CONFLICT`/`EFFECT_UNRESOLVED`)
  until its resolution is recorded. Rerun and reset of a purged run are refused `CONFLICT`/`RUN_PURGED`
  after the ADR-0039 identity guard (review S10).
- **D1118 — The run row stays as a stub** (ids, workflow/version, status, times, archive pointer;
  `correlation_id` and 018's `business_id` cleared; `started_by` replaced on erasure; `input_digest`
  pointing to the tenant's tombstone payload). Tree, chain and rerun provenance keep
  their foreign keys. Custom attribute rows are deleted with the content.
- **D1119 — Catalog additions.** `search-attribute:manage`, `run:view:manage`, `run:history:export`,
  `analytics:sink:read`, `analytics:sink:manage`, `analytics:sink:export` (the brief's `sink:export`,
  spelled with the analytics prefix; trivially renamable before implementation), `retention:manage`,
  and the machine role `analytics-exporter`; `database-runner` additionally holds
  `analytics:sink:export`, usable only for `POSTGRESQL` and `CLICKHOUSE` sinks it is bound to.

### Cross-feature decisions (2026-10-27, coordinator round 2)

- **D1615 — Region epoch in every downstream identity (C2).** The cell/region epoch of 032 (0 while 032
  is absent) is part of every analytic record key, of `analytics_batches` (recorded at plan), of every
  object name, of the ClickHouse `insert_deduplication_token`, of the stream event id and of the pull
  batch id. After a failover, records of the lost timeline therefore never collide with the new
  timeline's records of the same sequences. 032 owns the activation rule (projectors and sinks rewind
  to the replicated head and emit `run_tombstone` records for lost ranges); 020 only carries the epoch.
- **D1616 — 020 owns the content-store registry (C8).** `crates/infra-postgres/run-content-references.json`
  lists, besides every catalog foreign key to `payloads`/`blobs`, each content store of the drafts with
  its purge and erasure rule (data-model §6): 018 retained Update arguments, 019 `step_checkpoints`, 022
  `step_cache_entries` (invalidated by `source_run_id`) and `step_cache_hit_provenance`, 025 transcripts
  and request staging blobs, 026 `run_stream_events`, 030 `service_operation_messages`; 024
  `DATA_SNAPSHOT_PART` is listed as excluded (ADR-0058). Each draft registers its store in its own data
  model with a pointer to 020; a store not in the registry fails the migration test.
- **D1617 — One `pii`-egress rule, proposed as a refinement of ADR-0033 (C9, carried by O1081).** A `pii`
  value may leave the platform (020 sink and archive, 025 LLM provider and MCP server, 026 channel, 031
  codec endpoint) only to a destination whose revision declares a ceiling ≥ `pii`, set by an
  administrator holding the reveal grant; the grant is rechecked at each delivery or batch; revocation
  pauses the destination; each delivery or batch is audited; `secret` never leaves. 020's sink
  `maxLevel` and retention `UP_TO_PII` are this rule's 020 instances; 025, 026 and 031 cite it.
- **D1618 — Reset rules of 020's journal events (C1).** In 018's reset-rule registry
  (`specs/018-schedules-bulk-operations/contracts/journal-and-events.md`): `search_attributes.upserted`
  is `REMATERIALIZE` (the reset transaction re-creates `run_search_attributes` by folding the copied
  events, as 018 already states); `search_attributes.rejected` is `COPY` (no side-table row). 020 adds no
  other journal event type. Analytics treats copied events as restored (`restored: true`, D1110/018).
- **D1619 — Visibility and export exclusions (C10, C18, C3).** List queries, counts, `RunSelector` and
  saved views decide run visibility through the 029 evaluator when 029 is enabled (folder visibility
  is applied before classification gating; a view never widens it). Draft test runs (022 origin
  `DRAFT_TEST` and their `test_tree` descendants) are never exported to sinks, never archived as
  production history and are excluded from `RunSelector`. Sink placement follows the cross-feature rule
  written by 021 in ADR-0055 (C3): fixed-shape batches to non-database destinations are sent by the
  engine through the guarded egress client; no engine process opens a customer database socket, so the
  PostgreSQL sink runs in `apps/database-runner` (D1104).

### Iceberg sink (owner decision, 2026-10-27)

D1080–D1119 are all in use, so these take the next free numbers of the shared overflow range.

- **D1670 — An `ICEBERG` sink kind.** The owner decided on 2026-10-27, while specifying feature 035 (Apache
  Iceberg writes and step-output destinations), that step outputs reach their destinations in a mode
  chosen per destination: synchronously inside the run (feature 035) or asynchronously through this
  feature's declared-record export, which gains an Iceberg sink reusing 035's connection kind,
  classification ceiling and effect-key lookup (035 D035-018..020, `docs/owner-decisions.md`). The
  owner's example is this feature's US04: a finished payment flowing down to the lake. FR-020-044,
  contracts/sinks.md, T020-062.
- **D1671 — An inconclusive lookup appends again.** Unlike 035's step writes, which go to manual review
  (FR-035-019), a sink batch may be appended twice when the table history no longer covers it, because
  analytics records are grow-only with a key and version (D1096) and readers already deduplicate by them,
  as documented for the object-storage sink. Duplicates are therefore a reader concern documented in the runbook, not a lost
  guarantee.

## 4. Gaps

- **G1080** — G69 (ADR-0023) and the payload half of G28 are *proposed* closed by ADR-0054; until the
  owner accepts it, committed blobs and payloads stay undeleted.
- **G1081** — Analytics lag cannot be lower than the age of the oldest running write transaction in the
  cluster (the `xmin` horizon). It is measured (`analytics_xmin_horizon_age_seconds`), not bounded
  beyond `idle_in_transaction_session_timeout` and statement timeouts.
- **G1082** — Envelope projection is one lease, not sharded; its rate is a benchmark result. The inbox
  purge (dedupe window, ADR-0012) does not wait for sinks: an event purged before a stuck sink exported
  it is counted in the sink's `retentionGaps`, not delivered.
- **G1083** — `object_store` uses its own `reqwest` client; routing it through the guarded resolver must
  be proven (custom HTTP connector) before customer object-storage endpoints are enabled; until then
  only operator-allow-listed endpoints are accepted.
- **G1084** — Parquet pulls the Arrow/Parquet crates; behind a feature, pinned, advisory-audited.
- **G1085** — ClickHouse deduplication is eventual (merge-time) and the insert-token window is finite;
  duplicates can remain visible without `FINAL`.
- **G1086** — Data in sinks and in customer-owned archive buckets cannot be deleted by the platform;
  tombstones are advisory.
- **G1087** — Audit records and the export ledger keep identifiers (subjects, run ids) after purge;
  audit retention is not part of this feature.
- **G1088** — Blob bytes are not archived in v1; purge deletes them after the descriptor is archived.
- **G1089** — Run stubs accumulate without bound until a stub-retention decision (O1082).
- **G1090** — Rewind cannot replay purged runs from archives into sinks.
- **G1091** — Multi-predicate search cost over large namespaces is unmeasured; ceilings are provisional.
- **G1092** — `text` uses PostgreSQL's `simple` configuration: no stemming, unlike Elasticsearch-backed
  Temporal visibility.
- **G1093** — No retroactive indexing: runs started before a version declared an attribute have none.
- **G1094** — Object-storage sinks have no private-network route (pull exporter only); PostgreSQL and
  ClickHouse sinks reach private networks through the managed executor's destination policy.
- **G1095** — No legal hold per run.
- **G1096** — Sinks are namespace-scoped; no tenant-wide sink.
- **G1097** — The worker-protocol minor (1.7 at the time of writing) may collide with 019/026; it is
  assigned at merge.
- **G1098** — Archives are JSON/NDJSON only; no Temporal-compatible protobuf export.
- **G1099** — Payload deletion needs an indexed reference check on every column that references
  `payloads`; the missing indexes add write cost to hot tables and are a benchmark gate.
- **G1100** — Reusing scheduler shard ownership couples analytics CPU and shard-row share locks to the
  scheduler. A release gate, not a fallback: `scheduler_shard_renewals_skipped_total` must stay zero
  under the contention test, else the separate `analytics_shards` lease table ships first.
- **G1101** — The constitution amendment of principle I (B2) is a proposal in ADR-0054; until an
  amendment PR is accepted, US08/US09 do not start.
- **G1102** — A compromised runtime role can still write a retention policy and audit rows; requiring an
  audited human policy change raises the bar but is not a defense against that compromise.
- **G1103** — 018 must let the purge function clear `runs.business_id` (its immutability trigger); this
  is a dependency contract on 018. (018's reset contract already re-creates `run_search_attributes` for
  restored `search_attributes.upserted` events, D1618.) 018 currently names `NOT_FOUND`/`RUN_PURGED` for a
  reset of a purged source where 020 uses `CONFLICT`/`RUN_PURGED`; the coordinator aligns one of them.
- **G1104** — Rolling engines back below 020 after a purge is supported only with degraded reads (empty
  journals on stubs); tested, but older Studios show purged runs as runs without history.

- **G1615** — Records of a lost timeline that already reached a sink are not deleted; they stay under
  the old epoch and are superseded only by the tombstones 032 requires at activation.
- **G1616** — The content-store registry is only as complete as the drafts' registrations; the migration
  test catches unregistered columns in this repository, not stores outside PostgreSQL (for example
  data already sent to an LLM provider, 025 G1285).
- **G1617** — ClickHouse is not an engine of the 017 connection catalog, so a ClickHouse sink carries its
  own destination settings and secret reference (validated against the managed executor's policy); if a
  later feature adds ClickHouse to the catalog, sinks should reference that connection instead.
  *2026-10-27:* feature 035 proposes a `clickhouse` (and an `object_storage`) 017 connection kind; whether
  the `CLICKHOUSE` and `OBJECT_STORAGE` sinks switch to them is owner question O035-005 of feature 035.

## 5. Owner questions (recommended defaults are what the plan assumes)

- **O1080 — Default retention.** *Recommended:* no default; a namespace without a policy keeps every
  run (today's behavior); operators may set a deployment ceiling. *Alternative:* a finite default (for
  example 30 days, Windmill's open-source maximum), which deletes customer data on upgrade.
- **O1081 — PII leaving the platform (one rule for 020, 025, 026, 031; D1617).** *Recommended:* the
  uniform rule of D1617 as a proposed refinement of ADR-0033 — a destination ceiling ≥ `pii` set by an
  administrator holding the reveal grant, rechecked per delivery or batch, revocation pauses, every
  delivery or batch audited, `secret` never leaves; default ceiling `internal`. *Alternative:* forbid
  `pii` egress entirely in v1.
- **O1082 — Run stubs after purge.** *Recommended:* keep stubs indefinitely in v1 (ids, workflow,
  status, times, archive pointer; no correlation id; subject replaced on erasure) so trees, chains,
  reruns and audit references stay intact. *Alternative:* delete runs completely, relaxing every
  foreign key to them.

## 6. Review revisions (2026-10-27)

Independent architecture reviews (`arch-018-020-022`, 020 section and the 018 items naming 020;
`arch-021-023-024` 024 B3) and the coordinator's decisions were applied: B1 → D1100 and FR-020-002/018;
B2 → D1112, ADR-0054, G1101; S1 → D1092, D1098, SC-020-004; S2 → D1094, G1100; S3 → D1092/D1093;
S4 → D1115; S5 → D1101, D1104, D1106; S6 → D1100 and ADR-0053; S7 → D1114, D1118, G1104;
S8 → D1114; S9 → D1089 and the `RunSelector` contract; S10 → D1117 and `RUN_PURGED`; S11 → D1117;
minors → D1105, D1112, G1102; 024 B3 → D1114 and ADR-0054 decision 6; 018 S6 → G1103 and the
`restored` marker of the analytics model. Round 2 (coordinator decisions C1, C2, C3, C6–C10, C16, C18,
C22): D1091 (revised), D1615–D1619, G1615–G1616; final verification: ClickHouse moved to the managed
executor (D1101, D1103 revised, G1617) and 024's snapshot metadata excluded in the registry; `HISTORY_TOO_LARGE` is HTTP 413; the history export
carries per-attempt output digests. No decision number was added (D1080–D1119 were all in use);
revised decisions say so in place. New gaps: G1101–G1104.
