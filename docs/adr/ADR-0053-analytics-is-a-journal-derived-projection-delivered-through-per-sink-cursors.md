# ADR-0053: Analytics is a journal-derived projection delivered to pluggable sinks through fenced per-sink cursors

**Status:** Proposed
**Decided by:** agent proposal, pending owner acceptance (2026-10-26). The owner discussed and preferred
the CQRS shape (no dual write, a leased projector, per-sink cursors, declared values); this record states
it as checkable rules.
**Refines / supersedes:** refines ADR-0017 (generalizes its per-run note and projector), ADR-0020 (reuses
its shard ownership and fence), ADR-0025 (a stream sink is an outbound subscription), ADR-0031 and
ADR-0033 (values leave only by an explicit, classified declaration), ADR-0028/ADR-0033 and FR-016-012
(the audit form of a `PII` export, decision 11), and reuses ADR-0035 unchanged (the PostgreSQL sink is
written by the managed database executor). Supersedes none. Revised 2026-10-27 after independent review.
**Enforced by:** planned checks of feature 020 — the write-path fitness check (WAL bytes, index-tuple
inserts and commit latency per transition against a disabled baseline,
`tests/integration/visibility-analytics-archival/write_path.rs`), the note-coverage assertion added to
`assert_consistent`, the analytics fault suite (`tests/fault/visibility-analytics-archival/analytics.rs`),
`scripts/check-architecture.py` (engine-side sink clients only in `crates/infra-analytics`; database
sink writers only in `crates/infra-database`, used by `apps/database-runner`), the shard-renewal contention test (decision 3), the IR publication
tests for `declared-analytics-export-v1`, and the canary sweep. None exists yet.

## Context

Feature 020 must deliver every run, attempt, journal event, inbound event and delivery of a namespace to
analysts' stores (object storage read by DuckDB/Athena, ClickHouse, PostgreSQL, a broker stream, or a
process inside the customer network), and business values only where an author allowed it. Forces:

- PostgreSQL is the authority and the transition transaction is the most expensive thing in the
  platform (ADR-0002, ADR-0020 §8). Telemetry already showed that anything produced on the write path is
  lost exactly where failures are (ADR-0017 context); it pays one note upsert per run-changing
  transaction instead (`telemetry_pending`).
- External stores cannot share a transaction with PostgreSQL; delivery is at least once (ADR-0004).
  Temporal Cloud's history export says the same and leaves deduplication to the consumer.
- A cursor over a sequence or a timestamp skips rows: allocation order is not commit order.
- Payload values are confidential by default (ADR-0028, ADR-0033); exports leave the platform's deletion.
- Constitution V forbids in-process plugins of untrusted code; constitution II forbids a new process
  without measurement.

## Decision

1. **No dual write; the cost that remains is stated.** No application statement is added to a
   transition. The existing note row gains `namespace_id`, `moved_xid xid8` and `moved_at`, set by a
   trigger when `head_seq` (or the purge tombstone) moves, plus one index entry. That trigger and index
   are write-path cost; they MUST be installed only when an operator enables analytics (migration
   role), and envelope triggers only when envelope capture is enabled. The fitness check is WAL bytes,
   index-tuple inserts and commit latency per transition against the disabled baseline. Telemetry
   semantics MUST stay unchanged. Every journal-appending transaction MUST move the note (checked by
   `assert_consistent`).
2. **Discovery by transaction horizon.** A consumer MUST read notes only with `moved_xid` below
   `pg_snapshot_xmin(pg_current_snapshot())`, in keyset order `(moved_xid, run)` inside its own
   `(tenant, namespace, shard)` index prefix, from a per (sink, shard) cursor, with the horizon and the
   note scan taken from the same snapshot. A sink MUST NOT scan another namespace's notes. It MUST NOT use
   a sequence or a timestamp as its watermark.
3. **Sharded and fenced.** The replica that owns scheduler shard *k* (ADR-0020) projects shard *k*.
   Every plan and acknowledgement transaction MUST read the shard row `FOR SHARE` in its own short
   statement, last before commit, never across the note scan or sink I/O, and proceed only with the
   owner, generation and expiry it believes: ADR-0020 renewal skips locked rows, so a long-held share
   lock would let leases expire. Enabling analytics is gated on zero skipped renewals
   (`scheduler_shard_renewals_skipped_total`) under a 16-sinks × 64-shards contention test; otherwise a
   separate analytics lease table ships first. The projector MUST NOT lock a run or make a transition
   wait. Envelope rows (inbox events, deliveries) are projected under one `engine_leases` row.
