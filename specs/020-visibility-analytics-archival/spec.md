# Feature Specification: Run visibility, analytics and archival

**Feature Branch**: `feature/020-visibility-analytics-archival`

**Created**: 2026-10-26

**Status**: Draft — specification, plan and tasks proposed; not implemented; pending owner review

**Input**: Temporal-class visibility over runs without weakening the durable core. (1) Typed, classified
**search attributes** declared per workflow version (additive IR), set at start and upserted by steps
(journaled), indexed in PostgreSQL, with a bounded **list query language**, counts and **saved views**,
consumed by 018 batch operations. (2) A journal-derived **analytics projection with pluggable sinks**
(the CQRS design the owner discussed): no dual write beyond the existing per-transaction note
(`telemetry_pending`, ADR-0017), a sharded leased projector, pure derivation of canonical grow-only
records, per-sink cursors advanced only on acknowledgement, values only through an explicit classified
`analytics` declaration; sinks: object storage (NDJSON/Parquet), ClickHouse, PostgreSQL, the outbound
broker pipeline (ADR-0025) and an external pull exporter. (3) **Archival and history export** under a
namespace retention policy: export, verify, then purge; look up archived runs; export one run's history
as a bounded document (also used by 028 replay tests).

## Objective and delivery boundary

An operator finds the runs that matter by business keys (`customerId = 'c-42' AND Status = 'FAILED'`)
without reading payloads; an analyst receives every run, attempt and transition in their own
warehouse, at least once and deduplicable, with business values only where an author declared them;
an administrator bounds how long run content stays in the platform and can prove where it went.

PostgreSQL stays the authority (ADR-0002). Search attribute values are part of the run's state and
commit with the transition that sets them. Analytics never runs on the write path and never locks a
run: it is derived from the journal after the fact, like spans (ADR-0017). A terminal run never
returns to running; retention removes content from terminal runs and leaves a stub. Retention changes
an obligation of constitution principle I, so it ships only after a MINOR amendment of that principle
(proposed in ADR-0054, not made here) and the owner's answers to O1080/O1082. External sinks
are at-least-once (ADR-0004): every record has a deterministic key and the platform never promises
exactly-once delivery.

Architecture decisions: [ADR-0053](../../docs/adr/ADR-0053-analytics-is-a-journal-derived-projection-delivered-through-per-sink-cursors.md)
(analytics projection) and [ADR-0054](../../docs/adr/ADR-0054-run-content-leaves-the-hot-store-only-through-a-verified-archive-under-a-namespace-retention-policy.md)
(retention, archival, G69). Both are Proposed. Decisions D1080–D1119, gaps G1080–G1100 and owner
questions O1080–O1082 are in [research.md](research.md).

## Dependencies and ownership

- **Uses (existing):** 001 contracts and identity; 002 journal, payloads, admission; 004 inbox and
  outbound pipeline (ADR-0012, ADR-0025); 007 Studio (ADR-0016, FR-007-008, FR-007-010); 008 telemetry
  note and monitor (ADR-0017); 009 quotas, secrets, egress (ADR-0027); 010 shards (ADR-0020); 011
  immutable versions; 014 additive IR, chains, declared disclosure (ADR-0031, ADR-0032); 016 payload
  classification (ADR-0033, FR-016-001..029); 017 OpenBao-backed secret references (ADR-0035).
- **Consumed by:** 018 batch operations select runs through the 020 query (`RunSelector`); without
  020 they use today's `RunFilter`. 028 replay tests read `sagat-run-history-v1` documents. 019 may add
  system attributes (for example a deployment version) through the registry of FR-020-005.
- **Interplay:** 024 `DATA_SNAPSHOT_PART` blobs are never deleted by run purge (ADR-0058 owns their
  retention; ADR-0054 owns run-purpose blobs). 018 consumes `RUN_PURGED` and must let the purge clear its
  `business_id` and write attribute rows for restored `search_attributes.upserted` events (G1103). 022
  cache entries copied from an erased run are invalidated by `source_run_id`.
- **Interplay (031):** 031 encrypted fields are opaque here: a search attribute source, an `upsertSearchAttributes`
  expression or an `analytics` projection that reads an encrypted field is refused at publication, and
  an encrypted envelope met at runtime is never indexed or exported (FR-020-043).
- **Not this feature:** reset/restart from a point (018); test-mode pinning (022); Workflow Streams
  (026); custom roles and folders (029); multi-region replication of archives (032).

## User Scenarios & Testing *(mandatory)*

### US01 — Find runs by typed, classified business keys (Priority: P1)

An author declares `customerId` (keyword, `internal`) and `orderTotal` (double, `internal`) on a
workflow version; runs set them at start from the input and a step upserts `fulfilmentState`. An
operator lists `customerId = 'c-42' AND Status IN ('FAILED','PARTIAL') ORDER BY StartedAt DESC`.

**Why this priority**: every other operational feature (018 batches, saved views, diagnosis) starts
from finding the right runs; today only workflow, status and origin filters exist.

**Independent Test**: publish a version with three attributes, start 500 runs with varied values, upsert
from a step, then compare search and count results with a direct evaluation of the same predicate over
the journal, including after an engine restart.

**Acceptance Scenarios**:

1. **Given** a published version declaring `customerId` with `from: input.customer.id`, **When** a run
   starts, **Then** `customerId` is indexed in the admission transaction and a search that matches it
   returns the run immediately after the `202`, with no projection lag.
2. **Given** a step with `upsertSearchAttributes: {fulfilmentState: "steps.ship.output.state"}`,
   **When** it completes, **Then** the new value, a `search_attributes.upserted` journal event and the
   step completion commit together; a crash before commit leaves neither.
