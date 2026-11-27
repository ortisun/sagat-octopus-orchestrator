# Approval read of design 042: architecture (2026-11-27)

Doc: docs/designs/042-stream-operators.md
Reviewed-commit: 4b0248c7b2019be951f38c1e6ec12dd4e662569b
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 5, Nit 9

## Previous review

1. Fixed. Every stateful emission is now a window and the apply never creates a run (FR-042-014 lines 205-218, FR-042-019 lines 265-277 and 302-304, Owner decision row line 675).
2. Fixed. The slot order is named, the port use is stated, and there is an outcome table per refusal (lines 279-291, 1112-1127, 1208-1218). The deviation is recorded in `docs/adr/ADR-0050-…md` line 103.
3. Fixed. Gap 042-006 is reopened and narrowed, and the residual risk is stated (lines 599-602, 1320, Constitution Check IV line 1378).
4. Fixed. Capabilities are listed per configuration, the drain checks are defined, and the rollback text is corrected (lines 494-507, 1287-1321, NFR-042-002 line 1434). A new gap for the engine kind is finding 1 below.
5. Fixed. Chains are registered and quotas checked in `switch` too (lines 769-775, 376-379, T042-14 lines 1565-1572).
6. Fixed. The pure window module, the evaluator method and the store port are named (lines 755-767, line 1376).
7. Fixed. There is a publication check (`OPERATOR_INPUT_SCHEMA`) and the refusal outcomes are stated (lines 138-141, 471-474, 284-285). A wrong claim about where the check runs remains (finding 6).
8. Partly. The design text is fixed (lines 573-574, 795, 1394, 1619-1623). ADR-0086 is still stale (finding 4), and some counts are stale (finding 12).
9. Fixed. There is no keyed hash and no `pgcrypto`; keys are plain digests (lines 957-971, line 676, ADR-0155 decision 6).
10. Fixed. Task 042-56 is its own small migration, shipped with slice 1 (lines 30-31, 1454-1456, 1791-1795).
11. Fixed. The delivery retry is a targeted re-apply that never returns the event to `PENDING` (lines 483-493, T042-18 lines 1601-1611).
12. Fixed. `MATERIALIZED` common table expressions plus the `admit_within` check (lines 1786-1788, ADR-0153 lines 63-66).
13. Fixed by decision. The start is unfenced and holds no key row (ADR-0155 decision 4, lines 1112-1124). The justification cites the wrong ADR (finding 7).
14. Fixed. Shared files and order points between the lanes are stated (lines 1465-1492).
15. Fixed. Task 042-3 edits no register of feature 022, and both tasks carry owner gates (lines 1502-1508, 1633).
16. Fixed. Labels are closed sets, `held_by_horizon` and the horizon age are added, and the lag is defined (lines 1149-1153, 1246-1258).
17. Partly. `BUFFERED` is now `HELD` (lines 438, 448). The lock-order text still says "chain" (lines 1037-1038, 1216), next to feature 014's "chain before run" in ADR-0050 slot 3R.
18. Fixed. `start_expires_at` caps how long a window stays `SEALED`, and the backlog index and its write cost are stated (lines 291, 299-300, 952-955).

## Findings

1. Major — No engine is ever told to list `collection-operators-v1`, but the requirement must be recorded for both kinds of process
   Where: section 10 lines 1287-1294; ADR-0153 decision 4 (lines 40-42 of its file); ADR-0082 decision 7 (line 27 of its file); T042-54 and T042-56.
   What: the design lists `collection-operators-v1` only for a control plane with `SAGAT_COLLECTION_OPERATORS_ENABLED` on. Yet "the operator records each requirement for both kinds". Engines are the processes that evaluate a chained `forEach` and restore its manifest, and no rule says when an engine lists the capability.
   Why: under ADR-0082 decision 7, recording the requirement for engines refuses every engine that does not list it, and a refused engine's whole gate closes. Recording it for control planes only breaks ADR-0153 decision 4 and Requirement 053-006 ("every kind of process that could meet the state"). Slice 1 cannot be activated as written.
   Correction: state that every engine that runs the scheduler's collection entry lists `collection-operators-v1`, whatever the switch (it processes stored chained scopes), as `LeaseWithdrawalV1` does. Add an activation test with both kinds to T042-54.

