# EGM design critique of design 042, third revision, round 1 (2026-11-27)

Doc: docs/designs/042-stream-operators.md
Reviewed-commit: 531af9fb03386ad8f1deaed27933872f35156504
Reviewer: egm-design
Note: the reviewer's output as returned, with this header added and machine paths made relative. Stages 5 to 8 of the
EGM review (the design critique with the zoom-out and comprehension lenses) folded into one run of the `egm-design`
agent of the accepted EGM plugin, with no conversation history, as for design 040.

Counts: Blocker 1, Major 16, Nit 6

1. Blocker — Applies to new keys or `distinct` marks on one chain deadlock, or the whole chain runs one event at a time
   Where: docs/designs/042-stream-operators.md lines 648, 687-690, 202-203, 233, 727, 383, 929-931
   What: `key_count` and `mark_count` are columns of the `operator_chains` row (line 648). Every apply locks that row `FOR SHARE` (line 688), then inserts a key "under `key_count`" (line 689-690); every `distinct` mark that passes counts against `maxMarks`. Keeping either counter exact means updating the chain row. When two transactions both hold `FOR SHARE` and both try to `UPDATE` the same row, PostgreSQL detects a deadlock and aborts one. If the apply takes `FOR UPDATE` instead, every new key and every passing `distinct` element on a chain waits for the one before. If the counter is counted with `count(*)` instead, concurrent inserts go past the ceiling. The lock mode on the chain row is also not stated for the seal (line 233, 713) or the purge (line 727), and the purge has to decrement `key_count`.
   Why: As written the design cannot meet NFR-042-001's 1,000 events per second over 10,000 keys, or SC-042-006's flood of 50,000 new keys, without deadlocks or a broken ceiling. G042-007 says only one hot key is serial; in fact a whole chain becomes serial.
   Correction: Take the counters off the chain row. Use a separate counter row, locked `FOR UPDATE` only on the new-key path, ordered after the chain row and before the key row. Bound marks per key under the key lock that is already held (for example `maxMarks / maxKeys`), so that no chain-wide row is written per element. State the lock mode on the chain row for apply, seal and purge. Update G042-007, NFR-042-001 and the ADR-0050 slot registration to match.

2. Major — Section 1 says "publication only reads a switch"; section 10 says the engine runs with a switch, and neither says what a switched-off dispatcher does
   Where: lines 510-519, 826-831, 140-142
   What: Section 1 says "The switches are checked at publication only … only a new publication reads a switch". Section 10 says an execution engine lists `trigger-operators-v1` "when its dispatcher and its `operator_windows` pass run with the switch on". So there is an engine-side switch. Nothing says what a dispatcher with that switch off does when it meets a `workflow_triggers` row that has a chain: fail closed, leave the event pending, or start one run per event.
   Why: This decides whether an engine that is configured wrong delivers too many runs. It also decides what the capability actually certifies, and what Task 042-54 tests.
   Correction: Say whether the engine has a switch. If it does, specify that an engine without the capability never applies or starts a chained trigger: it leaves the event pending (or, if `attempts` is incremented, it no longer holds windows) and never falls back to today's start. Then reconcile section 1's sentence with section 10.

3. Major — The rollout ignores the Approved design 053, which sets rules for every registry capability
   Where: lines 819-844, 832-836, 663-665
   What: Section 10 relies on `docs/runbooks/scaling.md` ("mixed old/new writers are unsupported"). It also says "Publication reads the requirement without a lock and the guard is the registry's removal check". Design 053 (Approved; Requirements 053-006 and 053-007) makes rolling upgrades with N-1 and N writers a supported state. It requires every capability's design to answer the late-activation case and the deactivation case, to register a descriptor (fence form (a), form (b), or "no fence" with a reason), to use `ServingGate::uses`, and to drain in two phases. 053's Problem section names exactly the race this design keeps: the drain check runs before the delete, so a write that began before it can land after it. Here, an unlocked publication racing `remove`'s `verify` can commit a chain after the requirement is gone. Separately, the new `event_deliveries.outcome` column has to be nullable or defaulted, or an N-1 dispatcher's insert fails during a rolling upgrade. The rollback below 0.3.0 that section 10 describes is a path 053 rules out ("a rollback never crosses the stopped upgrade").
   Why: Principle IV (compatibility). This changes what Task 042-54 must build and what reviewers must check.
   Correction: Add a subsection on design 053. Give the late-activation and deactivation answers and the descriptor (likely "no fence: every release a rollback can reach, 0.3.0 and later, processes chains", with the reason). Close the publication/removal race with form (a) or a `FOR KEY SHARE` read of the requirement row. State the expand-only shape of the new columns. Replace the rollback procedure with 053's drain phases.

