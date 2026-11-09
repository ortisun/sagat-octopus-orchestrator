-- Feature 018, US06 (T018-057): the expiry of a reset that waits for its source (D2059, owner decision OD-2026-11-08-1).
-- An entry still `PENDING_SOURCE` a whole window after its request (24 hours; the engine's setting may make it 1 hour to
-- 7 days), while its source is open, ends `FAILED` with the closed reason `SOURCE_DID_NOT_END`: no run is made and its
-- place in the namespace is freed (the count of a namespace is the count of the entries that wait).
--
-- The only change of the schema is the list of reasons a reset can fail with, which the CHECK holds closed. Additive: an
-- older reader reads the reason as text and shows it as written (the REST field, the SDKs, the Studio and the CLI say a
-- reason they do not know as it is written), and no older writer ever writes it. No table, column or index is added: the
-- scan of the entries past their window uses the partial indexes of the entries that wait
-- (`run_resets_pending_idx`, `run_resets_pending_namespace_idx`); no grant changes (the runtime role updates the
-- table already, and takes the share lock of the row of the source with the `UPDATE` it holds on `runs`); no column is
-- added, so nothing is classified.
--
-- ROLLOUT ORDER: apply the migration before the engine that expires: an engine that writes the reason against a schema
-- without it is refused by the CHECK, which the pass treats as a failure of the platform (the entry stays pending and is
-- tried again after a pause) and never as the end of the entry.

ALTER TABLE run_resets DROP CONSTRAINT run_resets_failure_reason_valid;
ALTER TABLE run_resets ADD CONSTRAINT run_resets_failure_reason_valid CHECK (failure_reason IS NULL OR failure_reason IN
    ('BOUNDARY_NO_LONGER_ELIGIBLE', 'ADMISSION_REFUSED', 'BUSINESS_ID_IN_USE',
     'CONCURRENCY_PERMIT_UNAVAILABLE', 'RUN_PURGED',
     'AUTHORITY_REVOKED', 'REEXECUTION_NOT_ACKNOWLEDGED', 'REAPPLY_TOO_MANY',
     'SOURCE_DID_NOT_END'));
