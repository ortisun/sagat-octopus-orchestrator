# Architecture review of design 042, third revision, round 1 (2026-11-26)

Doc: docs/designs/042-stream-operators.md
Reviewed-commit: dfc7611236ed0f675f78769250152d13d0cadad8
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 1, Major 10, Nit 8

1. Blocker — A chain without a window can never start its run
   Where: FR-042-014 (design lines 189-193), section 5 step 4 (lines 622-624), FR-042-019 (line 229); `crates/scheduler/src/events.rs:539-544`.
   What: the apply transaction inserts the `trigger:{id}` delivery row marked `STARTED`, and then the start runs "as today". But today's start writes that same row inside the run's transaction. When the row already exists it returns `Started::No` and rolls the run back, so no run is ever created. If the start commits first, the apply conflicts and rolls back the `distinct` marks and the `skip`/`take` counters. A crash between the two leaves `STARTED` with no run. A run refused by the input check (design 043, `record_refused_delivery`) still reads `STARTED`. A `flatMap` fan-out has one delivery row but needs N runs.
   Correction: send every emission through the window path, as a window sealed at once with one member. The apply then writes `BUFFERED`, and the start owns the run and its record. Alternatively, apply the stateful stages inside the run's admission transaction.

2. Major — The window start does not fit the 022 port or the lock order of ADR-0050
   Where: section 6 "start" (lines 641-643), section 5 (lines 626-627), section 8 item 6; `crates/application/src/runs.rs:445`; ADR-0050 decision 7, slots 2, 3, 6 and 7.
   What: `RunStore::create` opens its own transaction and takes the receipt (slot 2) and the run row (slot 6) first. So "lock chain, key and window rows, then the port" cannot happen in one transaction unless 022's port accepts a transaction owned by the caller. Chain, key and window rows create runs, so they belong in slot 3; locking them before the slot-2 receipt goes back to a lower slot. The design cites only ADR-0088. It also does not say which of 022's refusals leave the window `SEALED` and which make it `REFUSED`. On the 022 worktree those refusals are `CONCURRENCY_QUEUE_FULL`, `FLOW_CONTROL_LIMIT` and `BinaryCapabilityMissing`, and a failed key starts a run that is already `FAILED`.
   Correction: register the operator rows in ADR-0050 slot 3 (or record a deviation with a no-cycle argument), state the port change, and add a table mapping each 022 refusal to a window state.

3. Major — "The fleet rule is enforced" is not true for binaries from before 0.3.0
   Where: section 10 (lines 733-739), G042-006 marked closed (line 340), research D042-165; `crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql:21-22`; G1607 in the 018 register.
   What: a binary older than the registry release does not register, so an activation cannot see it. The registry itself is new in 0.3.0 (feature 018). The case the design names, an older dispatcher starting one run per event, is exactly the one the registry cannot refuse. Every 0.3.0 binary lists the capability, so the requirement in fact refuses no process. The Constitution Check IV condition rests on this claim (Principle VIII).
   Correction: reopen G042-006 for pre-registry binaries, keep the runbook rule, and state the over-delivery as the residual risk.

4. Major — Capability listing, removal and rollback conflict with ADR-0082
   Where: design line 735 ("every … of 0.3.0 lists them"), line 740 (rollback), NFR-042-002; ADR-0082 decision 7 and its Consequences; `crates/infra-postgres/src/instances.rs:226-271` (the `verify` step of `remove`).
   What: under ADR-0082 a process lists a capability only for what its configuration runs, not for what the binary could do. The design does not define the `verify` step that should refuse removing the requirement while chains or chained versions exist. A rollback of the binary below a recorded requirement is unsupported, but section 10 offers rollback. A publication fenced on the requirement epoch needs a definer function (G2001), which NFR-042-002 forbids.
   Correction: list the capability only where the dispatcher and the pass run with the switch on. Define the removal `verify`. State that publication reads without a lock and that `verify` is the guard. Fix the rollback text.

5. Major — The chain lifecycle is tied to publication only
   Where: T042-14 (line 917), FR-042-021 (quotas "checked at publication"), section 1 (lines 457-460); `crates/application/src/publishing.rs:1031`.
   What: activation, rollback and promotion also rewrite `workflow_triggers` through `triggers_of`. So a rollback to an older chained version registers or drains chains and can exceed the tenant quotas without any check.
   Correction: run chain registration and draining, the quota checks and the capability check in `switch` as well, with a test for a rollback onto a chained version.

6. Major — The domain and port placement for slices 2 and 3 is not named
   Where: section 1 (lines 447-449), section 5 (lines 614-621), Constitution Check II (line 781).
   What: only the declaration rules are placed in `crates/domain`. Nothing places the window state machine (deadlines, sessions, `onFull`, late elements, `skip`/`take` epochs), an evaluator method for trigger stages, or the store port the dispatcher and the pass call. As written, these business rules would end up in SQL or in the scheduler (Principle II).
   Correction: name a pure domain module that decides the transitions and has property tests, one `ExpressionEvaluator` method for a trigger chain, and a store trait in `crates/application`.

7. Major — The window start ignores the run-input check of design 043
   Where: FR-042-019 (lines 224-226); `crates/scheduler/src/events.rs:454-474` (`run_inputs.check` before every trigger start).
   What: the input has a fixed shape, `{window, events}`. A version whose `inputSchema` describes one event would refuse every window, after holding the values for up to 24 hours. The outcome of that refusal (`REFUSED`? audited? held values kept?) is not stated.
   Correction: check at publication that the emitted shape fits `inputSchema`, and state the start's refusal outcome.

