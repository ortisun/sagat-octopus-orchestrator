# Approval read of design 042: correctness (2026-11-27)

Doc: docs/designs/042-stream-operators.md
Reviewed-commit: 4b0248c7b2019be951f38c1e6ec12dd4e662569b
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 6, Nit 7

## Previous review

1. Blocker, one burst becomes two runs: **fixed.** Lines 229-235 add session merging, line 239 clears the horizon on any deadline change, and line 1163 covers out-of-order applies. SC-042-001 at lines 532-538 now has the stranded-batch kill and the two-replica barrier. ADR-0155 decision 2 records the rule. The merge rule brings its own defect (finding 1).
2. Major, the horizon on the pass side: **fixed.** Lines 247-251 and 1091-1094 define it, the fitness check is at 1803-1804 and the hand mutation at 1836. The race test of T042-55 (1782-1785) still cannot tell `now()` from `clock_timestamp()`; only the static check catches that.
3. Major, `maxHoldMs` and attempted events: **fixed.** The hold now covers events with any attempt count (252-256), with a 90 s floor (816-817) and the `status = 'PENDING'` index (952-955). `quietMs` is stated in SC-042-001 (532).
4. Major, the start's fence: **fixed.** The start is declared unfenced at lines 270-277, 1112-1124, 1209-1212 and 1419-1420, and in ADR-0155 decision 4.
5. Major, window identity: **fixed.** A random `window_id` is the identity (266-268, 944; ADR-0155 decision 1).
6. Major, `flatMap` fan-out: **fixed.** See lines 305-355, the table rows at 288-289, T042-24 (1636-1653) and the owner-decision row at 683. The replay path it adds has a defect (finding 3).
7. Major, an apply that finds its chain `DRAINING` or gone: **fixed.** See lines 216-218 and 441, and T042-15 at 1581. A retry that counts no attempt is the dispatcher's 30-45 s capacity back-off, which ends in `DEAD` after a day (architecture approval #14).
8. Nit, a redelivery colliding with its own member row: **fixed** (lines 440, 1062, 1160).
9. Nit, the `START_EXPIRED` retry and the unlocked read of held values: **fixed** (lines 268, 271-272, 290-296, 1142-1144).
10. Nit, the backlog index: **fixed** (lines 952-955). Wildcard triggers are not served by it (finding 11).
11. Nit, ADRs contradicted without amendment: **fixed.** ADR-0155 supersedes the decisions (lines 1332-1343 here; ADR-0155 lines 6-9 and 66-71), the deviation is recorded in ADR-0050 at line 103, and T042-58 (1803-1808) checks it. Architecture approval #4 lists the ADR-0086 text that is still stale.
12. Nit, the collection text against the code: **fixed** (lines 38-43 and 880-882, T042-8 at 1530-1533, T042-56 at 1791-1795).
13. Nit, untestable criteria and budgets: **partly fixed.** The SC-042-006 bound (551-555) and the failpoints (534-538) are fixed. The statement budget is still miscounted (finding 7).

**Already raised in this round; I confirmed each and do not count it again:**
- Security approval #1: the 72-hour bound is really about 96 hours for a `flatMap` window.
- Architecture approval #2: the `SEAL_EXPIRED` retry reopens a window without its key row or key slot.
- Architecture approval #3: nobody owns discarding a removed trigger's windows, and the work is unbounded.
- Architecture approval #6: `MissingReference` is mapped wrongly, and design 043's check runs outside the port.
- Architecture approval #9: the flush order is held only by trigger names. Finding 5 below shows a concrete inversion.
- Architecture approval #14: events go `DEAD` after a day while the capability is not in use.

## Findings

