# Second correctness re-read of design 040 (2026-11-26)

Doc: docs/designs/040-ocsf-event-standard.md
Reviewed-commit: 3f1aa6284446f2fe21864eebfc18d2de4dd2e0d7
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 3, Nit 3

Previous re-read: 1 mostly fixed (the `rebase` guard, the discovery cursor's cluster, the sink that never delivered and the post-restore test row are fixed; Success criterion 040-007 is not, see Nit 1); 2 fixed; 3 fixed, but the fix brings in finding 2 below; 4 fixed (the ports list still has no `release`, Nit 3); 5 partly fixed (Gap 040-008 states the risk, but section 4 still makes the old claim, Nit 2); 6 fixed.

1. **Major. Gap 040-007's remedy loses every new record** (lines 244-245 against lines 403-408). This applies to a logical restore with no epoch step (managed service). Recreating a sink with `startFrom: t`, `t` before the restore, sets the cursor below the restored rows, whose `tx_id` stay high. New rows get low ids and sort below that cursor, so they are never sent. The restored rows sit above `xmin` and are not sent either. The sink is `ACTIVE` with the new cluster id, so `CURSOR_INVALID` never fires: the loss is silent. No test recreates a sink on a cluster whose counter is behind. Smallest fix: `startFrom` refuses under `rebase`'s guard (`max(tx_id) >= xmax`), or reads its window only over `tx_id < xmin`. Add that test. Confirmed in text.

2. **Major. The scope exclusion added for finding 3 exists only in section 5** (lines 512-514 against Requirement 040-009, lines 124-126, and the query at line 388). Requirement 040-009 still says the sink MUST get "its tenant's records less the withheld actions and `audit.sink_delivered`". The cursor query filters only by `action = ANY($withheld)` and has no predicate on the resource. "Look" and the index say nothing either. Example: the record of a URL change on sink A is not sent to A's SIEM, and the requirement says it must be. A test built from the requirement or from the query contradicts the plan. Fix: write the exclusion into Requirement 040-009 and into the query and "Look" (`NOT (action LIKE 'audit.sink_%' AND resource_id = $sink)`), then test it. Confirmed in text.

3. **Major. An operator configuration change is not fenced against stream writes** (lines 518-530). The change raises `audit_operator_config.revision`. Per-tenant writes compare-and-set only their own row's `revision`. In a rolling deploy, the old replica holding shard 0 keeps sending to the old destination and its commits succeed, so the new destination never gets those records. If an old-config replica restarts during the deploy, the stored fingerprint and `start` flip back and forth. With `start: now`, each flip "changes its own value", so it is applied again and cursors jump past undelivered records. Fix: every stream write also checks the config revision. Apply `start` only when the revision is first raised for a new fingerprint, never when an earlier one returns. Add a two-config race test. Plausible.

4. **Nit. Success criterion 040-007 still contradicts section 4** (lines 197-198). With the epoch step, sinks are also `CURSOR_INVALID` until `rebase`. Without the step, the signal that tells the cases apart is "`rebase` refuses", not `CURSOR_INVALID`. As written, the criterion cannot tell the two cases apart.
5. **Nit. Section 4 still overclaims** (lines 405-408). "a window longer than any transaction the platform lets live" and "never fewer" contradict Gap 040-008. Change them to point at the gap.
6. **Nit. Storage and ports trail the text** (lines 563 and 570-571). `audit_operator_config` lists no discovery `cursor_cluster`. Neither table has a field for the "10 refused within an hour" count, and the operator sink has no tenant trail to count from. `AuditExportStore` has no `release`. Task 040-26 should also say that re-sealing compares on the old key id, as `replace_sealed` does, so that it cannot overwrite a concurrent URL `PATCH`.

Files read:
- docs/designs/040-ocsf-event-standard.md
- docs/designs/040-ocsf-event-standard.reviews/2026-11-26/reread/correctness.md
- specs/040-ocsf-event-standard/research.md
- crates/application/src/secrets.rs
