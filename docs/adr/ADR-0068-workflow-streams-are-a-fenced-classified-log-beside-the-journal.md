# ADR-0068: Workflow Stream events are a fenced, budgeted, value-carrying log beside the journal, read only through a classified feed

**Status:** Proposed
**Decided by:** agent proposal, pending owner acceptance (2026-10-26); number allocated by the parity
coordinator after the architecture review of feature 026
**Refines / supersedes:** refines ADR-0016 (adds a second live feed next to the graph stream, which
stays payload-free), ADR-0028 and ADR-0033 (a new place that shows values, under the feature-016
projection), ADR-0027 (a new retained-content table) and ADR-0017 (the journal stays the only source
of telemetry); supersedes none.
**Enforced by:** proposed checks of feature 026, none of which exists yet — an integration test that
an append changes no `runs.version`, journal row, outbox row or `telemetry_pending` row
(T026-021); fault tests for lease expiry, lost responses and the append/completion race (T026-046); a
byte-accounting test that the tenant's content bytes after the purge equal those before the first
append (T026-024); the storage-classification manifest test, which fails if the data column is not
classified `content` (T026-012); disclosure canaries over the feed for every audience (T026-023); a
test that a workflow expression, a Query/Update and a step input cannot observe an appended event
(T026-021); and review of the evaluation-context builders (for example
`crates/application/src/operation_evaluation.rs`), because no layer check can express "this port is
never read by that code path".

## Context

Feature 026 needs live progress and partial results from running steps: rows processed, a percentage,
text produced so far, and, for feature 025, model token deltas at a much higher rate. Clients must
resume after a disconnection without losing or repeating events, see which events belong to an attempt
that was later superseded, and see only the values their audience may see (ADR-0033, feature 016).

The platform's invariants push against every obvious place to put this data. The journal is the
authority of run state; its sequence is the run version, telemetry is derived from it (ADR-0017), the
graph stream follows it (ADR-0016), and replay must equal the projection. The graph stream, journal
listing and telemetry carry no values (ADR-0028). Retained content is charged exactly and released on
deletion (ADR-0027), but the shared content-addressed `payloads` table is never deleted from, so data
put there cannot be purged on a stream's own schedule. NATS is a hint, never the only record
(constitution VII). Temporal's Workflow Streams keep the log in workflow state, which exists only
because of code replay and is not available to a declarative engine.

## Decision

1. **A separate log.** Stream events MUST be stored in their own append-only table, one row per event,
   keyed by tenant, run and a per-run gap-free offset allocated in the append transaction, with the
   event data inline in the row. The data MUST NOT be written to the journal, the outbox, the
   `payloads` table or object storage.
2. **No effect on execution.** An append MUST NOT change the run version, the journal, the run's state,
   the scheduling, the outbox or `telemetry_pending`, and stream data MUST NOT be readable by workflow
   expressions, Query/Update evaluation or step inputs. What a workflow depends on remains the step
   output.
3. **Fenced by the lease.** Only the holder of the current lease of a node that declares the channel
   MAY append, through a worker-protocol rpc whose transaction locks the step's lease row in share mode
   and the run's stream head for update, never the run row. A stale lease stores nothing. Deduplication
   is by node instance, attempt, generation and publisher sequence.
4. **Declared and bounded.** Channels are declared in the pinned version with a closed classified
   schema and a budget; `sagat.`-prefixed channels are reserved for features that fix their schema,
   classification and budget (feature 025's token stream). Each channel, the author pool, the reserved
   pool and a per-run hard ceiling are enforced before storage, and bytes are charged to
   `tenant_content_bytes`. Exhaustion refuses the append and never fails the step.
5. **Read only through a classified feed.** Events leave the platform only through the run's stream
   feed (SSE and a JSON page), which projects every event's data for the reader's audience under
   feature 016, audits a `pii` reveal, never sends `secret`, and is bounded and rechecked like the graph
   stream. The feed also sends derived control events (an `attempts` snapshot on open or resume,
   `superseded`, `settled`) computed from committed attempt state at read time. The graph stream,
   journal listing, telemetry and audit stay payload-free.
6. **Deleted on its own schedule, never copied.** Rows are deleted, data included, a bounded time after
   the run ends (never later than the run's retention; erasure deletes them at once), one run per
   transaction, releasing exactly the bytes they were charged; the store is an entry of feature 020's
   content-store registry. A reset of feature 018 does not copy stream events: the new run starts an
   empty log (its entry in feature 018's reset-rule registry).
7. **Cursors carry the cell epoch.** The feed's cursor is `<epoch>.<offset>` with feature 032's cell
   epoch (0 without 032); a cursor of another epoch makes the feed reset the reader instead of resuming,
   because a promoted cell may reuse offsets of a tail lost inside the RPO.
8. **Every publisher uses the same append.** SDK workers and managed executors call the rpc; scripts use
   feature 023's reserved, capability-gated `stream.append` host call, which their supervisor turns into
   the same append under the attempt's lease.

## Considered options

1. **Own table with inline data (chosen).** Keeps the journal, replay and telemetry unchanged; the lease
   fence and per-run offsets give exactly-once storage per publisher sequence and resumable reads;
   inline data makes purging and accounting exact. Cost: a second retained store with its own purge,
   classification and budget rules, and a second feed to secure.
2. **Journal events (`stream.appended`).** One store and one sequence, but every token delta would
   become run history: it would grow replay, telemetry derivation and the graph stream with high-rate
   non-state data, move values into the projection path that ADR-0028 keeps payload-free, and make a run
   version change for something that changes no state. Rejected.
3. **NATS JetStream stream per run or per tenant.** Natural streaming with consumer offsets, but NATS
   would hold the only copy of a retained value, against constitution VII; per-run subjects are an
   unbounded subject space (`docs/architecture/storage-and-events.md`); tenant isolation, classification
   and exact byte accounting would have to be rebuilt outside PostgreSQL. Rejected; NATS may still carry
   identifier-only hints later (G1323).
4. **Blob objects (append chunks to object storage, ADR-0023).** Cheap for large volumes, but object
   storage has no append, no transactional offset allocation with the lease fence, and no per-event
   classification; small events would cost an object each or need a buffer that loses events on a
   crash. Rejected for events; large step outputs keep using blobs.
5. **Data in the shared `payloads` table, referenced by digest.** Reuses content addressing and
   accounting, but nothing deletes `payloads` rows, so a purge could not free bytes and per-run counts
   would drift from tenant accounting (architecture review, feature 026). Rejected.

## Consequences

- A second value-carrying feed exists; it is a release-blocking surface for disclosure tests.
- Write amplification: each event is a row and index entry; budgets and the per-attempt rate bound it,
  and the benchmark of feature 026 measures it (no throughput is promised).
- The feed polls the database like the graph stream (G143, G1323); a push path belongs to scale work.
- Consumers must handle superseded attempts; the SDKs and the Studio implement the rule once.
- Compatibility: additive table, rpc minor, IR fields and REST operation; older workers never append;
  rollback disables new publication of `streams`/`emits` and keeps the feed and purge for stored rows.

## Evidence

None yet: this is a proposal. Required before acceptance: the tests named under **Enforced by**, the
five-language `workflow-streams` conformance scenario, and the stream measurements of
`tests/performance/reports/invocation-human-surfaces.md` with environment and failures. See
[spec](../../specs/026-invocation-human-surfaces/spec.md) FR-026-013..021,
[data model](../../specs/026-invocation-human-surfaces/data-model.md) and
[worker protocol](../../specs/026-invocation-human-surfaces/contracts/worker-protocol.md).
