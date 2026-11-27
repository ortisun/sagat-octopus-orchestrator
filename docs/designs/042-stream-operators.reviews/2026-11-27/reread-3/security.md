# Third security re-read of design 042 (2026-11-27)

Doc: docs/designs/042-stream-operators.md
Reviewed-commit: 31466abf39cfbe1618fe7fc3ff2cbbdcc8ca6714
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 2, Nit 6

## Previous re-read

1. Fixed. Deliveries to a value-dependent chain read `WITHHELD`, and its counts and windows need `reveal=pii` (361-377, 398-405, 1130-1137, T042-17 at 1480-1487, T042-39 at 1571-1574). What publishers still learn is stated as Gap 042-018 and left to the owner (582-587, 631-634). How far that residual reaches is understated; see finding 7.
2. Fixed. The seal, the flush and an apply that seals at once check the `flatMap` list against the held-bytes quota, with the lock taken last. Each index input is checked against `MAX_PAYLOAD_BYTES`. A commit-time content refusal is a counted wait with a bound (288, 297-308, 1009-1016, 1040-1042, T042-24 at 1524-1530). The new final refusal causes finding 2, and the 24-hour bound causes finding 3.
3. Fixed. Both passages now say the switch turns on only the apply and the capability listing (729-733, 1185-1187). The drain check requires that no window is left, and final windows are purged at once while the capability is `draining` (1195-1199, T042-54 at 1638-1641).
4. Fixed as asked. Held bytes are recorded as per-transaction deltas in `operator_held_deltas` and flushed at commit by `operator_held_flush()` (936-948, 977-982, NFR-042-002 at 1329-1332, T042-13 at 1443-1446, T042-22 at 1518-1519, ADR-0050 line 103). New problems in this area are findings 1 and 4.
5. Fixed for the key row: it is inserted `ON CONFLICT DO NOTHING RETURNING` after a savepoint (965-970, T042-15 at 1471-1473). The wider claim about logs is finding 6, and a race left at the last slot is finding 5.
6. Fixed. The `OPERATOR_WINDOW_INPUT` level is in place (426-430), the member span links are dropped (1163-1166) and the window actions answer without reveal (1133-1135). What the constant is attached to is not stated; see finding 8.

## Findings

1. Major — Step 5 reads this transaction's pending content delta, which `sagat_app` is not allowed to read, and the design allows no definer function to do it
   Where: design 977-982 (step 5: "the committed tenant content usage plus this transaction's pending content delta"), 947-948, 952-954 ("These four … are the only definer functions"), NFR-042-002 at 1329-1332; `crates/infra-postgres/migrations/20261024000000_durable_quotas.up.sql:564-572`; `crates/infra-postgres/src/content_usage.rs:268-327`.
   What: Verified in the migration. `quota_transaction_deltas` is `REVOKE ALL … FROM sagat_app`, and only `tenant_retained_usage` gets `SELECT`.
   - The existing ways to read a pending delta are definer functions. `sagat_quota_lock_log_usage` (lines 283-294) also locks the usage row `FOR UPDATE` before commit, which would break the "slot 8b, last" claim. Design 041's step-log allowance functions are definer functions too.
   - The design does not say whether `sagat_app` gets `SELECT` on `operator_held_deltas` either.
   - As the runtime role, step 5 therefore fails with permission denied on every chained apply that adds bytes. That fails closed, but every such event stays pending.
   - The likely "fix" during implementation is a grant on the delta tables. That widens a boundary the quota migration deliberately closed ("never permission to … inspect transaction deltas directly", lines 280-282).
   Why: This is the least-privilege class. The list of definer functions in the privilege audit's allow-list would be wrong from the start.
   Correction:
   - Add one owner-owned definer function, for example `operator_held_room(tenant)`. It returns the committed held counter plus this transaction's pending held delta, and the committed content usage plus the pending content delta. It takes no row lock, pins `search_path = pg_catalog, pg_temp`, is revoked from `PUBLIC` and is executable by `sagat_app`.
   - Count five definer functions in section 4 and in NFR-042-002.
   - Say that `sagat_app` has no privilege on `operator_held_deltas`.
   - Have Task 042-13 run step 5 with `SAGAT_TEST_AS_RUNTIME_ROLE=1`.