3. **Given** a derived value of the wrong type, **When** the step completes, **Then** the run continues,
   the attribute is not indexed and `search_attributes.rejected` with reason `TYPE_MISMATCH` is journaled.
4. **Given** a `pii` attribute `customerEmail`, **When** a developer without the reveal grant filters on
   it, **Then** the search is refused with `FORBIDDEN`/`PII_REVEAL_NOT_GRANTED`; **When** a grant holder
   sends `reveal: ["pii"]`, **Then** results include it and one `run.search_pii` audit record is written
   before the response.
5. **Given** a query with 40 predicates, **When** it is submitted, **Then** it fails with
   `VALIDATION_FAILED`/`QUERY_LIMIT` before any SQL runs.
6. **Given** another tenant's run with the same attribute value, **When** searching, **Then** it never
   appears and a foreign namespace header is an opaque `NOT_FOUND`.

---

### US02 — Save and share run views (Priority: P2)

An operator saves "Failed gold orders this week" (query, columns, sort) privately or for the namespace;
a viewer opens a shared view with their own rights.

**Why this priority**: repeated diagnosis needs reusable views; it depends on US01.

**Independent Test**: create, edit with a revision conflict, share and open a view as three roles; a
view using a `pii` attribute is refused for a non-grant holder.

**Acceptance Scenarios**:

1. **Given** a shared view whose query uses only `public` attributes and whose columns include an
   `internal` one, **When** a viewer opens it, **Then** the Studio runs it with the viewer's rights and
   shows that column as withheld.
2. **Given** a view with a `pii` predicate, **When** a principal without the grant opens it, **Then** it
   is refused with `VIEW_NOT_EVALUABLE` naming the attribute, never run with the predicate dropped.
3. **Given** two editors, **When** both save revision 3, **Then** the second gets `CONFLICT`/`REVISION_CONFLICT`.

---

### US03 — Stream run analytics to object storage without touching the write path (Priority: P1)

An administrator creates an object-storage sink; every run, attempt, journal event, inbound event and
delivery of the namespace arrives as payload-free canonical records, readable with DuckDB or Athena.

