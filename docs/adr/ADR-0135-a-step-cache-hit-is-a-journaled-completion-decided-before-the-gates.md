# ADR-0135: A step cache hit is a journaled completion decided in a lookup round before the gates, and a cache entry is a retained reference stored after the fact

**Status:** Proposed
**Decided by:** agent proposal (feature 022, design of the rest of release 0.3.0, 2026-11-24) under the owner's delegation (OD-2026-11-24-4, "Tomem a melhor decisão"); the coordinating session accepts or rejects it after the independent architecture, correctness and security reviews (task T022-200)
**Refines / supersedes:** refines ADR-0094 (a lookup round, asked before its gates round, joins the evaluation rounds before `persist`; its decision 4 covers the new round) and ADR-0056 decision 2 (where the cache rows sit in the as-built order, amended 2026-11-24); builds on ADR-0089 decision 1 (a step waits outside `READY` until a journaled decision), ADR-0096 (a paused run makes no progress), ADR-0027 (deferred content charge) and ADR-0050 decision 9 (fleet activation). Supersedes none.
**Enforced by:** proposed checks, built by the slices of US01 in `specs/022-flow-control-step-policies/tasks.md`: `crates/domain/tests/step_cache.rs` (the reducer never makes a cacheable step `READY` before `step.cache_hit` or `step.cache_missed`; the planner asks `Evaluation::CacheLookup` before `Evaluation::Gates`; an answer that leaves a step undecided is refused); `integration_flow_control_step_policies::cache` (one test per readiness source, a hit with no lease, attempt or permit, a miss that reaches its gates in the same commit, the task-ready hint absent for a step a hit completes, a paused run looked up on resume); `verify_run` (invariant 11 as amended and invariant 14 of `data-model.md`); a rule of `scripts/check-architecture.py` that only the invalidation, purge, erasure and store-writer files lock a cache row exclusively and that those files lock no run, semaphore, window or counter row after one (task T022-211); `fault_flow_control_step_policies::cache` (task T022-020).

## Context

Feature 022 US01 (`specs/022-flow-control-step-policies/spec.md`, FR-022-001 to FR-022-007) lets an author declare that a
side-effect-free root `activity` or `script` step may reuse a stored result for a period. The approved design (D1164,
D1165) did the lookup "inside the readiness transaction" and copied the entry's value into the consumer run. The release
0.3.0 slice, built since, changed the ground under it:

- Readiness of a gated step is decided by an evaluation round that the planner asks once it has nothing else to emit,
  before `persist` (ADR-0094), because `persist` charges usage rows first and cannot route a failure or continue the run.
  A lookup has the same two needs: a hit completes the step and the planner must continue with its successors in the
  same commit.
- `persist` writes the task-ready hint on `step.released` of any node that declares no gate
  (`crates/infra-postgres/src/execution.rs`, the `StepReleased` arm near lines 5692 to 5703). A step that a hit completes
  in the same commit would be announced to workers that can never lease it.
- Invariant 11 (`data-model.md`) requires that a gated step leaving towards readiness commits with its grants or its
  `*.queued`. A hit has neither: it completes the step before any gate would be asked.
- Run outputs are already content-addressed per tenant in the append-only `payloads (tenant_id, digest)` table
  (migration `20261009000100_execution`), inserted `ON CONFLICT DO NOTHING`, and charged per run as logical bytes
  (`charge_logical_output_bytes`). An inline copy in the entry would be a second content store.
- A concurrency or rate limit protects what a step calls. A hit calls nothing.

## Decision

1. **A cacheable step waits outside `READY` until its lookup is decided.** For a node that declares `cache`, the reducer
   MUST map the step's first `step.released` in a run to `WAITING` (wait kind `CACHE_LOOKUP`). Only `step.cache_hit`
   (followed by `step.completed` with `source: "CACHE"`, attempt 0) or `step.cache_missed {outcome}` (`miss`, `expired`,
   `invalidated`, `unauthorized`, `bypassed`) MUST move it on: a hit to `SUCCEEDED`, a miss to `READY`, or to `WAITING` on
   its gates when the node also declares `concurrency` or `rateLimit`. A step is looked up once per run: a retry, the
   scheduler's or an operator's, is an attempt of the miss and MUST NOT be looked up again.
2. **The lookup is an evaluation round, asked before the gates round.** Once the planner has nothing else to emit it MUST
   ask `Evaluation::CacheLookup` for every step that waits for its lookup, before it asks `Evaluation::Gates`. The
   application evaluates the key over the committed run data (an evaluation error or an oversized value is `bypassed`,
   never a failure), computes its keyed digest, and checks that the publisher may still use every secret the step
   declares (metadata only, no value resolved; otherwise `unauthorized`). The adapter reads the scope row, then the entry
   row, `FOR SHARE`, sorted, and decides on the database clock of that statement: a live entry whose scope was not
   invalidated after its creation is a hit. The domain checks the answer and journals it in the same commit.
3. **It fails closed.** `persist` MUST refuse a commit that leaves a cacheable step waiting for its lookup without a
   decision, except while its run is paused (decision 6). A consumer of the pending progress that cannot answer the round
   MUST refuse its command (ADR-0094 decision 4). A store that cannot answer fails the transaction, which its driver
   retries; an unanswered lookup MUST NOT be read as a miss.