2. Major — A held-bytes refusal at the seal or the flush is final and cannot be retried, so a full tenant pool destroys windows whose events were already accepted
   Where: design 304-307, 288, 1012-1014, 1041-1042; 291-293 (only `START_EXPIRED` is retryable); 431-432 and T042-18 (a delivery retry accepts only `FAILED`, refuses `HELD`).
   What: Checked in the text.
   - Setup: a Developer in namespace A of a tenant publishes large events to a chain of their own. These fill the tenant's `operator_held_bytes` (1 GiB) with `OPEN` windows.
   - Any `flatMap` window of namespace B that reaches its deadline is then refused `QUOTA_EXCEEDED`, final, and its held values are deleted.
   - The members' deliveries say `HELD`, so no delivery retry applies. A window retry refuses anything that is not `START_EXPIRED`.
   - The events are lost to that trigger, although the shortage is temporary: bytes come back as soon as other windows start.
   - The design treats the same situation more gently elsewhere. An element refused at the apply ends `FAILED` but stays retryable by an operator, and a commit-time content refusal at the seal is a wait.
   Why: A temporary, shared condition that any publisher of the tenant can cause makes accepted data unrecoverable, across namespaces. FR-042-021 states that a quota refusal of held data is one outcome per case; here a window's outcome depends on unrelated traffic.
   Correction: Treat the seal's and the flush's held-bytes refusal like the commit-time content refusal:
   - roll back, count `quota_wait`, and leave the window `OPEN` with a back-off, so the select and validator runs are not repeated every tick;
   - keep the 24-hour bound, which ends in a retryable refusal (finding 3).
   The flush answers `409 QUOTA_EXCEEDED` without changing the window. If a final outcome is kept instead, make `QUOTA_EXCEEDED` windows retryable back to `OPEN`.

3. Nit — A window refused while still `OPEN` reuses `START_EXPIRED`, so its retry makes it `SEALED` without ever running the seal
   Where: design 307-308 and 1015-1016 ("a window still `OPEN` 24 hours after `D` is refused `START_EXPIRED`"), 291-293, 1043-1045, T042-57; state list 925-926 (which has only `SEALED` → `REFUSED`).
   What: Checked in the text.
   - The retry sets `SEALED` on a `flatMap` window that has no `fan_out` or `fan_out_values`. Its `select`, index-validator, `MAX_PAYLOAD_BYTES` and held-bytes checks never ran.
   - The start then either stops on a missing value and loops for 24 hours, or, if an implementer "completes" it, evaluates `select` at the start and skips those checks.
   - The state list also lacks `OPEN` → `REFUSED`, the path the seal's own refusals take (`EXPRESSION_ERROR`, `OPERATOR_FAN_OUT`, `RUN_INPUT_INVALID`, `RUN_DATA_LIMIT`).
   Correction: Give the never-sealed expiry a code of its own (for example `SEAL_EXPIRED`) whose retry returns the window to `OPEN`, so the seal and all its checks run again. Add `OPEN` → `REFUSED` to the state list and to the guard of Task 042-58.

4. Nit — Two slot-8b rows are locked at commit in an order the design does not state
   Where: design 942-946 ("the counter row is the last lock a transaction takes"), 1023-1024 ("the only operator row the start touches"); `docs/adr/ADR-0050-…md:95` (8b already includes "the deferred content flush") and `:103`; `crates/infra-postgres/migrations/20261024000000_durable_quotas.up.sql:296-370,446-558`.
   What: A suspicion, based on PostgreSQL's documented firing order: deferred events fire in queue order, and triggers on one row fire in name order. Not run.
   - The content flush is a constraint trigger on every tracked table, and the first one that fires does all the transaction's content work. The member `value` is content, so the operator tables carry both flushes.
   - In an apply, the member insert queues both on one row. With a name like `…_held_flush`, which sorts before `…_quota_flush`, the apply locks `operator_held_usage` first and then `tenant_retained_usage`.
   - In a start, `runs_quota_flush` is queued first, so the start locks `tenant_retained_usage` first and then the held counter.
   - An apply and a start of the same tenant committing together can therefore deadlock. The apply's event is retried and the start backs off, so this costs availability, not a bypass.
   - Line 1024 also omits that the start writes member rows (`value` set to `NULL`) and a delta row.
   Correction: State the commit order: content usage row first, then the held counter. Name the held-flush triggers so that they sort after `*_quota_flush` on every operator table. Add a barrier test of an apply and a start of one tenant committing together. Fix line 1024 and ADR-0050 line 103 to match.

5. Nit — Two applies of the same new key at the chain's last free slot: the second one fails `KEY_LIMIT` although the key now exists
   Where: design 965-970 (step 3: "none left: `FAILED`/`KEY_LIMIT` … nothing else written"), 318-320.
   What: Checked in the text, together with PostgreSQL's re-check of an `UPDATE` under READ COMMITTED.
   - A and B apply the same new key when `key_count = max_keys - 1`. Both find no key row.
   - A reserves the last slot and inserts the key. B's reservation waits on the usage row.
   - When A commits, B's `UPDATE … WHERE key_count < max_keys` re-checks against the new count, matches no row, and B ends `FAILED`/`KEY_LIMIT` without ever reaching the insert or the second read.
   - SC-042-001's two-replica interleaving can produce this. On a value-dependent chain the wrong outcome is also hidden behind `WITHHELD`.
   Correction: When the reservation updates no row, read the key row `FOR UPDATE` again, and refuse with `KEY_LIMIT` only if it is still absent. Add a barrier test at `maxKeys - 1` to Task 042-15.

