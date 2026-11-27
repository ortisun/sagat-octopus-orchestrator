# Security re-read of design 042, fourth revision (2026-11-27)

Doc: docs/designs/042-stream-operators.md
Reviewed-commit: 3d2b53e2fa3a7f44f7cfd0d8cd22ed18b0a8fc65
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 2, Nit 5

## Previous review

1. Fixed. The counters are off the share-locked row: lines 208-210, 312-319, 827, 883-886 and 1142, the barrier test at 1346-1347 and the fitness check at 1520-1521.
2. Fixed. Both waits end at `D + maxHoldMs` (252-256, 970). The lag now counts windows past every hold (950-954), the alerts are at 1056-1058, the risk row at 1144, and ADR-0153 has a revision at 72-78.
3. Fixed. The retry accepts only a `FAILED` delivery, is a targeted apply that never returns the event to `PENDING`, needs `run:start` + `run:operate`, and answers the uniform 404 with the probe audit (397-403, 337-338, 1036, 1356-1361). New problems in this path are findings 5 and 6.
4. Fixed. `keyIdleMs` now applies to every keyed chain (221-222, 702), keys are purged when they have no `OPEN` or `SEALED` window (326-327), and no key text is stored (343-345, 627-628, 1349-1351).
5. Fixed as asked: no key text exists, and a key is revealed only through the audited event read (343-350, 559). The linkage that remains is finding 1.
6. Fixed. Publication and `switch` take a form (a) fence under `FOR KEY SHARE` (409-411, 869-872, 1086-1091), with a held-lock test in both orders (1493-1494).
7. Fixed. Flush and discard have a lock order (263-264, 942-946), the start's update must hit exactly one row (275-276, 926-927), the `requested_by` column exists (830), and the apply joins only `OPEN` windows it has locked (887-889).
8. Fixed. The keyed hash, `pgcrypto` and the HMAC definer function are gone (842-849, 1196-1198). The two fences pin `pg_catalog, pg_temp` and schema-qualify every name (869-872). The threat model is stated, but part of it is wrong: finding 4.
9. Fixed. `trigger_chain` parses the event once, caches compiled expressions per chain digest and runs in a blocking task (646-651).
10. Fixed. Final windows are purged 7 days after they end, `START_EXPIRED` values included (288, 328, 830, 572).
11. Fixed. A removed trigger's `SEALED` windows are refused `TRIGGER_REMOVED`, and the remaining race is stated (287, 292-293, 333-335, 974, 1523-1526).
12. Partly fixed. `KEY_LIMIT` now has a usage row and a durable per-minute row (313-316, 832, 936-937), and the held-bytes check has a lock (892-894). But how held bytes are measured is not named, and the mechanism for the tenant sums is only in research D042-169, not in the design (322-324). Repeated as finding 3.
13. Fixed. The citations no longer carry line numbers (114-117).

## Findings

1. Major — The key id, window membership and the `distinct` reason let a caller without the reveal grant compare and confirm key values
   Where: design 346-349 (FR-042-024), 362 (reason `distinct`), 1031-1032 and 1037 (`keyId`, members, delivery `windowId`), 559; `crates/application/src/event_payloads.rs:598-602`; `crates/domain/src/identity.rs:630-700`.
   What: Checked in the code. Revealing personal data needs `payload:read-internal` plus the namespace reveal grant. A Viewer has `event:read` only; Developer and Operator also have `event:publish`.
   - A Viewer lists a chain's windows and groups their events by `keyId`. For a key such as `data.email`, that tells them which events share the same value, although an event read shows them only the structure.
   - A Developer without the reveal grant can publish to a trigger that accepts `api`-sourced events. They publish an event with `data.email = "alice@…"` and read its delivery. If its `windowId` or `keyId` matches the target event's, the guessed value is confirmed.
   - Likewise, a `SUPPRESSED` delivery with reason `distinct` confirms that a guessed `by` value was seen within the horizon.
   None of this writes `payload.pii_revealed` or `PayloadRevealDenied`. The reason given for OD-2026-11-27-424 does not cover this.
   Why: Constitution V and VI (no oracle for callers without the permission; access to personal data is audited); the feature 016 reveal model.
   Correction: Return `keyId`, delivery `windowId`, window membership and the `distinct` reason only on `reveal=pii`, through the same projector and the same two audit actions, with a test for each. If that is too costly for the API, the owner must accept this linkage, and the publish-and-confirm case, explicitly in a decision row.

