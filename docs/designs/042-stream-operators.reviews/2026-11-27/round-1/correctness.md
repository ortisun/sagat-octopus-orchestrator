# Correctness review of design 042, third revision, round 1 (2026-11-27)

Doc: docs/designs/042-stream-operators.md
Reviewed-commit: 531af9fb03386ad8f1deaed27933872f35156504
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 1, Major 6, Nit 6

The design gets wrong the property it exists to provide: a debounce or session window can turn one burst into two runs in normal multi-replica operation, so SC-042-001 and CUJ 3 cannot hold as written. Six Major findings sit beside it, mostly in the horizon proof, the backlog hold, the window start and window identity.

**Scope and limits.** I read the doc at `531af9fb`, ADR-0086, ADR-0153, ADR-0050, research.md and owner-decisions.md, and checked the doc's claims against `crates/` and the migrations. I could not read `feature/022-throughput-control` (no git access from my tools), so nothing about `decide_run_gates`, `rate_creators.rs` or the outcome table of FR-042-019 is verified against 022.

**Already raised and not yet answered.** The security review of 2026-11-27 was written at this same commit, so the doc cannot answer it yet. I do not count these again. I confirmed #1 in the doc text and #6 in the code:
- #1 Blocker: `FOR SHARE` on the chain row, then an `UPDATE` of `key_count`/`mark_count` on the same row, deadlocks two dispatchers.
- #2: the wait on the transaction horizon is unbounded, and the lag metric leaves those windows out.
- #3: the delivery retry is not restricted to `FAILED` deliveries.
- #6: publication races the capability removal (`crates/infra-postgres/src/instances.rs:232-271`: `verify` reads committed rows, and publication takes no lock).
- #7: the start's guarded update may hit zero rows.
- #11: removing a trigger discards only its `OPEN` windows.
- #12: how three limits are enforced is not stated.

1. Blocker — Session and debounce windows depend on the order events are applied, so one burst becomes two runs
   - Where: `docs/designs/042-stream-operators.md` lines 216-219 (FR-042-016), 691 (section 5, step 3), 321-323 (SC-042-001), 743 (section 7, "Events out of order"); `docs/adr/ADR-0086-…md` lines 46-48 and 79. Code: `crates/infra-postgres/src/dispatch.rs:155-167`, `crates/application/src/dispatch.rs:31`, `crates/scheduler/src/events.rs:4-7`.
   - What, verified in code: a claim sets `next_attempt_at = now + 60 s` and takes up to 100 events per batch. The dispatcher is "safe to run in several replicas".
   - What, the scenario: debounce `quietMs` 30,000 s on key 81, 500 events received between t0 and t0+3 s.
     - Replica A claims events 1–100 and is killed while applying event 1. That is SC-042-001's "kill at each write of the apply".
     - Replica B claims 101–200 at once, because they are due, and applies 101 first. Window W1 opens at r101, and 102–500 join and extend it.
     - About 60 s later, events 1–100 are claimed again. No open window covers r1, because r1 is earlier than W1's `opened_at`. A session window's span below `opened_at` is not defined. So event 1 opens W2, whose deadline r1+30 s has already elapsed and so can never be extended. Events 2–100 then match W2, or W1 and W2 both.
     - Both windows seal, and two runs start.
     - Two replicas applying concurrently cause the same inversion with no crash at all.
     - Separately, "an elapsed deadline is never extended" means any dispatcher lag longer than `quietMs` splits a burst whose gaps are all shorter than `quietMs`. That defeats the backlog hold, whose stated purpose is to cover a dispatcher restart.
   - Why: ADR-0086 rejected option 4 because "a burst is split by dispatcher lag", and this rule brings that failure back. SC-042-001 and CUJ 3 cannot hold. The fault test of T042-27 passes only if no kill ever strands an earlier-received batch.
   - Correction:
     - Define session windows by merging. An element with time r joins every `OPEN` window of the key whose interval `[opened_at − gapMs, closes_at]` contains r, under the key lock, whether or not the deadline has elapsed. If it bridges two such windows, merge them: `opened_at = min`, `closes_at = min(max r + gapMs, min opened_at + maxWaitMs)`.
     - Any change to `closes_at` clears `seal_horizon`. Today nothing resets a horizon recorded for an earlier deadline.
     - Identify windows by `window_id` (see finding 5).
     - Add to SC-042-001 a kill that strands the first claimed batch, and a two-replica interleaving forced with a barrier.

