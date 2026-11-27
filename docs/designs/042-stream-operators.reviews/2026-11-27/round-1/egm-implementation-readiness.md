# EGM implementation-readiness review of design 042, third revision, round 1 (2026-11-27)

Doc: docs/designs/042-stream-operators.md
Reviewed-commit: 531af9fb03386ad8f1deaed27933872f35156504
Reviewer: egm-implementation-readiness
Note: the reviewer's output as returned, with this header added and machine paths made relative. Stage 9 of the EGM
review with the codebase-details lens, run once as the `egm-implementation-readiness` agent of the accepted EGM plugin,
with no conversation history.

Counts: Blocker 2, Major 17, Nit 4

1. Blocker — Windows that have no key row have no defined identity, yet the run's idempotency key is built from that identity
   Where: lines 199-209 (FR-042-014), 237-241 (FR-042-019), 649 and 651 (section 4 table), 687-693 (section 5 steps 1-3), T042-15 and T042-24 (lines 1038-1043, 1074)
   What: The primary key of `operator_windows` is `(chain_id, key_hmac, opened_at, sequence)`, and the "next sequence" counter lives on the `operator_keys` row. But a chain of only `filter`, `map` and `flatMap` "takes no key row and no key lock", and still writes "a window sealed at once with its one member". For that chain the text gives no `key_hmac` and no `sequence`. Nothing says what is HMACed for a keyless chain, where "absent means one key for the whole trigger". Nothing says how `opened_at` is chosen for a window sealed at once. Two events with the same `received_at` under a `FOR SHARE` chain lock would collide. The derivation of the idempotency key and the membership fingerprint is not given either: format, length (8 to 160 characters, like `start_key`), the bytes hashed, and the "suffixed with the index" form used by `flatMap`.
   Why: The exactly-once start rests on this key. A builder of T042-15 or T042-24 cannot fill the primary key or derive the key without inventing the identity.
   Correction: Define, for every chain class:
   - the `key_hmac` input, including the keyless sentinel;
   - where `sequence` comes from when there is no key row (for example a chain-level counter, or `(source, event_id)` as the identity of a window sealed at once);
   - `opened_at` for windows sealed at once and for tumbling windows;
   - the exact key and fingerprint encodings, modelled on `start_key` in `crates/application/src/dispatch.rs`.

