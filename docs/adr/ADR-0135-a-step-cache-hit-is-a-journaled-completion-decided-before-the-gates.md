# ADR-0135: A step cache hit is a journaled completion decided in a lookup round before the gates, and a cache entry is a retained reference stored after the fact

**Status:** Accepted (2026-11-25, delegate OD-2026-11-24-4, after three review rounds recorded in specs/022-flow-control-step-policies/reviews/2026-11-25/); revised after the reviews of `14b9e40b`, the re-reviews of `6d02632e` and the third reads of `c910345b` (decisions D022-222 to D022-271); wording revised 2026-11-26 after the re-reads (decision D022-285)
**Decided by:** agent proposal (feature 022, design of the rest of release 0.3.0, 2026-11-24) under the owner's delegation (OD-2026-11-24-4, "Tomem a melhor decisão"), revised with the coordinating session's rulings of 2026-11-25 (decisions D022-222 to D022-240, then D022-242 to D022-259); accepted on 2026-11-25 by the coordinating session, delegate (OD-2026-11-24-4), after three review rounds (decision D022-271; this line corrected 2026-11-26, decision D022-285)
**Refines / supersedes:** refines ADR-0094 (a lookup round, asked before its gates round, joins the evaluation rounds before `persist`; its decision 4 covers the new round) and ADR-0056 decision 2 (where the cache rows sit, amended 2026-11-24 and 2026-11-25); ADR-0050 decision 7 gives the cache rows their slot at admission (amended 2026-11-25); builds on ADR-0089 decision 1 (a step waits outside `READY` until a journaled decision), ADR-0027 (deferred content charge), ADR-0093 (per-key digest rotation) and ADR-0050 decision 9 (fleet activation). Supersedes none.
**Enforced by:** proposed checks, built by the slices of US01 in `specs/022-flow-control-step-policies/tasks.md`: `crates/domain/tests/step_cache.rs` (the reducer never makes a cacheable step `READY` or leased before `step.cache_hit` or `step.cache_missed`; the planner asks `Evaluation::CacheLookup` before `Evaluation::Gates`; an answer that leaves a step undecided is refused; a retry is not looked up again); `integration_flow_control_step_policies::cache` (one test per readiness source; a hit with no lease, attempt, permit or slot; a miss reaching its gates in the same commit; no task-ready hint for a step a hit completes; two publishers with one access list; a revoked worker binding; a hit over the retained-output limit failing `RUN_DATA_LIMIT`); `verify_run` (invariants 11 and 14 of `data-model.md`); the rule of `scripts/check-architecture.py` that only the invalidation, purge, erasure and store-writer files lock a cache row exclusively and that after a cache row they lock only usage rows (task T022-211); the barrier tests of the store against invalidation and erasure, including T2 held open across T5 and a barrier held after T3's delete against T18 (task T022-231); a `slot_order.rs` case of a cacheable admission against a revocation (task T022-224); `fault_flow_control_step_policies::cache` (task T022-020).

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
- The reviews of 2026-11-25 found that a hit could hand out a result another publisher's secret produced, that shared
  scopes could be poisoned by internal definitions, that the store could serve a value computed before an invalidation,
  lose a delete against erasure, and be blocked by a dead entry, and that a hit beyond the run's retained-output limit
  would be retried for ever.

## Decision

1. **A cacheable step waits outside `READY` until its lookup is decided.** For a node that declares `cache`, the reducer
   MUST map the step's first `step.released` in a run to `WAITING` (wait kind `CACHE_LOOKUP`). Only `step.cache_hit`
   (followed by `step.completed` with `source: "CACHE"`, attempt 0) or `step.cache_missed {outcome}` (`miss`, `expired`,
   `invalidated`, `unauthorized`, `bypassed`) MUST move it on: a hit to `SUCCEEDED`, a miss to `READY`, or to `WAITING` on
   its gates when the node also declares `concurrency` or `rateLimit`. A step is looked up once per run: a retry, the
   scheduler's or an operator's, is an attempt of the miss and MUST NOT be looked up again.
