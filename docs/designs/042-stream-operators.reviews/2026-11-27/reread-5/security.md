# Fifth security re-read of design 042 (2026-11-27)

Doc: docs/designs/042-stream-operators.md
Reviewed-commit: f3622a5a54e61b44a6baaf1a10a9417b07673920
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output as returned, with this header added and machine paths made relative. It was asked as the
approval read; it found a Major, so it is filed as a re-read.

Counts: Blocker 0, Major 1, Nit 5

## Previous re-read

1. **Partly fixed.** A successful `flatMap` seal now nulls its members' values. The room check is on the net delta, the hold-and-wait is stated, and an alert and a test are added (309-320, 945-946, 1066-1071, 1221-1223, T042-24 at 1597-1599). Two problems remain: the net delta is still positive for every `select`, and the stated end of the hold is wrong. Repeated as finding 1.
2. **Fixed.** At step 5 only the held-bytes part is final, and a content shortfall becomes the dispatcher's wait (343-347, 1035-1038). The case of a `count` or `onFull` seal at the apply that lacks room only for the fan-out is handled (321-323, 1039-1040). The room function now includes `content_reserved_bytes` (989, 1034). One ceiling gap in the new "left due" rule is finding 6.
3. **Fixed.** The `seal_expires_at` column and its rule are in (914, 290, 1073-1074). "Except `START_EXPIRED` or `SEAL_EXPIRED`" is now written (943-944). `next_attempt_at` is in the seal's selection (1057-1058). The retry resets the bound (293-295, 1103-1104).
4. **Fixed.** `operator_held_room` is `VOLATILE`, takes the advisory lock itself and then reads, and a barrier test is added (983-992, 1031-1032). The lock key matches `lock` in `crates/infra-postgres/src/quotas.rs:37-41`. NFR-042-002 was not updated to match; see finding 3.
5. **Partly fixed.** The three `ALTER ROLE` lines and a privilege-audit assertion are added (1018-1024, T042-40 at 1650-1656). The assertion, however, checks the role the test harness creates, not the shipped script. Repeated as finding 2.

## Findings

1. Major — One namespace can still lock the tenant's held-bytes pool for about 8 to 9 days, by two routes, and the doc says the hold ends earlier
   - Where: design 309-313 and 317-320 (the claim that a `select` that does not enlarge its envelope never waits, and that the hold lasts "until … `seal_expires_at`"); 290-291 and 943-944 (values of an expired window kept until the purge); 356 and 914 (purge 7 days after `final_at`); 283 (`CONCURRENCY_QUEUE_FULL`, `FLOW_CONTROL_LIMIT` and quota refusals keep a window `SEALED`); 1597-1599 (T042-24 expects an identity `select` never to wait); `crates/domain/src/identity.rs:630-661` (a Developer has `definition:publish` and `event:publish`).
   - What: Checked in the text.
     - **Route (a): `flatMap`.** What is held for a member is only its `value` (915). The envelope that `select` sees adds the summary and, per member, `{source, eventId, receivedAt}`. Any list adds at least its brackets.
       - So `element.events`, `[element]` and even `[element.event.value]` all have a net delta above 0.
       - A Developer in namespace A fills the pool to within a few bytes with `OPEN` `flatMap` windows (event sizes are under their control). Every seal then gets `quota_wait`.
       - At `seal_expires_at` each window becomes `SEAL_EXPIRED` and keeps its values for 7 more days.
       - The statement at 317-320 is therefore wrong on both scope (it is not only an enlarging `select`) and end (it is not `seal_expires_at`). The T042-24 test at 1598-1599 cannot pass as written.
     - **Route (b): no `flatMap` at all.** The same Developer publishes a workflow with a `concurrency` key whose first run waits forever on a signal, so every later start gets `CONCURRENCY_QUEUE_FULL`. Exhausting their own namespace's run quota works too.
       - Every window of their chain stays `SEALED` for 24 hours, becomes `START_EXPIRED`, and keeps its values until the purge 7 days later.
       - Reread-4 assumed this path was capped at `maxWaitMs`. It is not.
     - **Effect of either route.** For about 8 to 9 days, every element in any namespace of the tenant that adds held bytes ends `FAILED`/`QUOTA_EXCEEDED`, which is final.
     - **Remedies.** An operator can only cancel the blocking run, or discard the windows one at a time once they are `REFUSED`. During the 24 `SEALED` hours they cannot be discarded at all.
     - **Alerts.** None covers `START_EXPIRED` or held bytes near the quota.
   - Why: This is a cross-namespace denial that one publisher can cause and that outlasts its cause, the harm reread-3 finding 2 and reread-4 finding 1 were meant to remove. The doc also misstates its extent.
   - Correction:
     - Keep an expired window's held values only for a stated retry grace, for example 24 hours, not until the purge. After that, delete them and release the bytes; the window row can still live 7 days. State in 290-291 and 943-944 that a retry after the grace finds no values.
     - For `flatMap`, either pre-charge each member at its member-envelope size plus the summary, so that a `select` no larger than the envelope really nets 0 or less, or reword 312-313 and fix the T042-24 test to match the real rule.
     - Restate 317-320 with the true duration.
     - Add an alert on `operator_windows_total{outcome="expired"}`, and on held bytes above, say, 90% of `operator_held_bytes`.

