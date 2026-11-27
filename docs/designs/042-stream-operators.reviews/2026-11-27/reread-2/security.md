# Second security re-read of design 042 (2026-11-27)

Doc: docs/designs/042-stream-operators.md
Reviewed-commit: eff3719ae363b1b4302b4c93e20dc8561b2bb01f
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 2, Nit 4

## Previous re-read

1. Partly fixed. `keyId`, the member list, the delivery `windowId` and the reasons now need `reveal=pii` (349-363, 384-389, 1080-1090, T042-17 at 1420-1426, T042-39, T042-53). The outcomes, error codes, counts and timestamps that stay visible are still a key oracle. Repeated as finding 1.
2. Partly fixed. Only applies, delivery retries and publications are gated, and the drain check now waits for held values, keys and marks (426-430, 1141-1152, 1153-1159, T042-54 at 1566-1574). But sections 1 and 10 still contradict this, and final windows outlive the drain. Repeated as finding 3.
3. Fixed. The counter is named (906-911). Step 5 checks both quotas (935-941). The commit-time refusal is stated as the dispatcher's wait (322-325). The mechanism for the tenant sums is in the text (326-330). New problems in this area are findings 2 and 4.
4. Fixed. The digest input is length-prefixed tenant, chain and canonical bytes (871-876), matching `start_key` (`crates/application/src/dispatch.rs:181-192`). The threat model is corrected (877-883) and Gap 042-017 states the residual (556-561). The storage class is named (902-906). ADR-0155 decision 6 agrees.
5. Fixed. The retry now uses `FOR NO KEY UPDATE`, answers 404 for a missing delivery and is refused without the capability (411-421, T042-18 at 1427-1436). I checked the code: each trigger start is its own transaction, and `finish_event` and `retry_event` update the inbox row in separate transactions (`crates/scheduler/src/events.rs:261-358`). So the retry's lock cannot form a cycle with a re-dispatch.
6. Fixed. The engine is the starter of every window's run; the caller is recorded only in the audit and in the "requested by" line (342-347, 989-997, 1092-1095, T042-57).
7. Fixed. Task 015-48 (`docs/designs/015-opentelemetry-and-openbao.md:1898-1902`) and OD-2026-11-27-448 (`docs/owner-decisions.md:56`) handle the stale key text. The `FOR KEY SHARE` lock is credited to design 053 and `uses` is cited (424-426, 433-434). The order point on Task 053-5 is there (1327-1330), and design 053 does contain that change (`docs/designs/053-rolling-upgrades.md:287-288,304-337`). Form (b)'s table locks are separated out (1156-1157).

## Findings