2. Major — Sealing, starting and purging stop whenever an engine does not use the capability, so a deactivation cannot finish and held personal data is never deleted
   Where: design 406-408 ("its pass does nothing"), 672-674, 1089-1100, 326-335 (removed windows are refused "by their start"); `docs/designs/053-rolling-upgrades.md:337` (a `draining` row makes `ServingGate::uses` false).
   What: Checked in the documents. Phase 1 makes `uses` false on every engine, so the `operator_windows` pass does nothing.
   - The `SEALED` windows of a removed trigger stay `SEALED`, and the `OPEN` windows of a replaced version never seal. The drain check ("no window `OPEN` or `SEALED`") therefore never passes, and phase 2 can never run.
   - If the operator instead turns `SAGAT_TRIGGER_OPERATORS_ENABLED` off, nothing purges what is left: `START_EXPIRED` held values, key and mark digests, and member rows stay indefinitely. That contradicts the 7 days promised by FR-042-022/024 and OD-2026-11-27-433.
   Why: Retention of personal data (FR-042-024); the design 053 drain contract.
   Correction: Gate only new applies and new publications on `uses`. Keep sealing, starting, the `TRIGGER_REMOVED` refusal and the purge running in any binary that has the code. Extend the drain check to require no held values and no key or mark rows. Test a deactivation that has a `SEALED` window of a removed trigger and a `START_EXPIRED` window.

3. Nit — Repeats previous finding 12: the held-bytes measurement is not named, and the tenant content quota is enforced at commit
   Where: design 319-322, 322-324, 866-868, 892-894, 1215; `crates/infra-postgres/migrations/20261024000000_durable_quotas.up.sql:177-187,296-366`; `crates/infra-postgres/src/content_usage.rs:268-270`; `crates/scheduler/src/events.rs:283-299,1045-1049`.
   What: Checked in the code.
   - The existing delta triggers feed only the content, reserved and log counters, and only at commit. The runtime role has no write access to the counters. So `operator_held_bytes` needs either a new counter (a change to definer functions, which NFR-042-002 does not mention) or a per-apply sum over the tenant's held rows under a per-tenant lock, on the dispatcher every tenant shares.
   - A held value also counts against `tenant_content_bytes`, which `sagat_quota_flush_deltas` enforces at COMMIT, after the savepoint is gone. That error is `QuotaExceeded`, which the dispatcher treats as "waiting for room": the event stays pending for up to a day, every trigger is re-dispatched on each attempt, and its pending state keeps windows held. So "final `QUOTA_EXCEEDED`" is not true there.
   Correction: Name the counter and who maintains it. At step 5, check both the held-bytes counter and committed tenant content usage plus this transaction's pending delta. State that a refusal at commit is the dispatcher's wait. Move the tenant-sum mechanism from D042-169 into the design text.

4. Nit — The digests have no tenant or chain separation, and the stated threat model does not hold over time
   Where: design 342-346, 842-849, 830, 856-857; `crates/infra-postgres/storage-classification.json:10-15`; for comparison, `crates/application/src/dispatch.rs:181-199`.
   What: Checked.
   - A `distinct` mark lives up to 7 days, and a window's `key_digest` lives 7 days past `final_at`. The inbox row of a suppressed or filtered event is gone after its dedupe TTL, and such an element never reaches a run input. From then on a database or backup reader holds only the digest, and an email or phone number falls to a dictionary.
   - Plain SHA-256 gives the same digest for the same value in two tenants. That allows cross-tenant linkage and lets one precomputed table serve every tenant.
   - "Classified as personal data" has no storage class to map to: the only classes are content, log, step_log and protected_system.
   Correction: Hash length-prefixed `tenant_id ‖ chain_id ‖ canonical bytes`, as `start_key` already does. Correct the threat-model sentence, and name the storage class of each digest column.