2. Major — The horizon proof has a hole on the pass side, and Task 042-55's race test cannot detect it
   - Where: doc lines 224-228 (FR-042-017) and 708-710; `docs/adr/ADR-0153-…md` lines 28-33.
   - What, from PostgreSQL semantics (suspicion until built): suppose the pass records `seal_horizon = pg_snapshot_xmax(pg_current_snapshot())` in a statement whose predicate is `… <= clock_timestamp()`.
     - Under READ COMMITTED the statement's snapshot is taken before `clock_timestamp()` is evaluated.
     - Timeline: the pass statement's snapshot is taken at D−1 ms, with xmax = 1000. An admission takes xid 1000 at D−0.5 ms and `received_at` = D−0.4 ms. The pass reads `clock_timestamp()` ≥ D and records horizon 1000.
     - The next pass sees xmin = 1000 ≥ 1000 while the admission is still open, for example waiting on its rate bucket.
     - The backlog read cannot see the uncommitted event, so the window seals. The event becomes late and starts a second run.
   - Why: ADR-0153 calls the horizon a proof, but it holds only if the "D has passed" reading comes before the snapshot.
     - The admission side was fixed with `MATERIALIZED` CTEs (architecture review #12); the pass side was not.
     - T042-55's test holds an admission open from before the deadline, so its xid is always below the horizon and the test passes either way.
   - Correction:
     - State that the deadline predicate uses `now()` (the transaction start, which precedes every statement snapshot), or a time read by an earlier statement, and never `clock_timestamp()` in the statement that reads the snapshot.
     - Add an architecture check on the statement that writes `seal_horizon`.

3. Major — `maxHoldMs` equals the claim lease, and the backlog rule ignores events that have been attempted
   - Where: doc lines 229-230 (FR-042-017) and 711-712; `docs/owner-decisions.md:132` (OD-2026-11-26-134: "covers a dispatcher restart"); `crates/application/src/dispatch.rs:31` (`CLAIM_LEASE` = 60 s); `crates/infra-postgres/src/dispatch.rs:258-264`; `crates/scheduler/src/events.rs:280-299` and 326-358.
   - What, verified in code:
     - A stranded claim is redispatched at claim time + 60 s. The hold ends at D + 60 s. So the hold covers a crash only when the claim happened at or before D, with zero margin.
     - Scenario A: debounce `quietMs` 1,000. e1 (received at 0 s) is applied, so D = 1 s. e2 (received at 0.5 s) is claimed at 2.5 s, because the dispatcher is 2 s behind, and the replica dies. Redispatch comes at 62.5 s, after the hold ends at 61 s. The window seals without e2, e2 is late, and a second run starts.
     - Scenario B: `retry_event` increments `attempts` for any non-capacity failure of any consumer of the event: another trigger's start, a waiting-step wake, or this chain's own deadlock (security #1). The event then leaves the `attempts = 0` rule, and the window seals without it.
   - Why: SC-042-001's "exactly one run" with kills is not guaranteed, and OD-2026-11-26-134's stated reason is false.
   - Correction:
     - Hold on every `PENDING` event received at or before D, not only those with `attempts = 0`. `maxHoldMs` still caps poison events. Change the partial index predicate to `status = 'PENDING'`.
     - Require `maxHoldMs > CLAIM_LEASE` plus a lag allowance (validated at publication), or measure the hold from the covered events' `next_attempt_at`, under an absolute cap.
     - State `quietMs` in SC-042-001.

4. Major — The window start's fence contradicts the creator port and the dispatcher's immediate start
   - Where: doc lines 260-261 (FR-042-019), 697-698, 716-718, and 785-786 ("needs no change to 022's port"); `crates/application/src/runs.rs:259-272` and 443-448; `crates/application/src/schedules.rs:391-410`; `crates/infra-postgres/src/schedules.rs:84-106`; ADR-0050 decision 6.
   - What, verified on `develop`:
     - `RunStore::create(new)` opens its own transaction. The only fenced creator, a schedule action, gets its fence checked in that transaction by carrying it inside `NewRun.schedule.fence`.
     - So "one transaction: read the shard row FOR SHARE, call the admission port" needs a new `NewRun` field, which is a port change.
     - The dispatcher holds no shard lease (doc line 114), and ADR-0050 decision 6 forbids inventing one. "The same code as the pass" therefore either fails the fence on every window sealed at once, leaving each to wait for the pass, or runs without a fence, in which case the fence protects nothing.
   - Why: the start's transaction boundary cannot be built as written, and the claim of no port change is false. 022's merged port is unverified (see scope).
   - Correction: declare the start unfenced, idempotent by the slot-2 receipt and the guarded `WHERE state='SEALED'` update, and drop the shard read from section 6. Or specify a `NewRun` window admission (optional fence, window id) as an explicit change to 022's port.

5. Major — The window identity used for the idempotency key is not unique
   - Where: doc lines 239-240 (FR-042-019), 208-209 (no key row for chains of only `filter`, `map` and `flatMap`), 649-651, 282-283 (key rows purged); ADR-0086 decision 6.
   - What, from the design text (suspicion):
     - The key is derived from (namespace, chain, key HMAC, `opened_at`, sequence), and the sequence lives in the key row.
     - A chain of only `filter`, `map` and `flatMap` has no key row, so it has no sequence. Two events whose `received_at` falls in the same microsecond collide.
     - A key row can be purged while its `SEALED` or `STARTED` windows remain, and then the sequence restarts at 0.
     - Scenario: a tumbling window with `skip`, `keyIdleMs` 60 s and `sizeMs` 300 s. W0 = (b, 0) is sealed by `onFull`. The key row is purged after it goes idle. A new element in the same span opens (b, 0) again, assuming `opened_at` is the boundary, which the doc leaves undefined. The result is a primary-key conflict, counted retries and an event ending `DEAD`, or the start key of another window and a lost run.
   - Why: "exactly one run per window" rests on this identity.
   - Correction:
     - Derive the start key from `window_id`, which is random and recorded when the window opens. Finding 1's merge makes `opened_at` mutable anyway.
     - For a window sealed at once, use (source, event_id).
     - Never reset an ordering sequence.

6. Major — The `flatMap` fan-out cannot be "all-or-nothing", and its failure has no window outcome
   - Where: doc lines 262-266 (FR-042-020), 248-256 (the outcome table), 670, and 1074-1075 (Task 042-24: "all-or-nothing start").
   - What, from the design text:
     - One start transaction per index commits runs 0 to k−1 before index k.
     - Each index has its own input, so design 043 can refuse index k with `RUN_INPUT_INVALID`. The window becomes `REFUSED` with its held values deleted and k runs already started, and indexes after k never run.
     - The doc does not say where `select` is evaluated (at the seal or at each start).
     - "More than `maxFanOut`, or a value that is not a list, fails the emission as a whole" has no row in the outcome table. The window stays `SEALED`, retries, and expires after 24 h while holding personal data.
     - No target count is recorded beside `started_count`.
   - Why: the task contradicts the requirement, and an error path is undefined.
   - Correction:
     - Evaluate `select` once at the seal and record `fan_out` on the window. A `select` failure becomes `REFUSED` with its code.
     - Define a partial fan-out (`REFUSED` with `started_count` < `fan_out`, and which runs exist).
     - Reword Task 042-24.

7. Major — An apply that finds its chain `DRAINING` or gone has no defined outcome
   - Where: doc lines 688 and 284-285 (FR-042-022), 503-508; `crates/infra-postgres/src/definitions.rs:75-116` (`register_triggers` updates the trigger row in place); `crates/infra-postgres/src/dispatch.rs:281-301` (`triggers_matching` reads without a lock).
   - What, verified that the read is unlocked; the outcome is a suspicion:
     - The dispatcher reads the trigger and its chain id, then a publication or `switch` commits a new chain and marks the old one `DRAINING`.
     - The apply then locks the old chain row and "takes no element".
     - If that is recorded as a delivery, the event is never applied to the new chain and is lost to the new version. If it is an error, it takes the counted retry path, which feeds finding 3.
   - Why: an event admitted before a republication can be lost silently.
   - Correction: on a `DRAINING` or missing chain, re-read the trigger row in the same transaction and apply to the current chain. If the trigger is gone, write `SUPPRESSED` with a code. Test it with a barrier between the read and the lock.

8. Nit — A redelivery after a short dedupe window collides with its own member row
   - Where: doc line 652 (member key `window_id, source, event_id`), line 281 (windows up to 24 h); `crates/domain/src/events.rs:35-38` (minimum dedupe TTL 60 s).
   - What, verified on the TTL side: a provider resends event X after its 60 s dedupe window ends. It is admitted as new and applied to the same open window, where (window_id, source, event_id) already exists. The outcome is not defined: a unique violation leads to counted retries and `DEAD`.
   - Correction: on an existing member in the window, write `SUPPRESSED` with reason `duplicate` and change no counter.

9. Nit — The retry of a `START_EXPIRED` window re-expires at once, and the start reads held values without a lock
   - Where: doc lines 255-258 and 715-716.
   - What, from the design text:
     - The expiry is "24 h after it was sealed". After an operator retry, `sealed_at` is unchanged, so the next pass refuses the window again.
     - The start reads held values without a lock while a concurrent start (the dispatcher's and the pass's) may delete them. The second starter can build an input from `NULL` values, and a fingerprint computed over values would then mismatch.
   - Correction: reset an expiry timestamp on retry. Read values and state in one statement and abort when the state is not `SEALED` or any value is `NULL`. Fingerprint over member identities, not values.

10. Nit — The backlog query's "the trigger could match" is not served by the planned index
    - Where: doc lines 677-680 and 711.
    - What: `(tenant_id, namespace_id, received_at) WHERE … attempts = 0` has no source or type. Either every window in the namespace is held by any unrelated pending event, or each pass scans the namespace's whole backlog per trigger.
    - Correction: index `(tenant_id, namespace_id, source, event_type, received_at)`, or state that matching is by namespace only and accept the over-hold.

11. Nit — Accepted ADRs are contradicted without an amendment
    - Where: `docs/adr/ADR-0086-…md` lines 42-44 (decision 3: chain and key always locked) and 59-62 (decision 7: "holding the chain, key and window rows before the port"), against its "Revision of 2026-11-27 … without changing the decisions"; `docs/adr/ADR-0050-…md` lines 51-54 (order (b) "MUST NOT lock a slot-3 row after its run").
    - What: the design registers the same window row as slot 3 (for the apply and the seal) and as slot 8a (for the start). That is a deviation, not a registration. Its no-cycle argument also depends on "a `SEALED` window is touched only by its start", which the fix proposed in security #11 would break.
    - Correction: supersede decision 7 explicitly. Record an ADR-0050 deviation, as D2223 does, and add a fitness check that no slot-3 transaction locks a `SEALED` window.

12. Nit — The collection half's text does not match the built code
    - Where: doc line 64 ("server half of slice 1 built"), 518-519, 591-593, 74; `crates/application/src/instances.rs:126-164`; `apps/control-plane/src/lib.rs:861,889`; `crates/scheduler/src/passes.rs:1486`, 1573-1595, 3171-3208, 3213-3267.
    - What, verified in code:
      - No `COLLECTION_OPERATORS_UNSUPPORTED` code exists and no refusal at startup. The flag is wired and publishes.
      - `collection-operators-v1` is not in `Capability::ALL` (6 values), so ADR-0153 decision 4's "MUST refuse … while the requirement is not recorded" is unmet. Task 042-56 is open, which the "built" claim hides.
      - A `ChainFailure` is routed through `stop_collection_for_chain`, not `stop_collection_for_source`.
      - The collection entry is at about line 1486, not 1094.
      - A blob source over the chain input ceiling is stopped through `stop_collection_for_source`, which does not count `workflow_primitive_rejections_total`. The same `COLLECTION_OPERATOR_BUDGET` from an inline list is counted, so the counter disagrees with the failed runs.
    - Correction: update the text, keep Task 042-56 visible as a gap, and route the blob input ceiling through the counted path.

13. Nit — Some success criteria and budgets cannot be tested as written
    - Where: doc lines 335-337 (SC-042-006), 928-929 (NFR-042-001).
    - What:
      - SC-042-006 asserts "another tenant's dispatch latency stays within the stated bound", but no bound is stated anywhere.
      - "At most 6 statements" for joining an open window does not fit the steps the doc lists: chain lock, HMAC call, key lock, window lock, member insert, window update, held-bytes quota check, delivery insert. That is eight or more.
      - SC-042-001's "each write" needs the kill points listed, including the stranded-batch case of finding 1.
    - Correction: state the latency bound, recount the statements or fuse them explicitly, and enumerate the failpoints.
