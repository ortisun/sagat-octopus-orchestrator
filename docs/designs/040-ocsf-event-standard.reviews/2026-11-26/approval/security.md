# Approval read of design 040: security (2026-11-26)

Doc: docs/designs/040-ocsf-event-standard.md
Reviewed-commit: 09e6543020101d0db05cabc1f7a853c95a0148b6
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 0, Nit 2

Previous re-read: 1 fixed, 2 fixed, 3 fixed.
- 1 (Major, re-seal): fixed. Task 040-26 (`docs/designs/040-ocsf-event-standard.md:851-854`) now updates a column only while its key id, its sealed value and the sink's revision are all unchanged. `PATCH` and rotation raise the revision (`:497`), so the failing case from the last re-read can no longer write the old URL back. The task also adds a test that races a re-seal with a `PATCH`.
- 2 (Nit, refusals on the operator stream): fixed. Refusals go only to logs and metrics, never into a tenant's trail (`:146`, `:805`). Task 040-24 adds the refused-window columns (`:803-804`). A small leftover is Nit 2 below.
- 3 (Nit, `search_path` and the runbook): fixed. The system-identifier function now states `SET search_path = pg_catalog` (`:420`). The runbook gives `pg_read_all_stats` to the monitoring role, never to `sagat_app` (`:661-662`).

I found no new Blocker or Major in the text changed for Decision 040-019. That text is Task 040-27, the operator store port, the `start` rule, `AUDIT_EXPORT_REBASE_REQUIRED` and the halving order.

1. **Nit: the `start` rule cannot tell the configured `now` from a stored time** (`:538-541`, `:604`, `:577`)
   - `adopt_config` takes `start: Option<OffsetDateTime>`, so `now` has to become a time when the engine starts. That time always differs from the stored `start_at`, so each revision raise for a credential rotation would apply `start` again. That skips the operator SIEM's backlog, including cross-tenant probes queued during the outage. The test in Task 040-24 (`:805-806`) should catch this, which is why it is only a Nit.
   - Fix: store and pass the configured value as written (`enum StartAt { Now, At(OffsetDateTime) }`), and compare that value, not the resolved time.

2. **Nit: the storage block's `audit_operator_export` list lacks `refused_window_start` and `refused_in_window`** (`:579-581`)
   - Task 040-24 requires them (`:803-804`), but whoever builds from the block would leave them out.
   - Fix: add both to the comment line.

Suspicion, not verified: Task 040-27 (the restore guard) is only ordered before Task 040-13. Nothing stops Task 040-23's loop from being activatable before the cluster check exists. Activation is gated on the `audit-export-v1` capability (`:673-675`), so the risk is low as long as slice 2 ships as one unit. Confirm that in the merge plan.

Files read:
- docs/designs/040-ocsf-event-standard.reviews/2026-11-26/reread-2/security.md
- specs/040-ocsf-event-standard/research.md (row D040-019)
- docs/designs/040-ocsf-event-standard.md (lines 138-153, 395-693, 790-879)