1. Major — A merge can break `maxWaitMs`, `maxCount` and the byte ceiling, and then the windows depend on the order the events are applied in
   - Where:
     - `docs/designs/042-stream-operators.md` lines 229-235 (FR-042-016 merge rule and the claim "do not depend on the order"), 356-359 (FR-042-021 ceilings), 824 (`maxCount` on session windows and `debounce`), 346-348 (`full` refuses a merge).
     - T042-15 at 1574-1575 (the order-independence property test) and SC-042-002 at 539-541.
   - What (confirmed in the text; this is arithmetic on the stated rule):
     - Take session windows with `gapMs` 30 s and `maxWaitMs` 300 s. Elements arrive at r = 0, 25, …, 275, so W1 has `closes_at = min(275+30, 300) = 300`. Two more arrive at r = 295 and r = 310.
     - **Applied in order:** 295 joins W1, and 310 is outside `[−30, 300]`, so it opens W2. The result is two windows, {0…295} and {310}.
     - **Applied with 310 first:** W2 opens with interval `[280, 340]`. Then 295 falls into both W1 and W2, so they merge. The survivor has `closes_at = min(310+30, 0+300) = 300`. The result is one window holding a member received after its own deadline.
     - The same merge can join two windows of 600 members each into 1,200 members, above `maxCount` (1,000). It can also produce an input above 256 KiB, which no row of the outcome table covers. The port would refuse it on every retry until `START_EXPIRED`.
     - If both windows already hold the same `(source, event_id)` (a re-admission after a 60 s dedupe window, with `gapMs` ≥ 60 s), moving the members violates the members' primary key. The apply then retries, counted, until `DEAD`.
   - Why: The "one run per burst whatever the order" promise only holds while `maxWaitMs` and the ceilings never bind. The T042-15 property test either fails, or passes only because its generator avoids these cases.
   - Correction:
     - On a merge, re-partition the union of the participating `OPEN` windows' members deterministically: sort by `(received_at, source, event_id)`, then split at gap breaks, at `opened_at + maxWaitMs`, at `maxCount` and at the byte ceiling. Recompute each part's deadline, and let the earliest part keep the surviving `window_id`.
     - Alternatively, forbid any merge that would pass a bound, and state that session windows are order-dependent once a bound binds.
     - Define what happens to a duplicate `(source, event_id)` during a merge (keep one, `SUPPRESSED`).
     - Make the T042-15 generator cover `maxWaitMs`, `maxCount` and the byte ceiling.

2. Major — A merged-away window leaves `HELD` deliveries naming a deleted window
   - Where: lines 232-234 (the other windows "are deleted"), 214-215 and 980-981 (a delivery row is never updated), 446 (`event_deliveries.window_id`), 722-724 (CUJ 3: "Each update's event page shows its delivery `HELD` with the window's id, and the window names the run"), 1230-1236 (the API).
   - What (confirmed in the text):
     - In the two-replica interleaving of SC-042-001, events 1–100 open W2 while events 101–500 are in W1. A bridging element then merges W2 into W1.
     - The 100 deliveries recorded `window_id = W2`. Their `windowId` now points at no window, and `GET …/windows/W2` answers `404`.
     - With a foreign key, the delete would either fail (the apply retries until `DEAD`) or need `SET NULL`, which updates a delivery row.
   - Why: The delivery projection then disagrees with the window rows (category 4), and the CUJ 3 claim fails exactly in the case merging exists for. No test reads a delivery after a merge.
   - Correction:
     - Keep a merged window as a terminal `MERGED` row with `merged_into`, purged with its survivor, and resolve reads through it.
     - Or drop `window_id` from deliveries and find the window through `operator_window_members (source, event_id)`.
     - State that there is no foreign key from deliveries to windows.
     - Add to T042-29 a test that reads a delivery after a merge.

3. Major — The replay path sets a `flatMap` window `STARTED` after one index, which silently drops the rest of the fan-out
   - Where: lines 276-277 (on a replay, "a short transaction sets `STARTED` with the replayed run id under the same guard"), 348-355 (one transaction per index, `started_count`), 1116-1118 ("or advances `started_count` after `flatMap`", guard `state = 'SEALED'` only), 302-304 and 1081-1082 (the dispatcher and the pass both start a window sealed at once), 944 (one `run_id` column).
   - What (confirmed in the text):
     - Take a chain of `filter` and `flatMap`, whose windows are always sealed at once, with `fan_out` 3.
     - The dispatcher starts index 0 and commits `started_count = 1`. Index 1 is next.
     - The pass reads `started_count = 1` and calls the port with key `…/1` at the same time as the dispatcher. One of them creates the run.
     - The other gets `Replayed` and runs the short transaction "sets `STARTED` … under the same guard". `state = 'SEALED'` is still true, so the window becomes `STARTED` with `started_count = 2 < fan_out`. Index 2 never runs, and nothing is refused or audited.
     - The race is likely: the dispatcher walks up to 100 indexes one transaction each, while the pass picks `SEALED` windows as soon as `next_attempt_at` has passed.
   - Why: This breaks FR-042-020 ("one run per element") with no error. No test runs two starters on one index.
   - Correction:
     - Guard every `flatMap` write with `started_count = $k`: `record_window_start` advances it to `k+1` and must change exactly one row; the deletion of fan-out row `k` must delete exactly one row. On a replay of index `k`, the short transaction advances only `WHERE state = 'SEALED' AND started_count = $k`, and sets `STARTED` only when `k+1 = fan_out`.
     - Record each index's run id, for example in an index-to-run table, because the fan-out row is deleted.
     - Add a barrier test with two starters on index 1 of a fan-out of 3 that ends with three runs.