**Why this priority**: it is the analytics foundation (the owner's preferred CQRS design); object storage
is the universal, cheapest sink and the base the other sinks reuse.

**Independent Test**: run 1,000 workflows while killing the engine during delivery, the sink
endpoint during PUT and the database connection during ack; afterwards, deduplicating the sink by
record key equals a fresh derivation from the journal, and no record is missing.

**Acceptance Scenarios**:

1. **Given** analytics enabled and an active sink, **When** runs change, **Then** the application issues
   no statement it did not issue before (the note upsert is unchanged; a trigger sets `moved_xid`), the
   added WAL, index work and commit latency are within the measured report, and the projector takes no
   run lock.
2. **Given** a batch delivered but not acknowledged (crash), **When** the projector restarts, **Then** the
   same frozen batch is delivered again with the same object names and identical bytes, then acknowledged once.
3. **Given** a write transaction that started before and commits after a later one, **When** both moved
   notes, **Then** no note is skipped: the cursor never passes the oldest running transaction.
4. **Given** an engine replica losing scheduler shard 7 mid-batch, **When** its ack commits after the
   takeover, **Then** the ack is refused by the fence and the new owner re-delivers.
5. **Given** a sink created with `startPosition: EARLIEST`, **When** it activates, **Then** existing runs
   are backfilled from the journal while live changes flow, and overlaps deduplicate by key.

---

### US04 — Export declared business records (Priority: P2)

An author declares on `charge-card` an `analytics` record `payment` with a closed schema (`amount`
`internal`, `currency` `internal`, `cardHolder` `pii`) and a field→expression projection over the step
output; a `pii` sink receives all three fields, an `internal` sink receives two.

**Why this priority**: business analytics need values, but values leave only by declaration
(ADR-0031 pattern); depends on US03.

**Independent Test**: publish declarations with each level and a blob output; derive twice and compare;
revoke the disclosure owner's grant and observe the sink pause before the next batch.

**Acceptance Scenarios**:

1. **Given** a node without `analytics`, **When** its output is exported, **Then** no value of it appears
   in any record.
2. **Given** a projection reading a field the node's `outputSchema` marks `pii`, **When** the record
   schema marks the target `internal`, **Then** publication is refused with `VALIDATION_FAILED`/`CLASSIFICATION`.
3. **Given** a value that fails the record schema or looks like a credential, **When** projected,
   **Then** a `declared_record` with `withheld: [reason]` and no value is exported and
   `analytics_records_withheld_total{reason}` grows.
4. **Given** a `$blob` descriptor output, **When** projected, **Then** only the descriptor is exported.
5. **Given** a `pii` sink whose disclosure owner loses the reveal grant, **When** the next batch is
   planned, **Then** the sink becomes `AUTO_PAUSED`/`DISCLOSURE_REVOKED` and nothing more is exported.
6. **Given** an `internal` search attribute declared without `export`, **When** a `run_snapshot` reaches
   an `internal` sink, **Then** the attribute appears by name with `withheld` reason `NOT_DECLARED` and no
   value; declared with `export: "declared-analytics-export-v1"`, its value is exported.

---

### US05 — Operate sinks: ClickHouse, PostgreSQL, broker streams, health and lag (Priority: P2)

An administrator adds a ClickHouse sink and a Kafka stream sink, watches lag, pauses, resumes, rewinds
to a date, and is alerted when a sink is stuck.

**Why this priority**: analysts use warehouses and streams, and a sink nobody can operate is a silent
data leak or a silent gap.

**Independent Test**: inject a permanent rejection, a credential failure, a stuck endpoint and an
export-quota exhaustion; each produces its typed sink state, alert and resumption without loss.

**Acceptance Scenarios**:

1. **Given** a ClickHouse table missing a column, **When** a batch is inserted, **Then** the batch is
   `REJECTED`, the sink `AUTO_PAUSED`/`SINK_SCHEMA_MISMATCH`, an `ANALYTICS_SINK_STUCK` alert fires, and no later
   batch is attempted until resume.
2. **Given** a transient 503, **When** it repeats past the threshold, **Then** the sink is `FAILING` and
   still retrying; no record is skipped.
3. **Given** a stream sink over a Kafka subscription dedicated to it, **When** a batch is acknowledged,
   **Then** its delivery rows commit in the same transaction as the cursor and are delivered by the
   ADR-0025 pipeline with stable delivery ids; a delivery that ends `DEAD` auto-pauses the sink.
4. **Given** `rewind to 2026-10-01`, **When** confirmed, **Then** records from runs created since then
   are re-exported (duplicates by design) and the action is audited.
5. **Given** the tenant's daily export bytes exhausted, **When** planning, **Then** the sink is
   `THROTTLED` and resumes the next day where it stopped.

---

### US06 — Export through a pull exporter inside the customer network (Priority: P3)

A data team runs an exporter (any of the five SDKs) inside its private network; it leases batches,
writes them to an internal store and acknowledges them.

**Why this priority**: private destinations cannot be reached by the engine's guarded egress
(D1101); this keeps network policy closed while serving them.

**Independent Test**: two exporter instances lease concurrently; one is killed after writing; the other
receives the same batch after lease expiry; an ack with a stale token is refused.

**Acceptance Scenarios**:

1. **Given** an exporter without a binding to the sink, **When** it leases, **Then** it is refused with
   `PERMISSION_DENIED`/`NOT_BOUND`.
2. **Given** a leased batch, **When** the lease expires, **Then** the generation increases and the old
   holder's ack is `ABORTED`/`STALE_ATTEMPT`.
3. **Given** credit for 500 records, **When** the batch has 1,200, **Then** the stream stops at the credit
   and continues on the next request without reordering.

---

### US07 — Export one run's history as a bounded document (Priority: P2)

A developer exports a failed run's history to attach to a bug, or to feed a 028 replay test.

**Why this priority**: diagnosis and replay testing need a portable, faithful history; it is also the
document model of archives (US08).

**Independent Test**: export runs in both modes and from an archive; compare digests; exceed the bound.

**Acceptance Scenarios**:

1. **Given** `mode: STRUCTURE`, **When** exported, **Then** the document has every journal event with
   payload digests, sizes and levels and no value.
2. **Given** `mode: DISCLOSED` by a caller without the reveal grant, **Then** `pii` values are withheld
   markers and `complete: false`.
3. **Given** a run whose history exceeds the bound, **Then** the export fails with
   HTTP 413 `BAD_REQUEST`/`HISTORY_TOO_LARGE` and nothing partial is returned.

---

### US08 — Bound retention with a verified archive (Priority: P2)

An administrator sets 90 days of hot retention with archival required to the platform archive. Terminal
runs older than that are exported, verified, then their content is purged; the run remains as a stub.

**Why this priority**: run content grows without bound today (G28, G241, G69); deleting customer data
is irreversible, so it ships behind an explicit policy (O1080).

**Independent Test**: purge 200 eligible runs while killing the engine between export, verification and
purge, and corrupting one archive object; no run loses content without a verified archive, the
corrupted one stays `BLOCKED`.

**Acceptance Scenarios**:

1. **Given** an eligible run, **When** export succeeds but verification fails, **Then** nothing is
   deleted and the job is `BLOCKED`/`ARCHIVE_INTEGRITY`.
2. **Given** a crash after the archive PUT, **When** the job resumes, **Then** the same object name is
   rewritten with identical bytes and verified.
3. **Given** a non-deleted analytics sink (paused included) that has not applied the run, **When** eligible, **Then** the purge
   waits until the sink applies it or the sink hold expires; an expired hold is counted on the sink as a
   retention gap.
4. **Given** a purged run, **When** listed, **Then** it appears with `ArchiveState = 'ARCHIVED'` and its
   journal endpoints answer `NOT_FOUND`/`RUN_PURGED`.

---

### US09 — Look up and inspect archived runs; erase a run (Priority: P3)

An auditor finds an archived run by id or system attributes and exports its history from the archive;
an administrator erases one terminal run at a subject's request.

**Why this priority**: an archive nobody can read is not evidence; erasure is the LGPD counterpart of
retention.

**Independent Test**: export history from an archive and compare with the pre-purge export; erase a run
whose input payload is shared with a retained run and check what survives.

**Acceptance Scenarios**:

1. **Given** an archived run, **When** its history is exported, **Then** the archive digest is verified
   first and the document equals the pre-purge export of the same mode and caller rights.
2. **Given** an erasure of a terminal run, **When** it completes, **Then** its content, archive object
   (platform destination) and custom attribute values are gone, sinks receive a `run_tombstone`, and a
   payload still referenced by another retained run survives (reported, not hidden).

### Edge Cases

- A transaction that writes a note runs for minutes (long migration): analytics lag grows by its age
  (G1081); nothing is skipped.
- A note purged for a finished run and later recreated (purge tombstone): the projector derives from the
  first sequence the sink has not applied (sequence 1 if its progress row was already removed); sink keys
  absorb the duplicates.
- A sink edited while a batch is in flight: the batch is `SUPERSEDED` and re-planned under the new revision.
- Two publications register the same attribute name with different types concurrently: one commits,
  the other fails `VALIDATION_FAILED`/`SEARCH_ATTRIBUTE_TYPE_CONFLICT` and publishes nothing.
- Tightening an attribute from `internal` to `pii` while a saved view uses it: the view becomes
  non-evaluable for non-grant holders on its next open.
- Retiring an attribute: new values are refused (journaled as rejected); existing values are removed by a
  bounded cleanup; queries naming it fail `SEARCH_ATTRIBUTE_RETIRED`.
- A continuation successor: its attributes come from its own start; a chain is found by `ChainId`.
- Purge eligibility for a child run whose parent is still running: not eligible until the tree is terminal.
- A concurrent run reusing a payload digest that the purge is deleting: that writer gets one retryable
  failure and succeeds on retry (D1114).
- An archive destination bucket deleted by its owner: verification fails, jobs are `BLOCKED`, nothing is purged.
- A rerun requested while the source is being purged: the purge's run lock and the ADR-0039 guard
  serialize; the rerun either commits first (its references keep the shared payloads) or is refused
  `RUN_PURGED`.
- A paused sink and an old finished run: the note and the run's progress are kept until the sink
  applies it or its hold expires; the expiry is recorded as a retention gap, never a silent skip.
- Timeout/signal races, duplicate deliveries and lost broker hints on runs that set attributes: the
  first commit wins; the loser sets no attribute and journals nothing.
- Clock: every eligibility, lease and lag decision uses the database clock.

## Requirements *(mandatory)*

### Functional Requirements — search attributes and queries

- **FR-020-001 — Registry.** Each namespace MUST keep a registry of custom search attributes: name
  (`^[a-z][a-zA-Z0-9_]{0,63}$`), type (`keyword`, `text`, `int`, `double`, `bool`, `datetime`,
  `keywordList`), level (`public`, `internal`, `pii`), state (`ACTIVE`, `RETIRED`) and revision. Names of
  system attributes are reserved. Count and per-type ceilings apply (plan, Finite ceilings).
- **FR-020-002 — Declaration.** A workflow version MAY declare `searchAttributes` (additive IR,
  ADR-0032): name, type, level, an optional `from` expression over `input`, and an optional
  `export: "declared-analytics-export-v1"` that lets its values leave to analytics sinks. Publication MUST register
  new names in its own transaction and MUST refuse a type or level that differs from the registry, a
  `secret` level, a retired name, a source reading an encrypted field, and a level looser than the
  strictest carrier level the expression reads (an unresolvable path counts as `pii`). The two carriers
  this adds are stated in "Amendment to feature 016" below; versions under rules 0 and 1 keep their
  meaning.
- **FR-020-003 — Values at start.** `startRun` MAY carry `searchAttributes` for declared names; invalid
  explicit values MUST refuse the start (`VALIDATION_FAILED`/`SEARCH_ATTRIBUTE_VALUE_INVALID`) with no
  run created. Declared `from` values are evaluated in the admission transaction. The indexed values and
  one `search_attributes.upserted` journal event (values stored as a payload, referenced by digest) MUST
  commit with the run.
- **FR-020-004 — Upserts.** `activity`, `script`, `waitEvent` and `child` nodes MAY declare
  `upsertSearchAttributes` (name → expression, `null` removes). The engine MUST evaluate them over the
  committed output in the completion transaction and commit index rows and the journal event with the
  completion. A derived value that fails type or size validation MUST NOT be indexed and MUST be
  journaled as `search_attributes.rejected` with a closed reason and no value; the run continues.
- **FR-020-005 — System attributes.** The query MUST support `RunId`, `WorkflowId`, `WorkflowName`,
  `WorkflowVersion`, `Status`, `Origin`, `StartedAt`, `FinishedAt`, `Duration`, `ParentRunId`,
  `RootRunId`, `ChainId`, `RerunOf`, `CorrelationId`, `TraceId`, `StartedBy` and `ArchiveState`, read
  from existing columns, and `BusinessId` and `ScheduleId` once 018's columns exist; later features MAY
  add system attributes only through the registry contract.
- **FR-020-006 — Consistency.** Index rows MUST equal the fold of the run's `search_attributes.*`
  journal events at every committed version (`verify_run`/`assert_consistent` extended). A transition
  that loses a race (first commit wins) writes no attribute. In 018's reset-rule registry
  `search_attributes.upserted` is `REMATERIALIZE` and `search_attributes.rejected` is `COPY` (D1618).
- **FR-020-007 — Query language.** A closed grammar ([contracts/list-query.md](contracts/list-query.md)):
  comparisons, `IN`/`NOT IN`, `BETWEEN`, `STARTS_WITH` (keyword), `CONTAINS` (keywordList), `MATCH`
  (text), `IS [NOT] NULL`, `AND`/`OR`/`NOT`, parentheses, one `ORDER BY` key plus the run-id tie-break.
  Parsing produces a typed AST in the domain; SQL is parameterized; bounds (length, predicates, list
  sizes, depth) are checked before any SQL runs.
- **FR-020-008 — Search and count APIs.** `POST /runs/search` and `POST /runs/count` (namespace header)
  take the query in the body, never in a URL. Search returns runs with disclosed attribute values and an
  opaque keyset cursor bound to the query (`CURSOR_MISMATCH` on misuse). Count is exact up to the count
  ceiling and returns `exact: false` beyond it. Each search runs under a statement budget; exceeding it
  is `QUOTA_EXCEEDED`/`QUERY_BUDGET_EXCEEDED`. `GET /runs` and `getRun` are unchanged. Unattended
  selection for 018 uses the `RunSelector` contract (contracts/list-query.md): keyset pages per round
  under the job creator's current rights, a count preview bounded by the caller's cap, no total ceiling
  of its own.
- **FR-020-009 — Classification gating.** A predicate or sort key on an attribute MUST require the
  right to see its level, else the search is refused: `public` with `run:read`; `internal` with
  `payload:read-internal`; `pii` with the reveal grant and an explicit `reveal: ["pii"]`, audited as
  `run.search_pii` before the response and never collapsed. A requested column the caller may not see
  is not a refusal: its values are returned as `{withheld: <level>}`; a `pii` value is returned only
  under the same grant and reveal flag, with one `payload.pii_revealed` record per run whose value is
  returned (FR-016-012 form; reason `search-attributes:reveal=pii`).
- **FR-020-010 — Registry management.** `search-attribute:manage` holders MUST be able to tighten a
  level and retire an attribute, with optimistic revision and audit. Loosening and type change MUST be
  refused. Retirement refuses new values and removes existing rows by a bounded, resumable cleanup.
- **FR-020-011 — Saved views.** Namespace-scoped views (name, query, ≤ 16 columns, sort, `PRIVATE` or
  `NAMESPACE`, owner, revision) with CRUD, optimistic concurrency and audit. Shared views need
  `run:view:manage`. A view always executes with the viewer's rights; a view the viewer cannot fully
  evaluate is refused with `FORBIDDEN`/`VIEW_NOT_EVALUABLE`. The Studio keeps query text out of the
  address bar and of persisted caches; a link carries a view id. When 029 is enabled, which runs a
  search, count, selection or view can see is decided by the 029 evaluator before classification
  gating applies.
- **FR-020-012 — Authoring and clients.** The five SDK builders MUST emit byte-identical
  `searchAttributes` and `upsertSearchAttributes` documents; the five clients MUST offer search and
  count; `sagat workflow runs --query` and the `RunSelector` port for 018 MUST use the same compiler and
  gating.

### Functional Requirements — analytics projection

- **FR-020-013 — No dual write; honest write-path cost.** No application statement is added to a
  transition. The added cost is a trigger on the existing note upsert (`namespace_id`, `moved_xid`,
  `moved_at`) and one index entry per note write, installed only when an operator enables analytics;
  envelope triggers and indexes are installed only when envelope capture is enabled. The cost is
  measured as WAL bytes, index-tuple inserts and commit latency per transition against a baseline.
  Telemetry projection semantics are unchanged.
- **FR-020-014 — Sharded, fenced projector.** The replica holding scheduler shard *k* projects shard *k*;
  every plan and ack transaction MUST read the shard row `FOR SHARE` in its own short statement, last
  before commit and never across the note scan or sink I/O, and proceed only with the owner, generation
  and expiry it believes. The projector MUST NOT lock a run or make a transition wait. Enabling analytics
  is gated on zero skipped shard renewals under the contention test (G1100).
- **FR-020-015 — Discovery and progress.** Per (sink, shard) a keyset cursor over `(moved_xid, run)`
  inside the sink's own `(tenant, namespace, shard)` index prefix, bounded above by the snapshot `xmin`;
  per (sink, run) the applied sequence. A sink never scans another namespace's notes.
  The cursor MUST never pass a transaction that is still running.
- **FR-020-016 — Frozen batches.** A batch (window, per-run ranges, sink revision, counts, digest) MUST
  commit before delivery; delivery MUST re-derive identical records; at most one open batch per
  (sink, shard); the cursor and per-run progress advance only in the ack transaction.
- **FR-020-017 — Canonical model.** Records follow `sagat.analytics.v1`
  ([contracts/analytics-model.schema.json](contracts/analytics-model.schema.json)): kinds `run_snapshot`,
  `step_attempt`, `journal_event`, `inbound_event`, `delivery`, `declared_record`, `run_tombstone`, each
  immutable under a deterministic key with `schemaVersion`. Every key, batch id, deduplication token and
  object name includes the 032 region epoch (0 without 032), so a lost timeline never collides with the
  new one (D1615). The model only grows.
- **FR-020-018 — Payload-free by default.** Without a declaration no payload value, event `data`,
  delivery body or blob byte MUST appear in a record; a search attribute value appears only when the
  run's pinned version declared `export` for it, otherwise its name with `withheld: NOT_DECLARED`.
  Metadata fields carry the fixed levels of data-model §4 and are filtered by the sink's `maxLevel`.
  Draft test runs (022 `DRAFT_TEST` origin and their test trees) are never exported. A `pii` field leaves
  only under the uniform egress rule of D1617 (O1081).
- **FR-020-019 — Declared records.** `activity`, `script`, `waitEvent` and `child` nodes MAY declare
  `analytics` entries `{disclosure: "declared-analytics-export-v1", record, schema, project}` where
  `schema` is a closed bounded classification carrier and `project` maps each field to an expression.
  Event endpoints MAY declare the same over `event`/`data` (data is `pii` as a whole, ADR-0033 addendum).
  The projector MUST evaluate from immutable payloads under the pinned declaration, validate against the
  schema, scan for credential-shaped values, keep blobs as descriptors, and export a `withheld` marker
  instead of any value that fails.
- **FR-020-020 — Sink resource.** Namespace-scoped sinks with typed destination (`OBJECT_STORAGE`,
  `CLICKHOUSE`, `POSTGRESQL`, `STREAM`, `PULL`), credentials as a secret reference (never a value),
  filters (record kinds, workflows), batch policy, `maxLevel`, start position (`NOW`, `EARLIEST`, a
  time), immutable revisions with optimistic concurrency, and actions pause, resume, rewind, skip, probe
  and delete, each audited. Destinations pass the egress guard; platform state endpoints are refused.
  `inbound_event`/`delivery` kinds are refused until envelope capture is enabled (`ENVELOPES_NOT_ENABLED`).
- **FR-020-021 — Native sink semantics.** Object storage: deterministic names, manifest, HEAD
  verification before ack. ClickHouse and PostgreSQL are written by the managed database executor
  (`apps/database-runner`, ADR-0035) with fixed, platform-generated statements only: ClickHouse over its
  HTTP interface with `insert_deduplication_token` per batch and kind, documented `ReplacingMergeTree`
  DDL and a `DESCRIBE` probe; PostgreSQL through a 017 connection with `ON CONFLICT DO NOTHING`
  templates on the record key and documented DDL; no platform-run DDL and no database client in the
  engine. Placement follows the cross-feature rule written by 021 in ADR-0055 (fixed-shape batches to
  non-database destinations from the engine through guarded egress; no engine or control-plane
  connection to a customer database, whatever the protocol).
  Each writer MUST map outcomes to `ACKED`, transient or permanent.
- **FR-020-022 — Stream sink.** A `STREAM` sink references a webhook or broker subscription dedicated to
  it (one that receives no run events of its own); the ack transaction MUST insert its delivery rows
  (within the 64 KiB delivery bound, stable event id) together with the cursor; delivery, retry and
  redrive follow ADR-0025, and a delivery that ends `DEAD` MUST auto-pause the sink
  (`STREAM_DELIVERY_DEAD`).
- **FR-020-023 — Pull exporter.** `AnalyticsExportService` (worker protocol, next additive minor) for a
  machine identity holding `analytics:sink:export` and a current per-sink binding — `analytics-exporter`
  for `PULL` sinks, `database-runner` for `POSTGRESQL` and `CLICKHOUSE` sinks (which also resolves the
  destination through the 017 two-phase path under the batch lease): lease with
  finite record/byte credit, renew, ack and nack, each fenced by token hash, generation and database-clock
  expiry. Five SDK helpers MUST pass shared conformance scenarios.
- **FR-020-024 — Failures never skip silently.** A permanent rejection MUST mark the batch `REJECTED` and
  the sink `AUTO_PAUSED` with a typed reason; transient failures retry with capped backoff and mark
  `FAILING` past a threshold; credentials or disclosure authority lost MUST auto-pause; `skip` (explicit
  window, audited, recorded in the ledger) is the only way past a batch. A sink edit supersedes an
  unacknowledged batch.
- **FR-020-025 — Health and lag.** Each sink MUST expose state, reason, last ack time, per-shard and
  overall lag, retention gaps and recent batch outcomes; `analytics_sink_lag_seconds{sink_kind}` is
  exported; the monitor raises `ANALYTICS_SINK_STUCK` per namespace with a runbook link.
- **FR-020-026 — Quotas.** Sinks per namespace, batch bounds, declared records per node and
  `analytics_export_bytes_per_day` per tenant MUST be enforced before work; exhaustion makes the sink
  `THROTTLED`, never drops records.
- **FR-020-027 — Backfill and rewind.** `EARLIEST`, a start time or a rewind MUST derive from the journal in
  run-id order per shard, run concurrently with live discovery and deduplicate by key; purged runs are
  reported as unreachable, not silently omitted.
- **FR-020-028 — Ledger, tombstones and erasure statement.** Acknowledged batches and their run
  manifests MUST be retained for the ledger window and answer which sinks received a run. Purge and
  erasure MUST emit `run_tombstone`. Documentation MUST state that exported data is outside platform
  deletion and is the namespace controller's responsibility (LGPD).
- **FR-020-029 — Studio sinks.** Create, edit, probe, pause, resume, rewind, skip with confirmation and
  consequence text; health and lag with a table alternative; disclosure level shown in text, not colour.

### Functional Requirements — retention, archival and history export

- **FR-020-030 — Retention policy.** `retention:manage` holders MAY set a namespace policy:
  `hotRetentionDays` (1..3650, clamped to the operator ceiling), `archive` (`REQUIRED`, `NONE`),
  `archiveValues` (`METADATA_ONLY`, `UP_TO_INTERNAL`, `UP_TO_PII`), destination (`PLATFORM` or an object
  storage destination with a secret reference) and `sinkHoldDays`. `UP_TO_PII` needs the reveal grant.
  Without a policy nothing is deleted. Changes are audited and apply to eligibility from then on.
- **FR-020-031 — Eligibility.** A run is eligible only when terminal, past its retention on the database
  clock, its whole tree terminal, its chain final (for segments), and every non-`DELETED` sink (paused and
  auto-paused included) has applied it or its hold expired. A run ended `COMPENSATION_FAILED` or
  `PARTIAL`, or with an unresolved unknown outcome or manual review, is not eligible until an audited
  resolution (feature 003's operator decision). Non-terminal or held runs are never purged. The policy
  revision applied must be an audited change by a human principal.
- **FR-020-032 — Archive format.** `sagat-run-archive-v1`
  ([contracts/run-archive.schema.json](contracts/run-archive.schema.json)): run stub, pinned version
  digest and classification rules, every journal event, payload metadata (digest, size, level) and
  values within `archiveValues` (never `secret`, never above the policy), blob descriptors, a manifest
  with record counts and SHA-256.
- **FR-020-033 — Verified before delete.** Each run follows the job states of data-model §6; export
  uses deterministic object names; verification re-reads size and digest; deletion MUST NOT start
  before `VERIFIED`. A crash at any point resumes without loss or duplicate deletion.
- **FR-020-034 — Purge.** Bounded chunk transactions per run (one for a run that fits a chunk), under the
  run-tree lock order: the first marks the stub purged, the last records `PURGED`; together they delete
  journal, steps, timers, subscriptions, attribute rows and run-owned records, releases content counters
  (ADR-0027), deletes payloads and run-purpose blobs (`STEP_OUTPUT`, `RUN_INPUT`, through ADR-0023
  deletion) that no retained row references — references found from every foreign key in the catalog
  plus registered non-FK copies, a test failing on an unregistered one — never `DATA_SNAPSHOT_PART`
  blobs (024), keeps the run row as a stub with archive pointer and a tombstone `input_digest`, applies
  the purge rule of every store in the content-store registry (data-model §6), emits the tombstone note
  and audits `run.purged`.
- **FR-020-035 — Archived runs.** Stubs remain listable and searchable by system attributes
  (`ArchiveState`); hot-only endpoints answer `NOT_FOUND`/`RUN_PURGED`; rerun and reset of a purged or
  purging run are refused `CONFLICT`/`RUN_PURGED` after the ADR-0039 identity guard (consumed by 018);
  `getRun` gains an additive, payload-free `archive` object.
- **FR-020-036 — History export.** `POST /runs/{runId}/history-exports` returns a bounded
  `sagat-run-history-v1` document ([contracts/run-history.schema.json](contracts/run-history.schema.json))
  in `STRUCTURE` or `DISCLOSED` mode, from the hot store or a verified archive, carrying what 028's
  replay needs (producer and event-schema version, the pinned version document on request, codec-level
  event detail, the recorded projection, per-attempt output digests in both modes; values only in
  `DISCLOSED` mode, where 028's `answers` needs them), with `complete`,
  `Cache-Control: private, no-store`, `run:history:export` plus FR-016-007 audiences, and audit
  (`run.history_exported`; `pii` reveals never collapsed). Oversize is HTTP 413
  `BAD_REQUEST`/`HISTORY_TOO_LARGE`.
- **FR-020-037 — Erasure.** `retention:manage` holders MAY request erasure of a terminal run: same
  pipeline, eligibility now (still refusing unresolved effects with `CONFLICT`/`EFFECT_UNRESOLVED`),
  optional archive, archive object deleted (platform destination), subject replaced in the stub,
  every store of the content-store registry handled by its erasure rule (022 cache entries by
  `source_run_id` through `step_cache_hit_provenance`, 026 stream events, 019 checkpoints, 025
  transcripts, 030 operation messages, 018 retained Update arguments), tombstone emitted, and a report
  of content that survived because another retained run references it.
- **FR-020-038 — Studio and CLI.** Retention policy panel, archival status (backlog, blocked jobs),
  archived badge and history export in run detail; `sagat workflow export-history <RUN>`,
  `sagat namespace retention get|set|status` and `sagat analytics sinks list|create|update|act`.

### Functional Requirements — cross-cutting

- **FR-020-039 — Authorization and tenant boundary.** The role matrix of the plan applies; every query,
  row and message is tenant- and namespace-scoped; foreign resources are opaque `NOT_FOUND` with the
  existing cross-tenant audit; authorization fails closed and is rechecked on cursor reuse, view open,
  batch plan and exporter lease.
- **FR-020-040 — Observability.** Spans, fixed-label metrics and redacted logs of the plan; no value,
  query text, attribute value, record content, destination URL or credential in any log, span, metric or
  error; typed errors reuse the closed `Error.code` set with the `details.reason` values of
  [contracts/errors.md](contracts/errors.md).
- **FR-020-041 — Audit.** Registry changes, `pii` searches, view changes, sink lifecycle and
  auto-pauses, `pii` batch exports, exporter bindings, retention changes, purges, erasures and history
  exports MUST be audited in the transaction of the decision. Returned `pii` values follow FR-016-012
  (`payload.pii_revealed` per run); a `PII` sink batch writes one `analytics.batch_exported` with its
  retained run manifest as per-run evidence — an explicit refinement of FR-016-012 recorded in ADR-0053.
- **FR-020-042 — Compatibility.** IR, OpenAPI, protobuf and AsyncAPI change additively; versions with
  rules 0/1 and existing runs keep their behavior; each capability is enabled separately and rolled back
  by disabling new publication/creation while readers stay.
- **FR-020-043 — Encrypted payloads (031).** Search-attribute `from`, `upsertSearchAttributes` and
  analytics `project` positions are entries of 031's encoded-field registry
  (`specs/031-client-side-encryption/contracts/workflow-encoding.md`); declarations reading an encoded
  location are refused at publication through 031's encoded-location check (`VALIDATION_FAILED` with 031's reasons
  `ENCODING_READ_BY_SEARCH_ATTRIBUTE` / `ENCODING_READ_BY_ANALYTICS`; without 031 there is no encoded
  location to detect); an encrypted envelope met at runtime is rejected for indexing (`ENCRYPTED`) and
  withheld from export; archives keep it as an opaque value only within `archiveValues`.

### Amendment to feature 016

This feature adds two classification carriers to FR-016-002's closed list: (1) the `level` of each
`searchAttributes` declaration, with the floor rule of D1091 for its `from` expression; (2) the
`schema` of each `analytics` declaration (`x-sagat-classification`, unannotated means `pii`). It does
not open the amendment alone: the single amendment of FR-016-002 and of classification rules version 2
for the consolidated list of all drafts (`SPECIFICATION-INDEX.md`) is opened by the first of those
features to be implemented; 020 contributes these two entries (T020-011).

### Non-functional requirements

- **NFR-020-SEC**: zero cross-tenant or cross-namespace result, record or archive in the security
  battery; zero canary leakage (values, query literals, credentials) into logs, spans, metrics, errors
  or audit; `secret` never declared, indexed, exported or archived.
- **NFR-020-DUR**: no acknowledged transition, attribute value, sink record or archived content lost
  under the fault matrix; nothing deleted before verification; at-least-once to sinks with
  deterministic keys and no exactly-once claim.
- **NFR-020-PERF**: write-path overhead of the trigger, attribute rows and indexes, search latency
  p50/p95/p99, projector throughput and lag, purge rate are measured on a stated environment
  (`tests/performance/reports/visibility-analytics-archival.md`); no figure is promised before then.
- **NFR-020-COMP**: existing clients, runs, versions, Studio builds and SDK protocol minors keep working.
- **NFR-020-OBS**: every accepted operation has correlated spans and audit; projection lag and archival
  backlog are visible in API, metrics and Studio.
- **NFR-020-PRIV**: retention, erasure and export ledger behavior documented with the LGPD consequences;
  the default changes no retention (O1080).
- **NFR-020-A11Y**: new Studio views meet FR-007-008 (keyboard path, labels, table alternative, level not
  by colour alone) and FR-007-010 (no sample values).

### Key Entities

- **Search attribute registration**: namespace name → type, level, state, revision.
- **Search attribute value**: run × attribute × element, typed, transactional, journaled.
- **Saved view**: named query, columns, sort, visibility, owner, revision.
- **Journal note**: the existing per-run note, now with `moved_xid`/`moved_at`.
- **Analytics sink / revision**: typed destination, secret reference, filters, disclosure level, state.
- **Sink cursor / run progress**: per (sink, shard) keyset; per (sink, run) applied sequence.
- **Batch / batch manifest**: frozen plan, state, lease (pull), ledger.
- **Exporter binding**: machine subject ↔ sink.
- **Retention policy**: namespace rule; **archival job**: per-run state machine; **run stub**.
- **History document / archive object**: `sagat-run-history-v1` / `sagat-run-archive-v1`.

## Success Criteria *(mandatory)*

### Measurable Outcomes

All are future evidence, not completed tests.

- **SC-020-001**: For 10,000 runs with mixed attributes, search and count results equal an independent
  evaluation over the journal in 100% of 200 generated queries, before and after restart (FR-020-001..008).
- **SC-020-002**: 100% of the classification battery (each level × each role × predicate/sort/display ×
  reveal, before and after a level is tightened or an attribute retired) and of the tenant isolation
  battery over every new operation is refused or audited as specified; zero canary leaks
  (FR-020-009..011, FR-020-039, NFR-020-SEC).
- **SC-020-003**: Crash injection at each commit point of admission and completion leaves index rows equal
  to the journal fold in 100% of runs (FR-020-003, FR-020-004, FR-020-006).
- **SC-020-004**: With analytics disabled the write path is byte-identical to today (no trigger, no
  index); with it enabled, WAL bytes, index-tuple inserts and commit latency per transition are reported
  against that baseline, and no application statement is added (FR-020-013, NFR-020-PERF).
- **SC-020-005**: Under the analytics fault matrix (crash before/after delivery and ack, sink timeout,
  partial PUT, shard takeover, long transaction, duplicate ack, `EARLIEST` backfill racing live changes),
  deduplicated sink content equals a fresh derivation with zero missing records in 1,000 runs
  (FR-020-014..017, FR-020-021, FR-020-027).
- **SC-020-006**: Declared-record battery: undeclared values (including search attribute values without
  `export`) never exported; every level filtered;
  credential canaries, blobs and encrypted values withheld; grant revocation pauses before the next
  batch (FR-020-018, FR-020-019, FR-020-043).
- **SC-020-007**: Each sink failure class yields its typed state, alert within one monitor round and
  resumption with zero skipped records unless an audited `skip` names the window (FR-020-020..026).
- **SC-020-008**: The search-attribute authoring, search/count and pull exporter scenarios pass in all
  five SDKs with byte-identical documents and identical observations, including lease expiry and stale
  ack (FR-020-012, FR-020-023).
- **SC-020-009**: Archival fault matrix (crash after PUT, before verify, during purge; corrupted object;
  missing bucket) deletes no content without a verified archive in 100% of 200 runs (FR-020-030..034).
- **SC-020-010**: History export from hot store and from archive produce identical documents for the
  same mode and rights; archived runs are found by `ArchiveState` and hot-only endpoints answer
  `RUN_PURGED`; 028 replays a `complete` export (FR-020-035, FR-020-036).
- **SC-020-011**: Erasure removes all erasable content, reports surviving shared payloads exactly, and
  every sink that received the run gets its tombstone and appears in the ledger answer (FR-020-028,
  FR-020-037).
- **SC-020-012**: Existing contract baselines (OpenAPI, AsyncAPI, worker protocol, IR corpus) pass
  unchanged except for additive diffs; old SDK minors keep working (FR-020-042).
- **SC-020-013**: Real-browser journeys for search, views, sinks, retention and archived runs pass with
  keyboard-only use and the table alternative (FR-020-011, FR-020-029, FR-020-038, NFR-020-A11Y).
- **SC-020-014**: The benchmark report states environment, write overhead, search p50/p95/p99,
  projector records/s and lag, purge runs/s and failures (NFR-020-PERF).
- **SC-020-015**: Every new operation emits its span and its audit record in the decision's transaction,
  metrics carry only closed labels, and the canary sweep finds no value, query literal, destination or
  credential in logs, spans, metrics, errors or audit (FR-020-040, FR-020-041, NFR-020-OBS).

## Assumptions

- PostgreSQL ≥ 13 (`xid8`); the reference compose uses 16.
- Owner defaults O1080–O1082 as recommended in research.md until answered.
- Classification audiences are those of FR-016-007; the reveal grant list is FR-016-011.
- Search attribute values are business metadata that authors choose; the platform classifies them but
  cannot judge whether a declared `public` value is in fact sensitive (as ADR-0031 states for results).

## Out of scope

Tenant-wide sinks; in-process (`.so`/WASM) sink plugins (constitution V); exactly-once delivery;
archive-to-sink replay; legal hold; Temporal-compatible protobuf exports; stemming/full-text analyzers;
retroactive indexing of existing runs; audit-record retention; cross-region archive replication (032).

## Owner questions

O1080 (default retention), O1081 (`pii` leaving the platform) and O1082 (run stubs) are stated with
recommended defaults in [research.md](research.md#5-owner-questions-recommended-defaults-are-what-the-plan-assumes).
See also the [requirements checklist](checklists/requirements.md).
