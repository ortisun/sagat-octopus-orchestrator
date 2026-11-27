# ADR-0053: Analytics is a journal-derived projection delivered to pluggable sinks through fenced per-sink cursors

**Status:** Accepted (2026-11-24)
**Decided by:** agent proposal of 2026-10-26, revised and accepted by an agent, delegate (Owner decision
2026-11-24-4), after the readiness review of feature 020 on 2026-11-24
(`specs/020-visibility-analytics-archival/research.md` §5a, Decision 020-013): its rules hold against
`origin/develop` at `546d5aa5`. The owner discussed and preferred the CQRS shape (no dual write, a leased
projector, per-sink cursors, declared values); this record states it as checkable rules. Revised with the
acceptance: decision 3 no longer fences on the scheduler shard row (Decision 020-003); decision 8 places
every connection-backed sink in the managed executor, as the owner's answer to feature 035's O035-005
(2026-11-02) requires (Decision 020-002); decision 7's `pii`-egress rule is the one decided under owner
question O1081 of feature 020; capabilities are switched on through the instance registry of ADR-0082
(Decision 020-004).
**Refines / supersedes:** refines ADR-0017 (generalizes its per-run note and projector), ADR-0020 (reuses
its shard ownership to route work, without its fence), ADR-0025 (a stream sink is an outbound subscription), ADR-0031 and
ADR-0033 (values leave only by an explicit, classified declaration), ADR-0028/ADR-0033 and FR-016-012
(the audit form of a `PII` export, decision 11), ADR-0071 (feature 035's connection kinds and writers
serve the connection-backed sinks), and reuses ADR-0035 unchanged (every connection-backed sink is
written by the managed executor) and ADR-0082 (activation). Supersedes none. Revised 2026-10-27 after
independent review and 2026-11-24 at acceptance.
**Enforced by:** planned checks of feature 020 — the write-path fitness check (WAL bytes, index-tuple
inserts and commit latency per transition against a disabled baseline,
`tests/integration/visibility-analytics-archival/write_path.rs`), the note-coverage assertion added to
`assert_consistent` (`tests/support/engine.rs`), the analytics fault suite (`tests/fault/visibility-analytics-archival/analytics.rs`),
the lock-monitor assertion that analytics takes no run lock and no `scheduler_shards` lock (decision 3),
`scripts/check-architecture.py` (no sink client linked into `apps/execution-engine` or
`apps/control-plane`, by `cargo tree -e features` as feature 035's D035-058 check does; sink writers only
in `crates/infra-database` and `crates/infra-lake`, used by `apps/database-runner`), the managed-executor
sink suite (`tests/integration/analytics-sinks/`), the IR publication
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
   are write-path cost; they MUST be installed only when an operator activates the `analytics-v1`
   capability of the instance registry (ADR-0082; schema-owning role, the index built `CONCURRENTLY`
   first, the trigger in the activation transaction), and envelope triggers only when envelope capture
   is enabled. The fitness check is WAL bytes,
   index-tuple inserts and commit latency per transition against the disabled baseline. Telemetry
   semantics MUST stay unchanged. Every journal-appending transaction MUST move the note (checked by
   `assert_consistent`).
2. **Discovery by transaction horizon.** A consumer MUST read notes only with `moved_xid` below
   `pg_snapshot_xmin(pg_current_snapshot())`, in keyset order `(moved_xid, run)` inside its own
   `(tenant, namespace, shard)` index prefix, from a per (sink, shard) cursor, with the horizon and the
   note scan taken from the same snapshot. A sink MUST NOT scan another namespace's notes. It MUST NOT use
   a sequence or a timestamp as its watermark.
3. **Sharded for work, fenced by the batch** (revised 2026-11-24). The replica that owns scheduler shard
   *k* (ADR-0020) plans shard *k*, from the shards its keeper holds in memory; ownership only routes
   work. Correctness MUST NOT depend on it: at most one open batch per (sink, shard) by a unique index;
   every batch transition conditional on the batch row read `FOR UPDATE`; a leased batch fenced by its
   token, generation and database-clock expiry. Two replicas that plan the same shard collide on the
   index and one rolls back; a late acknowledgement of an acknowledged or superseded batch writes
   nothing. The projector MUST NOT read `scheduler_shards`, lock a run or make a transition wait:
   ADR-0020 renewal skips locked rows, so any share lock analytics took would put the scheduler's
   leases at risk. Envelope rows (inbox events, deliveries) are projected under one `engine_leases` row.
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
   rechecked at every batch. This is the 020 instance of one uniform `pii`-egress rule, a refinement of
   ADR-0033 decided under owner question O1081 of feature 020 on 2026-11-24 (delegate, Owner decision
   2026-11-24-4; cited by 025, 026 and 031): a `pii` value
   leaves only to a destination whose revision declares a ceiling ≥ `pii`, set by an administrator holding
   the reveal grant; the grant is rechecked at each delivery or batch; revocation pauses the destination;
   each delivery or batch is audited; `secret` never leaves. Search attribute values are payload-derived, so
   they leave only when the pinned version opts in per attribute (`export:
   "declared-analytics-export-v1"`); otherwise the name travels with a `withheld` marker. Encrypted
   (031) and credential-shaped values are withheld with a marker, never silently dropped.
