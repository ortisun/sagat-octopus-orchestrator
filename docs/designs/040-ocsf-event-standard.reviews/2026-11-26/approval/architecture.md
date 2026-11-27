# Approval read of design 040: architecture (2026-11-26)

Doc: docs/designs/040-ocsf-event-standard.md
Reviewed-commit: 09e6543020101d0db05cabc1f7a853c95a0148b6
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 2, Nit 3

Previous re-read: 1 fixed, 2 partly fixed (Requirement 040-009, the query and step 6 now agree, but no test names the exclusion), 3 fixed, 4 fixed, 5 fixed (recorded in Gap 040-007, but see Nit 5).

1. **Major:** the operator's `start` rule cannot hold with the port as written. Lines 538-541 and 604, against the Task 040-24 test (line 806).
   - **What:** `start` is applied "only when that value differs from the stored `start_at`". But `adopt_config` takes `Option<OffsetDateTime>`, and `start_at` is a time. A configured `now` therefore becomes a new time at every start, never equals the stored value, and is applied again. Raising the revision to rotate a credential then skips the backlog, which Decision 040-018 says it must not do.
   - **Fix:** store and compare the value as configured, for example `enum OperatorStart { Now, At(OffsetDateTime) }` with a text column `start_spec`. Turn `now` into a time only on the first adoption of a changed value.

2. **Major:** a record refused on an operator stream leaves no lasting trace. Line 146, line 805 and Constitution Check row VII (line 701), against the constitution's dead-letter path (docs/constitution.md:203).
   - **What:** Decision 040-019 sends these refusals to logs and metrics only. Nothing stored and no API or command tells the operator which records the data lake lacks. Those records are exactly the cross-tenant probes that User story 040-02 exists to deliver. Log retention is not a durable store.
   - **Fix:** add a refused-record table for the operator, keyed by `(tenant_id, record_id)`, that tenants cannot read, with a `control-plane audit-export refused` listing. Write it in the same transaction as the cursor move. Row VII should then cite it.

3. **Nit:** the fix for previous Major 2 has no named test. Task 040-11 (lines 793-796) says only "with tests of each guard". Add a test showing that a sink's own `audit.sink_record_refused` is excluded, while its `audit.sink_changed`, `sink_paused` and `sink_resumed` records still reach it.

4. **Nit:** Decision 040-019 says the store ports are written with signatures, but `AuditSinkStore` lists method names only (line 585). Task 040-11 also names a `pause` that the port calls `set_state` (line 600). Give `AuditSinkStore` signatures and use the port's name in the task.

5. **Nit:** Gap 040-007 (lines 247-250) limits the guard's blind spot to managed services ("after a logical restore there"). It applies to any logical restore done without the epoch step. Write the residual risk without that condition.

Files:
- docs/designs/040-ocsf-event-standard.md
- docs/designs/040-ocsf-event-standard.reviews/2026-11-26/reread-2/architecture.md
- specs/040-ocsf-event-standard/research.md (Decisions 040-018 and 040-019, lines 27-28)
- docs/constitution.md (line 203)
