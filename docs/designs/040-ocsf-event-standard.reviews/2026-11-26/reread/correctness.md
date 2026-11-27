# Correctness re-read of design 040 (2026-11-26)

Doc: docs/designs/040-ocsf-event-standard.md
Reviewed-commit: 9b994b9503f4d3e1196ca684188065d2a6f40d16
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 3, Nit 3

Round 1 correctness: 1 fixed, 2 fixed, 3 not fixed (finding 1; the detection is fixed but the repair is not), 4 fixed, 5 fixed, 6 not fixed (finding 2; only the success commit is fenced), 7 fixed (the fix itself brings in finding 3), 8 fixed, 9 fixed, 10 fixed, 11 fixed, 12 fixed, 13 fixed, 14 fixed, 15 fixed, 16 fixed

1. Major — After a restore without the epoch step, `rebase` still loses records, and the operator's discovery cursor is never checked
   - Where: section 4 "A cluster whose transaction ids went back" (lines 401-412); line 542; Success criterion 040-007.
   - What: on a managed service (no `pg_resetwal`), restored rows keep their high `tx_id`, while new rows get the new cluster's low ones. `rebase` uses `startFrom`'s rule over the restored window, so the cursor lands at the high values. New rows sort below it and are never sent. Restored rows stay above `xmin` and are not sent until the counter catches up. `audit_operator_config` stores the discovery cursor with no `cursor_cluster`, so a tenant first seen after the restore is never discovered.
   - The test moves the counter past the restored maximum before `rebase`, so it cannot fail for this. Success criterion 040-007 ("with the epoch step ... delivers") also contradicts section 4: the cluster id still differs, so `CURSOR_INVALID` holds until `rebase`.
   - Fix: `rebase` refuses while `pg_current_xact_id() <= max(tx_id)`. Add `cursor_cluster` to the discovery cursor and include it in `rebase`. Define `rebase` for a sink that has no last delivered record. Add a test row written after the restore with a `tx_id` below the restored maximum.

2. Major — Only the success commit compares claim, revision and state; the other writes of the pass can undo an administrator
   - Where: section 5 steps 3, 4 (renew `WHERE claim_token = $t` only) and 6; Success criterion 040-006.
   - What: an admin `/pause` or `/resume` commits while a request gets a `503`. The failure write (`failures + 1`, `STALLED`) or step 3's `PAUSED_AUTHORITY_LOST` then overwrites `PAUSED`, or overwrites the new authorizing subject's resume.
   - The renewal ignores state and revision. After a pause or a URL `PATCH`, the next half-batch or request still goes out to the old destination. That breaks "no batch leaves after the change ... except the one in flight".
   - The text never says which admin acts bump `revision` or clear the claim.
   - Fix: every write of the pass (renew, failure, stall, pause states, `record_refused`) uses `WHERE claim_token = $t AND revision = $r AND state IN (…)`. Every admin act bumps `revision`. Extend Success criterion 040-006 to the failure path and to the renewal.

3. Major — The refusal rule can skip the whole trail one record at a time, and its audit record feeds itself
   - Where: section 5 step 6 ("if this sink has ever had a `2xx`") against Requirement 040-009 ("while the receiver accepted a sibling half").
   - What: under section 5's rule, a collector that worked once and later answers `400` to everything (a changed parser, a quota answered as `400`) gets every record isolated, refused and passed. That is the mass skip Requirement 040-009 forbids.
   - Each refusal writes `audit.sink_record_refused` into the tenant trail. That record is in the sink's scope, so it is refused in turn, which writes another one, with no end.
   - Fix: keep Requirement 040-009's sibling-accepted rule. Cap refusals per sink per window, then go to `PAUSED_DESTINATION_REFUSED`. Leave a sink's own `audit.sink_record_refused` out of that sink's scope, or never refuse-pass it. Add a test with an "all `400` after one `2xx`" receiver.

4. Nit — The claim is never released, so lag can reach 30 seconds
   - Where: section 5 steps 1-2. The ports list has no `release`.
   - What: a sink is due again only after its claim expires. A record that arrives right after a pass therefore waits up to 30 seconds, against the 10-second p99 target of Non-functional requirement 040-003.
   - Fix: clear the token at the end of the pass (with a compare-and-set on the token).

5. Nit — `startFrom` and `rebase` rely on a limit that is only alerted, not enforced
   - Where: section 4, "Where a sink starts".
   - What: the 10-minute window is called "longer than any transaction the platform lets live", but the 5-minute oldest-holder alert does not end a transaction. A record from a transaction that lived longer than 10 minutes falls below the cursor, so "never fewer" is too strong.
   - Fix: enforce a `transaction_timeout` or `statement_timeout` above 10 minutes on the platform pools, or state the residual risk.

6. Nit — Success criterion 040-006's "endpoint held longer than the claim's expiry" cannot happen
   - Where: Success criterion 040-006; section 5 step 4.
   - What: the request timeout is 10 seconds and the claim lasts 30 seconds, renewed only between requests. A slow endpoint therefore cannot outlast the claim. The scenario needs a stalled process, or a fault hook between the renewal and the send.
   - Fix: say how the test lets the claim lapse.

Files read:
- docs/designs/040-ocsf-event-standard.md
- docs/designs/040-ocsf-event-standard.reviews/2026-11-26/round-1/correctness.md