4. Major — The backlog hold waits on any old pending event, so one stuck event delays every matching window to `D + maxHoldMs`
   - Where: lines 252-253 (FR-042-017), 1095-1097 ("read once the oldest `PENDING` inbox event … and skip the windows that event is not past"), 724 (CUJ 3: "about thirty seconds after the last arrival").
   - Code: `crates/scheduler/src/events.rs:323-358` and `380-386` (any failing consumer retries the whole event), `280-299` (waiting failures stay `PENDING`); `crates/domain/src/events.rs:335` (`GIVE_UP_AFTER` 24 h) and `341-344` (30-45 s back-off).
   - What (confirmed in code):
     - An event E received at 09:00 is already `HELD` by chained trigger T. It stays `PENDING` because another trigger's run start waits for a quota, retried every 30-45 s for up to 24 hours.
     - E is older than every later deadline, so every window of every chain matching E's source and type is held until `D + 180 s`.
     - So CUJ 3's run starts about 3.5 minutes after the burst instead of about 30 s, and every seal counts `sealed_past_hold`, which fires `OperatorSealedPastHold` continuously.
     - Events that could never join the window (received before `opened_at − gapMs`, or matched to a `DRAINING` chain) hold it too.
   - Why: The hold exists to cover events that can still join, and this one cannot. The latency promised in CUJ 3 and the alert's meaning both fail in ordinary operation. No test has a stuck event that was already applied.
   - Correction:
     - Hold only on `PENDING` events with no `event_deliveries` row for this trigger, received in the range that can still join: `[opened_at − gapMs, D]` for a session, the span for tumbling, `≤ D` for count.
     - Read them with `LIMIT 1` over the index with `NOT EXISTS`.
     - Add a test with a stuck event that was already applied.

5. Major — A window-opening apply takes the held-bytes counter before the content usage row at commit, so it can deadlock with a start of the same tenant
   - Where: lines 1004-1015 (commit order "by name order": `zz_operator_held_flush` on members, windows and fan-out rows, "after that table's `*_quota_flush` trigger"), 1060-1064 (the apply inserts the window, then the member), T042-13 at 1554; `docs/adr/ADR-0050-…md` line 103; `crates/infra-postgres/migrations/20261024000000_durable_quotas.up.sql:296-371` (the content flush processes every delta of the transaction when it first fires).
   - What (plausible; it depends on whether `operator_windows` gets a content flush trigger, which section 4 does not require because the table has no content column):
     - Deferred triggers fire at commit in queue order. In an apply that opens a window, the first queued event is the `operator_windows` insert. If that table has only `zz_operator_held_flush`, that trigger fires first and locks the tenant's `operator_held_usage` row. It flushes every held delta of the transaction, including the member's that was queued later.
     - Only afterwards does `operator_window_members_quota_flush` lock `tenant_retained_usage`.
     - A start of the same tenant queues payload inserts first, so it locks content, then held. The two transactions wait on each other.
     - A deadlock is a counted store error, so the dispatcher retries the event, counted, until `DEAD`.
     - The T042-13 barrier test catches this only if its apply opens a window.
   - Why: The ADR-0050 deviation rests on "content first", and an ordinary window-opening apply breaks it.
   - Correction:
     - Have `operator_held_flush()` itself lock the tenant's `tenant_retained_usage` row (or run the content flush) before the counter, so the order no longer depends on trigger names or the order of table events.
     - Or require a `*_quota_flush` trigger on all three tables, with a catalog test.
     - Name a window-opening apply in the barrier test.

6. Major — Trigger removal has no transaction boundary or lock order, and the stated building blocks deadlock with an apply
   - Where: lines 388-390 (FR-042-022), 1140-1141 (a discard locks "the same way" as a flush: chain `FOR SHARE`, key, window), 1130-1134 (the purge takes the chain `FOR SHARE`, then "`FOR UPDATE` and deleted"); `crates/infra-postgres/src/definitions.rs:84-92` (`register_triggers` deletes the trigger row inside the publication transaction).
   - What (plausible; the text does not fix the order):
     - If the publication transaction discards `OPEN` windows through the discard path and then sets the chain `REMOVED`, a deadlock follows. The publication holds the chain `FOR SHARE` and key K, then waits to update the chain. Meanwhile an apply holds the chain `FOR SHARE` and waits for key K.
     - The purge has the same upgrade from `FOR SHARE` to `FOR UPDATE` in one transaction. Two passes on one shard after a lapsed lease deadlock on it.
   - Why: Category 1. Architecture approval #3 raised who does the discard; this finding is about lock order, which neither the reviews nor T042-59 test.
   - Correction:
     - The publication or `switch` updates the chain row to `REMOVED` first and touches no window. The pass discards in bounded batches (chain, key, window).
     - The purge deletes an empty chain in a transaction of its own that takes the chain `FOR UPDATE` first and re-checks that nothing is left.
     - Add both to the slot test.

