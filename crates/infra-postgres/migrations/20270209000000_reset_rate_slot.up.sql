-- Feature 022, task T022-086 (decision D022-069): a reset of a version whose start takes a slot of a rate-limit key takes
-- a fresh slot right after `run.reset`, or is refused `RATE_LIMIT_SLOT_UNAVAILABLE` and never waits. A pending reset
-- (`CANCEL_SOURCE`) that the key refuses ends `FAILED` with that closed reason.
--
-- The only change of the schema is the list of reasons a reset can fail with, which the CHECK holds closed. Additive: an
-- older reader reads the reason as text and shows it as written (the REST field, the SDKs, the Studio and the CLI say a
-- reason they do not know as it is written), and no older writer ever writes it. The CHECK is added `NOT VALID`, so this
-- statement takes no scan of the table under its `ACCESS EXCLUSIVE` lock; `20270209000100` validates it under `SHARE
-- UPDATE EXCLUSIVE` (the rule of design 043, D043-145, D043-153). No table, column, index or grant changes (the runtime
-- role updates the table already), so nothing is classified.
--
-- ROLLOUT ORDER: apply the migration before the engine that writes the reason: against a schema without it the CHECK
-- refuses the write, which the completer treats as a failure of the platform (the entry stays pending and is tried again
-- after a pause), never as the end of the entry.

ALTER TABLE run_resets DROP CONSTRAINT run_resets_failure_reason_valid;
ALTER TABLE run_resets ADD CONSTRAINT run_resets_failure_reason_valid CHECK (failure_reason IS NULL OR failure_reason IN
    ('BOUNDARY_NO_LONGER_ELIGIBLE', 'ADMISSION_REFUSED', 'BUSINESS_ID_IN_USE',
     'CONCURRENCY_PERMIT_UNAVAILABLE', 'RUN_PURGED',
     'AUTHORITY_REVOKED', 'REEXECUTION_NOT_ACKNOWLEDGED', 'REAPPLY_TOO_MANY',
     'SOURCE_DID_NOT_END',
     'RATE_LIMIT_SLOT_UNAVAILABLE')) NOT VALID;