4. Major — It is unclear which chain failures are final delivery outcomes and which leave the event pending
   Where: lines 214, 278-280, 700-701, 687, 196-198, 684
   What: Section 5 says "A failure here leaves the event pending and the dispatcher's existing retry applies, except `KEY_LIMIT`". But FR-042-015 says a failing expression "drops the element with outcome `FAILED`", and FR-042-021 says a held-bytes overflow "fails with `QUOTA_EXCEEDED` and holds nothing". A failing `key` expression (line 687) has no outcome at all. For a chain of only `filter` and `map`, nothing says whether a stage `filter` that drops an event writes a `SUPPRESSED` delivery or nothing (today a trigger filter that is false or fails writes nothing), or what a stage that is not boolean or fails does. In the dispatcher, `QuotaExceeded` counts as "waiting for room" (`crates/scheduler/src/events.rs:1045-1050`): it keeps `attempts = 0` for up to 24 hours, so under the backlog rule it holds windows.
   Why: Each of these choices decides whether an event stays in the backlog, holds every matching window for up to `maxHoldMs`, or ends. The SDK conformance tests and the event page both depend on the answer.
   Correction: Add one table: each failure (key expression, stage expression, non-boolean `filter`, `KEY_LIMIT`, mark ceiling, held-bytes quota, transient store error), for both the stateless path and the locked path, mapped to a final delivery outcome with its code, or to "transient, retried". State that held-bytes `QUOTA_EXCEEDED` is a final `FAILED` outcome, not a dispatcher `QuotaExceeded`.

5. Major — The default `maxHoldMs` equals the dispatcher's claim lease, so SC-042-001 and CUJ 3 are not guaranteed
   Where: lines 228-230, 321-323, 430, 457-463, 747
   What: `CLAIM_LEASE` is 60 s (`crates/application/src/dispatch.rs:31`), and the default `maxHoldMs` is also 60 s. Take an event received at r ≤ D whose apply is killed. It can only be claimed again at r + 60 s, and it holds the window only until D + 60 s. The margin is D − r plus the dispatcher's tick, and it reaches zero when the event arrives just before the deadline. The event is then late: it opens a new window, so a second run starts. SC-042-001 kills the process at each write of the apply and still promises "exactly one run". CUJ 3 says the pass seals "thirty seconds after the last arrival", but in its own scenario the killed seventh event holds the window until it is claimed again, at about 60 s or more. `maxHoldMs` has no minimum.
   Why: The headline promise ("one run per burst, across a crash") depends on a margin the design does not establish.
   Correction: Require `maxHoldMs` to be at least `CLAIM_LEASE` plus the dispatcher interval plus a margin, with that as the minimum and a larger default. Alternatively, release a crashed claim sooner. Then fix CUJ 3's timing and state SC-042-001's quiet period relative to these values.

6. Major — The lag metric hides exactly the delays the risk table says it reveals, and no alert covers the transaction horizon
   Where: lines 730-732, 878, 805-807, 937, 52-53 of the ADR-0153 file docs/adr/ADR-0153-a-window-is-sealed-only-past-the-transaction-horizon-of-its-deadline.md
   What: Section 6 leaves windows held by the horizon or by the backlog rule out of `scheduler_lag_seconds{pass="operator_windows"}`. Yet the risk table says "A long transaction delays every window … the pass's lag metric and alert show it". The only alert (NFR-042-003) is on that lag metric. `pg_current_snapshot()`'s xmin covers every transaction with an assigned id in the whole cluster, including other databases, migrations and forgotten prepared transactions. Separately, events waiting for room keep `attempts = 0` for up to a day and hold windows by `maxHoldMs` each, which `held_by_backlog` also hides.
   Why: Windows can stay unsealed for hours with no alert firing. Principle VI ("its lag is displayed").
   Correction: Ship an alert on `operator_seal_horizon_age_seconds`, plus one on the time windows spend held by the backlog rule, with thresholds. Correct the risk table. State in the runbook that the horizon is cluster-wide and how to find the transaction holding it.