7. Nit — The statement budget still leaves out the savepoint
   - Where: lines 1070 ("rolls back to a savepoint taken before step 3"), 1078-1080 ("seven, at most eight"), 1429-1430 (NFR-042-001: "at most 8").
   - What (confirmed in the text): every apply that may add held bytes needs a `SAVEPOINT` statement before step 3. With it the common case is the savepoint plus seven statements plus the evaluation read: nine. Task 042-33 then fails, or the savepoint gets dropped.
   - Correction: count the savepoint and set the budget to 9, or explain why a rollback to the transaction start suffices.

8. Nit — The `KEY_LIMIT` audit can disagree with the refusals it counts
   - Where: lines 361-364, 947, 1128-1129.
   - What (plausible):
     - Undercount: the pass reads the minute's count (37), writes the audit, then its `DELETE` waits on an apply that started at 10:00:59.9 and upserted 38. The delete then removes the row, so the audit says 37 while `operator_key_limit_total` says 38.
     - Double record: if the upsert lands after the delete, it re-creates the row for that minute, and a second audit record is written for the same minute.
   - Correction: write the audit from `DELETE … RETURNING count` in the same transaction. Audit only minutes older than the statement timeout, or allow and test a second record per minute.

9. Nit — `next_attempt_at` is shared by the seal and the start, and nothing resets it at the seal
   - Where: lines 944, 950-951, 1091-1092 (seal: "null or at or before `now()`"), 1112 (start: "has passed"), 1125-1126, 289.
   - What:
     - A window sealed after a `quota_wait` keeps its seal back-off of up to 5 minutes, which delays its start.
     - A `NULL` fails "has passed", so a freshly sealed window can sit unstarted until `START_EXPIRED`.
     - A value of `now()` makes the pass race the dispatcher on every window sealed at once, which is what triggers finding 3.
   - Correction: have every seal set `next_attempt_at` explicitly (the dispatcher's own windows a short grace ahead) and reset `attempts`. Test it.

10. Nit — `seal_expires_at` is defined against a deadline that moves
    - Where: lines 290, 944, 1111.
    - What: if `seal_expires_at` is set when the window opens, a session with `gapMs` 1 s and `maxWaitMs` 24 h has its deadline near 24 h while `seal_expires_at` is about 24 h + 1 s. A hold of up to `maxHoldMs` then refuses it `SEAL_EXPIRED` instead of sealing it past the hold.
    - Correction: set it to `max_close_at + 24 h` (a fixed value), or recompute it with every deadline change. Say which.

11. Nit — The backlog read is not defined for wildcard triggers
    - Where: lines 952 and 1095-1096; `crates/domain/src/triggers.rs:136-139` and `171-193` (an `event` trigger may have no source; `webhook` and `message` triggers may have no type); `crates/infra-postgres/src/dispatch.rs:286-288`.
    - What: "for each (tenant, namespace, source, type) the chain matches" cannot be listed when one side is `NULL`. A wildcard on the leading columns turns the index read into a scan of every pending event.
    - Correction: state the wildcard query (prefix `(tenant, namespace, source)` with a minimum over all types, or a second partial index), and include it in the budget.

12. Nit — Every element after an `onFull` seal counts as "late"
    - Where: lines 256-259 (late = "applied after the window that covered it was sealed"), 244-245; CUJ 4 at 726-732.
    - What: with tumbling windows, `maxCount` 1,000 and 2,400 readings, elements 1,001 and later arrive after the window covering their span sealed on `onFull`. By the definition they are late, so the second and third runs carry `late: true`, contrary to CUJ 4, and the late counter is inflated.
    - Correction: define late as a window sealed by its deadline (or a flush) having covered the element, not one sealed by a ceiling.

13. Nit — The version a window starts on, and the fate of a removed chain, are undefined when identities persist
    - Where: lines 269-270 and 385-388, 940 ("the version that registered it"), 944 (`version_id`); `crates/application/src/dispatch.rs:36-38` (trigger ids stay the same across versions); `crates/infra-postgres/src/definitions.rs:84-86`.
    - What:
      - A chain kept across a republication with the same digest has windows opened after v2 is active. The text does not say whether they record v1 (the version that registered the chain) or v2 (the trigger's version), so a deletion of v1 refuses live windows with `VERSION_UNAVAILABLE`.
      - If a trigger is removed and later re-added with the same id and digest, a still-stored `REMOVED` chain collides with the new registration's identity.
      - Any foreign key from `operator_chains` to `workflow_triggers` would cascade the chain away when the trigger row is deleted.
    - Correction:
      - Take a window's `version_id` from the trigger row at the apply that opens it.
      - Define re-adding a trigger whose chain is `REMOVED`: a new `chain_id`, with the identity unique only among `ACTIVE` and `DRAINING` chains.
      - State that there is no foreign key to `workflow_triggers`.

I could not run git, so I read the worktree copy of the design, which you said matches `4b0248c7`.
