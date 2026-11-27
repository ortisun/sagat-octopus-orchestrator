# EGM stages 5 to 8 (zoom-out, comprehension, critique) of design 040 (2026-11-26)

Doc: docs/designs/040-ocsf-event-standard.md
Reviewed-commit: 89e72316bfc8e70f548ea444b81dade138b58f29
Reviewer: egm-design
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 7, Nit 10

1. Major — The restore guard stops blocking once the counter catches up, and records written meanwhile are skipped for ever
   Where: section 4 "A cluster whose transaction ids went back" (lines 396-405); the `CURSOR_INVALID` row (433); the Risks table (599); Success criterion 040-007.
   What: the guard is `max(tx_id) >= xmax`. When the new cluster's counter passes the restored maximum, the guard clears and the pass "resumes where they stood". Every record written since the restore has a `tx_id` below the old cursors, so it is skipped silently. This breaks Requirement 040-009. The pass cannot tell "the epoch step was run" from "the counter caught up". Also, `pg_resetwal` is not available on managed PostgreSQL, and the Risks table's "resumed from time" is defined nowhere.
   Correction: store the cluster identity (`pg_control_system().system_identifier`) with every cursor. A mismatch makes `CURSOR_INVALID` sticky until an explicit operator command clears it. That command either confirms the epoch step or rebases each cursor from a time with `startFrom` semantics. Test that the state holds after the counter passes `max(tx_id)`.

2. Major — The operator sink cannot keep up with its own latency target and multiplies duplicates
   Where: section 5 "The operator sink" (lines 487-493); Non-functional requirement 040-003 (161-162); Risks "flooded by duplicates" (603).
   What: a batch of 500 records over 50 tenants becomes up to 50 messages, sent one after another, each with a claim renewal. At 100 ms each that is at least 5 s per batch, or about 100 records/s, while 200 records/s are written, so lag grows without bound. A single tenant's failing group also holds the prefix, so every retry resends every other tenant's acknowledged records. That contradicts "resent once per failed commit". Halving and refused records are not defined for this sink.
   Correction: send each tenant's group concurrently (with a stated limit) or keep a cursor per tenant. Define halving and refusal per group. Correct the risk row.

3. Major — The halving algorithm is undefined and conflicts with cursor order
   Where: section 5 step 6, third bullet (lines 481-485); Requirement 040-009 (128-131).
   What: "Refused only when a sibling half was accepted" needs the right half sent before the left single record is passed. But the cursor cannot commit the right half past an unresolved left record, and "commit after each accepted half" assumes it can. A batch of one record (a quiet tenant) has no sibling, so one bad record pauses the sink. The same record inside a busy batch is skipped. The outcome depends on batching.
   Correction: write the algorithm as left-first, depth-first recursion. Say which commits wait. Define the outcome for a one-record batch, for example: retry once alone, then refuse if any earlier batch of this sink was ever accepted.

4. Major — The text gives three answers for a `pii` sink when the acting administrator lacks the grant
   Where: Requirement 040-013 (line 149, "drops to `internal`"); state table, `PAUSED_GRANT_REVOKED` (430, "`/resume` by a subject whose grant passes"); Task 040-25, `PII_CEILING_NOT_PERMITTED` (750); Terms (45-46).
   What: on resume or `PATCH` by an administrator without the grant, the sink could silently drop to `internal`, be refused with `403`, or stay paused. The text says all three. It also does not say whether a credential change leaves `PAUSED_DESTINATION_REFUSED`, which matters because a `401` is usually a credential problem. The Terms definition of the authorizing subject leaves out the home namespace.
   Correction: one rule. Recommended: refuse with `403` unless the request also sets `ceiling: internal`. List which `PATCH` fields leave each paused state. Align the Terms with Requirement 040-013.

5. Major — Sealed-column key ids: the prose and the storage disagree
   Where: section 5 "Credentials" (lines 498-501) against the `audit_sinks` table (516) and the framing note (529); `SealedSecret` in `crates/domain/src/secrets.rs:125` carries its own `key_id`.
   What: the prose says "each with its own key id, nonce and ciphertext columns". The table has one `seal_key_id` for four columns, framed as `nonce || ciphertext`. After the key ring rotates, a signing-key rotation seals the new key under a different key id than the URL. One column cannot record both, and `open` then fails, which the pass counts as retryable for ever.
   Correction: one `*_key_id` column per sealed column, or a stated rule that any write reseals all four under the active key. Update the prose and the table together.