2. Blocker — The element and the run-input envelope are not specified, and both lanes depend on them
   Where: lines 137-139 (FR-042-003), 197-198 (FR-042-013), 241-244 (FR-042-019), 262-266 (FR-042-020), 497-498 (`trigger_stage(stage, element, event)`), T042-22 and T042-43
   What: Several shapes are undefined:
   - What `element` is at stage 0: the event document `{event, data}` that `event_document` builds, or `data`.
   - What `trigger_stage` returns for each operator (bool, value, list, or a key's canonical bytes).
   - The JSON shape for `emit: first`/`last` ("the element with the summary": under which property?), after `ignoreElements`, and for each `flatMap` run (the element alone, or with the summary?).
   - The types and formats of the summary fields (`keyId`, `openedAt`, `late`, `overflow`).
   - The input of a `filter`/`map`-only chain whose `map` yields a scalar. `is_json_object` in `crates/application/src/execution.rs` says "a run input must be one".
   - Which component runs the `OPERATOR_INPUT_SCHEMA` check (the codec in lane A, or `DefinitionService` in lane B), and on what exact sample document.
   Why: Lane A writes the publication check, the SDK scenarios and the goldens. Lane B writes the start. Each would invent an incompatible shape.
   Correction: Add a normative section with JSON examples for each emission kind and for the stateless-chain input (wrap a non-object, or refuse it at publication). Add the `trigger_stage` signature and its typed answer. Name the module that runs the input-schema check and give the exact sample.

3. Major — The two lanes share crates, and the order points miss real dependencies
   Where: lines 956-974; T042-15 (1038-1043), T042-16, T042-20, T042-23, T042-35 (1097), T042-43 (1116-1119)
   What: The plan claims "no crate in common", but:
   - Lane B's T042-15 writes `crates/domain/src/operators/window.rs` and the `trigger_stage` implementation in `crates/infra-ir/src/evaluator.rs`. Both crates are lane A's. T042-16, T042-20 and T042-23 put their rules in that same domain module.
   - Lane A's T042-43 must add the `IrError` variants (`OPERATOR_ORDER`, `OPERATOR_NEVER_EMITS`, `OPERATOR_INPUT_SCHEMA`) in `crates/application/src/definitions.rs`, a lane B crate.
   - The journal note of T042-35 touches `crates/domain/src/execution/event.rs`.
   - T042-13 (the `chain` column format), T042-14 (`TriggerRegistration` needs the chain type) and T042-54 (detecting a trigger chain) all depend on T042-43. Only T042-15 is ordered after it.
   Why: Two builders would edit the same crates and would ask who owns what.
   Correction: Either move the trigger domain rules and `trigger_stage` to lane A as an ordered deliverable that lane B consumes, or drop the "no crate in common" claim and list the shared files. Make T042-43 precede T042-13, T042-14 and T042-54.

4. Major — Manual acts and listing have no owning task and no defined semantics
   Where: lines 257-259, 288-290 (FR-042-023), 306-307 (FR-042-026), 668-669, 797-799 (section 9), 1047 (T042-17), 1094 (T042-34)
   What: Lane B is assigned only the delivery-retry application method (T042-18). No lane B task writes the application and store methods for:
   - listing chains and windows;
   - flush, discard and window retry;
   - the event resource's delivery outcome (T042-17 gives lane A only the response shape).
   The semantics are open:
   - Which states flush accepts.
   - Whether flush bypasses the horizon and backlog rules.
   - Who starts the flushed run. FR-042-023 records the caller as the starter, but `operator_windows` has no `started_by` column, and the pass starts as the engine.
   - What a retry of a `START_EXPIRED` window resets (`attempts`, `next_attempt_at`, the 24-hour clock).
   - The REST paths are elided with "…", including the delivery-retry path.
   - Error codes and HTTP statuses for a wrong-state flush, discard or retry are not given. Only `DELIVERY_EVENT_PURGED`/`409` is.
   Why: T042-34 cannot be wired, and the OpenAPI baseline cannot be written.
   Correction: Add a lane B task for these application and store methods. Specify the state preconditions and transitions, the starter column, the full paths, the response shapes and an error-code table.

5. Major — Lock modes on the chain row are unspecified, and the counters on it cannot be written under `FOR SHARE`
   Where: lines 648, 688-690 (section 5 steps 2-3), 713 and 727 (section 6), 270-272 (FR-042-021)
   What: The apply locks `operator_chains` `FOR SHARE`, then inserts a new key "under `key_count`". `key_count` and `mark_count` are columns of that same row. Two concurrent applies that each hold a share lock and then update the row is the lock-upgrade deadlock, and SC-042-006's key flood would hit it on every new key. The seal, the start and the purge "lock the chain row" in an unstated mode. The `KEY_LIMIT` audit is "collapsed per chain and minute with a count", but `audit_records` is append-only (`20261008000000_identity.up.sql`), and nothing says where the running count is kept.
   Why: The builder must choose lock modes and counter storage, and these choices decide deadlocks and contention.
   Correction: State the lock mode of each path. Move the counters out of the share-locked row (for example to a per-chain counter row taken `FOR UPDATE`, or check them by counting rows), or take `FOR NO KEY UPDATE` and accept the serialisation. Specify the row that accumulates `KEY_LIMIT` counts and when the audit record is written.

6. Major — The contract for the new `event_deliveries` columns is incomplete and contradicts itself
   Where: lines 199-206 (FR-042-014), 304-305 (FR-042-025), 663-665
   What: The new `outcome`, `window_id` and error-code columns have:
   - no closed value set or `CHECK`;
   - no nullability or default for existing rows or for wait deliveries (`wait:{run}/{node}`);
   - no value for design 043's refused deliveries (`record_refused_delivery`, `run_id` NULL).
   FR-042-025 says a delivery says `STARTED`, `HELD`, `SUPPRESSED` or `FAILED`. Yet a `filter`/`map`-only chain "is applied as a trigger is today": a filter that does not match writes no row, and a filter error is silently skipped (`crates/scheduler/src/events.rs`, the `dispatch` loop). So the outcome of a `map` error or a `false` stage on the stateless path is undefined. The set of error codes a `FAILED` delivery may carry (`KEY_LIMIT`, `QUOTA_EXCEEDED`, an expression failure, a key over 512 bytes or not a scalar) is not listed.
   Why: Both the migration (T042-13) and T042-17's response depend on it.
   Correction: Give the column definitions, the defaults and backfill for legacy rows, the closed outcome and code sets, and the stateless-chain behaviour for a `false` stage and for an erroring stage.

7. Major — The trigger operator parameter schema is not specified
   Where: lines 191-196 (FR-042-013), 210-222 (FR-042-015, FR-042-016), 229-230, 267-281 (FR-042-021), 417 (O042-007 row)
   What:
   - Required/optional status, defaults and ranges are missing for `horizonMs`, `keyIdleMs`, `maxKeys` (is 10,000 the default or the maximum?), `maxCount` (default?), `emit` (default?), `maxFanOut` (default?), `sizeMs`, `gapMs` and `count`.
   - The IR placement of the chain-level limits (`maxKeys`, `maxMarks`, `maxHoldMs`, `keyIdleMs`) is not given: on the trigger, or on a stage?
   - "Then at most one each of `distinct`, `skip`, `take`" does not say whether their relative order is fixed. Lane A's `OPERATOR_ORDER` and lane B's apply order (step 3: distinct, skip, take) could disagree.
   - The key's canonical text before HMAC is not given: is `81` the same key as `"81"` or `81.0`?
   - What the key's text column stores is not given.
   Why: The codec (lane A) and the apply (lane B) must agree on all of this.
   Correction: Add a parameter table per trigger operator and per chain-level limit, with name, location, required/default and range. State the intra-group order. Define key canonicalisation (for example FR-042-011's canonical form) and the absent-key input.

8. Major — The start outcome table is incomplete, and the fan-out has no failure semantics
   Where: lines 248-261, 262-266 (FR-042-020), 785-786, T042-22 (1065-1071), T042-24 (1074)
   What:
   - The design 043 gate also refuses finally with `RUN_INPUT_SCHEMA_UNUSABLE` (`InputRefusal::SchemaUnusable` in `crates/application/src/run_input.rs`). It has no row.
   - Today's start maps `StoreError::DatabaseUnavailable | DatabaseLimit` to an audited "database denied" no-start (`crates/scheduler/src/events.rs`, `start`). It has no row.
   - "The version can no longer run … with its code" names no code.
   - A `flatMap` over `maxFanOut`, or not a list, "fails the emission", with no code or state. Nor does it say whether this happens at apply or at start.
   - Nothing says what happens when index k is refused finally after indexes before it started. T042-24 says "all-or-nothing", while FR-042-020 starts one transaction per index.
   - A replay (`Admitted::Replayed`) has no open transaction, so the text does not say which transaction sets `STARTED`.
   - The window write "in the run's open transaction" needs a new `RunTx` method, as `record_delivery` is today. That method is not named, and "needs no change to 022's port" overstates this.
   Why: T042-22 asks for "a test per row", and the rows are incomplete.
   Correction: Complete the table. Define fan-out partial failure and its code. Name the `RunTx` method and the replay transaction.

9. Major — Quotas and the held-bytes ceiling are under-specified, and one is left to a spike
   Where: lines 267-281 (FR-042-021), 673-676, T042-2 (979-982), T042-13 (1027-1032), T042-14
   What:
   - Only `operator_held_bytes` is named. The tenant quotas for the number of chains, the sum of `maxKeys` and the sum of `maxMarks` have no resource names, no `quota_limits` and `quota_ceilings` entries, and no task in T042-13.
   - `QUOTA_EXCEEDED` on held bytes "holds nothing", but nothing says whether the delivery ends `FAILED` (terminal, retried by an operator) or follows the dispatcher's "waiting for room" retry (`counted = false`). That retry would keep `attempts = 0` and hold windows behind the backlog rule.
   - The usage accounting for held bytes (increment and decrement, and which table) is not given.
   - The window byte ceiling ("4 MiB as an input blob") depends on T042-2's undecided input-blob path.
   Why: The migration and the apply cannot be written without these choices.
   Correction: Name every quota resource with its defaults. Specify the `QUOTA_EXCEEDED` outcome and its accounting. Decide the input-blob path in the design, not in a slice 2 spike.

10. Major — Provisioning of the per-tenant HMAC key is unspecified and conflicts with NFR-042-002
    Where: lines 654-660, 932-935 (NFR-042-002), T042-53 (1153-1156)
    What: The key is "generated for a new tenant by the migration's function". The text does not say:
    - how that function runs (a trigger on `tenants`? a lazy insert inside `operator_key_hmac`?);
    - how tenants that exist at migration time get a key;
    - how a runtime-role tenant creation writes a table it may not read.
    Any of these needs a second `SECURITY DEFINER` function or trigger function. NFR-042-002 says "one … and no other".
    Why: T042-53 cannot be built or audited as written.
    Correction: Specify the generation mechanism (for example lazy, inside `operator_key_hmac`, or a definer trigger), the backfill, and the down migration. Then amend NFR-042-002 and the privilege-audit rows.

11. Major — The capability removal guard races with publication
    Where: lines 830-836, T042-54 (1157-1163)
    What: Publication "reads the requirement without a lock", and `remove`'s `verify` (`crates/infra-postgres/src/instances.rs`) runs in another transaction. A publication that read "recorded" and has not yet committed its chain row is invisible to `verify` under READ COMMITTED, so the requirement can be removed while a chain then commits. The registry precedent for the same problem (`lease_withdrawal_required()` in `20270211000000_lease_withdrawal_functions.up.sql`) reads the requirement `FOR SHARE` through a definer function and guards removal with a trigger.
    Why: This changes what T042-54 builds and tests, and "no definer function is needed" is likely wrong.
    Correction: Either take the requirement row `FOR SHARE` in the publication and `switch` transactions (with the definer function that requires), or argue why the race is harmless. Add a held-lock race test to T042-54.

12. Major — The engine-side trigger switch has no name and no defined behaviour
    Where: lines 826-829, 838-841 (section 10)
    What: An engine "lists `trigger-operators-v1` when its dispatcher and pass run with the switch on". The configuration variable is not named (the collection side has `SAGAT_COLLECTION_OPERATORS_ENABLED`). Nothing says what a dispatcher with the switch off does with a chained `workflow_triggers` row: start a run per event, skip the trigger, or fail the event. Nor whether the pass still drains open windows ("open windows drain" on rollback).
    Why: Lane B must implement this, and the over-delivery risk depends on it.
    Correction: Name the variable and state the dispatcher and pass behaviour for each switch state.

13. Major — The lock placement contradicts accepted ADR text, and the slot test cannot be written when T042-13 asks for it
    Where: lines 699-700, 720-724, 790-791, T042-13 (1029-1030)
    What:
    - ADR-0050 defines 8a as bookkeeping "of a transaction whose subject is one existing run". Its decision 3 says order (b) "MUST NOT lock a slot-3 row after its run". The start creates a new run and then updates the window row, which is slot 3 for the apply and the seal.
    - ADR-0086 decision 7 still says the start holds the chain, key and window rows before the port. Its 2026-11-27 revision claims to record the opposite "without changing the decisions".
    - The constitution (Governance) requires a new ADR for a divergence.
    - T042-13 (slice 2) must register the slots "with the slot test", but the start path only exists in T042-22, after feature 022 merges. The harness (`tests/support/lock_order.rs`, the `Slot` enum, `tests/fault/schedules-bulk-operations/slot_order.rs`) is not named.
    Why: The builder would follow contradictory normative text, and T042-13 cannot be completed in its slice.
    Correction: Record the deviation in an ADR or an ADR-0050 recorded addition, and fix ADR-0086 decision 7's text. Split the slot test: apply and seal in T042-13, start in T042-22. Name the harness.

14. Major — Several owner gates cannot open under the checker's rules
    Where: lines 976-978 (T042-1); the Owner-gate lines of T042-5, 6, 7, 18, 20, 39, 43, 45, 47, 56
    What: `scripts/check-design-docs.py` (`owner_row`/`citable`) requires `Status` to be exactly `Decided`, and requires the row's `Covers` to name the task.
    - OD-2026-11-13-15's status in `docs/owner-decisions.md` is "Decided; not built", so no task gated on it can ever be ticked.
    - Its `Covers` also omits T042-45 and T042-47.
    - OD-2026-11-26-136's `Covers` omits T042-56.
    - T042-1 checks only `Covers`.
    Why: This changes what must be approved before the tasks can be ticked.
    Correction: Have the rows corrected by their owners: the owner's own row for OD-2026-11-13-15; a delegate act for 136. Extend T042-1 to check the status cell.

15. Major — Several success criteria and tasks are not testable as written
    Where: lines 335-337 (SC-042-006), 321-323 (SC-042-001), 928-931 (NFR-042-001), 936-937 (NFR-042-003), 1080 and 1088 (T042-27, T042-31), 1194-1198
    What:
    - SC-042-006 requires "another tenant's dispatch latency within the stated bound", but no bound is stated.
    - SC-042-001 and T042-27 kill "at each write" of the apply, seal and start. No failpoint facility exists under `crates/`, and no crash harness is named.
    - The slice 4 test target is unnamed.
    - The lag alert threshold is unspecified.
    - The 6-statement budget for joining a window does not reconcile with the listed writes: chain lock, key lock or insert, window lookup or lock, member insert, window update, held-bytes usage, delivery insert.
    Why: These tasks cannot pass or fail objectively.
    Correction: State the latency bound and the alert threshold. Name the crash mechanism (for example a per-statement hook or a held lock plus connection kill) and the test target. List the statements counted against the budget.

16. Major — The metrics contract for triggers is not closed
    Where: lines 301-305 (FR-042-025), 730-732, 803-807
    What:
    - The `operator_elements_total{surface="trigger", operator, outcome}` outcome values are not listed; FR-042-025 requires a closed set.
    - `operator_open_windows` has no stated labels or source.
    - `operator_window_emit_lag_seconds` has no definition (from deadline, or from seal, to start?).
    - `operator_windows_total` mixes outcomes (`retried`, `expired`) with no stated mapping to state transitions.
    - The lag is said to be computed by the pass, but `scheduler_lag_seconds` is set in `crates/scheduler/src/monitor.rs`.
    Why: T042-35, the runbook and the alert depend on exact names and labels.
    Correction: Add a metric table: name, type, labels with closed values, the definition, and the emitting module.

17. Major — The journal note on a run started by a window is an undefined contract change
    Where: lines 808-809, T042-35 (1097)
    What: "The run's first journal event records the window that started it (chain, window id, count)" does not say which event, which field, its schema in `events.asyncapi.yaml` and `crates/domain/src/execution/event.rs`, or how an older reader replays it.
    Why: A journal change touches replay and the compatibility baselines.
    Correction: Specify the field and the event, and state whether it is additive for older readers. Add it to the contract baselines and to lane A's or lane B's file list.

18. Major — The pure window module's rules have gaps
    Where: lines 215-222 (FR-042-016), 231-232 (FR-042-017), 470 (CUJ 4), 651
    What:
    - "A late element joins an open window or opens one" does not say which open window, nor which span and deadline a new one gets (for tumbling, the past span).
    - `opened_at` for tumbling windows (the boundary, or the first element) is not given.
    - The meaning of `overflow` is not given. CUJ 4 sets it when a window is sealed exactly at `maxCount`, where nothing overflowed.
    - How `count {count}` interacts with `maxCount` is not given.
    - The derivation of the `shard` column for chains and windows is not given.
    Why: The property tests of T042-15, T042-20 and T042-25 need these rules decided, not chosen by the builder.
    Correction: State each rule explicitly in FR-042-016 and FR-042-017.

19. Major — The chain lifecycle and identity across versions are unspecified
    Where: lines 503-508, 647-648, 284-286 (FR-042-022), T042-14 (1033-1037)
    What:
    - Trigger ids are stable across versions and exclude the filter (`trigger_id` in `crates/application/src/dispatch.rs`; the upsert in `register_triggers`, `crates/infra-postgres/src/definitions.rs`). Nothing says whether `chain_id` is random or derived, or whether a rollback reactivates a `DRAINING` chain or creates a new one.
    - Whether `chain_digest` covers `key` and the chain-level limits is not stated.
    - Whether the dispatcher decodes `workflow_triggers.chain` (and with which codec) or reads the chain from the parsed version is not stated.
    - The hook is `register_triggers`, which is not named. `triggers_of` is in `dispatch.rs`, not `publishing.rs`.
    Why: T042-14 and the window identity depend on it.
    Correction: Define `chain_id` derivation, rollback reuse, the digest's coverage, the dispatcher's source of the chain, and the exact hook.

20. Nit — Stale line references to existing code
    Where: lines 74, 80-82, 111-113, 115, 607
    What: The text cites `passes.rs` "about line 1094". The collection entry is now about lines 1480-1950. The other references have also drifted:

    | Cited | Now |
    |---|---|
    | `canonical_items` "about line 304" | `collection_source.rs:459` |
    | `parse_stored` lines 144-150 | `lib.rs:172` |
    | `published_collection_definition` lines 262-271 | `convert.rs:757` (reached from `lib.rs:300`) |
    | claim "about line 227" | `crates/infra-postgres/src/dispatch.rs:150-170` |

    The cited `primitives.rs` ranges also no longer match (`enter_collection` is at 1529 and 4138).
    Why: These are misleading pointers.
    Correction: Cite functions instead of line numbers.

21. Nit — The size statement is off by one
    Where: lines 947-949, 1202-1205
    What: The text says "55 tasks", with slices 0 and 1 counted as "eighteen". T042-56 (slice 1) is in neither count. There are 56 tasks, and `docs/designs/README.md` says "0 of 56".
    Correction: Count T042-56.

22. Nit — Two passages are stale
    Where: lines 126 (FR-042-001), 402-434
    What: FR-042-001 says "Release 0.3.0 has the seven that a collection has", which no longer holds. The Owner decisions section stops at 2026-11-26-136, but tasks are gated on OD-2026-11-27-420, and OD-2026-11-27-421 to 423 shape this design.
    Correction: Update FR-042-001 and list OD-2026-11-27-420 to 423.

23. Nit — A wrong claim about the new index's lifetime
    Where: lines 679-680
    What: "One index entry per admitted event, removed at its first attempt" does not hold for retries that are not counted. `retry_event(..., counted = false)` keeps `attempts = 0`, so such events stay in `inbox_events_unattempted_idx` and keep holding windows until `maxHoldMs`.
    Correction: State this, and decide whether the backlog rule should exclude events that wait for room.