8. **Sinks** (revised 2026-11-24). No engine or control-plane process opens a connection to a customer
   destination or holds a customer credential; this is stricter than the cross-feature rule written by
   feature 021 in ADR-0055 (rule 11), so this record does not rely on its clause (b). Every sink whose
   destination is a 017 connection references that connection (the owner's answer to feature 035's
   O035-005, 2026-11-02: object storage and ClickHouse through 035's `object_storage` and `clickhouse`
   kinds, Iceberg through its `iceberg` kind, PostgreSQL through a `postgresql` connection), pins its
   revision, and is leased and written by a bound `database-runner` identity with feature 035's writer
   for that kind: PostgreSQL and ClickHouse on the runner's `database` profile, object storage and
   Iceberg on its `lake` profile. The runner resolves the credential through the ADR-0035 two-phase path
   under the batch lease (answered with the existing `ResolvedDatabase`) and performs only fixed,
   platform-generated, key-idempotent operations (inserts, create-only objects, append commits found by
   their batch identity, and a fixed probe) under 017's destination policy (ADR-0035 decisions 3, 4 and
   11 unchanged).
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
8. **Object storage written by the engine through the guarded egress client (the draft of 2026-10-27).**
   Rejected at acceptance: a bucket would be configured twice (a sink secret here, an `object_storage`
   connection for feature 035's synchronous destinations), a customer credential would live in the
   engine, the guarded `object_store` connector was never proven (feature 020 G1083), and private
   buckets were out of reach.
9. **A fence on the scheduler shard row as the last statement of plan and ack (the draft of 2026-10-27).**
   Rejected at acceptance: the batch row already gives every guarantee the fence gave, and the fence
   coupled analytics to the lock that keeps the scheduler's leases alive, behind a contention gate
   (feature 020 G1100). A separate analytics lease table was the draft's fallback; it would add a second
   lease system for no gain in correctness.

## Consequences

- Analytics lag is at least the age of the oldest running write transaction in the cluster (horizon).
  Long transactions delay analytics; they never lose records. Measured as
  `analytics_xmin_horizon_age_seconds`.
- Sinks see duplicates by design: a crash between delivery and acknowledgement redelivers the same batch.
  ClickHouse deduplication is eventual (`FINAL` for exact reads); object-storage readers deduplicate by
  key; PostgreSQL uses `ON CONFLICT DO NOTHING`.
- Per-sink derivation multiplies CPU by the number of sinks of a namespace (bounded by the sink ceiling).
- Scheduler and analytics share a replica's shards for routing only; two replicas may both plan a shard
  for a moment after a takeover, and the loser's work is counted (`analytics_plan_conflicts_total`).
- The `database-runner` gains `analytics:sink:export` for the `POSTGRESQL`, `CLICKHOUSE`,
  `OBJECT_STORAGE` and `ICEBERG` sinks it is bound to and for archives to an `object_storage` connection
  (ADR-0054); it reuses feature 035's writers and still never receives the platform database
  credentials. A deployment without it can use only stream and pull sinks.
- Data that reached a sink is outside platform deletion; purge and erasure emit tombstones and the export
  ledger says which sinks received a run. This is a stated LGPD consequence, not a guarantee.
- Rollback: disable sink creation and planning; cursors, batches and the note columns stay; the trigger is
  harmless to the write path when no sink exists, and is removed after the operator removes the
  `analytics-v1` requirement.

## Evidence

The design was accepted on 2026-11-24 against the code it relies on (the note writers, `purge_projected`,
the `SKIP LOCKED` renewal of `crates/infra-postgres/src/partitioning.rs`, feature 035's connection kinds
in `worker.proto` and its runner profiles); nothing of it is built. Required before the implementation
is accepted: the write-path fitness check against the disabled baseline, the lock-monitor assertion, the
analytics fault matrix (crash before/after delivery and ack, partial PUT, shard takeover, two planners,
long transaction, duplicate ack, backfill racing live changes), the managed-executor sink suite, the
declared-record security battery, the six-SDK exporter scenario and the benchmark report
`tests/performance/reports/visibility-analytics-archival.md`. Specification:
[specs/020-visibility-analytics-archival](../../specs/020-visibility-analytics-archival/spec.md),
research D1092–D1111 and Decisions 020-002, 020-003, 020-004 and 020-013.
