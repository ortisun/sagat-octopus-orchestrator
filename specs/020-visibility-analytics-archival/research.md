# Feature 020 — Research: baseline, sources, decisions, gaps and owner questions

Baseline investigated on 2026-10-26 in the worktree `sagat-octopus-parity-specs` (branch
`feature/018-032-parity-specs`, cut from `origin/develop` at `70f09435`, release 0.2.0). Nothing in this
document is implemented. Identifiers: decisions D1080–D1119, gaps G1080–G1119, owner questions
O1080–O1119 (this feature's allocation).

## 1. What exists today (verified paths)

| Area | Evidence | What it means for 020 |
|---|---|---|
| Run listing | `crates/api-http/src/routes/execution.rs::list_runs`, `crates/application/src/runs.rs::RunFilter` (`workflow`, `status`, `origin`, and since feature 018 `business_id`, `business_id_prefix`, `schedule`), OpenAPI `listRuns` (`GET /runs`, query parameters `workflowId`, `origin`, `status`, `businessId`, `businessIdPrefix`, `scheduleId`, cursor, limit 1..200) | No search attribute, no query language, no count. 018 must keep working on `RunFilter` when 020 is absent. *Corrected 2026-11-24:* the 018 filters are built. |
| Run row | `crates/infra-postgres/migrations/20261009000100_execution.up.sql` (`runs`, `run_events`, `steps`, `payloads`); later columns `trace_parent_span` (`20261015000000_trace_links`), `rerun_of` (`20261015000300_operator_actions`), `origin` (`20261125000000_database_ad_hoc`, values `AD_HOC_STEP`, `AD_HOC_WORKFLOW`, `CONNECTION_PROBE`, null otherwise), `schedule_id` (`20261230000200_schedule_run_provenance`), `business_id` (`20270103000000_business_ids`) | `runs.input_digest` is `NOT NULL` with the foreign key `runs_input_same_tenant_fk` to `payloads`; `run_events`, `steps` reference `runs` without cascade; payloads are content-addressed per tenant and shared between runs. Run ids are UUIDv7 (`crates/domain/src/macros.rs`). *Corrected 2026-11-24:* `runs` has no chain column: a chain is `execution_chains.chain_id` (the initial run's id) and `chain_segments` (`20261102000000_execution_chains`), so `ChainId` is read through `chain_segments`. `runs` has no generated `shard` column; shard queries use expression indexes on `run_shard(id)`. |
| Telemetry note | `crates/infra-postgres/migrations/20261015000100_telemetry_projection.up.sql` (`telemetry_pending` with exactly `tenant_id`, `run_id`, `head_seq`, `applied_seq`, `behind_since`, foreign key to `runs` `ON DELETE CASCADE`; `engine_leases` with `name`, `holder`, `expires_at`), writers in `crates/infra-postgres/src/{execution.rs,execution/path.rs,chain_progress.rs,loop_progress.rs,approval_progress.rs,primitives.rs}`, store `crates/infra-postgres/src/telemetry.rs`, port `crates/application/src/telemetry.rs::TelemetryStore`, loop `crates/scheduler/src/telemetry.rs` (`TelemetryProjector::run_once`) | One standalone upsert per run-changing transaction (ADR-0017): `INSERT … ON CONFLICT (tenant_id, run_id) DO UPDATE SET behind_since = …, head_seq = GREATEST(…)`; design 038 did not merge it into another statement. One projector at a time (the `engine_leases` row named by the constant `PROJECTOR_LEASE`, created when the lease is first taken, 30 s). Notes of finished, projected runs are purged after 24 h by `purge_projected` (`crates/infra-postgres/src/telemetry.rs`, called from `apps/execution-engine/src/lib.rs` with 24 h and 1,000 rows). `derive`/`attempts` are pure functions of `RunFacts` + journal. *Corrected 2026-11-24:* the purge's location and the lease row. |
| Shards | `crates/infra-postgres/migrations/20261017000000_scheduler_shards.up.sql` (`run_shard(uuid)`, `scheduler_shards`), `crates/domain/src/sharding.rs`, renewal SQL `renew` in `crates/infra-postgres/src/partitioning.rs`, loop `crates/scheduler/src/shards.rs` | 64 fixed shards, owner/generation/expiry on the database clock, fence read `FOR SHARE` inside each action (ADR-0020). Renewal locks with `FOR UPDATE OF s SKIP LOCKED`, so a row another transaction holds in share mode is skipped, and no metric counts a skipped renewal today (only `scheduler_shards_acquired_total`, `_released_total`, `_lost_total`). This is why Decision 020-003 removes the analytics fence. |
| Classification | `specs/016-payload-classification/spec.md` FR-016-001..029, `crates/application/src/{classification.rs,payload_disclosure.rs,reveal_grants.rs}`, migration `20261201000000_payload_classification` (`workflow_versions.classification_rules IN (0,1)`) | Annotation `x-sagat-classification`; carriers are exactly workflow `inputSchema`, `outputSchema` and node `outputSchema` (FR-016-002); annotation elsewhere is `CLASSIFICATION_NOT_APPLICABLE`. `payload:read-internal` (developer, operator, admin) and the per-namespace `payload:reveal-pii` grant list. |
| Declared disclosure | ADR-0031, `$defs/resultSchema` and `$defs/query` in `specs/001-platform-foundation/contracts/workflow.schema.json` | Closed bounded result schemas and an explicit `disclosure` constant are the precedent for "values leave only by declaration". The evaluator has no object literal (ADR-0032 decision 7). |
| Outbound pipeline | ADR-0025, `crates/application/src/outbound.rs`, `webhook_deliveries` (payload ≤ 65,536 bytes, CHECK) | Typed destinations, `credentials_secret`, one worker for rate/lease/retry/DEAD/redrive. |
| Blob store | ADR-0023, `crates/infra-blobstore` (`object_store` 0.14.2, S3/R2/fs/memory), purposes `STEP_OUTPUT` and `RUN_INPUT` (`20261020000000_run_input_blobs`, constraint `blobs_owner_valid`), deletion path `claim_delete`/`finish_delete` (`crates/infra-postgres/src/blobs.rs`) and `sweep` (`crates/application/src/blobs.rs`) | Claim-check descriptors; "committed blobs are never deleted … retention … not decided (G69)". *Added 2026-11-24:* feature 035's `step_lease_blobs` (`20270111000000_lake_blob_pins`) references `blobs` by foreign key, so the catalog discovery of D1114 covers it. |
| Retention gaps | `specs/002-durable-execution/research.md` G28 (journal/payload grow without bound, Open), `specs/010-scalability-ha/research.md` G241 (journal not partitioned or archived), G251 (object-store backup not covered) | No run, payload or blob is ever deleted today. |
| Quotas | ADR-0027, `crates/domain/src/quota.rs`, `crates/infra-postgres/storage-classification.json` | Byte resources, policy resolution namespace → tenant → operator ceiling, manifest of every variable-size column. |
| Egress | `crates/infra-egress/src/lib.rs` (`EgressClient`, guarded resolver, no redirects/proxies) | *Corrected 2026-11-24:* no 020 sink sends from the engine to a customer destination any more (Decision 020-002), so G1083 no longer applies. |
| Connection kinds (035) | `specs/001-platform-foundation/contracts/worker.proto` (`ResolvedDatabase` fields 13 to 15: `IcebergConnection`, `ObjectStorageConnection`, `ClickHouseConnection`), `crates/api-grpc/src/lake_connection.rs`, `apps/database-runner/src/lake/`, `crates/infra-lake`; `specs/035-iceberg-write-steps/plan.md` (profiles `database` and `lake`) | *Added 2026-11-24:* feature 035 adds the 017 connection kinds `iceberg`, `object_storage` and `clickhouse`, resolved to `apps/database-runner` through `ResolveDatabase`. Its `database` profile writes PostgreSQL and ClickHouse (`crates/infra-database`); its `lake` profile (image `lake-runner`) writes Iceberg and object storage. The owner decided on 2026-11-02 (O035-005, D035-070) that 020's `OBJECT_STORAGE` and `CLICKHOUSE` sinks reference these kinds. The Iceberg kind is built; the object-storage and ClickHouse kinds are 035's Phase 11 (tasks T035-106 onwards), not built. |
| Permissions | `crates/domain/src/identity.rs` (`Permission::ALL` = 53 on 2026-11-24, roles admin, developer, operator, viewer, worker, runner, database-runner) | New permissions and one machine role are additive. *Corrected 2026-11-24:* 53, not 48; no permission of this feature exists yet. |
| Instance registry (018) | ADR-0082, `crates/application/src/instances.rs` (`Capability`: `schedules-v1`, `access-narrowing-v1`, `payload-encoding-v1`, `run-input-validation-v1`), `docs/runbooks/instance-registry.md` | *Added 2026-11-24:* a capability every engine and control plane must honour is switched on by one operator command that records a requirement (`platform_requirement_activate`, schema-owning role) after every live process lists the capability. 020 uses it (Decision 020-004). |
| Run creation (043) | `crates/application/src/run_input.rs` (`CheckedRunInput`), design 043 (thirteen creating paths; `PgRunStore::create` and `continue_evaluated` take a `CheckedRunInput`) | *Added 2026-11-24:* declared `from` values are evaluated where the checked input is consumed, so every creating path sets them (Decision 020-005). |
| Step logs (041) | `docs/designs/041-step-logs.md`, ADR-0084, ADR-0085; server on branch `feature/041-step-logs-server` | *Added 2026-11-24:* `step_log_streams` and `step_log_lines` have no foreign key to `runs` (G041-004); a stream expires `step_log_retention_days` (1..365, default 14) after its last batch; reading needs `run:logs:read` and the namespace's PII reveal grant in every namespace (FR-041-013, FR-041-021); "export to a customer's storage and retention beyond 365 days belong to feature 020". `run_log_usage` was dropped (D041-079). Decision 020-007. |
| Reset registry (018) | `specs/018-schedules-bulk-operations/contracts/journal-and-events.md` §"Reset-rule registry", code `crates/domain/src/execution/reset.rs` (`ResetRule`, `rule_of`) | *Added 2026-11-24:* the contract already holds 020's two rows (D1618); `rule_of` does not, and 020 adds them with its event types (T020-015). `RUN_PURGED` is `CONFLICT` (409) in 018's `contracts/errors.md` and in its failure-reason checks; nothing raises it yet (018 G1975 waits on 020). |
| Alerts | `crates/domain/src/operations.rs::AlertKind` (5 kinds), monitor `crates/scheduler/src/monitor.rs` | New kinds are additive; conditions are found by looking (ADR-0017 §6). |
| Studio | `frontend/apps/studio/src/features/runs/RunsPage.tsx` (with `businessId.ts`, `runList.ts`) | Runs page has the structured filters only (status, workflow, origin, business id). The Astryx design system migration (ADR-0101, PR #267) is open; 020's screens follow whatever `develop` holds when they are built. |
| CLI | `apps/cli/src/args.rs` (`workflow runs`, `workflow inspect`) | Query and history export are additive flags/commands. |
| Worker protocol | `specs/001-platform-foundation/contracts/worker.proto` (latest additive minor in the file 1.6, `ResolveDatabase`); register of minors in `sdks/SPEC.md` §9 | A pull exporter needs a new service in the next free minor. *Corrected 2026-11-24:* 1.7 is feature 041's, 1.8 delivery acknowledgement's (ADR-0099, PR #264), 1.9 design 045's; 020 takes the next free minor when its contract merges (1.10 or later at the time of writing; feature 019 names three minors and 031 one). |
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
  *Revised 2026-11-24 (Decision 020-006):* the selector pages as 018's built selection does: by `RunId`
  ascending (018 stores it in `batch_jobs.select_cursor`), over runs created at or before
  `selection_bound_at`, starting once `pg_snapshot_xmin(pg_current_snapshot()) > selection_horizon`.
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
  separate `analytics_shards` lease table is mandatory (G1100). *Revised 2026-11-24 (Decision 020-003):*
  the fence is removed. Shard ownership only decides which replica plans which shard; correctness rests on
  the batch row, so analytics never locks a `scheduler_shards` row and the gate is gone.
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
  private destinations use the pull exporter inside the customer network. *Revised 2026-11-24 (Decision
  020-002):* every sink whose destination is a 017 connection is written by `apps/database-runner`:
  PostgreSQL and ClickHouse by its `database` profile, object storage and Iceberg by its `lake` profile.
  The engine writes only into the platform's own stores (stream delivery rows, platform archives) and
  holds no customer credential.
- **D1102 — Object storage.** NDJSON+gzip is the baseline open format; Parquet behind a feature.
  Deterministic names `<prefix>/sagat.analytics.v1/<kind>/dt=<UTC plan date>/shard=<k>/<sink>-<shard>-<batch>.<ext>`
  plus a manifest object per batch; PUT, then HEAD verification of size and checksum before ack.
  *Revised 2026-11-24 (Decision 020-002):* the sink references an `object_storage` connection of feature
  035 and is written by the `lake` profile with 035's create-only writer (FR-035-040): an existing object
  whose `sagat-records-sha256` equals the batch's is the write's success, so a retry needs equal records,
  not byte-identical files.
- **D1103 — ClickHouse through the managed database executor** (revised 2026-10-27). A bound
  `database-runner` leases the batches, resolves the sink's settings and credential through
  `ResolveSinkConnection` (017's two-phase path, fenced by the batch lease) and sends fixed,
  platform-generated statements over ClickHouse's HTTP interface: `INSERT … FORMAT JSONEachRow` with
  `insert_deduplication_token = <sink>:<epoch>:<shard>:<batch>:<kind>`, and a `DESCRIBE TABLE` probe as a
  zero-record `PROBE` batch. The destination is checked against the managed executor's destination
  policy. Documented `ReplacingMergeTree(version)` DDL ordered by the record key; exact reads need
  `FINAL` (G1085). *Revised 2026-11-24 (Decision 020-002):* the sink references a `clickhouse` connection of
  feature 035 (O035-005), so it carries no URL or secret of its own and `ResolveSinkConnection` answers the
  existing `ResolvedDatabase` with its `clickhouse` field; the writer is 035's (`crates/infra-database`,
  FR-035-041). Unlike 035's step writes, an uncertain analytics insert is inserted again, because records
  are grow-only with a key and version (the rule of D1671).
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
  *Revised 2026-11-24:* `database-runner` serves `POSTGRESQL`, `CLICKHOUSE`, `OBJECT_STORAGE` and
  `ICEBERG` sinks and archives to an `object_storage` connection (Decisions 020-002 and 020-012); the minor
  is the next free one when the contract merges (Decision 020-009).
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
  revision it applies to be an audited change by a human principal (review, minor). *Revised 2026-11-24
  (Decision 020-012):* the destination is `PLATFORM` (the platform blob store, written by the engine) or
  `CONNECTION` (an `object_storage` connection of feature 035, written by the `lake` profile of
  `apps/database-runner`); no archive destination carries a secret of its own.
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
  *Revised 2026-11-24:* and for the `OBJECT_STORAGE` and `ICEBERG` sinks and the `CONNECTION` archive
  destinations it is bound to (Decisions 020-002 and 020-012). The catalog grows from 53 to 60 entries
  (53 + 7).

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
  PostgreSQL sink runs in `apps/database-runner` (D1104). *Revised 2026-11-24 (Decision 020-002):* no 020
  sink sends from the engine to a customer destination; every connection-backed sink runs in
  `apps/database-runner`.

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
  only operator-allow-listed endpoints are accepted. *Closed 2026-11-24 (Decision 020-002):* the engine
  no longer writes to customer object storage; the `lake` profile's relay and 017 destination policy
  confine the writer.
- **G1084** — Parquet pulls the Arrow/Parquet crates; behind a feature, pinned, advisory-audited.
  *2026-11-24:* the encoder is feature 035's object-storage writer under the `lake` cargo feature; 020 adds
  no Parquet dependency of its own.
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
  *Closed 2026-11-24 (Decision 020-002):* every connection-backed sink, object storage included, reaches
  private networks through 017's destination policy.
- **G1095** — No legal hold per run.
- **G1096** — Sinks are namespace-scoped; no tenant-wide sink.
- **G1097** — The worker-protocol minor (1.7 at the time of writing) may collide with 019/026; it is
  assigned at merge. *Settled 2026-11-24 (Decision 020-009):* the next free minor of `sdks/SPEC.md` §9 when
  the contract merges (1.10 or later at the time of writing).
- **G1098** — Archives are JSON/NDJSON only; no Temporal-compatible protobuf export.
- **G1099** — Payload deletion needs an indexed reference check on every column that references
  `payloads`; the missing indexes add write cost to hot tables and are a benchmark gate.
- **G1100** — Reusing scheduler shard ownership couples analytics CPU and shard-row share locks to the
  scheduler. A release gate, not a fallback: `scheduler_shard_renewals_skipped_total` must stay zero
  under the contention test, else the separate `analytics_shards` lease table ships first. *Closed
  2026-11-24 (Decision 020-003):* analytics takes no lock on `scheduler_shards`.
- **G1101** — The constitution amendment of principle I (B2) is a proposal in ADR-0054; until an
  amendment PR is accepted, US08/US09 do not start. *2026-11-24:* still open and not a delegate's to close
  (OD-2026-11-24-4 does not cover amending the constitution); the text is proposed for the coordinator to
  carry with constitution 5.0.0 (OD-2026-11-24-2) or a later MINOR amendment. User stories 020-08 and
  020-09 stay gated; the other seven are not.
- **G1102** — A compromised runtime role can still write a retention policy and audit rows; requiring an
  audited human policy change raises the bar but is not a defense against that compromise.
- **G1103** — 018 must let the purge function clear `runs.business_id` (its immutability trigger); this
  is a dependency contract on 018. (018's reset contract already re-creates `run_search_attributes` for
  restored `search_attributes.upserted` events, D1618.) 018 currently names `NOT_FOUND`/`RUN_PURGED` for a
  reset of a purged source where 020 uses `CONFLICT`/`RUN_PURGED`; the coordinator aligns one of them.
  *2026-11-24:* aligned: 018 as built uses `CONFLICT` (409) `RUN_PURGED`. The `business_id` part stays: the
  guard `runs_business_id_guard` (`20270103000000_business_ids`) consults no purge authorization yet, so
  020's retention migration replaces that function (T020-046).
- **G1104** — Rolling engines back below 020 after a purge is supported only with degraded reads (empty
  journals on stubs); tested, but older Studios show purged runs as runs without history.
  *Revised 2026-11-24 (Decision 020-004):* once `retention-v1` is a recorded requirement, the instance
  registry refuses an older engine or control plane (it stays up with a closed gate, ADR-0082); a rollback
  needs the operator to remove the requirement first, and then reads stubs as runs without history.

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
  *Closed 2026-11-24:* the owner answered O035-005 yes on 2026-11-02 (D035-070); both sinks reference the
  connection kinds (Decision 020-002).
- **G1618** — *2026-11-13 (feature 041, G041-004):* the step-log tables (`step_log_streams`, `step_log_lines`,
  `run_log_usage`) have no foreign key to `runs`, so that log ingest never locks a run row. When this feature
  deletes or archives a run, it MUST delete that run's step-log rows in the same purge, and the content-store
  registry MUST list them; nothing else does it. *2026-11-24:* `run_log_usage` was dropped by 041
  (D041-079); the rule is now Requirement 020-046 (Decision 020-007).

## 5. Owner questions (recommended defaults are what the plan assumes)

- **O1080 — Default retention.** *Recommended:* no default; a namespace without a policy keeps every
  run (today's behavior); operators may set a deployment ceiling. *Alternative:* a finite default (for
  example 30 days, Windmill's open-source maximum), which deletes customer data on upgrade.
  **Decided 2026-11-24: no default, delegate (OD-2026-11-24-4).** Deletion cannot be undone, and ADR-0054
  decision 1 lets the purge apply only a policy that a human principal set for that namespace; a default
  would delete data nobody chose to delete, at the upgrade. The operator ceiling (`run_retention_days`)
  still bounds every policy that is set.
- **O1081 — PII leaving the platform (one rule for 020, 025, 026, 031; D1617).** *Recommended:* the
  uniform rule of D1617 as a proposed refinement of ADR-0033 — a destination ceiling ≥ `pii` set by an
  administrator holding the reveal grant, rechecked per delivery or batch, revocation pauses, every
  delivery or batch audited, `secret` never leaves; default ceiling `internal`. *Alternative:* forbid
  `pii` egress entirely in v1.
  **Decided 2026-11-24: the uniform rule of D1617, delegate (OD-2026-11-24-4).** A warehouse that cannot
  receive a customer id cannot join platform facts to the business, so a ban would push teams to copy
  values out through activities, with no ceiling, recheck or audit at all. The rule keeps `internal` as
  the default and gives one control to four features. Its text as a refinement of ADR-0033 is for the
  coordinator to add to that record (the 020 instance is ADR-0053 decision 7).
- **O1082 — Run stubs after purge.** *Recommended:* keep stubs indefinitely in v1 (ids, workflow,
  status, times, archive pointer; no correlation id; subject replaced on erasure) so trees, chains,
  reruns and audit references stay intact. *Alternative:* delete runs completely, relaxing every
  foreign key to them.
  **Decided 2026-11-24: keep stubs, delegate (OD-2026-11-24-4).** Deleting the row would relax the
  foreign keys that the ADR-0039 guard, chains and trees rely on, and would leave audit records pointing
  at nothing. A stub is one small row per run; its own retention is a later decision (G1089).

## 5a. Readiness review (2026-11-24)

Checked against `origin/develop` at `546d5aa5` (features 018, 038, 040, 041, 042, 043 merged; 022's
throughput slice specified and being built; 035's Iceberg write steps built up to the lake profile;
delivery acknowledgement at protocol 1.8 and lease withdrawal in flight). Decisions take the
feature-scoped form because D1080–D1119 and the overflow range are used up. Each is a delegate's,
under the owner's delegation (OD-2026-11-24-4, which carries on OD-2026-11-20-6).

- **Decision 020-001 — Six SDKs.** The draft named five; the Rust SDK (feature 034, ADR-0070) shares the
  canonical contract, so builders, clients, the history client and the exporter helper cover six
  (TypeScript, Go, Python, C#, Java, Rust), with the Rust work in its own tasks (Task 020-063).
- **Decision 020-002 — Every connection-backed sink is written by the managed executor.** The owner
  decided on 2026-11-02 that the `OBJECT_STORAGE` and `CLICKHOUSE` sinks reference feature 035's
  `object_storage` and `clickhouse` connection kinds (O035-005, D035-070), so a destination is configured
  once for both modes. A 017 connection's credential is resolved only to `apps/database-runner`
  (ADR-0035), so the writer follows the connection: PostgreSQL and ClickHouse batches go to the runner's
  `database` profile, object-storage and Iceberg batches to its `lake` profile, each with 035's writer for
  that kind. The engine plans and derives batches, writes stream delivery rows into its own database and
  writes platform archives into the platform blob store; it opens no connection to a customer
  destination and holds no customer credential. Advantages over the draft: one placement rule, private
  networks for every sink (closes G1094), no guarded `object_store` connector to prove (closes G1083), GCS
  and Azure through 035's `lake` build, and no new adapter crate (`crates/infra-analytics` is dropped).
  The cost: a deployment needs `database-runner` (with the `lake` profile for object storage and
  Iceberg) for those sinks (Gap 020-001). Rejected: keep object storage in the engine (two ways to
  configure one bucket, a customer credential in the engine, G1083 open); a new analytics runner (a
  process without measurement, constitution II).
- **Decision 020-003 — No shard fence.** ADR-0020 renewal skips locked rows (`FOR UPDATE OF s SKIP
  LOCKED`), so any share lock analytics takes on `scheduler_shards` risks the scheduler's leases, and the
  draft needed a contention gate (G1100) to bound that risk. The fence is not needed for correctness:
  a batch is frozen in its own row; the partial unique index allows one open batch per (sink, shard); every
  transition is conditional on the batch row's state (read `FOR UPDATE`), and a leased batch is fenced by
  its token, generation and expiry. Two replicas that plan the same shard collide on the index and one
  rolls back; a late acknowledgement of a batch already `ACKED` or `SUPERSEDED` changes nothing; a
  repeated delivery carries the same records under the same keys. So shard ownership only routes work
  (the replica plans the shards its keeper holds in memory), analytics never reads `scheduler_shards`, and
  G1100 is closed. Counted: `analytics_plan_conflicts_total`. Rejected: the fence with its gate (couples
  analytics to the scheduler's most sensitive lock); a separate `analytics_shards` lease table (a second
  lease system for no correctness gain).
- **Decision 020-004 — Activation through the instance registry.** An older engine treats an unknown
  journal event as corrupt in `verify_run`, Update replay and chain progress (only telemetry tolerates
  it, `derive_unsupported`), so a version that declares search attributes must not run while such an
  engine serves. Three capabilities join `crates/application/src/instances.rs`: `search-attributes-v1`
  (engines and control planes; publication of `searchAttributes`, `upsertSearchAttributes` and
  `analytics` declarations is refused until it is required), `analytics-v1` (engines; sinks can be
  created only once it is required) and `retention-v1` (engines and control planes; a policy can be set
  only once it is required). One operator command, `control-plane visibility-activate --capability
  <name> --operator NAME --expect-live N [--envelopes] (--dry-run | --apply)`, runs with the
  schema-owning role, as `schedule-trigger-handover` does. For `analytics-v1` it first builds the note
  index `CONCURRENTLY` (refusing an invalid one), then in the activation transaction records the
  requirement and creates the note trigger under the activation's lock timeout; `--envelopes` does the same
  for the envelope tables. Rejected: per-process switches (ADR-0081 is the fallback only for features not
  yet on the registry).
- **Decision 020-005 — Search-attribute values at every creating path.** Design 043 counts thirteen
  creating paths, and `PgRunStore::create` and `continue_evaluated` consume the `CheckedRunInput`. Declared
  `from` values are evaluated there, over the checked input, so triggers, schedule actions, backfills,
  children, continuations, batch items, pending resets and the API paths all set them. Explicit values
  are accepted where a caller names them: `startRun`, Signal-with-Start, Update-with-Start, and a schedule
  action's start template (an additive field of 018's schedule action, validated when the schedule is
  saved). An explicit value that a person sent is refused synchronously; one stored in a schedule that no
  longer validates when it fires (the attribute was retired) is journaled as `search_attributes.rejected`
  and the run starts, because nobody waits for the answer, and design 043 found that a schedule action
  refused for a reason that cannot pass is otherwise fired again at every tick. A rerun takes the source's explicit values from its first `search_attributes.upserted` event and
  recomputes the `from` values; a reset copies events by the reset rules (D1618). Design 044 (workflow as
  code) is being designed: an upsert command of workflow code, if it adds one, maps to the same journal
  event and the same rows.
- **Decision 020-006 — `RunSelector` follows 018 as built.** 018 pages a selection by run id (UUIDv7,
  creation order) in `batch_jobs.select_cursor uuid`, bounds it by `selection_bound_at` and starts it when
  `pg_snapshot_xmin(pg_current_snapshot()) > selection_horizon`, where the horizon is the creating
  transaction's identity (its code says why `xmax` would be wrong). The contract now says the same, so 018
  needs no migration; the draft's `(StartedAt, RunId)` keyset and `xmax` horizon are corrected. `RUN_PURGED`
  is `CONFLICT` in both features. 018's FR-018-029 still names the old keyset (Gap 020-005).
- **Decision 020-007 — Step logs (feature 041).** Purge and erasure delete a run's `step_log_streams` and
  `step_log_lines` rows in the run's purge, through the content-store registry (G1618). An archive holds
  the lines that still exist only when `archiveValues = UP_TO_PII`, because a log line is read as `pii` in
  every namespace (FR-041-021); otherwise it holds each stream's metadata (attempt, counts, expiry, no
  text). A history export never carries log lines: logs keep their own routes, permission
  (`run:logs:read`) and grant. Analytics never exports log text: no declaration can name it.
- **Decision 020-008 — Facts of design 045 and of buffered signals.** `step_attempt` records and history
  documents gain, additively and optionally, `startDeadlineAt`, `closeDeadlineAt`, `attemptDeadlineAt`,
  `attemptDeadlineCause` and the `deadline` cause (`startToClose`, `scheduleToClose`, `heartbeat`) of the
  event that ended an attempt; the failure codes `SCHEDULE_TO_START_TIMEOUT` and `SCHEDULE_TO_CLOSE_TIMEOUT`
  appear in `failureCode` as any code does. They are `internal`. Buffered signals are a separate design
  (ADR-0111, proposed): a pending-signal count on `run_snapshot` and a system attribute would be additive;
  020 designs neither.
- **Decision 020-009 — Protocol.** 020's service takes the next free minor of `sdks/SPEC.md` §9 when its
  contract merges (1.7 is 041's, 1.8 delivery acknowledgement's, 1.9 design 045's; 1.10 or later at the
  time of writing). `ResolveSinkConnection` answers the existing `ResolvedDatabase`, whose fields 13 to 15
  already carry feature 035's `IcebergConnection`, `ObjectStorageConnection` and `ClickHouseConnection`,
  so the draft's `ResolvedClickHouse` message is dropped and no new credential-bearing message is added.
  Batch leases are not task leases: delivery acknowledgement (ADR-0099) and lease withdrawal (ADR-0098)
  do not apply to them.
- **Decision 020-010 — Audit in OCSF.** Design 040 (FR-040-001) requires one OCSF mapping per audit action
  in one table of the domain. Each of 020's new actions gets its mapping in the same task that adds the
  action, once that table exists (Task 020-010); until then the actions follow the native shape.
- **Decision 020-011 — Journal partitioning.** OD-2026-11-24-1 gives the partitioning of the journal
  tables a new design. The purge is defined per run and assumes no physical layout. If that design
  partitions by time, a partition whose runs are all `PURGED` may be dropped instead of deleted row by
  row, but only through the same eligibility and archive verification (ADR-0054 decision 3); the
  partitioning design decides, and 020 adds nothing that blocks it (Gap 020-003).
- **Decision 020-012 — Archive destinations.** `PLATFORM` (the platform blob store, written and verified
  by the engine) or `CONNECTION` (an `object_storage` connection of feature 035, revision pinned by the
  policy revision). A `CONNECTION` archive is written by the `lake` profile under a lease of the archival
  job: the job's export step becomes a lease of kind `ARCHIVE` on the same service, held on the job's own
  row (token hash, generation, database-clock expiry); the runner writes the
  object create-only, re-reads it and acknowledges with its size and SHA-256; the job moves to `VERIFIED`
  only when both equal what the engine computed while streaming it. The same placement rule as Decision
  020-002 applies, for the same reasons; a customer bucket is configured once.
- **Decision 020-013 — ADR-0053 revised and accepted**, delegate (OD-2026-11-24-4). Its core (no dual
  write, horizon discovery, frozen batches, declared values, failures that never skip) held against
  today's code. Revised with this review: decision 3 (no shard fence, Decision 020-003), decision 8
  (placement by connection, Decision 020-002), the `pii`-egress rule as decided under O1081, activation
  through the registry (Decision 020-004).
- **Decision 020-014 — ADR-0054 accepted**, delegate (OD-2026-11-24-4). Accepting the record fixes the
  design; the code of User stories 020-08 and 020-09 still waits for the principle-I amendment (G1101),
  which a delegate cannot make. Revised with this review: archive destinations (Decision 020-012), step
  logs (Decision 020-007), the instance registry for rollback (G1104), the partitioning note (Decision
  020-011).
- **Decision 020-015 — Checklist items accepted or fixed**, delegate (OD-2026-11-24-4); each item says
  what was done (`checklists/requirements.md`).

Gaps added by this review:

- **Gap 020-001** — A deployment without `apps/database-runner` can use only `STREAM` and `PULL` sinks and
  `PLATFORM` archives; object storage and Iceberg sinks and `CONNECTION` archives need its `lake`
  profile. Accepted: the runner is the platform's one route to customer destinations.
- **Gap 020-002** — 035's `object_storage` and `clickhouse` connection kinds are its Phase 11 (T035-106
  onwards) and are not built. The `OBJECT_STORAGE` and `CLICKHOUSE` sinks wait for them; the analytics core
  is proven first with the `STREAM` and `PULL` sinks and then `ICEBERG` (built kind). Open, depends on 035.
- **Gap 020-003** — The journal-partitioning design of OD-2026-11-24-1 does not exist yet. Open.
- **Gap 020-004** — Design 040's mapping table does not exist yet. Open, depends on 040.
- **Gap 020-005** — 018's FR-018-029 says the selector pages by `(StartedAt, RunId)`; it should say
  `RunId`, as its code does. For the coordinator (another feature's file).

Stale items corrected in place: the SDK count (five to six); the permission count (48 to 53); the
worker-protocol minor; `runs` has no chain column; the note purge's location; feature 035's connection
kinds and profiles; feature 018's built filters, selection, reset registry and `RUN_PURGED` code; feature
041's tables (`run_log_usage` dropped, no foreign key); ADR-0055 is still Proposed, and after Decision
020-002 020 no longer relies on its rule 11(b).

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

Note (2026-11-15, from feature 041): the owner gave worker-protocol minor 1.7 to feature 041 (step logs, OD-2026-11-15-8).
Where this feature names 1.7 for its own addition, it takes the next free minor of the register in `sdks/SPEC.md` §9 when
its contract merges.