5. Nit — The delivery retry can deadlock with a re-dispatch, answers 409 for a delivery that does not exist, and is not gated on the capability
   Where: design 397-403, 877-896; `crates/scheduler/src/events.rs:317-358`; `crates/infra-postgres/migrations/20261012000000_events.up.sql:95`.
   What: Checked in the code.
   - Scenario: event E has a `FAILED` delivery to trigger T1, and its delivery to T2 failed transiently, so E is re-dispatched to every trigger. The dispatcher holds T1's key row `FOR UPDATE`, then its delivery insert takes `FOR KEY SHARE` on E's inbox row through the foreign key. Meanwhile an operator's retry holds E's inbox row `FOR UPDATE` and then asks for T1's key row. That is a deadlock, and the abort lands on the serial dispatcher.
   - An event in scope with no delivery for the named trigger answers `409 DELIVERY_NOT_RETRYABLE`, not 404.
   - Nothing says the control plane checks `ServingGate::uses` before applying. During phase 1, a retry can open new windows.
   Correction: Lock the inbox row `FOR NO KEY UPDATE`: it serialises retries and blocks the purge, and does not conflict with the foreign key's `KEY SHARE`. Answer 404 with the probe audit when the delivery row is missing. Refuse with the switch's typed error when the process does not use the capability.

6. Nit — `requested_by` cannot work for a delivery retry and outlives a revocation
   Where: design 338-339, 401, 854, 946-948; `crates/scheduler/src/events.rs:491`; `crates/application/src/approval_scheduler.rs:190`; `crates/domain/src/execution/approval.rs:83,149`.
   What: Checked in the code. `started_by` becomes the initiator of an approval, and a policy with initiator exclusion bars that person from approving.
   - The design promises that the caller of a delivery retry becomes the starter. But `event_deliveries` gains no `requested_by` column. The retried element may join a shared window, whose starter is the engine or an earlier flusher, or go through today's stateless start, which hard-codes `Subject::engine()`.
   - A window's `requested_by` is used by a start that can come 24 hours or more later. Nothing re-checks that the subject still holds `run:start`, so a removed member can appear as the starter of a run created after the removal.
   Correction: Record the delivery-retry caller in the audit record only, and drop "or the delivery" from FR-042-023. In the start's `RunStarted` audit record, write "requested by X at T".

7. Nit — Leftovers in other documents, and the dependency on design 053 is not enforced
   Where: `docs/designs/015-opentelemetry-and-openbao.md:32,690`; `docs/owner-decisions.md:143`; design 410-411, 870-871, 1098, 1262-1266; `crates/infra-postgres/migrations/20270211000000_lease_withdrawal_functions.up.sql:40`; `crates/application/src/instances.rs:427`.
   What: Checked.
   - Design 015 (O015-008) still refers to "design 042's per-tenant HMAC key", and OD-2026-11-26-117 is not marked as superseded.
   - The design says `lease_withdrawal_required()` takes `FOR KEY SHARE`, but the code takes `FOR SHARE`. Design 053 makes that change in 0.3.0.
   - `ServingGate::uses` does not exist yet; the code has only `requires`.
   - The order points do not make slices 2 to 5 wait for design 053's draining state, `uses` and the lock change. The fix for previous finding 6 depends on them.
   - Line 1098's table locks in share mode apply only to design 053's form (b), not to form (a).
   Correction: Update design 015 and row 117. Cite design 053 for the `FOR KEY SHARE` fence. Add an order point on design 053.

Classes I looked at and found nothing new in:
- Cross-tenant 404 on chains and windows.
- Metric labels.
- Grants: `sagat_app` already has DELETE on `event_deliveries` and UPDATE on `inbox_events`.
- The window idempotency key: public starts use the scope `run.start:{ns}`, so a caller cannot plant a receipt under `trigger:{id}`.
- The unfenced start's guarded write.
- Injection, regular expressions, SSRF and egress.