6. Major — The `startFrom` lookup cannot finish in 5 seconds for a busy tenant
   Where: section 4 "Where a sink starts" (lines 390-394).
   What: the minimum `tx_id` over 90 days of a tenant's records reads the `(tenant_id, created_at)` index and fetches every row from the table, because `tx_id` is not in that index. At 10 records/s that is about 78 million fetches. The typed timeout makes `startFrom` unusable for exactly the tenants who need it.
   Correction: bound the scan to `[t, t + W]`. W must exceed the longest open transaction, which is already alerted at 5 minutes. State the bound as the at-least-once assumption, or add a covering index. Test it at volume in Task 040-14.

7. Major — `region_epoch` is not defined: the epoch the record was written in, or the epoch it was sent in
   Where: section 2, `project(row, ceiling, epoch)` (line 337); "After a failover that lost work" (535-539); Requirement 040-006.
   What: `audit_records` stores no epoch, so the projection stamps the reader's current epoch. A record written in epoch 1 and redelivered after an activation carries epoch 2. A tombstone keyed by a time window would then mark surviving events delivered under epoch 1 as lost. The field is in the 0.3.0 wire contract now, even though the tombstone is deferred.
   Correction: define it as "the epoch in force when the event was sent" in the requirement and the AsyncAPI contract. Define the tombstone rule now: on activation, rewind each cursor to the first record at or after `lostFrom`, so survivors are re-sent under the new epoch, and receivers treat older-epoch events in the window that were not re-sent as lost.

8. Nit — Two truncation markers
   Where: Requirement 040-003 (lines 97-98) against section 2 (339-340) and `crates/telemetry/src/redact.rs:59`.
   What: the requirement says every cut value ends with `…[truncated]`. `scrub` appends `…[N characters cut]` after 2,048 characters.
   Correction: say identifiers use `…[truncated]` and the reason keeps `scrub`'s marker, or use one marker.

9. Nit — Class count
   Where: section 1 code, "the attributes the five classes use" (line 312).
   What: there are four classes after Decision 040-014, plus Base Event 0 for the tombstone.
   Correction: say four, and name class 0 as a follow-up.

10. Nit — Outcomes of the sender port are not classified
    Where: section 5 step 4 (lines 458-459) and step 6 (476-480).
    What: `SendFailure::Invalid` is not in any class. A key-authority error is not a `SendFailure`. `OutboundResponse.retry_after` on `429`/`503` is ignored.
    Correction: classify `Invalid` as a destination refusal, and honour `Retry-After` within the 5-minute cap.

11. Nit — `audit.sink_delivered` costs more than it gives
    Where: step 5 (lines 473-475); Task 040-3.
    What: it writes one row per sink per minute into a trail kept for ever, carries no count, and is never exported. `last_delivered_*` and the metrics already show deliveries.
    Correction: drop it, or say what question it answers that they do not.

12. Nit — Operator evidence goes in two places
    Where: section 4 (lines 403-404) against Requirement 040-011 (142-143).
    What: section 4 says `CURSOR_INVALID` is "audited" for every sink. The operator sink's evidence must never go into a tenant's trail.
    Correction: say logs and metrics only for the operator sink.

13. Nit — Slot choice under concurrent creates
    Where: section 6 (lines 551-553).
    What: two creates can pick the same free slot.
    Correction: take the lowest free slot; on a unique violation retry once, then answer `409 AUDIT_SINK_LIMIT`.

14. Nit — The operator cursor cannot be reset
    Where: lines 419-421.
    What: pointing the sink at a new destination, or re-adding its configuration months later, sends the whole backlog.
    Correction: add a configured start (now, or a time) that applies when the destination changes.

15. Nit — Zoom-out: the `pii` ceiling carries only the scrubbed reason
    Where: Requirement 040-010 (lines 132-138).
    What: the state, the store function, the commit recheck and a family of race tests exist to ship one free-text field.
    Correction: record the cost in Complexity Tracking, or move `pii` to a follow-up and ship `internal` only in 0.3.0.

16. Nit — Unclear wording
    Where: the `/test` endpoint (lines 546-547); section 4 (411); `user.uid` parsing (329).
    What: the `/test` event's class, its `metadata.uid` and the states it is allowed in are not given. "the fence" in section 4 is never defined. Splitting `user.uid` from `<namespace>/<subject>` assumes namespace ids contain no `/`.
    Correction: say all three.

Main files read: `docs/designs/040-ocsf-event-standard.md`, `docs/adr/ADR-0150-security-and-audit-events-leave-as-ocsf-projections-of-the-native-record.md`, `specs/040-ocsf-event-standard/research.md`, `docs/constitution.md` and `AGENTS.md`. To check the document's claims I also read `crates/domain/src/audit.rs` (127 actions, all covered by the appendix), `crates/domain/src/secrets.rs`, `crates/telemetry/src/redact.rs`, `crates/application/src/signing.rs`, `crates/application/src/outbound.rs`, `crates/infra-postgres/src/reveal_grants.rs` and `docs/runbooks/control-plane.md` in the same worktree.