7. Major — The run input of a trigger chain is not fully defined, and some shapes break "every run input is a JSON object"
   Where: lines 241-243, 137-139, 200-201, 157-158 (catalogue line 529)
   What: (a) A chain of only `filter` and `map` makes the `map` output the run input ("only the input changes"). Since `map` cannot build an object (G042-001), it usually yields a scalar or a list, but `crates/application/src/runs.rs:1349` refuses any run input that is not an object. (b) For `emit: first`/`last` the input is "the element with the summary", but the field names are not given. (c) Adding a `distinct` to a chain silently changes the input from the event document to `{window, events:[e]}`. (d) The publication check uses "an empty `events` list", which never occurs and does not cover the first/last shape.
   Why: Design 043's validation, the SDK conformance tests, the publication check and the CUJs all need one exact input contract.
   Correction: Give the exact JSON for every emission kind: stateless chain, `all`, `first`/`last`, `ignoreElements`, and each `flatMap` index. Either refuse at publication a chain without a window that ends in `map` (or wrap its value, e.g. `{value: …}`), or define how it is wrapped. Check the input schema against a one-element sample of each shape.

8. Major — `flatMap` on a trigger is called all-or-nothing in one place and started per index in another, and its evaluation and failure points are undefined
   Where: lines 262-266, 1074, 248-256, 208-209
   What: Task 042-24 says "all-or-nothing start". FR-042-020 says one transaction per index and resumes "at the next one" after a crash. So a `RUN_INPUT_INVALID` or `START_EXPIRED` at index k leaves k runs started under a `REFUSED` window. Several things are unsaid: what `element` is when `select` runs on an emission (the envelope, or each member); whether `select` is evaluated at seal or at each start; and which window outcome "fails the emission as a whole" maps to (it is not in the outcome table).
   Why: These choices change what Tasks 042-22 and 042-24 build, and what the fault tests in Task 042-27 assert.
   Correction: Choose one model (per-index with partial completion is the honest one) and remove "all-or-nothing". Evaluate `select` once, at seal, and record the count. Add a row `REFUSED` (`FLATMAP_FAN_OUT` / not a list) to the outcome table, and say what each index's input is.

9. Major — The `KEY_LIMIT` retry action re-dispatches the event to every consumer, not just the one trigger
   Where: lines 271-276, 1049-1051
   What: The retry deletes one delivery row and returns the event to `PENDING`, and "the others find their delivery rows and change nothing". That is false in several cases: triggers registered since the first dispatch (trigger ids are stable, filters can change with a republish), triggers whose filter did not match (no delivery row; they re-evaluate a filter that may have changed), and waiting steps (`waits_for_event`). The returned event also has `attempts = 0` and an old `received_at`, so it re-enters the backlog rule.
   Why: An operator act on one trigger can start runs on other workflows and wake steps. That is an audit and correctness surprise that Principle VII's "consumer tolerates duplicates" does not cover.
   Correction: Run the retry as a targeted apply of the one trigger, in the action's own transaction or through a delivery-scoped re-dispatch, without returning the inbox row to `PENDING`. Alternatively, mark the redelivery as limited to that consumer.

10. Major — An event re-admitted after its dedupe window conflicts with its own member row
   Where: lines 652, 738, 281, 216-219
   What: `operator_window_members` is keyed by `(window_id, source, event_id)`. The dedupe TTL can be as short as 60 s (`MIN_DEDUPE_TTL`, `crates/domain/src/events.rs:35`), while a window lasts up to 24 hours. A provider that re-sends the same event id after the dedupe window gets a new inbox row and a new delivery row (the old one was removed by cascade). Its member insert into the still-open window then fails on the primary key, the apply errors, and the event retries until `DEAD`.
   Why: A routine provider retry turns into a dead event. The failure table only covers "Same event id inside the dedupe window".
   Correction: Define the outcome: insert the member `ON CONFLICT DO NOTHING` and record `SUPPRESSED` (reason `duplicate_member`). Alternatively, refuse at publication a window longer than the dedupe TTL of the endpoints it matches. Add the case to section 7 and a test.

11. Major — Without `keyIdleMs` a key's epoch never ends, so key rows are never purged and a debounce-per-order chain hits `KEY_LIMIT` for good
   Where: lines 212-213, 282-283, 649, 457-458
   What: The purge removes "a key row with no open window whose epoch has ended". An epoch ends after `keyIdleMs`, which is "required with `skip` or `take`" only. CUJ 3's chain (`key` plus `debounce`) has no `keyIdleMs`. Read literally, its key rows (one per order) are never purged, and after `maxKeys` (10,000) orders every new order ends in `FAILED`/`KEY_LIMIT`. `keyIdleMs` also has no declared place (catalogue, FR-042-013), no range and no default.
   Why: The flagship journey degrades permanently in production.
   Correction: Purge a key row that has no `OPEN` or `SEALED` window and no `skip`/`take` state, or define a default idle period for every keyed chain. Declare `keyIdleMs`: where it lives, its range and its default. Add a test that 2 × `maxKeys` sequential keys are all served.