4. **A hit takes no permit and no slot.** Because the lookup round precedes the gates round, a hit completes the step
   before it asks for any gate. Invariant 11 reads: every commit in which a step of a gated node leaves towards readiness
   contains, for that step, the grants of all its gates, a `*.queued`, a `step.cache_hit` or a `step.mocked`. A miss of a
   gated step is decided by the gates round of the same commit.
5. **Only a step `READY` at the end of its commit is announced.** `persist` MUST write the task-ready hint for a step
   only when the step is `READY` when the commit ends, whatever event released it.
6. **A paused run makes no progress.** While its run is paused, a cacheable step that is released waits for its lookup
   undecided (`CACHE_LOOKUP`), and the planner MUST NOT ask its lookup; the resume asks it, as ADR-0096 decision 2 does for
   gates.
7. **Cache rows are shared by deciders and exclusive only to transactions that lock nothing after them.** A deciding
   transaction MUST lock cache rows only `FOR SHARE`, after the path's entity locks and the active-run counter row, and,
   in its first lookup round, before any semaphore or window. A cacheable successor released after the gates round of the
   same commit (by a key failure's route) is asked in a later lookup round of that commit. The only exclusive takers of a
   cache row (invalidation, purge, erasure, and the store writer's insert) MUST lock no run, semaphore, window or counter
   row after it, only usage rows last; they can therefore be in no cycle with a decider, which is the argument ADR-0050
   decision 7 makes for the shard lease row.
8. **An entry is a retained reference.** `step_cache_entries` MUST reference its value by `(tenant_id, payload_digest)`,
   a foreign key to `payloads (tenant_id, digest)`, and MUST NOT hold an inline copy. A hit's `step.completed` references
   the same payload; no payload row is written; the consumer run is charged the output's logical bytes like any
   completion, so `RUN_DATA_LIMIT` and the storage-quota path apply to a hit as to a completion. An entry adds only its
   fixed-size row, counted against the namespace's cache-entries ceiling.
9. **An entry is stored after the fact.** A successful completion of a miss with an inline output MUST write a store
   obligation in its completion commit, referencing the completed attempt's payload; a leased writer inserts the entry if
   absent, with its expiry on the database clock. Storing MUST NOT fail, delay or roll back the step or the run.
10. **Isolation and activation.** A draft test tree looks up and stores only in the version scope of its private
    version; shared scopes are bypassed (D1188). The behaviour is the registry capability `step-cache-v1`, required of
    control-plane and engine processes; until an operator activates it, publication and starts that use `cache` are
    refused (`FLOW_CONTROL_ACTIVATION_PENDING`).

## Considered options

1. **Wait outside `READY`, a lookup round before the gates round, an entry by reference** — chosen. Safety does not
   depend on any driver remembering to look up: the reducer never makes the step `READY` before the decision, and the
   decision is a journal fact, so replay reaches the same state without reading the cache. The round shape is the one the
   slice proved for gates, and the reference reuses the store every output already lives in.
2. **The lookup inside `persist`**, as D1164 first placed it — rejected: `persist` cannot run the planner, so a hit could
   not continue the run in its commit, and it is where the hint is written for a step the hit completes; it would also
   lock cache rows after the usage rows `persist` charges (the reason ADR-0094 moved the gates out).
3. **The step `READY`, looked up by the claim or the worker** — rejected: it puts cache rows on the claim's hot path
   (NFR-022-007), spends a lease and an attempt to complete a step that needs neither, and a worker-side cache has no
   tenant boundary or audit (the reason US01 exists).
4. **A scheduler pass that looks up waiting steps** — rejected: a hit would wait for a pass (up to its interval) instead of
   completing in the commit that released the step, and the pass adds a transaction per hit.
5. **The gates first, then the lookup** — rejected: a hit would hold a permit or consume a rate slot for a call it never
   makes, and wait behind a full key for a result it already has.
6. **An inline copy of the value in the entry**, as the data model first said — rejected: up to 256 KiB duplicated per
   entry, a second content store with its own byte triggers, classification and erasure, for bytes `payloads` already
   holds once per tenant.

## Consequences

- Good: a hit completes in the commit that released the step, with no lease, attempt, permit or slot; replay never reads
  the cache; the task-ready hint becomes one rule for every step; the cache adds no content store.
- Bad: a cacheable node adds the lookup round's statements (the key's keyed digest, the scope and entry reads, the
  secret-usability check) to every first readiness of the node; the line is recorded in the hot-path budget by the
  change that adds it. A definition without `cache` adds nothing (NFR-022-007).
- Harder: every consumer of the pending progress must answer `Evaluation::CacheLookup` or refuse; a new writer of cache
  rows must keep decision 7; erasure of a source run must delete its entries before anything can touch the payload they
  pin (the 022-side port, decision D022-217 of the feature's research).
- Journal: `step.cache_missed` is new vocabulary (reset rule `COPY`); `step.cache_hit` keeps `REMATERIALIZE`.
- Compatibility and rollback: additive tables, events and wait kind; activation through the registry, so no older binary
  serves a cacheable step. Disabling `stepCache` stops new publications and starts; the store writer and the purge keep
  running until obligations and entries drain; tables are dropped only when empty.

## Evidence

None yet: the tests named under **Enforced by** are built by tasks T022-015 to T022-020, T022-209 to T022-212. Spec:
FR-022-001 to FR-022-007 and their amendments of 2026-11-24; research: D1161 to D1168, D1188, D022-203, D022-204,
D022-205, D022-206; data model: the cache tables and invariants 11 and 14.
