# Architecture re-read of design 040 (2026-11-26)

Doc: docs/designs/040-ocsf-event-standard.md
Reviewed-commit: 9b994b9503f4d3e1196ca684188065d2a6f40d16
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 5, Nit 2

Round 1 architecture: 1 fixed, 2 fixed, 3 fixed, 4 not fixed, 5 fixed, 6 not fixed, 7 fixed, 8 not fixed, 9 fixed, 10 fixed

1. Major — Round-1 item 4 is still open: a receiver that answers `400` to everything still passes the whole backlog
   - Where: section 5 step 6 (lines 489-494) disagrees with Requirement 040-009 (lines 127-130).
   - What: the requirement passes a record only "while the receiver accepted a sibling half". Section 5 passes it "if this sink has ever had a `2xx`". Say a collector once accepted events and its parser later answers `400`/`422` to every event. Halving then marks every record `audit.sink_record_refused` and moves the cursor past the entire backlog, which breaks "MUST NOT skip".
   - Correction: in section 5, pass a single record only when a sibling part of the same split got a `2xx`. When no part of a split is accepted, go to `PAUSED_DESTINATION_REFUSED`. Add a Success criterion 040-006 case: `400` on 1,000 records after an earlier `2xx` passes none.

2. Major — Round-1 item 6 is still open: on a logical restore without the epoch step, `rebase` skips every new record for good
   - Where: section 4, lines 401-412; Success criterion 040-007 (lines 193-194); ADR-0150 decision 4 (lines 34-35).
   - What: restored rows keep their old high `tx_id`, so they stay at or above the new cluster's `xmin` and cannot be read. `rebase` puts the cursor just below those old values, so every record written on the new cluster has a lower `tx_id`, sorts below the cursor and is never sent. The planned test only passes because it lets a toy counter overtake the restored maximum; in production that gap is billions of transactions. The documents also disagree. The ADR makes the epoch step mandatory, but the design calls it optional ("where the operator can"). A cluster restored with the epoch step still has a different `system_identifier`, so the sink still goes `CURSOR_INVALID`, which contradicts Success criterion 040-007's "with the epoch step delivers".
   - Correction: make `rebase` refuse while `max(tx_id) >= pg_snapshot_xmax(pg_current_snapshot())`. Make the epoch step mandatory for a logical restore, as the ADR already does. Where `pg_resetwal` is not available (managed services), record as a gap that only a physical restore keeps the export. In the test, do not let the counter catch up.

3. Major — Round-1 item 8 is still open: sink columns sealed under today's key ring are left out of today's key rotation
   - Where: lines 514-521; `apps/control-plane/src/lib.rs:439-459`; `docs/designs/015-opentelemetry-and-openbao.md:64`.
   - What: `reseal-secrets` only re-seals secret versions. The documented rotation is "run `reseal-secrets`, then remove the old key". After that, no key opens `url_sealed`, `credential_sealed` or `signing_key_sealed`, and every sink retries `STALLED` forever. Design 015 has no `audit_sinks` entry, despite line 518 ("join design 015's re-seal inventory"); only the operator sink's KV custody is listed (line 452).
   - Correction: add a Task 040 that extends `reseal-secrets` to the `audit_sinks` columns, with a rotation test. Amend design 015's inventory in the same change.

4. Major — The operator sink cannot recover from `PAUSED_DESTINATION_REFUSED` without skipping records
   - Where: lines 426-428 and 498-509; Requirement 040-011.
   - What: the fingerprint hashes the URL only. Rotating the collector credential (the usual fix for a `401`) is therefore not seen as a change, and the stream stays paused. Changing the URL does count as a change, but it "applies the configured `start` (`now`, …)", which jumps past the backlog that built up while paused. Line 427 says the opposite ("a change of its configured URL keeps the cursor"). The state is also kept per tenant row even though the destination is shared.
   - Correction: include a hash of the credential in the fingerprint. Keep the cursors on a fingerprint change, and apply `start` only when `start` itself changes. Alternatively, add an operator `audit-export resume-operator` command. Fix line 427.

5. Major — The operator sink's discovery cursor does not record its cluster, so `rebase` cannot repair it
   - Where: line 542 (`audit_operator_config`); lines 404-408.
   - What: only the sink cursors and the per-tenant cursors carry `cursor_cluster`. After a logical restore, the global discovery cursor stays at the old cluster's high `tx_id`. A tenant created after the restore is never discovered, so its records never reach the operator sink, including "a newly provisioned attacker's probes" (line 505). Nothing marks this as `CURSOR_INVALID`.
   - Correction: add `discovery_cursor_cluster` to `audit_operator_config`, have the pass check it, include it in `rebase`, and assert it in the restore test of Task 040-13.

6. Nit — Requirement 040-006 says `unmapped` holds two keys "and nothing else", but the tombstone adds `unmapped.sagat.tombstone`
   - Where: lines 107-109 and 338 against line 557; Task 040-4 ("closed per action").
   - Correction: list `tombstone` as a third closed key, used only on the class-0 event.

7. Nit — "Persists only its cursor" no longer matches the operator sink's storage
   - Where: Requirement 040-011 (line 141); ADR-0150 decision 5 (line 39); line 426 ("one row … keyed by a constant").
   - What: the operator sink now stores a configuration row (fingerprint, revision, discovery cursor) and one row per tenant (lines 498-501, 542-545).
   - Correction: amend all three to match.

Files:
- docs/designs/040-ocsf-event-standard.md
- docs/adr/ADR-0150-security-and-audit-events-leave-as-ocsf-projections-of-the-native-record.md
- docs/designs/015-opentelemetry-and-openbao.md
- apps/control-plane/src/lib.rs
