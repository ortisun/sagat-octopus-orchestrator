# Implementation plan — 020 run visibility, analytics and archival

Status: Ready to build — not started. Revised 2026-10-27 after independent architecture review (research §6)
and on 2026-11-24 by the readiness review (research §5a: connection-backed sinks in the managed executor,
no shard fence, activation through the instance registry, six SDKs, step logs, design 045's facts).

**Branch**: `feature/020-visibility-analytics-archival` | **Date**: 2026-10-26 | **Spec**: [spec.md](spec.md)
| **Research**: [research.md](research.md) | **ADRs**: [ADR-0053](../../docs/adr/ADR-0053-analytics-is-a-journal-derived-projection-delivered-through-per-sink-cursors.md),
[ADR-0054](../../docs/adr/ADR-0054-run-content-leaves-the-hot-store-only-through-a-verified-archive-under-a-namespace-retention-policy.md) (both Accepted 2026-11-24, delegate, Owner decision 2026-11-24-4)

## Summary

Three capabilities on one rule: PostgreSQL is the authority and nothing derived runs on the write path.
(1) Search attributes are run state: indexed rows and a journal event commit with the transition that
sets them, queried through a closed grammar compiled to parameterized SQL, gated by classification.
(2) Analytics is a CQRS projection: the existing note gains a commit-safe horizon (`moved_xid`), the
replica owning a scheduler shard plans frozen batches per sink, pure functions derive canonical records,
the managed executor (or the platform's own outbox for streams) delivers at least once and the cursor moves
only with the sink's acknowledgement. (3) Retention is
opt-in: terminal runs are archived, verified, then purged by one privileged function; the run row stays as
a stub. Retention needs a MINOR amendment of constitution principle I (proposed in ADR-0054) before any of
it is built. Each capability ships behind its own enablement and can be rolled back by disabling new use.

## Technical context

- **Language/runtime:** Rust (pinned toolchain), Tokio, Axum, tonic, SQLx 0.9 (`begin_safely`, ADR-0014);
  React Studio; six SDKs (TypeScript, Go, Python, C#, Java, Rust).
- **New dependencies:** none. *Corrected 2026-11-24 (research Decision 020-002):* there is no
  `crates/infra-analytics`; the engine links no sink client. The sink writers are feature 035's, in
  `apps/database-runner`: PostgreSQL (`tokio-postgres` 0.7.18) and ClickHouse (`reqwest` 0.13 over HTTPS)
  in `crates/infra-database` under the `database` profile; object storage (`object_store` 0.14.2 with the
  `gcp` and `azure` features, Parquet and NDJSON) and Iceberg in `crates/infra-lake` and
  `apps/database-runner/src/lake/` under the `lake` cargo feature. 020 adds batch-shaped entry points to
  those writers, not new crates.
- **Storage:** PostgreSQL ≥ 13 (`xid8`); reference 16. Platform blob store (ADR-0023) for platform archives.
- **Testing:** real PostgreSQL for integration and fault suites; MinIO-class S3 server (the existing
  `s3s` dev-dependency path), a real ClickHouse server and a second PostgreSQL as sink targets in the
  compose test profile, reusing feature 035's fixtures where they exist (its ClickHouse, moto, azurite
  and fake-gcs starters); Kafka/NATS from feature 004's fixtures for stream sinks.
- **Performance goals:** none promised. Criteria to measure (NFR-020-PERF): write overhead per
  transition, search p50/p95/p99, projector records/s, lag under a long transaction, purge runs/s.
- **Constraints:** finite ceilings below; no value in logs/spans/metrics; database clock for every time
  decision; tenant and namespace on every row and statement.

## Architecture and crate placement

| Concern | Crate / file | Notes |
|---|---|---|
| Attribute names, types, levels, value validation, query AST, parser, bounds | `crates/domain/src/visibility.rs` (new) | Pure; no serde_json, no SQL |
| Search services: creating-path and upsert evaluation hooks, search/count, registry, views, `RunSelector` | `crates/application/src/run_search.rs` (new), `runs.rs`, `run_input.rs` (the checked input the `from` values read), `publishing.rs`, `execution.rs`, `schedules.rs` (a schedule action's explicit values) | Ports only |
| Query compiler and stores | `crates/infra-postgres/src/run_search.rs` (new); `PgRunStore::create` and `continue_evaluated` in `crates/infra-postgres/src/execution.rs` | Parameterized SQL, `SET LOCAL statement_timeout` |
| Expression evaluation | `crates/infra-expr`, `crates/infra-ir` | Existing bounded evaluator; IR codec and capability gates |
| Reset rules of the two new events | `crates/domain/src/execution/reset.rs` (`rule_of`) | 018's registry already lists them (D1618) |
| Capabilities | `crates/application/src/instances.rs` (`Capability`: `search-attributes-v1`, `analytics-v1`, `retention-v1`), `apps/control-plane/src/cli.rs` (`visibility-activate`) | ADR-0082; research Decision 020-004 |
| Analytics derivation (pure) and record encoding | `crates/application/src/analytics.rs` (new) | Reuses `telemetry::{RunFacts, attempts}`; canonical NDJSON as telemetry encodes its spans |
| Analytics stores (notes, cursors, batches, ledger) | `crates/infra-postgres/src/analytics.rs` (new) | |
| Projector, envelope loop, monitor condition | `crates/scheduler/src/analytics.rs` (new), `monitor.rs` | Application layer; ports only; plans the shards the keeper holds, takes no shard lock |
| Sink writers (all connection-backed kinds) | `apps/database-runner` (lease, resolve, ack loop for batches); `database` profile: `crates/infra-database/src/{destination.rs,clickhouse.rs}` (035's PostgreSQL and ClickHouse writers, planned by its Phase 11); `lake` profile: `apps/database-runner/src/lake/`, `crates/infra-lake` (035's object-storage and Iceberg writers) | ADR-0035 boundary: no engine or control-plane connection to a customer destination; *corrected 2026-11-24*, research Decision 020-002 |
| Analytics enablement | `apps/control-plane` command `visibility-activate --capability analytics-v1 [--envelopes]` (schema-owning role) | Builds the note index `CONCURRENTLY`, then records the requirement and installs the trigger in one activation; nothing on the write path before it |
| Pull exporter service | `crates/api-grpc` (`AnalyticsExportService`), served by the execution engine | The next free minor when the contract merges (1.10 or later at the time of writing) |
| REST | `crates/api-http/src/routes/{execution.rs,visibility.rs,analytics.rs,retention.rs}` | |
| Retention, archival, purge | `crates/application/src/retention.rs`, `crates/infra-postgres/src/retention.rs`, migration with the definer function | |
| Wiring | `apps/execution-engine/src/lib.rs` (loops), `apps/control-plane` (routes) | |
| Studio | `frontend/apps/studio/src/features/runs/` (query bar, views), `features/analytics/` (new), retention panel in `features/operations/` | Design tokens only |
| CLI | `apps/cli/src/{args.rs,workflow.rs}` | `--query`, `export-history`, `namespace retention` |
| SDKs | `sdks/{typescript,go,python,dotnet,java,rust}` | Builders, clients, history client, exporter helper |

`scripts/check-architecture.py` keeps passing: the domain gains no dependency; `scheduler` depends on
application ports; a new rule checks that `apps/execution-engine` and `apps/control-plane` link no sink
client (`cargo tree -e features`, the check of feature 035's D035-058).

## Transaction boundaries (what commits together)

| # | Operation | One transaction contains | Outside the transaction |
|---|---|---|---|
| T1 | Publication with new attributes | version, registry inserts (or type/level check under `FOR SHARE`), namespace attribute count check, audit `search_attribute.registered` | — |
| T2 | Run creation (every creating path of design 043) | existing run/journal/tasks/outbox/receipt/audit + `search_attributes.upserted` (payload) + attribute rows + note (trigger sets `moved_xid`) | — (the `from` expressions read the `CheckedRunInput` inside the transaction) |
| T3 | Step completion with upserts | existing completion/journal/next tasks/outbox + evaluated upserts or `search_attributes.rejected` + rows + note | — (upsert expressions are evaluated inside the transaction over the committed output, as conditions are today) |
| T4 | Search with `pii` | snapshot read + audit `run.search_pii`; results leave after commit | — |
| T5a | Discovery (read only) | note scan inside the sink's `(tenant, namespace, shard)` prefix with the horizon computed in the same statement | — |
| T5 | Batch plan | sink row `FOR SHARE` (state, revision, disclosure owner recheck), batch + manifest insert (per-run `upto_seq` frozen; the partial unique index admits one open batch per (sink, shard), so a second planner's insert fails and it rolls back), cursor `next_batch_seq` | discovery (T5a), derivation and sink I/O |
| T6 | Batch ack | batch row `FOR UPDATE`, then only if it is still `PLANNED` (engine, `STREAM`) or `LEASED` with the caller's token hash, generation and an unexpired lease (managed executor or exporter): batch `ACKED`, manifest run progress (`GREATEST`), cursor advance, sink health, export bytes, `pii` batch audit, stream delivery rows (`STREAM`); otherwise nothing is written and the caller is answered `already_applied` or `STALE_ATTEMPT` | — |
| T7 | Batch reject / auto-pause | batch `REJECTED`, sink `AUTO_PAUSED` + reason, audit `analytics.sink_auto_paused` | — |
| T8 | Sink management | revision, head, supersede open batches, audit | probe I/O after commit; its outcome is recorded in its own short transaction |
| T9 | Retention policy change | policy row, audit | — |
| T10 | Archival job steps | one transaction per arrow of data-model §6 (lease `FOR UPDATE SKIP LOCKED`; for a `CONNECTION` destination the export is an `ARCHIVE` lease on the job row, taken by the managed executor, whose acknowledgement carries the object's size and SHA-256) | object PUT, HEAD, read-back (engine for `PLATFORM`, managed executor for `CONNECTION`) |
| T11 | Purge chunk (repeated until done) | definer function: job lock, policy revision is an audited human change, run row `FOR UPDATE` and eligibility recheck under run-tree locks (first chunk), authorization row, one bounded chunk of deletes, payload/blob candidates of that chunk; first chunk also stub + note tombstone; last chunk job `PURGED` + audit `run.purged` | — |
| T12 | History export | `REPEATABLE READ` snapshot read + audit (`pii` never collapsed); bytes leave after commit | archive object read and verification (before the audit transaction) |
| T13 | Rerun/reset of a stub (018, ADR-0039) | existing identity guard (key share), then `purged_at`/job state read; refusal `CONFLICT`/`RUN_PURGED` | — |

The projector never takes a run, step, chain or scheduler shard lock. A transition never waits for
analytics. Plan and ack transactions are short and independent of sink latency.

**Lock order.** Search-attribute writes happen under the run lock the transition already holds
(chain-before-run, 014). Publication takes the workflow quota lock, then registry rows. Purge takes the
chain (if any), the root run, descendants in depth then id order, then blob catalog rows, then payload
rows; it never waits on a policy or usage lock while holding payload rows (ADR-0027 order). The first
chunk's `FOR UPDATE` on the run row conflicts with the ADR-0039 guard's key-share lock, so a concurrent
rerun either commits before the purge (its references then keep the shared content) or waits and then
sees the purge. Analytics plan/ack: sink row, batch row, cursor row, run-progress rows in run-id order;
no shard row (*corrected 2026-11-24*, research Decision 020-003).

## Authorization matrix

| Capability (permission) | admin | developer | operator | viewer | machine |
|---|---|---|---|---|---|
| Search, count, list registry, private views (`run:read`) | ✓ | ✓ | ✓ | ✓ | — |
| See/filter `internal` attributes (`payload:read-internal`, existing) | ✓ | ✓ | ✓ | — | — |
| See/filter `pii` attributes (reveal grant list, FR-016-011) | grant | grant | grant | — | — |
| Tighten/retire attributes (`search-attribute:manage`) | ✓ | — | — | — | — |
| Shared views (`run:view:manage`) | ✓ | ✓ | ✓ | — | — |
| History export (`run:history:export`; values by FR-016-007) | ✓ | ✓ | ✓ | — | — |
| Read sinks, health, ledger (`analytics:sink:read`) | ✓ | ✓ | ✓ | — | — |
| Manage sinks, bind exporters (`analytics:sink:manage`) | ✓ | — | — | — | — |
| Sink `maxLevel: PII`, retention `UP_TO_PII` | admin with grant | — | — | — | — |
| Lease/ack batches (`analytics:sink:export`) | — | — | — | — | `analytics-exporter` + binding (`PULL`); `database-runner` + binding (`POSTGRESQL`, `CLICKHOUSE` on the `database` profile; `OBJECT_STORAGE`, `ICEBERG` and `CONNECTION` archives on the `lake` profile) |
| Retention policy, erasure (`retention:manage`) | ✓ | — | — | — | — |

Every check is against current membership on each request, cursor reuse, view open, batch plan and
exporter call. Foreign resources are opaque `NOT_FOUND`; cross-tenant attempts write the existing
`access.denied.cross_tenant` record. `admin` does not hold `analytics:sink:export` usefully: the machine
role and a binding are required, as for `database:execute`.

## Finite ceilings

| Resource | Default | Hard ceiling |
|---|---:|---:|
| Registered custom attributes per namespace | 64 | 256 |
| … of type `text` or `keywordList` per namespace | 8 | 32 |
| Attributes declared per workflow version | 32 | 64 |
| Distinct attributes with values per run | 32 | 64 |
| Keyword value bytes | 256 | 1,024 |
| Text value bytes | 2 KiB | 8 KiB |
| Keyword-list elements | 16 | 64 |
| Attribute bytes per run (all values) | 8 KiB | 32 KiB |
| Upserts per node / journaled upsert events per run | 16 / 1,000 | 16 / 10,000 |
| Query text / predicates / `IN` values / depth | 2 KiB / 16 / 64 / 3 | 4 KiB / 32 / 256 / 4 |
| Page size | 50 | 200 |
| Search statement budget | 5 s | 15 s |
| Concurrent searches per namespace | 8 | 32 |
| Count ceiling | 10,000 | 100,000 |
| Saved views per namespace / per subject | 200 / 50 | 1,000 / 200 |
| Sinks per namespace | 4 | 16 |
| Records / bytes / runs per batch | 1,000 / 4 MiB / 200 | 10,000 / 16 MiB / 1,000 |
| Open batches per (sink, shard) | 1 | 1 |
| Declared records per node / fields per record / bytes per record | 4 / 32 / 16 KiB | 16 / 128 / 256 KiB |
| Export bytes per tenant per day | 10 GiB | operator ceiling, at most 1 TiB |
| Transient retries before `FAILING` / backoff cap | 12 / 15 min | 12 / 1 h |
| Pull lease / exporters per sink / credit per call | 60 s / 2 / 1,000 records | 600 s / 8 / 10,000 records, 16 MiB |
| Sink hold before purge proceeds | 7 days | 30 days |
| Export ledger retention | 30 days | 90 days |
| History export events / bytes | 10,000 / 8 MiB | 100,000 / 32 MiB |
| Concurrent history exports per namespace | 2 | 8 |
| Hot retention | unset (no deletion) | 1..3,650 days, clamped to operator ceiling |
| Archive object bytes per run | 64 MiB | 256 MiB |
| Runs purged per pass / archival concurrency per engine | 100 / 4 | 1,000 / 16 |

Effective = min(request, namespace, tenant default, operator ceiling) through ADR-0027 policy resolution;
zero refuses new admission, it is never unlimited. Count resources (`search_attributes`, `run_views`,
`analytics_sinks`) and policy resources (`analytics_export_bytes_per_day`, `run_retention_days`,
`analytics_ledger_days`) are added to `crates/domain/src/quota.rs`. Values are proposals to be confirmed
or lowered by the benchmark (G1091), not capacity claims.

## Failure and recovery matrix

| Failure | Behavior | Test |
|---|---|---|
| Crash before T2/T3 commit | no run/upsert, no rows, no note | fault: crash per statement |
| Timeout vs signal race on a node with upserts | first commit wins; loser writes no attribute and no event | fault: race |
| Duplicate completion / lost response | receipt replay; no second upsert event | fault: duplicate |
| Lost broker hint | unchanged scheduling; attributes unaffected | fault: hint drop |
| Long-running transaction | horizon held; lag grows; no skip | fault: open txn 120 s |
| Crash after sink write, before T6 | same frozen batch redelivered, identical bytes/tokens | fault |
| Sink partial write (one object PUT) | full batch rewritten under same names | fault |
| Shard takeover during delivery | one open batch per (sink, shard); a late ack of an acknowledged or superseded batch writes nothing; a redelivery carries the same records under the same keys | fault |
| Two replicas plan the same sink and shard | the second insert fails on the unique index and rolls back; `analytics_plan_conflicts_total` grows | fault |
| Sink permanent rejection | `REJECTED`, `AUTO_PAUSED`, alert; no later batch | integration |
| Sink outage | backoff, `FAILING`, resume without gap | integration |
| Credentials revoked / disclosure owner loses grant | auto-pause at next plan | security |
| Duplicate pull ack / stale lease | `already_applied` / `STALE_ATTEMPT` | contract |
| Export quota exhausted | `THROTTLED`, resumes next day | integration |
| Crash after archive PUT | job resumes at `EXPORTING`, same name, verify | fault |
| Archive corrupted or bucket gone | `BLOCKED`, nothing purged | fault |
| Crash during purge chunk | job stays `PURGING`; resumes deleting remaining chunks; stub already marks purged | fault |
| Concurrent reuse of a purged digest | one retryable failure, then success | fault |
| Failed compensation | analytics/archival cause no workflow effect; a run ended `COMPENSATION_FAILED` or `PARTIAL`, or holding an unresolved unknown outcome or manual review, is never eligible until an audited resolution, so retention cannot erase the evidence of an unresolved effect | fault: eligibility |
| Restart of every engine | cursors, batches, jobs resume from the database | e2e restart |
| Analytics vs shard renewal | analytics takes no lock on `scheduler_shards` (Decision 020-003) | lock-monitor assertion in the analytics integration suite |
| Paused sink and the 24 h note purge | note kept until applied or hold expiry; expiry counted as a retention gap | integration |
| `DEAD` stream delivery | sink `AUTO_PAUSED`/`STREAM_DELIVERY_DEAD`; redrive then resume | integration |
| Rerun requested during purge | serialized by run-row lock vs ADR-0039 guard; rerun first or `RUN_PURGED` | fault: race |
| Database-runner dies holding a `POSTGRESQL`, `CLICKHOUSE`, `OBJECT_STORAGE` or `ICEBERG` batch | lease expires, generation + 1, batch re-leased; key-idempotent inserts (PostgreSQL `ON CONFLICT`, ClickHouse token) absorb the repeat; an object already written with equal records is the write's success (create-only, FR-035-040); an Iceberg commit that landed is found by its batch identity (FR-035-018), an inconclusive lookup appends again and readers deduplicate by key and version | fault |
| Engine rolled back below 020 after a purge | refused by the instance registry while `retention-v1` is required (closed gate, ADR-0082); after the operator removes the requirement, stubs read as runs without history | compatibility test |
| A process without a capability while it is required | stays up with a closed gate (`BINARY_CAPABILITY_MISSING`), serves nothing | registry test (018's pattern) |

## Observability

**Spans** (exported by the tracing layer; attributes `sagat.tenant.id`, `sagat.namespace.id`,
`sagat.sink.kind`, `sagat.analytics.shard`, `sagat.outcome`, never a value or destination):
`runs.search`, `runs.count`, `run.search_attributes.upsert` (event span derived from the journal),
`analytics.batch.plan`, `analytics.batch.deliver`, `analytics.batch.ack`, `analytics.exporter.lease`,
`archival.export`, `archival.verify`, `retention.purge`, `run.history.export`.

**Metrics** (closed labels only): `search_attribute_upserts_total{outcome}`,
`search_attribute_rejections_total{reason}`, `run_search_total{outcome}`,
`run_search_duration_seconds{outcome}`, `analytics_batches_total{sink_kind,outcome}`,
`analytics_records_total{sink_kind,record_kind}`, `analytics_records_withheld_total{reason}`,
`analytics_export_bytes_total{sink_kind}`, `analytics_sink_lag_seconds{sink_kind}`,
`analytics_sinks{state}`, `analytics_xmin_horizon_age_seconds`, `archival_jobs{state}`,
`archival_oldest_eligible_age_seconds`, `retention_purged_runs_total{reason}`,
`retention_purge_duration_seconds`, `history_exports_total{mode,outcome}`,
`analytics_plan_conflicts_total`. *Corrected 2026-11-24:* `scheduler_shard_renewals_skipped_total` is
dropped with the fence (Decision 020-003). The monitor owns gauges (ADR-0020 §7: zeroed when its lease
is lost or a sample fails).

**Alerts:** namespace conditions `ANALYTICS_SINK_STUCK` and `ARCHIVAL_BLOCKED` added to `AlertKind`;
deployment rules for horizon age and archival backlog in `infra/observability/alerts.yaml`.

**Audit actions:** `search_attribute.registered`, `search_attribute.changed`, `run.search_pii` (`pii`
predicates; one per round for unattended selection), the existing `payload.pii_revealed` per run whose
`pii` attribute value a search returns (FR-016-012 form),
`run_view.created|changed|deleted`, `analytics.sink_created|changed|paused|resumed|rewound|skipped|deleted|auto_paused`,
`analytics.batch_exported` (`PII` sinks only, one per batch, the retained run manifest being the per-run
evidence: a refinement of FR-016-012 recorded in ADR-0053 decision 11), `analytics.exporter_bound|unbound`,
`retention.policy_changed`, `run.purged`, `run.erasure_requested`, `run.erased`, `run.history_exported`.
New resource types: `SearchAttribute`, `RunView`, `AnalyticsSink`, `RetentionPolicy`. Each new action
gets its OCSF mapping (class, activity, severity, status) in design 040's table in the task that adds it,
once that table exists (FR-040-001, research Decision 020-010).

**Logs:** redacted structured logs with closed codes; no query text (only its digest), no attribute
value, record content, destination URL, bucket, credential or provider message.

## Migration, compatibility and rollback

- Migrations (additive, in order): `…_search_attributes` (registry, values, views, quota resources),
  `…_analytics_projection` (note and envelope columns with constant defaults, sink tables; **no trigger,
  no note index**), `…_retention_archival` (policies, jobs, run stub columns, tombstone payload helper,
  authorization table, definer function, trigger function update, `run-content-references.json` check).
  `control-plane visibility-activate --capability analytics-v1 [--envelopes]` later builds the indexes
  `CONCURRENTLY` and, in the activation transaction of the instance registry, records the requirement and
  installs the triggers, with the schema-owning role; removing the requirement (`platform_requirement_remove`)
  and then the triggers is the way back once no sink needs them. Each `down` refuses while
  feature rows exist. Large tables: indexes built `CONCURRENTLY` before the migration on big databases,
  as `specs/010-scalability-ha/data-model.md` recommends.
- IR: additive keys under ADR-0032 with capability gates `searchAttributes`, `analyticsDeclarations`;
  020's two carriers enter the single FR-016-002 / classification-rules amendment shared by the drafts
  (opened by the first implementer, C6); 020 depends on that rules version and does not create it alone;
  rules 0/1 versions unchanged.
- OpenAPI `/v1` additive; `getRun`, `listRuns`, `listRunJournalEvents` unchanged except the optional
  `archive` object; worker protocol: one new service in a new minor; AsyncAPI: one new message.
- Journal: two new event types (`search_attributes.upserted`, `search_attributes.rejected`) decoded by
  `crates/infra-postgres/src/event_codec.rs`. *Corrected 2026-11-24:* only telemetry tolerates an unknown
  event (`derive_unsupported`); `verify_run`, Update replay and chain progress treat one as corrupt. So
  older engines are neither safe writers nor safe readers of runs that use the capability (ADR-0032
  decision 8), and publication is refused until `search-attributes-v1` is a recorded requirement of
  every engine and control plane (FR-020-045).
- Rollback per capability: disable publication of new declarations, sink creation/planning and policy
  setting; readers, rows and cursors stay; no rollback deletes stored values or receipts. After the first
  purge, engines may still be rolled back below 020 once the operator removes `retention-v1`: stubs keep
  every constraint (tombstone `input_digest`) and read as runs without history (tested, G1104).

## Cross-feature contracts (coordinator decisions, 2026-10-27)

| Decision | 020's part | Owner of the rest |
|---|---|---|
| C1 reset rules | `search_attributes.upserted` = `REMATERIALIZE`, `search_attributes.rejected` = `COPY` (D1618) | 018 registry `contracts/journal-and-events.md` |
| C2 region epoch | epoch in record keys, batch ids, ClickHouse tokens, object names, stream event ids (D1615) | 032 activation rewind and tombstones |
| C3 placement | *corrected 2026-11-24:* the engine sends nothing to a customer destination; stream batches are rows of the platform's own outbox; every connection-backed sink runs in `apps/database-runner` (stricter than rule 11, so 020 does not wait for ADR-0055's acceptance) | rule written by 021 in ADR-0055; connection kinds and writers of 035 (O035-005) |
| Instance registry | three capabilities and `control-plane visibility-activate` (Decision 020-004) | 018, ADR-0082 |
| Creating paths | `from` values over the checked input of every creating path (Decision 020-005) | design 043 |
| Step logs | purge, erasure, archive rules for 041's tables (Decision 020-007, FR-020-046) | design 041 |
| Deadline facts | optional `step_attempt` fields (Decision 020-008) | design 045 |
| Audit standard | one OCSF mapping per new action (Decision 020-010) | design 040 |
| C6 classification carriers | two carriers in the spec's "Amendment to feature 016" | single FR-016-002 / rules-v2 amendment, first implementer |
| C7 encoded fields | `from`, `upsertSearchAttributes`, `project` use 031's codes | 031 registry `contracts/workflow-encoding.md` |
| C8 content stores | owns `run-content-references.json` and its test (data-model §6) | each draft registers its store |
| C9 `pii` egress | proposes the uniform rule under O1081 (D1617) | 025, 026, 031 cite it |
| C10 visibility | searches, counts, selection, views use the 029 evaluator when enabled | 029 |
| C16 names | `SINK_DESTINATION_DENIED`; `HISTORY_TOO_LARGE` HTTP 413 | — |
| C18 test runs | `DRAFT_TEST` runs never exported, archived as history or selected | 022 |
| C22 history | per-attempt output digests; values only in `DISCLOSED` mode | 028 consumes |

## Constitution Check

Design checks, not implementation approval.

| Principle | Status | Evidence / gate |
|---|---|---|
| I — durable atomic transitions | Pass (search, analytics); **Requires amendment** (retention) | Attribute rows and events in the transition transaction (T2/T3); analytics outside it. Retention removes whole journals of terminal runs and leaves the archive as the only copy (or no copy with `archive = NONE`): that changes principle I's obligations, so ADR-0054 proposes a MINOR amendment; US08/US09 start only after its PR is accepted (T020-061). T020-014, T020-015, T020-046, T020-047. |
| II — pure domain, inward dependencies | Pass | Parser/AST/types in the domain without new dependencies; SQL in `infra-postgres`; sink clients only in `apps/database-runner` (035's crates), none in the engine or control plane; no new process (D1101, Decision 020-002); no empty mapper (the record schema is the one wire boundary). T020-006, T020-058. |
| III — at-least-once effects | Pass | Sinks at least once with deterministic keys; no exactly-once claim; archive verified before delete; no compensation semantics changed. T020-029, T020-047. |
| IV — pinned, compatible | Pass | Declarations pinned with the version; the shared new classification rules version applies only to new versions; additive contracts; six-SDK conformance; capabilities switched on through the instance registry (FR-020-045). T020-005, T020-020, T020-050, T020-063, T020-064. |
| V — isolation of code, tenant, secrets | Pass | No in-process plugins; customer credentials reach only the managed executor, through the 017 two-phase resolution under a batch lease; `secret` never indexed/exported; tenant on every row; opaque 404. T020-017, T020-035, T020-057. |
| VI — observability and audit | Pass | Spans, closed-label metrics, audit in the decision transaction, lag visible in API/Studio/metrics. T020-054. |
| VII — inbox/outbox | Pass | Stream sink deliveries are outbox rows written with the cursor (ADR-0025); the inbox is read, never changed in semantics. T020-038. |
| VIII — failure tested, honest | Pass | Fault matrices for each capability; benchmark criteria, not promises; pending items marked. T020-029, T020-047, T020-055, T020-060. |

## Complexity Tracking

| Item | Why needed | Simpler alternative rejected because |
|---|---|---|
| Privileged purge of append-only journals (definer function + per-transaction authorization), behind a constitution amendment | Bounded retention is impossible without removing journals | Granting `DELETE` to the runtime role lets any bug delete history; dropping the append-only trigger removes the guarantee for live runs; treating it as a clarification would change an obligation silently |
| Operator-enabled analytics triggers and indexes | Keep the write path byte-identical when analytics is unused | Always-on triggers and indexes charge every deployment for a feature it may not use |
| Connection-backed sinks only through the managed executor | One route to customer destinations, private networks, no customer credential in the engine | Object storage from the engine: a second way to configure one bucket and an unproven egress connector (G1083) |
| Horizon cursor plus per-run progress (two levels) | Commit-safe discovery without write-path fan-out | A sequence watermark skips late commits; a per-sink dirty row is a write-path write per sink |
| Frozen batch manifests | Identical retries (object names, ClickHouse tokens, pull leases) | Re-planning on retry produces overlapping objects and defeats insert deduplication |
| Run stubs | Keep tree/chain/rerun foreign keys and audit references | Full deletion requires relaxing every foreign key to `runs` |

## Delivery sequence

*Corrected 2026-11-24:* the order is the build slices of `tasks.md` ("Build slices (2026-11-24)"), two
lanes in parallel. In short:

1. Foundations: IR fragment and codec gates, domain types and parser, permissions, quotas, capabilities,
   migration for search attributes, errors and audit vocabulary (T020-005..011, T020-064).
2. User story 020-01 search attributes and queries, six SDKs (T020-012..020, T020-063, T020-065) — first
   releasable increment.
3. User story 020-03 analytics core, proven with the `STREAM` and `PULL` sinks (T020-021..025,
   T020-028, T020-029, T020-038, T020-049, T020-050) — second increment; benchmark the write path before
   enabling by default.
4. User story 020-02 views and Studio (T020-030..031); User story 020-04 declared records
   (T020-032..035); User story 020-05 managed-executor sinks and operations (T020-062 Iceberg first, then
   T020-026, T020-027, T020-036 and T020-037 once 035's connection kinds exist; T020-039..041).
5. User story 020-07 history export (T020-042..043) — unblocks 028.
6. User stories 020-08 and 020-09 retention, archival, archived runs and erasure (T020-044..048,
   T020-051..053, T020-066) — only after the principle-I amendment is ratified (T020-061, G1101).
7. Gates: observability, benchmark, runbooks, isolation battery, architecture/privilege sweep, browser
   journeys, regression (T020-054..060).

## Verification plan

New targets in `tests/Cargo.toml`: `contract_visibility_analytics_archival`,
`integration_visibility_analytics_archival`, `fault_visibility_analytics_archival`,
`e2e_visibility_analytics_archival`, `perf_visibility_analytics_archival`, with shared SDK scenarios in
`tests/contract/sdk-conformance/scenarios/` (`search-attributes.json`, `run-search.json`,
`analytics-declarations.json`, `history-export.json`, `analytics-exporter.json`). Fitness checks added:
WAL bytes, index-tuple inserts and commit latency per transition with analytics disabled and enabled;
no lock on `scheduler_shards` taken by analytics (lock monitor); no sink client linked into the engine or
control plane. Existing gates must stay
green: `scripts/check-architecture.py`, `contract_platform_foundation` (OpenAPI), the AsyncAPI
compatibility test, the worker baseline, `integration_security_multitenancy` (storage manifest and
runtime privileges), `contract_payload_classification`, `sdk_conformance`. Every result is recorded as it
is; a skipped or failing check is stated, and no task is ticked without its named evidence.