12. Major — The start's write of the window after the run departs from ADR-0050 without being recorded as a deviation, and a zero-row update is not handled
   Where: lines 717-723, 244-246, 790-791, 1029-1030; docs/adr/ADR-0050-many-run-operations-admit-under-stable-keys-and-never-buffer-or-reopen.md lines 51-54, 82, 95
   What: The window row is a slot-3 row for the apply and the seal, and an "8a" row for the start. ADR-0050 decision 3 says order (b) "MUST NOT lock a slot-3 row after its run". Decision 7 says never return to a lower slot, but the port takes 022's slot 7/8 rows, including 8b usage rows, before the window (8a) is written. Task 042-13 "registers" this; it does not record a deviation with an argument, as decision 7's other deviations are recorded. Separately, if the 24-hour expiry (or a concurrent start) has already moved the window off `SEALED`, `UPDATE … WHERE state = 'SEALED'` matches zero rows, and nothing says the start then rolls back its run.
   Why: Governance requires that a divergence from an accepted ADR be recorded. Committing a run whose window is `REFUSED` breaks "exactly one run" and "`REFUSED` is final".
   Correction: Add a "Recorded deviation" to ADR-0050 decision 7 with the no-cycle argument (including the 8b to 8a step). Specify that zero rows updated rolls back the start transaction, and test it against the expiry.

13. Major — The HMAC machinery protects little, and its definer function was admitted as a security exception by a delegate
   Where: lines 294-296, 649, 654-661, 932-935, 904; docs/owner-decisions.md OD-2026-11-27-422
   What: `operator_keys` stores the key text in clear, as content, in the same row as its HMAC. So for keys, the per-tenant HMAC key, `pgcrypto`, `operator_hmac_keys` and the `SECURITY DEFINER` function protect nothing that surrogate ids do not already protect (the API never shows a digest). Only `distinct` values are stored solely as HMACs. OD-2026-11-27-422 calls the definer function "an exception to NFR-042-002" and is decided by delegate. The constitution keeps "every security exception" with the owner.
   Why: Zoom-out: a security-sensitive part of the design with little benefit. Governance: the approval path may be invalid.
   Correction: Key `operator_keys` by `(chain_id, key_id)` with a unique index on the key text (or on an internal SHA-256 that is never exposed), and derive the start key from `window_id` (finding 14). Keep an HMAC only for `distinct` marks if that is worth the function, and have the owner decide the definer-function exception, or show that it is not one.

14. Major — The window identity depends on a key HMAC and a key-row sequence that some chains do not have
   Where: lines 208-209, 237-240, 649, 651
   What: The start key comes from "(namespace, chain, key HMAC, the instant it opened, its sequence)", and `next sequence` lives in the key row. A chain of `filter`, `map` and `flatMap` "takes no key row and no key lock", yet each of its emissions is a window keyed `(chain_id, key_hmac, opened_at, sequence)`. Nothing says where its sequence or HMAC comes from, or how two concurrent applies with equal `opened_at` avoid a collision.
   Why: The key for these windows cannot be built as specified.
   Correction: Derive the idempotency key from the window's random `window_id` (durable before any start) plus the membership fingerprint. Key `operator_windows` by `window_id`. Revise ADR-0086 decision 6 to match.

15. Major — A window input above the inline ceiling is required but left to a spike
   Where: lines 267-268, 979-981
   What: FR-042-021 lets a window grow to "4 MiB as an input blob". Task 042-2 lists "the input-blob path for a window above the inline ceiling" as a spike. Nothing describes how the start writes a blob (object-store I/O inside or before the run transaction, what happens on failure, quotas).
   Why: This is an unbuilt requirement inside slice 3. A simpler option meets the CUJs.
   Correction: For 0.3.0, bound a window by the inline run-input ceiling (`onFull` applies to bytes as well) and defer blob inputs to a follow-up. Otherwise, design the blob path before approval.