1. Major — Without the reveal grant, outcomes, error codes, counts and window timestamps still tell a guessed key or `distinct` value from a miss
   Where: design 361-362 (the claim), 373-376 (outcomes `HELD`, `SUPPRESSED`, `FAILED`/`KEY_LIMIT`), 1079-1080 (`keyCount`, `openWindows`, window `count`, `openedAt`, `deadline`, all without reveal), 1086 (`outcome`, `errorCode` without reveal), 229 and 236 (`opened_at` is the first member's `received_at`), T042-17 at 1420-1426; `crates/domain/src/identity.rs:630-700`; `crates/application/src/event_payloads.rs:98,116,597-604`.
   What: Checked against the code for roles and event reads; the rest is from the design text. A Developer has `event:publish` and `event:read` and may lack the namespace reveal grant.
   - On a chain `[distinct {by: "data.email", horizonMs}]`, the Developer publishes an event with a guessed email and reads its delivery. `HELD` means the value was not seen within the horizon; `SUPPRESSED` means it was. Masking the reason as `operator` changes nothing, because the outcome itself differs. `skip {count: 1}` gives the same oracle for keys.
   - On a keyed window chain:
     - A guessed key that is new raises `keyCount` on `GET /operator-chains` by one.
     - A guessed key that exists raises the `count` of the target's open window and moves its `deadline`.
     - At `maxKeys`, a new key gives `FAILED` with `errorCode: KEY_LIMIT` instead of `HELD`.
   - A Viewer, with no publish right, can link events too. A session or count window's `openedAt` equals its first member's `received_at` to the microsecond. A session window's `deadline` minus `gapMs` (or `quietMs`, both in the published definition) is its last member's `received_at`. Event reads show `receivedAt`. So the Viewer can name the first and last event of each window, which share a key.
   - As a result, T042-17's test ("learns nothing that tells a match from a miss") cannot pass.
   Why: FR-042-024 itself counts this linkage as personal data. Constitution V and feature 016 say callers without the permission get no oracle and personal-data access is audited. The sentence at 361-362 is false.
   Correction: Without `reveal=pii`, either:
   - show window instants and `count` only under reveal, and show one outcome value for `HELD`, `SUPPRESSED` and `FAILED`/`KEY_LIMIT`; or
   - (smaller) replace 361-362 with the residual for publishers and Viewers, narrow T042-17 to what holds, and record an owner row that accepts it. The same row should accept Gap 042-017: line 561 addresses it "for the owner" while line 605 says nothing is needed from the owner.

2. Major — `flatMap` values written by a deadline seal or a flush bypass the held-bytes quota, and a commit-time refusal of the seal has no outcome
   Where: design 296-300 and 966-967 (the seal evaluates `select` and records `fan_out_values`), 989-991 (the flush seals), 935-941 (the held-bytes check exists only in the apply), 906-911 (the trigger counts `fan_out_values`); `crates/infra-expr/src/limits.rs:25-28`; `crates/infra-postgres/migrations/20261024000000_durable_quotas.up.sql:296-366`.
   What: Checked in the design and the evaluator limits.
   - Only the apply's step 5 takes the tenant lock and compares against `operator_held_bytes`. A window sealed by the pass or by a flush writes `fan_out_values` with no check. That list can be up to `MAX_COPIED_BYTES` (about 516 KiB), for example `select: "[element, element]"` over a 256 KiB envelope.
   - The apply checked at most 256 KiB for that window, so a tenant using `flatMap` holds up to about three times its quota. Seals and applies also race, because the seal never takes the advisory lock.
   - When the tenant content quota refuses the seal at COMMIT, nothing says what happens. The window stays `OPEN` past `D + maxHoldMs`, and the pass re-evaluates `select` and up to 100 validator runs every tick.
   - Each index input is not checked against `MAX_PAYLOAD_BYTES` at the seal, and a start refused for size is not a row of FR-042-019's outcome table.
   Why: Constitution V lists quota bypass as release-blocking. FR-042-021 says "Ceilings each have one outcome."
   Correction:
   - In the seal and flush transactions, take the held-bytes lock (slot 8b, last) and check `fan_out_values` as step 5 does. Over the quota, refuse the window `QUOTA_EXCEEDED` (final, values deleted).
   - State that a commit-time content refusal leaves the window to be sealed again, and count it.
   - Check each index input against `MAX_PAYLOAD_BYTES` at the seal, refusing the window with its code.

3. Nit — Repeats previous finding 2: two passages still turn the pass off with the switch, and final windows survive the drain
   Where: design 701-703 and 1133-1135, against 426-430 and 1145-1147; drain check 1141-1144; 859, 882-883, 1162.
   What: Checked in the text.
   - Section 1 and section 10 still say `SAGAT_TRIGGER_OPERATORS_ENABLED` "turns on the dispatcher's apply and the `operator_windows` pass." An implementer who follows them stops the purge when the switch is off.
   - The drain check does not require final windows to be gone. `STARTED`, `REFUSED` and `DISCARDED` windows keep `key_digest` and `requested_by`, and their member rows keep `(source, event_id)`, for 7 days after `final_at`. After phase 2, the operator may roll back to a binary without the code (1162). That binary never purges these rows.
   Correction: Make 701-703 and 1133-1135 say the switch turns on the apply and the capability listing only. Add "no `operator_windows` row" to the drain check, or have the pass purge final windows at once while the capability is `draining`.

4. Nit — How `operator_held_usage_delta()` maintains the counter is not stated, and the in-place reading breaks the stated lock order
   Where: design 906-911, 935-941, 974-979, 1066-1068; T042-13 at 1392-1393; the slot test of T042-22; for the existing pattern, `crates/infra-postgres/migrations/20261024000000_durable_quotas.up.sql:177-187,296-371`.
   What: "Adds the change of held bytes" can be read two ways, and neither is described in full.
   - Updated in place: the tenant's counter row is locked from the member insert at step 4 until commit. That is before the slot-8b advisory lock, which the design says is taken last. The row is also locked in every start (`record_window_start` deletes values in the run's transaction), discard, refusal and purge. This contradicts "the window row is the only operator row the start touches" and ADR-0050's slot list, and "committed row plus this transaction's own" then counts the transaction's own bytes twice.
   - Recorded as a per-transaction delta: this needs a commit-time flush trigger, which is not named.
   Correction: Choose the delta form explicitly. Accumulate a per-transaction delta, as `sagat_quota_add_delta` does, and flush it under the counter row's lock at commit. Add the counter to ADR-0050's slot table and to the start's slot test.

5. Nit — A race on a new key raises a unique violation that writes the key digest to the PostgreSQL server log
   Where: design 926-928 (step 3: read the key row `FOR UPDATE`, and when it is absent reserve a slot and insert), 876 (no digest goes to a log).
   What: A suspicion from PostgreSQL's documented behaviour; I did not run it. Two applies of one new key both find no row, because `FOR UPDATE` locks nothing. The second waits on the usage row, then reserves and inserts after the first commits, and fails with 23505. At default settings the server log gets `DETAIL: Key (…, key_digest)=(…, \x…) already exists.` The event also fails transiently and uses up an attempt. SC-042-001's two-replica interleaving makes this case.
   Correction: Insert the key row with `ON CONFLICT DO NOTHING RETURNING`. When nothing is inserted, roll back to a savepoint taken before the reservation and re-read the key row `FOR UPDATE`. Test it with a barrier.

6. Nit — Three more channels link events by key outside the reveal gate
   Where: design 390-393 (the summary carries `keyId`; `emit: all` lists members), 1112-1114 (span links to up to 32 member traces), 1082-1084 (actions answer "with the window"); `crates/domain/src/classification.rs:41,219-230`.
   What:
   - Run input: unannotated fields are `pii` (checked), so this is safe by default. But an author who annotates `window` or the input `internal` exposes `keyId` and member lists to every Developer and Operator. They need no reveal, and no `payload.pii_revealed` is written. I did not trace the run-input projection path itself.
   - Span links put window membership in the trace backend, which FR-042-024 counts as personal data.
   - The flush, discard and retry responses are not said to use the shape without reveal.
   Correction: Treat `window.keyId`, `event` and `events` as `pii` whatever the schema declares (as `DATABASE_RESULT` does), or refuse such a schema with `OPERATOR_INPUT_SCHEMA`. Drop the member span links, or state them as a residual. Say the action responses use the shape without reveal.

Classes I looked at and found nothing new in:
- Cross-tenant 404 on chains, windows and the retry.
- Metric labels.
- The definer functions' pinned `search_path` and the counter grants.
- Expression errors: computed keys are never named (`crates/infra-expr/src/eval.rs:444-457`).
- Delivery-retry locking (verified above).
- The audit-only `requested_by`.
- Injection, regular expressions, SSRF and egress.