2. Major — The `SEAL_EXPIRED` retry reopens a window without its key row or key slot
   Where: FR-042-018 lines 263-264; FR-042-019 lines 293-296; section 6 lines 1141-1144; key purge rule at FR-042-022 lines 381-382 and line 942; session rule at lines 229-235; ADR-0155 deviation (lines 66-71 of its file).
   What: the retry locks only the `REFUSED` window and sets it `OPEN`. A key row is purged when it has no `OPEN` or `SEALED` window, and a `REFUSED` window does not count. So within the 24-hour grace the key row can be gone and its usage slot released, and the retry then creates an `OPEN` window for a key with no row and no slot. The seal path's "key row, then window" lock finds nothing. Separately, the retry adds an `OPEN` window to a key's set without the key lock, while an apply decides joins and merges from that set under the key lock.
   Why: `maxKeys` can be exceeded, and two overlapping session windows of one key can coexist. That is exactly the order dependence ADR-0155 decision 2 removed. The no-cycle argument covers deadlocks, not this invariant.
   Correction: the `SEAL_EXPIRED` retry takes the chain row `FOR SHARE` and the key row `FOR UPDATE` (re-reserving the slot, or refusing with `KEY_LIMIT`), then the window, as a flush does. Keep a key row while it has a `REFUSED` window inside its grace. Add the retry to the slot test and to T042-58's state-write check.

3. Major — Discarding a removed trigger's open windows has no owner and no bound
   Where: FR-042-022 lines 388-390; section 1 lines 769-775; T042-59 lines 1810-1813; `crates/infra-postgres/src/register_triggers` at `crates/infra-postgres/src/definitions.rs:75-129`.
   What: trigger removal happens inside the publication or `switch` transaction (`register_triggers` deletes the trigger row). The design says the chain's `OPEN` windows "are discarded, each audited and counted", but not by whom. If that transaction does it, the work grows with `maxKeys` × windows × members, plus one audit record per window.
   Why: the work is unbounded inside a statement-timed control-plane transaction. Removing a busy trigger would fail by timeout again and again, and it holds publication locks meanwhile.
   Correction: the publication or `switch` only sets the chain `REMOVED`. The `operator_windows` pass discards its `OPEN` windows in bounded batches per tenant, with their audit. Say so in section 6 and in T042-59, with a test that removes a trigger holding more windows than one batch.

4. Major — ADR-0086 still contradicts ADR-0155 and the design
   Where: `docs/adr/ADR-0086-…md` line 6 ("Enforced by … the HMAC test T042-53"); decision 4 lines 46-52 ("an elapsed deadline is never extended", "`attempts = 0`"); decision 10 lines 68-69 ("the caller is recorded as the starter of the run"); Consequences lines 92-93 (HMAC key, admission grace, "five tables"); the first 2026-11-27 revision, lines 116-124 (window as a "slot 8a row", "`operator_key_hmac` over `pgcrypto`").
   What: ADR-0155 supersedes only decisions 2, 3, 6 and 7. But the design makes the engine the starter (FR-042-023 lines 394-396), merges session windows across elapsed deadlines (ADR-0155 decision 2), holds on every pending event, and has no HMAC. The first revision paragraph was never withdrawn.
   Why: these are divergences from accepted decisions without a record (AGENTS.md rule 2). A builder who reads ADR-0086 gets instructions the design forbids, and under the docs-first rule the record has to be right before the code.
   Correction: extend ADR-0155's "Refines / supersedes" to cover ADR-0086 decisions 4 (the elapsed-deadline and `attempts` clauses), 10 and 11. Mark the first 2026-11-27 revision paragraph and the Enforced-by and Consequences text as superseded, pointing to ADR-0155.

5. Major — The apply is placed in `EventDispatcher`, but the control plane must also run it
   Where: section 5 line 1037 ("in `EventDispatcher`"); FR-042-033 lines 483-493 ("with the dispatcher's application method", run by a control plane); lane B line 1472; `apps/control-plane/Cargo.toml` (no `sagat-scheduler`); `scripts/check-architecture.py` lines 19-21 (no `tokio` in `crates/application` beyond `rt` for `collection_source.rs`).
   What: the delivery retry is served by the control plane and applies the event with the dispatcher's method. That method sits in `crates/scheduler`, which the control plane does not and should not depend on. Moving the orchestration into `crates/application` also moves the blocking evaluation there, which the `tokio` rule refuses.
   Why: as written, FR-042-033 needs either a new app-to-scheduler edge or a second copy of the apply (Principle II, Rule 6).
   Correction: name one use case in `crates/application/src/operators.rs` (for example `OperatorChains::apply(evaluated, …)`) that takes an already-evaluated `TriggerChainOutput`. Each caller (the dispatcher in `crates/scheduler`, the retry handler in the control plane) runs `trigger_chain` in its own blocking task first. State it in sections 1 and 5.

6. Nit — Design 043's input check does not run "in the port"
   Where: FR-042-019 line 270; table row line 286; `crates/scheduler/src/events.rs:438-474` and `:559-561`.
   What: today every creator calls `RunInputGate::check` itself before `RunStore::create`; the port only accepts a `CheckedRunInput`, and `scripts/check-architecture.py` enforces that. `MissingReference` from `create` means, in today's trigger path, that the event left the inbox. A missing version is found earlier, by `get_version_by_id` returning `None`.
   Correction: say the window start calls `RunInputGate::check` with `Creator::Trigger`, and that `InputRefusal::Unavailable` leaves the window `SEALED`. Map `VERSION_UNAVAILABLE` to the version read returning `None`.

