# Approval read of design 042: security (2026-11-27)

Doc: docs/designs/042-stream-operators.md
Reviewed-commit: 4b0248c7b2019be951f38c1e6ec12dd4e662569b
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output as returned, with this header added and machine paths made relative. It is the sixth
security re-read of the 2026-11-27 round, asked as the approval read; the five before it are under `../reread/` to
`../reread-5/`. Its Nits are answered by follow-up tasks.

Counts: Blocker 0, Major 0, Nit 6

## Previous re-read

1. **Partly fixed.** The 24-hour retry grace is in (290-291, 296-300, 383-384, 944, 973-977, 1130-1133, T042-26 at 1659-1663). So is the `flatMap` pre-charge: I checked its arithmetic against the code and it holds (319-334, 944-946, T042-24 at 1645-1650). The gauge and the three alerts are in section 9 (1258, 1263-1266). Two things are still open. The stated longest hold of 72 hours is wrong. "Never waits" ignores the content quota. Repeated as finding 1. The new alerts are also missing from the tasks (finding 2).
2. **Fixed.** T042-40 now has the harness apply the three lines. It also adds a contract test over `01-runtime-role.sh` and `NOTES.txt` (1703-1714).
3. **Fixed.** NFR-042-002 now says the function "takes the tenant's held-bytes advisory lock and no row lock" (1437).
4. **Fixed.** FR-042-024 is narrowed to keys, `distinct` values and their digests. It names `audit:read` as Admin-only, which `crates/domain/src/identity.rs:546-701` confirms. A value-dependent chain's span carries no member count (421-425, 1267-1268).
5. **Fixed.** `operator_window_fanout` stores one row per index, and each index start reads and deletes only its own row (317, 349-351, 946, T042-24 at 1639-1640).
6. **Partly fixed.** The pass's seal and the flush now evaluate outside any transaction under the 1-second budget (312-316, 1100-1104, 1138-1139). The `full` rule is in (345-348, 944, 1646). The `count` or `onFull` seal at apply step 4 still evaluates `select` and the validator inside the apply transaction. Repeated as finding 3.

## Findings

1. Nit — The stated longest hold (72 hours) is wrong, and "such a seal never waits" ignores the content quota
   - Where: docs/designs/042-stream-operators.md 296-300 and 338-341 (the 72-hour bound); 290-291 (`start_expires_at` is "24 hours after the seal"); 289-290 and 1111 (a waiting seal stays `OPEN` until `D` + 24 hours); 331-334 ("such a seal never waits"); T042-24 at 1648-1650; T042-26 at 1661-1663; Revisions at 1907-1910; 399-400.
   - What: Checked in the text.
     - **The bound.** Take a `flatMap` window whose `select` enlarges its envelope. It is open for up to 24 hours. It waits in `quota_wait` until room appears at `D` + 23 h 59 min and seals. Its start is then refused with `CONCURRENCY_QUEUE_FULL` for 24 hours, which makes it `START_EXPIRED` about 48 hours after `D`. Its values stay for 24 more hours of grace. That is about 96 hours after its first element, with no operator retry.
     - The sentence "at most 24 hours waiting to seal … or to start" is false, because one window can do both in turn.
     - A window without `flatMap` also gets `maxHoldMs` (up to 10 minutes) plus the pass's lag on top of 72 hours.
     - The T042-26 assertion ("no window holds values more than 72 hours") cannot pass for a `flatMap` window.
     - **"Never waits".** The pre-charge is only in `held_charge`, but the content quota counts the bytes of the `value` columns. For `[element]` or `element.events`, each recorded fan-out row holds the member's envelope fields (52 to 542 bytes) on top of its value. The content delta of the seal is therefore positive.
     - With the tenant content quota full, every such seal goes to `quota_wait`, which 335 says. The claims "never waits" at 334 and "every one of them seals, none waits" at 1650 do not hold.
     - FR-042-024 (399-400) also still says held values are deleted "in the transaction that starts the window's run (or discards it)". That omits the refusals, the grace and the deletion index by index.
   - Why: The previous Major asked for the true duration. The doc and a task test still state a bound the design does not meet, and the operator's alert and runbook will be calibrated to it.
   - Correction:
     - Either anchor both waits to one deadline, for example `start_expires_at = seal_expires_at = D + 24 hours`, so that 24 + 24 + 24 = 72 hours plus `maxHoldMs` holds. Or restate the bound as 96 hours (72 hours plus `maxHoldMs` without `flatMap`) in 298-300, 341, 1662 and 1908.
     - Say "never waits for held bytes" at 334, and make the T042-24 case run with content room.
     - List every deletion point in FR-042-024, or point it to section 4's paragraph.