2. **The lookup is an evaluation round, asked before the gates round.** Once the planner has nothing else to emit it MUST
   ask `Evaluation::CacheLookup` for every step that waits for its lookup, before it asks `Evaluation::Gates`. The
   application evaluates the key over the transaction's data, including outputs staged in the same commit (an
   evaluation error or a value beyond the `StepCache` purpose's bound, 1 KiB by default and 4 KiB at most, is
   `bypassed`, never a failure), and computes its keyed digest under `KeyPurpose::StepCache` for each digest key version
   in force (ADR-0093). The adapter first takes the secret-access and membership locks of the secret ids the node
   declares, which are the entry's too since the fingerprint covers them (`sagat_lock_secret_access`,
   `sagat_lock_memberships`, shared, ascending, as feature 017 fences an admission; at slot 5 for an admission, where
   the definition is known); then it pre-reads, without a lock, the entry's immutable `producer_subject`, takes that
   subject's membership lock and the activity's binding advisory lock shared (`pg_advisory_xact_lock_shared`, as
   `crates/infra-postgres/src/bindings.rs:21`; at slot 5 for an admission), then reads the scope row and the entry
   row `FOR SHARE`, sorted, and decides on the database clock of that statement. A hit needs: a live entry
   (`expires_at` later than now, `created_at` later than the scope's `invalidated_at` and its key's tombstone); the
   consumer's publisher able to use now every secret id the entry records (metadata only, no value resolved); and the
   producing worker's subject the entry records still a member of the namespace holding a binding that covers the
   activity (for a script, the producing runner's subject and its binding profile), checked again on the entry read
   `FOR SHARE` (at slot 7 for an admission): a subject that differs from the pre-read is `unauthorized`. No decider
   locks a step or a `worker_sessions` row before its lookup round, since a binding replacement locks them after its key
   (ADR-0056 decision 2(a); decision D022-265). A try-lock was rejected: contention would become spurious misses. Otherwise the outcome is
   `unauthorized` (a secret or a binding) or the miss it is. The domain checks the answer and journals it in the same
   commit.
3. **It fails closed.** `persist` MUST refuse a commit that leaves a cacheable step waiting for its lookup without a
   decision. A consumer of the pending progress that cannot answer the round MUST refuse its command (ADR-0094 decision
   4). A store that cannot answer fails the transaction, which its driver retries; an unanswered lookup MUST NOT be read
   as a miss.
4. **A hit takes no permit and no slot, and passes through the outputs path.** Because the lookup round precedes the
   gates round, a hit completes the step before it asks for any gate. Invariant 11 reads: every commit in which a step of
   a gated node leaves towards readiness contains, for that step, the grants of all its gates, a `*.queued`, a
   `step.cache_hit` or a `step.mocked`. A miss of a gated step is decided by the gates round of the same commit. The
   hit's bytes are read and enter through the outputs path of `persist`, so the charge, the digest and the projection
   come from one place; before it journals the hit the round checks the payload's size against the run's retained
   bytes, counting the hits staged earlier in the same round cumulatively and in round order, and fails the step
   `RUN_DATA_LIMIT` through its route when it would exceed them, as `evaluate_run_result` does.
5. **Only a step `READY` at the end of its commit is announced.** `persist` MUST write the task-ready hint for a step
   only when the step is `READY` when the commit ends, whatever event released it; `step.cache_missed` is one of the
   hint's arms.
6. **No paused exception.** No root step is first released while its run is paused (the planner is idle then, apart from
   the gates of steps the retry pass or an operator made ready, `crates/domain/src/execution/plan.rs:309-316`), so a
   cacheable step is never waiting for its lookup across a commit. *Revised 2026-11-25 (decision D022-238): the
   paused exception of the first text is removed.*
7. **Cache rows are shared by deciders and exclusive only to transactions that lock nothing after them but usage rows.**
   A deciding transaction MUST lock cache rows only `FOR SHARE`. The only exclusive takers of a cache row (invalidation
   T4, purge T5, erasure T18, the store writer's insert T3 and the `UPDATE (expires_at)` that lowers an expiry) MUST lock
   nothing after it but usage rows (one usage row holds the counts of entries and store obligations, so two takers
   cannot deadlock on them), and two writers replacing one dead entry use `ON CONFLICT DO NOTHING`; they take their
   store obligations, when they take any, before the cache rows. They can therefore be in no cycle with a decider,
   wherever the decider's shared cache locks fall relative to its semaphores and windows (the argument ADR-0050
   decision 7 makes for the shard lease row). That is why a cacheable successor released after the gates round of the
   same commit (by a key failure's route) may be looked up in a later round of that commit, and why an admission, which
   takes its start's gates first, may take cache rows between its run-scope and step-scope gates (ADR-0050 decision 7,
   slot 7).
8. **An entry is a retained reference to a fingerprinted producer.** `step_cache_entries` MUST reference its value by
   `(tenant_id, payload_digest)`, a foreign key to `payloads (tenant_id, digest)` with `ON DELETE RESTRICT`, and MUST NOT
   hold an inline copy. The node fingerprint covers the node's whole canonical IR except a closed list of fields that
   cannot change what runs (`next`, `onError`, `cache.ttlSeconds`, `cache.name`, `concurrency`, `rateLimit`, `retry`,
   `timeoutMs`, `compensationRetry`, `compensationTimeoutMs`), so the executable identity, the key expression, the
   author `version`, the `outputSchema` carrier, the sorted logical secret references and any field a later feature
   adds (an input mapping, a binding) enter it by construction; plus the classification rules version. The entry
   records the producing node's sorted secret ids (not names) and the producing worker's subject. In the `namespace`
   scope the key's classification is the producer's. An entry adds only its
   fixed-size row, counted against the namespace's cache-entries ceiling (expired entries count until purged).
9. **An entry is stored after the fact, and never over an invalidation.** A successful completion of a miss with an
   inline output (outcomes `miss`, `expired`, `invalidated`) MUST write a store obligation in its completion commit,
   copying the scope digest, key digest and key version the lookup kept on the step projection and referencing the
   completed attempt's payload; a full pending-obligations ceiling skips it, counted, never a failed completion. The
   writer takes no lease and no fence: it locks the obligation `FOR UPDATE SKIP LOCKED`, skips (counted) an obligation
   whose producing attempt was leased at or before the scope's `invalidated_at` or its key's tombstone, deletes a
   conflicting dead entry under its lock, and inserts the entry with `created_at` equal to the database instant of the
   producing attempt's lease (`step.leased`) and its expiry from it, so a value computed by an attempt leased before an
   invalidation is never stored as live. An invalidation by scope inserts the scope row when absent, only for a scope a
   published version can derive (any other target is `404`), and sets `invalidated_at = clock_timestamp()` read after
   its lock; an invalidation by key writes a per-key tombstone (`step_cache_key_invalidations`) and deletes the pending
   store obligations of the same scope and key digest. Store obligations keep one column `UPDATE` grant guarded by an
   `ALWAYS` trigger, so `FOR UPDATE SKIP LOCKED` works as `sagat_app`. Erasure of a source run locks its store
   obligations, then its entries. Storing MUST NOT fail, delay or roll back the step or the run.
   *(Revised 2026-11-25, decision D022-252: the entry was first stamped with its obligation's `created_at`, the
   completion's instant, which an attempt leased before an invalidation could pass.)*
10. **Isolation, classification and activation.** A run of an internal definition (a standalone step, a test tree)
    looks up and stores only in the `version` scope of its own private version; a test-tree run whose version is not
    its own private one bypasses the cache. A `namespace`-scope cache name is a declared resource of the namespace: an
    administrator's setting lists the workflows that may write the name, and any workflow of the namespace may read it
    (`version` and `workflow` scopes need no declaration). Publication refuses `continueAsNew` with `cache`. A cache key over a `secret` location is refused at publication
    (`CACHE_KEY_SECRET`), over `pii` warned (`CACHE_KEY_PII`); its label, the entry counts and a hit's source run are
    shown only to a caller who may disclose every location the key reads; for any other caller `entry_id`, the expiry
    and `entriesDeleted` are null, listing instants are cut to the minute, and an `unauthorized` outcome reads as
    `bypassed`. Spans and metrics carry no key, label or source run. Until the collection- and loop-finish paths
    are routed through the reducer (ADR-0146), publication refuses `cache` in a definition with a `forEach`, `while` or
    `repeatUntil` node (`UNSUPPORTED_NODE`, dependency `COLLECTION_FINISH_ROUTING`). The behaviour is the registry
    capability `step-cache-v1`, required of control-plane and engine processes and listed by a binary only once every
    slice of US01 is in it; until an operator activates it, publication and starts that use `cache` are refused
    (`FLOW_CONTROL_ACTIVATION_PENDING`).

## Considered options

1. **Wait outside `READY`, a lookup round before the gates round, an entry by reference to a fingerprinted producer** —
   chosen. Safety does not depend on any driver remembering to look up: the reducer never makes the step `READY` before
   the decision, and the decision is a journal fact, so replay reaches the same state without reading the cache. The
   round shape is the one the slice proved for gates, and the reference reuses the store every output already lives in.
2. **The lookup inside `persist`**, as D1164 first placed it — rejected: `persist` cannot run the planner, so a hit could
   not continue the run in its commit, and it is where the hint is written for a step the hit completes; it would also
   lock cache rows after the usage rows `persist` charges (the reason ADR-0094 moved the gates out).
3. **The step `READY`, looked up by the claim or the worker** — rejected: it puts cache rows on the claim's hot path
   (NFR-022-007), spends a lease and an attempt to complete a step that needs neither, and a worker-side cache has no
   tenant boundary or audit (the reason US01 exists).
4. **A scheduler pass that looks up waiting steps** — rejected: a hit would wait for a pass instead of completing in the
   commit that released the step, and the pass adds a transaction per hit.
5. **The gates first, then the lookup** — rejected: a hit would hold a permit or consume a rate slot for a call it never
   makes, and wait behind a full key for a result it already has.
6. **An inline copy of the value in the entry** — rejected: up to 256 KiB duplicated per entry, a second content store
   with its own byte triggers, classification and erasure.
7. **A fingerprint from a named list of fields** (the first text: activity name or artifact digest, key expression,
   author version, carrier, classification rules) — rejected after the reviews: a field the list forgets (the producing
   secrets, a later input mapping) shares entries that must not be shared; an exclude list fails closed.
8. **Entries never shared between publishers** — rejected: it removes most `workflow` and `namespace` scope hits; the
   secret references in the fingerprint and the recheck give the same safety.

## Consequences

- Good: a hit completes in the commit that released the step, with no lease, attempt, permit or slot; replay never reads
  the cache; the task-ready hint becomes one rule for every step; the cache adds no content store; a hit never crosses a
  secret or a binding the consumer's publisher or the producer no longer holds.
- Bad: a cacheable node adds the lookup round's statements (the keyed digests, the secret-access locks, the scope and
  entry reads, the binding check) and, on a hit, the content read to every first readiness of the node; each is a line
  of the hot-path budget, recorded by the change that adds it. `FOR SHARE` on a hot entry writes a multixact per hit,
  measured by the benchmark (task T022-062). A definition without `cache` adds nothing (NFR-022-007).
- Harder: every consumer of the pending progress must answer `Evaluation::CacheLookup` or refuse; a new writer of cache
  rows must keep decision 7; erasure of a source run must delete its entries before anything can touch the payload they
  pin (the 022-side port, decision D022-217 of the feature's research; payload foreign keys are `RESTRICT`).
- Journal: `step.cache_missed` is new vocabulary with the reset rule `REMATERIALIZE` (the reset copies
  `steps.cache_lookup` from the source step; revised 2026-11-25, decision D022-253); `step.cache_hit` keeps
  `REMATERIALIZE`.
- Compatibility and rollback: additive tables, events and wait kind; activation through the registry, so no older binary
  serves a cacheable step. Once a journal holds the new events an older binary cannot decode the run
  (`crates/infra-postgres/src/event_codec.rs:490`): rollback is switching `stepCache` off, never a binary downgrade; the
  store writer and the purge keep running until obligations and entries drain; tables are dropped only when empty.

## Evidence

None yet: the tests named under **Enforced by** are built by tasks T022-015 to T022-020, T022-209 to T022-212, T022-230,
T022-231, T022-232 and T022-236. Spec: FR-022-001 to FR-022-007 and their amendments of 2026-11-24 and 2026-11-25;
research: D1161 to D1168, D1188, D022-203 to D022-206, D022-225, D022-226, D022-229 to D022-231, D022-238,
D022-249, D022-252 to D022-255; data model:
the cache tables and invariants 11 and 14.