4. **Derived by pure functions.** Records MUST be produced by pure functions of the run facts, the
   immutable journal, content-addressed payloads and the pinned version (`application::telemetry::attempts`,
   `application::analytics::derive`). Deriving twice MUST give identical records.
5. **Frozen batches, per-sink cursors.** A batch (window, per-run sequence ranges, sink revision) MUST
   commit before delivery; a retry MUST deliver identical bytes under the same object names and
   deduplication tokens. The (sink, shard) cursor and the (sink, run) progress MUST advance only in the
   transaction that records the sink's acknowledgement. At most one open batch per (sink, shard).
6. **Canonical, versioned, grow-only model.** Records follow `sagat.analytics.v1`, each immutable under a
   deterministic `(key, version)`. Every key, batch id, deduplication token and object name MUST include
   the cell/region epoch of feature 032 (0 when 032 is absent), so that after a failover records of the
   lost timeline never collide with the new timeline's records of the same sequences; 032 owns the
   activation rewind and the tombstones for lost ranges. Draft test runs (022 `DRAFT_TEST`) are never
   exported. The model MUST only grow; an incompatible change is a new model version published beside the
   old one.
7. **Values leave only by declaration.** No payload value, event data, delivery body or blob byte may
   appear in a record unless a pinned `analytics` declaration with `disclosure:
   "declared-analytics-export-v1"` projects it into a closed, classified schema. Each field's level is the
   stricter of its annotation and every carrier path it reads (undeclared ⇒ `pii`). A sink receives fields
   up to its `maxLevel` (`internal` or `pii`); `pii` requires a disclosure owner holding the reveal grant,
   rechecked at every batch. This is the 020 instance of one uniform `pii`-egress rule proposed as a
   refinement of ADR-0033 (owner question O1081 of feature 020, cited by 025, 026 and 031): a `pii` value
   leaves only to a destination whose revision declares a ceiling ≥ `pii`, set by an administrator holding
   the reveal grant; the grant is rechecked at each delivery or batch; revocation pauses the destination;
   each delivery or batch is audited; `secret` never leaves. Search attribute values are payload-derived, so
   they leave only when the pinned version opts in per attribute (`export:
   "declared-analytics-export-v1"`); otherwise the name travels with a `withheld` marker. Encrypted
   (031) and credential-shaped values are withheld with a marker, never silently dropped.
8. **Sinks.** Placement follows the cross-feature rule written by feature 021 in ADR-0055 (rule 11): a
   delivery of a fixed, platform-defined shape to a tenant-configured non-database destination may be
   sent by the engine through the guarded egress client (11(b)); no engine or control-plane process opens
   a connection to a customer database, whatever the protocol (11(c)). Object storage MUST run in the
   execution engine through `crates/infra-egress` (public destinations only, platform state endpoints
   refused), with clients only in `crates/infra-analytics`. Database sinks MUST NOT be reached from the
   engine: PostgreSQL (through a 017 connection) and ClickHouse (over its HTTP interface, settings on the
   sink) batches are leased and written by a bound `database-runner` identity, which resolves the
   credential through the ADR-0035 two-phase path under the batch lease and sends only fixed,
   platform-generated, key-idempotent statements (inserts and a fixed probe) under the managed
   executor's destination policy (ADR-0035 decisions 3, 4 and 11 unchanged).
   A stream sink MUST reuse the ADR-0025 pipeline on a subscription dedicated to it; its delivery rows
   commit with the cursor, and a `DEAD` delivery pauses the sink. Other private destinations MUST use the
   pull exporter: a machine identity (`analytics-exporter`, `analytics:sink:export`, per-sink binding)
   that leases frozen batches with finite credit and acknowledges them under token, generation and
   database-clock expiry. No `.so`, WASM or in-process plugin is loaded.
9. **Failures never skip silently.** A permanent rejection MUST pause the sink with a typed reason; a
   transient failure retries with capped backoff; the only way past a batch is an explicit, audited
   `skip` recorded in the export ledger.