6. Nit — "No key digest can reach the server log" depends on the savepoint alone, but other paths still write digests there
   Where: design 969-970, 359-360, 906-907; precedent `docs/runbooks/step-logs.md:130-131`.
   What: A suspicion from PostgreSQL's documented behaviour; not run. Digests are bind parameters of the key-row, mark and window statements.
   - Slow-statement logging (`log_min_duration_statement`, with `log_parameter_max_length` at its default -1) writes the parameters.
   - A `CHECK` or `NOT NULL` violation on `operator_keys`, `operator_marks` or `operator_windows` writes `DETAIL: Failing row contains (…, \x<digest>, …)`.
   - A plain `INSERT` of a mark over an expired mark the purge has not removed yet raises 23505 with `value_digest` in the detail. The design does not say the mark write is an upsert.
   Correction: Require the settings the step-logs runbook already asks for on the runtime role (`log_parameter_max_length = 0`, `log_parameter_max_length_on_error = 0`, `log_error_verbosity = terse`) in Task 042-40's runbook, and cite them at 969-970. Write the mark `ON CONFLICT DO UPDATE SET expires_at = …`.

7. Nit — Gap 042-018 understates who learns what. The delivery-retry answer is unspecified, and the "Owner decisions needed" section calls the design already approved
   Where: design 582-587 and 373-376, T042-17 at 1486-1487, 1136, 631-635; `crates/domain/src/identity.rs:630-700`.
   What: The roles are verified; the rest is checked in the text.
   - A Viewer, who cannot publish and has only `run:read`, `event:read` and `definition:read`, can link events by key from timing alone. On a keyed `debounce` chain a run is created about `gapMs` (plus the horizon wait and a pass tick) after the last member's `receivedAt`. An event with no run after `receivedAt + gapMs` therefore had a later event with the same key. The gap names only publishers.
   - The quota usage view (`quota:read`, held by the Developer and the Operator) shows the tenant's content usage, and possibly the `operator_held_bytes` usage. Both grow when an element is `HELD` and not when it is `SUPPRESSED`, so a publisher gets an answer at once, before any run.
   - The delivery retry answers `202` for a `FAILED` delivery and `409 DELIVERY_NOT_RETRYABLE` otherwise. At `maxKeys` that tells a new key (`KEY_LIMIT`) from an existing one (`HELD`). Line 1136 does not say that the `202` body is shown without reveal.
   - Every role with `event:publish` also has `run:read`, so the last two points widen no role beyond the gap. The Viewer point does.
   - Line 634 says the design "is otherwise `Approved` by review". The status is Draft, and approval requires that no Major is open.
   Correction: Widen Gap 042-018 (and its line in research.md) to "a caller who sees runs (a Viewer included, by their timing) or the tenant's usage". Say that the delivery retry's `202` uses the shape without reveal (`WITHHELD`). Reword line 634 as "becomes `Approved` when the reviews end with no Blocker or Major open, once the owner has accepted Gaps 042-017 and 042-018".

8. Nit — The design does not say what `OPERATOR_WINDOW_INPUT` is attached to, and the obvious reading stops working when the window is purged
   Where: design 426-430, 1165-1166 ("The run carries no journal addition: the window names its run"), 1197-1198; `crates/application/src/classification.rs:112-114`, `crates/application/src/classification_index.rs:62-125`.
   What: Verified that classification is resolved per carrier, the pinned version's `Input`. Nothing on a run says that a window started it, so the only link is the window row's `run_id`.
   - An implementation that raises the level by looking up the run's window loses the protection 7 days after the window becomes final, or at once while the capability is `draining`.
   - From then on a looser `internal` annotation of `window` or `events` shows `keyId` and member lists to every reader allowed `internal` data, without reveal.
   Correction: Say that the level is a property of the version: the `Input` carrier of every version whose triggers declare an emitting chain has `window`, `event`, `events` and `value` raised to the stricter of the declared level and `pii`, whatever operator rows remain. Have Task 042-22 or 042-53 test a run read after its window is purged.

No finding in the other classes I checked:
- Cross-tenant `404` and the probe audit on chains, windows and both retries.
- Metric labels.
- The pinned `search_path` of the definer functions. Trigger functions cannot be called directly, and `sagat_app` cannot disable them.
- Rolling back the savepoint releases the reservation, the advisory lock and the deltas taken inside it.
- The held-bytes counter cannot be bypassed: every path that adds held bytes (apply, seal, flush, delivery retry) takes the tenant lock, merges do not change `value`, and the lock is held through the commit-time flush.
- The drain check and the purge at once while draining.
- The engine as the initiator of every run.
- Injection, regular expressions, SSRF and egress.
- Per-event predicate outcomes on chains that are not value-dependent: today's trigger filter already shows this through whether a delivery row exists.

Gap 042-017 is stated accurately and is the owner's to accept; it is not a finding.