2. Nit — Three of the six alerts in section 9 are missing from the NFR and the tasks
   - Where: 1260-1266 (six alerts) against 1444 (NFR-042-003, "the three alerts of section 9"), 1688-1689 (T042-35, "the three alert rules with their promtool cases") and 1380 (Constitution Check, "three alerts").
   - What: Checked in the text. T042-35 as written ships three rules. The ones the fifth re-read required against the cross-namespace pool hold (`OperatorSealQuotaWait`, `OperatorWindowsExpired`, `OperatorHeldBytesNearQuota`) are the ones most likely to be left out.
   - Why: These alerts are the stated mitigation of the pool exhaustion that one namespace can cause. A mitigation without a task is not built.
   - Correction: Write "the six alerts", or name them, in NFR-042-003, T042-35 and the Constitution Check. Add a promtool case for each new alert.

3. Nit — The `flatMap` seal at apply step 4 still evaluates inside the apply transaction, against FR-042-020 (repeat of the fifth re-read's finding 6, partly)
   - Where: 312-316 (`select` and the validator run "outside any transaction, over the members read without a lock"); 1063-1064 (step 4 seals at once on `count` or `onFull` and, "with `flatMap`, evaluate `select` and record the fan-out"); 1073-1074; 306-307 (where the evaluation of a window sealed at once runs is not stated).
   - What: Checked in the text. A `count` or `onFull` seal at step 4 includes the member that this transaction inserts. It cannot be evaluated before the transaction.
     - So `select` over an envelope of up to 256 KiB, and up to 100 validator runs, execute while the apply holds the key row and the window `FOR UPDATE`. For a new key the apply also holds the chain's usage row, which serialises every new key of the chain.
     - The connection sits idle in its transaction for up to the 1-second budget.
     - The FR's rule as written cannot be built for this path. An implementer will either break the rule silently or invent a path.
   - Why: The previous finding asked for this work to be off any lock. The pass path is fixed but the apply path, which the dispatcher (one queue for all tenants, G042-005) runs, is not.
   - Correction:
     - On a `flatMap` chain, a `count` or `onFull` seal at step 4 should not evaluate. It should mark the window `full` and due, as the path that lacks room already does (345-348), and leave the seal to the pass, whose evaluation is outside any transaction.
     - For a window sealed at once (a chain with no window stage), state that `select` and the validator run in step 1 with `trigger_chain`, before any lock.

4. Nit — A window retry can race the purge of its values at `values_expire_at`
   - Where: 1141-1144 (the retry checks that `values_expire_at` "has not passed", then clears it); 1130-1133 (the purge nulls the values of expired windows); 263-264 (FR-042-018 lists who locks a `REFUSED` window, and the purge is not listed); 271-272 (the start stops when a value is missing).
   - What: Checked in the text.
     - Suppose the retry's transaction starts just before `values_expire_at` and the purge's statement just after.
     - The purge updates member and fan-out rows without locking the window row, so the retry does not block it. Its subquery snapshot does not see the retry's uncommitted write.
     - Under READ COMMITTED, the predicate on the window row is not re-checked.
     - The retry commits `SEALED` (or `OPEN`) and answers `202`, while the purge commits `NULL` values.
     - The start then finds a value missing and stops on every tick until `start_expires_at`. A seal of an `OPEN` retry evaluates `select` over null values.
     - Nothing leaks, and the bytes are released correctly. But the operator is told that a retry succeeded when it did not.
   - Why: This is check-then-act outside the lock, in the new grace path.
   - Correction:
     - Have the purge take the `REFUSED` window `FOR UPDATE SKIP LOCKED` and re-check its state and `values_expire_at` under that lock before it deletes values.
     - Record the purge on the window (for example `values_purged_at`), and have the retry refuse `WINDOW_VALUES_PURGED` on that marker rather than on a clock comparison.
     - Add the purge to FR-042-018's lock list, and add a held-lock test of retry against purge in both orders to T042-26.

5. Nit — The single-window read does not apply FR-042-024's rule for value-dependent chains
   - Where: 1231 (`GET …/windows/{windowId}`: "one window; with `reveal=pii`, also its `keyId` and its members"), against 404-412 (for a value-dependent chain, every window resource — count, instants, flags, run id — needs `reveal=pii`) and 1230 and 1232 (the list and the flush do restrict it).
   - What: Checked in the text. Read literally, the single-window GET returns count, instants, `late`, `overflow` and `runId` to any `event:read` holder, a Viewer included (`crates/domain/src/identity.rs:689-700`).
     - All it needs is a window id. A value-dependent chain's id reaches the trace backend on `operator.window.start` (1267-1268).
     - The count tells how many events shared a key, which FR-042-024 classes as personal data.
   - Why: The surface table is what an implementer of T042-34 follows, and it contradicts the requirement.
   - Correction: In row 1231, state that for a value-dependent chain without `reveal=pii` the answer is only the window's id and state, as at 1232. Add that case to T042-17's or T042-53's tests.

6. Nit — Trigger elements have no depth ceiling, though the envelope adds levels (suspicion)
   - Where: 356-359 (FR-042-021 bounds a window by count and bytes only); 454-466 (FR-042-032: a member's value sits 3 levels deep in `{"window", "events": [{…, "value"}]}`, and a `flatMap` index value 1 level deep); `crates/application/src/dispatch.rs:154-170` (`event_document` wraps `data` one level down and parses it at serde_json's default recursion limit of 128); the collection side's `MAX_ELEMENT_DEPTH = 120` (`crates/application/src/collection_chain.rs:33`, D042-155).
   - What: The missing bound is verified in the text; the effect is suspected.
     - An event document nested 126 to 128 levels deep passes the dispatcher. It then yields a window input of 129 to 131 levels.
     - serde_json with its default limit cannot parse that input back. The readers are design 043's validator, the run's expression environment and the workers' SDKs.
     - If the port reports this as a transient error, the window stays `SEALED` with its values held for 24 hours, then 24 hours of grace. If it surfaces in the run instead, the run fails with an untyped error.
     - Either way, a publisher can produce it on purpose.
   - Why: The JSON-depth class. The collection side solved the same problem with D042-155, and the trigger side has no equivalent.
   - Correction: Check the projected element's depth at the apply against a ceiling that leaves room for the envelope (for example 120 minus 4). Refuse it `FAILED` with a typed code, and add the row to FR-042-031's table and a test at the ceiling.

No finding in the other classes I checked:
- **Pre-charge arithmetic.** Verified against the code. The member fields come to 52 + 63 + 400 + 27 = 542 ≤ 544, given that a `Slug` is at most 63 bytes of `[a-z0-9-]` and an `EventId` is at most 200 ASCII-graphic characters, so escaping at most doubles it. The summary plus the outer structure come to about 341 ≤ 512. `MAX_COPIED_BYTES` (516 KiB, `crates/infra-expr/src/limits.rs:28`) caps how far a `select` can enlarge an envelope.
- **The `lost_race` recheck.** Member values are immutable and the member insert is `ON CONFLICT DO NOTHING`, so the count plus the identity fingerprint pins the evaluated values. After `D`, only late elements can change membership, so the recheck cannot livelock. The held-bytes advisory lock is never held during an evaluation.
- **`operator_window_fanout`.** Content-classified, counted in retained bytes, granted `SELECT`, `INSERT` and `DELETE` only (no row lock is needed), deleted on every refusal path, and covered by `zz_operator_held_flush`.
- **The gauge and metric labels.** Closed labels, no tenant label.
- **Definer functions and grants.** Unchanged since the fifth re-read.
- **Tenant boundary.** A uniform `404` for every new path, including `WINDOW_VALUES_PURGED`, which needs `run:operate`.
- **Gaps 042-017 and 042-018.** Stated the same way in the doc and in `specs/042-stream-operators/research.md:129-130`.
- **Injection, regular expressions, SSRF and egress.** Nothing new.