8. Major — Stale text contradicts the third revision
   Where: Non-Goals line 317 ("No trigger operator in release 0.3.0"); catalogue line 468 and section 9 lines 723-724 ("later"); Goals line 309; Complexity Tracking line 799 ("grace"); Constitution Check VIII line 787; NFR line 825; T042-21 lines 938-939 ("fenced seal with the admission grace"); ADR-0086 "Enforced by" ("none is in release 0.3.0") and decision 4 / Consequences (grace).
   What: a builder following T042-21 implements the grace that ADR-0153 withdrew. Under the docs-first rule, the document has to be right before the code.
   Correction: rewrite these lines and annotate ADR-0086 decision 4 inline.

9. Major — The HMAC key cannot be built under the stated constraints
   Where: section 4 (lines 596-597), NFR-042-002 (line 827), T042-53 (lines 1026-1027).
   What: the key must never leave the database and the runtime role must have no read path, but NFR-042-002 forbids `SECURITY DEFINER` and any new role. A function cannot use a table its caller cannot read without one of those. No migration installs `pgcrypto`, so `hmac()` does not exist.
   Correction: allow one definer function for the HMAC (a precedent exists in the registry migration), or move the key outside the database, and name the HMAC primitive.

10. Major — The collection capability depends on slice 2's migration
   Where: T042-54 (lines 1029-1031, "in the migration of Task 042-13"), lines 840-842 (slices 0 and 1 "no migration").
   What: slice 1 ships in 0.3.0, but publishing a chained `forEach` needs `collection-operators-v1`. That value must be added to the registry's `CHECK` lists, which this plan does only in the trigger migration. If slice 2 slips, collection operators either cannot be published or ship ungated.
   Correction: split out a small migration that adds the capability, delivered with slice 1.

11. Major — The retry action for a `KEY_LIMIT` delivery has no mechanism
   Where: FR-042-021 (lines 237-238), FR-042-014 ("never updated"), T042-18; `crates/infra-postgres/src/dispatch.rs:214-219` (`DISPATCHED`) and line 442 (purge).
   What: a retry has to remove the delivery row and return a `DISPATCHED` event to `PENDING`. Neither move is defined, and the event may already be purged at the end of its dedupe window.
   Correction: specify the retry (delete the delivery row, return the event to pending, require the inbox row to still exist) and its typed error when the event is gone.

12. Nit — The horizon depends on evaluation order inside the statement
   Where: ADR-0153 decision 1; `crates/infra-postgres/src/events.rs:480-497`.
   What: a common table expression does not guarantee that `pg_current_xact_id()` is evaluated before `clock_timestamp()`. Also, xids are cluster-wide, not per database.
   Correction: use `MATERIALIZED` common table expressions in sequence. Add an architecture check that `INSERT INTO inbox_events` appears only in `admit_within`.

13. Nit — The start step is not fenced and holds the key row while the run is created
   Where: section 6.
   What: the start does not read the shard fence that ADR-0020 prescribes, and it holds the key row through run creation, so every apply on that key waits behind it.
   Correction: read the shard fence; take the chain row `FOR SHARE` and the window row only.

14. Nit — The two lanes share crates
   Where: lines 849-862.
   What: T042-43 (lane A) puts the switch in `DefinitionService` in `crates/application` (lane B). T042-18's REST retry needs `crates/api-http` (lane A). Both lanes edit `research.md`.
   Correction: reassign those parts or add order points.

15. Nit — Another feature's register is edited without an Owner-gate
   Where: T042-3 and T042-22.
   What: T042-3 edits 022's D022-018, and neither task carries an `Owner-gate:` line citing the dependency on 022.
   Correction: make the D022-018 change on 022's branch and add the gate lines.

16. Nit — Telemetry vocabularies are open
   Where: section 9 metrics.
   What: the outcome sets of `operator_window_seal_total` and `operator_windows_total` are not listed. No label separates holding for the horizon from holding for the backlog, so the lag alert also fires on waits that are by design.
   Correction: list the closed sets, add `held_by_horizon` and the age of the horizon, and define the lag.

17. Nit — Names collide
   What: the delivery outcome `BUFFERED` sounds like 022's buffered runs, and "chain" sounds like 014's "chain before run" (ADR-0050 slot 3R).
   Correction: rename, for example to `HELD` and to operator-chain wording in the lock-order text.

18. Nit — Two missing bounds
   What: a `SEALED` window can retry without limit while holding personal data. The backlog-rule query has no supporting index (only `inbox_events_backlog_idx(tenant_id)`), and adding one costs every admission write.
   Correction: cap how long a window may stay `SEALED`, and choose and state the index and its write cost in section 4.

Files read: `docs/designs/042-stream-operators.md`, `docs/adr/ADR-0153-a-window-is-sealed-only-past-the-transaction-horizon-of-its-deadline.md`, `docs/adr/ADR-0086-operator-chains-on-triggers-are-declared-data-and-a-fenced-pass-seals-a-window-that-starts-one-run.md`, `docs/adr/ADR-0050-many-run-operations-admit-under-stable-keys-and-never-buffer-or-reopen.md`, `docs/adr/ADR-0082-the-instance-registry-records-requirements-only-for-the-operator-and-a-refused-process-stays-up-with-a-closed-gate.md`, `crates/application/src/execution.rs`. I could not run `git`, so a duplicate of this work on another branch was not checked.