2. Nit — The log-settings check tests the harness's role, not the shipped script
   - Where: design 1018-1024, T042-40 at 1650-1656; `tests/support/mod.rs:470-482` (the harness creates `sagat_app` itself); precedent `tests/contract/scalability-ha/handover_chart.rs:266-272`.
   - What: Verified. With `SAGAT_TEST_AS_RUNTIME_ROLE=1`, the role comes from `ensure_runtime_role`, not from `infra/docker/postgres-init/01-runtime-role.sh`. The `SHOW` assertion only passes once the harness sets the three values, and then it passes whatever the shipped script and the Helm notes say.
   - Correction: Have the harness apply the same three `ALTER ROLE` lines. Add a contract test that reads `01-runtime-role.sh` and `infra/helm/templates/NOTES.txt` and asserts the lines are there, as `handover_chart.rs` already does for the init script.

3. Nit — NFR-042-002 still says `operator_held_room` reads "without a lock"
   - Where: design 1394, against 984-986 and 1031-1032.
   - What: Checked in the text. The function now takes the tenant's advisory lock. An implementer or auditor who follows the NFR could drop the lock, which reopens reread-4 finding 4.
   - Correction: Write "takes the tenant's held-bytes advisory lock and no row lock".

4. Nit — FR-042-024 says no personal datum reaches a span or an audit record, but section 9 puts some there
   - Where: design 378-381 and 394-395, against 1224-1225 (`operator.window.start` carries the window id and member count) and 1202-1204 (`operator.window_refused` and the others name windows and their codes).
   - What: Checked in the text. For a value-dependent chain, FR-042-024 classes a window's id, count and outcome as personal data. Read literally, line 395 cannot be built.
   - Correction: Either narrow 394-395 to key and `distinct` values and their digests, and name who can read audit records and traces (`audit:read` is Admin-only), or drop the member-count attribute for value-dependent chains.

5. Nit — Each index start may read the whole fan-out list (suspicion)
   - Where: design 271-272 and 1077 (one statement reads "the held values" per start), 324 (one transaction per index), 300-306 (up to 100 entries of 256 KiB each).
   - What: Suspected from the text. If each index's start reads all of `fan_out_values` (up to about 25 MiB), one window costs about 2.5 GiB of reads. At 1 GiB of held bytes that is about 40 such windows at once, which any Developer can cause against the shared database.
   - Correction: Store the fan-out as one row per index, read and delete one row per index start. This also releases held bytes as the fan-out progresses, which shortens finding 1's hold.

6. Nit — The seal's `select` and up to 100 validator runs have no stated budget, and a window "left due" at its ceiling has no stated rule for the next element
   - Where: design 300-306 and 1066-1069 (evaluated inside the fenced seal transaction while the key row is held `FOR UPDATE`), against 733-734 (`trigger_chain` runs in a blocking task with a 1-second budget); 321-323 and 1039-1040 (a full window is left `OPEN` and due); 231 and 329-332 (a session window accepts elements past its deadline; `maxCount` and the byte ceiling).
   - What: Checked in the text.
     - The seal runs CPU-bound work, up to 100 schema validations of 256 KiB each, inside the shared pass, holding a key lock, under no budget.
     - A window at `maxCount` or its byte ceiling that stays `OPEN` is not stated to refuse new members. The ceilings of FR-042-021 then no longer bound it, and only the pool does.
   - Correction:
     - Run the seal's evaluation through the port in a blocking task under the 1-second budget. Count an overrun as a back-off, as G042-009 does.
     - State that a window left due at a ceiling accepts no further member; the next element opens a new window, or is dropped under `onFull: drop`.

No finding in the other classes I checked:
- **Tenant boundary:** uniform `404` and the probe audit on every new path. The definer function's tenant argument exposes nothing `sagat_app` cannot already `SELECT`.
- **Permissions:** the Operator role has `event:publish` and `run:read`, so the `202`/`409` answer of the delivery retry tells them nothing beyond Gap 042-018.
- **Metric labels:** all closed values.
- **Definer functions:** exactly five, each with a pinned `search_path` and revoked from `PUBLIC`.
- **Commit and lock order:** the advisory lock comes before the content and held flushes, with no cycle.
- **Gaps 042-017 and 042-018:** stated the same way in the doc and in `specs/042-stream-operators/research.md` (lines 128-129).
- **Injection, regular expressions, SSRF and egress:** nothing new.