16. Major — Nothing fails closed if feature 022 slips, and the trigger half has no cut line
   Where: lines 773-779, 877, 951-954, 1202-1206
   What: Slice 3's start waits for an unmerged 022 branch. The plan says slices 2, 4 and 5 proceed, but nothing stops `trigger-operators` being switched on in a 0.3.0 that cannot start a window. Events would then be `HELD`, and windows would expire 24 hours later with values kept. The trigger half has 37 tasks across six tables, a pass, HMAC infrastructure, six SDKs and the Studio, and no sequencing that lets less critical operators slip without a redesign.
   Why: There is release risk and no defined degraded state. Zoom-out: the owner's driving journey is "one run per burst per order".
   Correction: Mirror D042-103: the control plane refuses `trigger-operators` until the start (Task 042-22) is built. Then define a cut order inside slices 2 to 5 (key, `filter`/`map`, `window`/`debounce`/`ignoreElements` first; then `distinct`/`skip`/`take` with marks and epochs, and trigger `flatMap`), so that the later ones can be deferred without touching the first.

17. Major — SC-042-006 refers to a "stated bound" that is never stated
   Where: lines 335-337, 926-931
   What: "another tenant's dispatch latency stays within the stated bound". No such bound appears in NFR-042-001 or anywhere else.
   Why: The success criterion cannot be checked. Principle VIII says a number is never invented and must not be left open either.
   Correction: State the bound in NFR-042-001 (for example, a p99 dispatch latency for a second tenant during the flood, on the stated environment), or mark it as pending a measurement and name the task.

18. Nit — The delivery outcome vocabulary is incomplete across paths
   Where: lines 205-207, 304-305, 663-665
   What: A window sealed at once (a `distinct` chain without a window) records `HELD` even though its run starts milliseconds later. `STARTED` appears only in FR-042-025. Nothing says what a trigger without a chain, or a chain of only `filter` and `map`, writes in `outcome`.
   Why: The event page will look wrong to someone reading it.
   Correction: Say which path writes which outcome, including null or `STARTED` for chainless and stateless triggers, and say that `HELD` on a window sealed at once is expected.

19. Nit — Stale and contradictory statements
   Where: lines 126-127, 620, 31-32, 1202-1206, 952, 973, 115
   What: FR-042-001 says "Release 0.3.0 has the seven that a collection has". Section 3 says "No migration" while Task 042-56 adds one. "55 tasks" when there are 56, and slice 1's "eighteen" leaves out Task 042-56. Task 042-3 sits in slice 2 but waits for 022, like slice 3. The cited line for "claiming does not increase `attempts`" (about line 227) is `finish_event`; the claim is at `crates/infra-postgres/src/dispatch.rs` lines 150-180.
   Why: A new reader cannot tell which text is current.
   Correction: Update each statement.

20. Nit — The Overview is a revision log, and terms are used before they are defined
   Where: lines 24-69, 137-139, 60, 699-700, 717-723, 421-434
   What: About 45 lines of history come before the problem. "Slice", "keyed stage", "window envelope", "summary", "backlog rule", "ADR-0050 slots/8a", "receipt" and "the port" are used before they are defined. The Owner decisions section stops at 2026-11-26-136, but tasks are gated on OD-2026-11-27-420, and OD-2026-11-27-421 to 423 shape the design.
   Why: Comprehension lens.
   Correction: Move the revision history to a "Revisions" section at the end. Add a short glossary before Requirements. List the 2026-11-27 rows.

21. Nit — The canonical order inside the keyed group is not stated
   Where: lines 192-195
   What: "at most one each of `distinct`, `skip`, `take`" does not fix their relative order, yet `OPERATOR_ORDER` and section 5 ("apply `distinct`, `skip`, `take`") need one.
   Why: Six SDKs and the Studio must enforce the same rule.
   Correction: State the order explicitly.

22. Nit — The dispatcher's immediate start cannot use "the same code as the pass"
   Where: lines 260-261, 697-698, 717
   What: The pass's start reads the shard row `FOR SHARE`. The dispatcher is not shard-fenced, and ADR-0050 decision 6 keeps fence reads to passes.
   Why: Two variants of the start code are needed, not one.
   Correction: Say that the dispatcher's start skips the fence and relies on the slot-2 receipt.

23. Nit — It is unclear whether chain state survives a republish with an unchanged chain
   Where: lines 284-286, 503-508, 648
   What: "republished with a different chain" implies that an unchanged chain keeps its row (keys, marks, epochs), but `operator_chains` carries a `version`.
   Why: This decides whether `distinct` and `take` reset on every deploy.
   Correction: State that the chain identity is (trigger, `chain_digest`), and what `version` on that row means.