7. Nit — The unfenced start cites the wrong ADR and records only part of its deviation
   Where: section 6 lines 1112-1114; ADR-0155 lines 6-9 and decision 4; ADR-0050 decision 6; ADR-0020 decision 3.
   What: ADR-0050 decision 6 *requires* passes to read the shard fence of their aggregate; it does not "keep fence reads to passes' own aggregates". The start run by the pass on window shards departs from that rule and from ADR-0020 decision 3 ("every action is fenced").
   Correction: fix the sentence. Add ADR-0020 decision 3 and ADR-0050 decision 6 to ADR-0155's deviations, with the argument already given: the receipt plus the guarded write; ADR-0020 itself says the fence is not what makes results correct.

8. Nit — The purge locks key before usage, against the stated slot-3 order, and no slot test covers it
   Where: section 6 lines 1130-1134 (chain, keys, then `key_count - 1` on the usage row); stated order at lines 1037-1038 and 209-210 (chain, usage, key, windows); T042-13 lines 1561-1562 (slot test of apply and seal only).
   What: ADR-0050 decision 7 requires the listed order inside a slot. No cycle is evident today, but nothing would catch one.
   Correction: either have the purge take the usage row before deleting key rows, or record the order and its no-cycle argument. Add the purge, flush, discard and both retries to the slot test in `tests/support/lock_order.rs`. That harness, not the Python script, should be the primary check behind ADR-0155's deviation.

9. Nit — The held-bytes counter duplicates the quota-delta machinery, and its commit order is held only by trigger names
   Where: section 4 lines 996-1032; NFR-042-002 lines 1434-1438; ADR-0050 line 103.
   What: a second counter, delta table, delta trigger and flush are built in the shape of `sagat_quota_add_delta` and `sagat_quota_flush_deltas`. That adds three of the five definer functions. "Content flush before held flush" rests on the `zz_` prefix plus one barrier test.
   Correction: add a catalog test that every table carrying `zz_operator_held_flush` also has a `*_quota_flush` trigger that sorts before it. Record in ADR-0155 why a held column on the existing delta path was rejected.

10. Nit — A `flatMap` seal at the apply evaluates `select` and the validator while holding row locks
    Where: section 5 lines 1063-1064; FR-042-020 lines 306-313 ("outside any transaction"); section 1 lines 759-763 ("one method"); line 1376.
    What: a `count` or `onFull` seal at the apply runs up to 1 s of evaluation, and possibly a validator permit wait, under the chain, key and window locks. The seal's `select` over the envelope also needs an evaluator method other than `trigger_chain`.
    Correction: such a seal leaves the window `OPEN`, `full` and due for the pass, as it already does for a room shortfall. Name the second evaluator method.

11. Nit — Two gauges cannot be computed as defined
    Where: lines 1151-1152 and 1255 (`operator_seal_horizon_age_seconds`) against the `operator_windows` columns at line 944; line 1258 (`operator_held_tenants_near_quota`, "tenants on the replica's shards").
    What: `seal_horizon` is an `xid8` with no recorded time, so it has no age in seconds. Tenants are not sharded, so every replica counts the same tenant.
    Correction: add `seal_horizon_at`. Have the monitor emit the near-quota gauge once, per ADR-0020 decision 7.

12. Nit — Stale counts and an unnamed switch
    Where: "three alerts" at lines 1380, 1444 and 1688, while section 9 lists six (lines 1260-1266); "Seven tables" at line 1393, while sections 4 and 4a define ten; the control plane's trigger switch (lines 495-497, 506-507, 777-788) has no variable name.
    Correction: fix the counts and name the control plane's variable.

13. Nit — FR-042-030 says "cost what it costs today", but every admission pays more
    Where: FR-042-030 lines 527-528 against lines 952-955 and T042-55.
    What: `inbox_events_pending_match_idx` and the reordered admission statement cost every tenant, whether or not it uses chains.
    Correction: state this as an accepted cost in FR-042-030 and measure it in Task 042-19.

14. Nit — Events wait a day and then go `DEAD` while the capability is not in use
    Where: FR-042-031 line 442; section 10 lines 1312-1318; `crates/scheduler/src/events.rs:280-289` (`GIVE_UP_AFTER`).
    What: during phase 1 `uses` is false everywhere, so events for chained triggers that are still registered wait as "room" waits and dead-letter after a day.
    Correction: state the bound, and order the runbook so that triggers are replaced before phase 1.

I could not run `git`, so I did not check whether this work is duplicated on another branch or worktree.