10. **Lag and health are found by looking.** Lag per (sink, shard) is now minus the oldest `moved_at`
    beyond the cursor; `analytics_sink_lag_seconds{sink_kind}` carries no sink, tenant or run label; the
    monitor raises `ANALYTICS_SINK_STUCK` per namespace (ADR-0017 §6). Every non-`DELETED` sink, paused
    ones included, holds the notes and progress of the runs it has not applied until its hold expires;
    the expiry is recorded as a retention gap on the sink.
11. **Audit of exported values refines FR-016-012.** FR-016-012 audits each `pii` reveal per run as
    `payload.pii_revealed`. A sink export of `PII` values is one disclosure decision per batch: it MUST
    write one `analytics.batch_exported` audit record in the acknowledgement transaction, and the batch's
    retained run manifest (the export ledger) MUST be the per-run evidence for its retention window. A
    search that returns `pii` attribute values keeps the FR-016-012 form (one record per run).

## Considered options

1. **Journal-derived projection with horizon cursors (chosen).** No write-path statement; correctness
   from the journal and a commit-safe horizon; sinks independent of each other.
2. **Dual write (emit analytics records in the transition transaction to an outbox).** Rejected: a write
   per record kind per transition on the hottest path, records lost or doubled with the transaction's
   own failure modes, and values copied into PostgreSQL before any sink asked for them.
3. **A derived feed table in PostgreSQL (derive once, deliver many).** Simpler cursors (one integer per
   sink and shard), but every journal entry is stored twice in the authority, declared values become a
   second retained copy subject to quotas and erasure, and a stuck sink fills the feed. Rejected; it
   remains the fallback if per-sink derivation CPU dominates in the benchmark.
4. **Logical decoding (CDC) of the platform database.** Rejected: a replication slot held by a slow sink
   retains WAL for the whole cluster, decoding exposes every column (including payload bytes) before any
   classification, and slot ownership is outside the shard fence.
5. **Sequence or timestamp watermarks.** Rejected: a transaction that allocated earlier and commits later
   is skipped forever.
6. **One shared note scan per shard fanning out to every sink of every namespace.** Rejected: isolation
   would rest on an in-memory filter and one slow sink would stall the scan of the others; the
   namespace-prefixed index gives each sink its own range.
7. **Database clients in the engine with public egress only (PostgreSQL, ClickHouse over HTTP).** Rejected
   after review: it diverged from ADR-0035 (customer database connections belong to the isolated managed
   executor, whatever the protocol) and could not reach private databases.

## Consequences

- Analytics lag is at least the age of the oldest running write transaction in the cluster (horizon).
  Long transactions delay analytics; they never lose records. Measured as
  `analytics_xmin_horizon_age_seconds`.
- Sinks see duplicates by design: a crash between delivery and acknowledgement redelivers the same batch.
  ClickHouse deduplication is eventual (`FINAL` for exact reads); object-storage readers deduplicate by
  key; PostgreSQL uses `ON CONFLICT DO NOTHING`.
- Per-sink derivation multiplies CPU by the number of sinks of a namespace (bounded by the sink ceiling).
- Scheduler and analytics share a replica's shards; the renewal-skip gate decides before release
  whether a separate lease table is required (feature 020 G1100), additive either way.
- The `database-runner` gains `analytics:sink:export` for the `POSTGRESQL`, `CLICKHOUSE` and `ICEBERG`
  sinks it is bound to (Iceberg through feature 035's connection and executor, added 2026-10-27) and an HTTP client for ClickHouse; it still never receives the platform database credentials.
- Data that reached a sink is outside platform deletion; purge and erasure emit tombstones and the export
  ledger says which sinks received a run. This is a stated LGPD consequence, not a guarantee.
- Rollback: disable sink creation and planning; cursors, batches and the note columns stay; the trigger is
  harmless to the write path when no sink exists.

## Evidence

None yet: this is a proposal. Required before acceptance of the implementation: the write-path fitness
check against the disabled baseline, the shard-renewal contention test, the analytics fault matrix (crash before/after delivery and ack, partial
PUT, shard takeover, long transaction, duplicate ack, backfill racing live changes), the declared-record
security battery, the five-SDK exporter scenario and the benchmark report
`tests/performance/reports/visibility-analytics-archival.md`. Specification:
[specs/020-visibility-analytics-archival](../../specs/020-visibility-analytics-archival/spec.md),
research D1092–D1111.
