-- Refuse to discard input provenance on a database that already uses the feature.
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM blobs WHERE purpose = 'RUN_INPUT') THEN
        RAISE EXCEPTION 'run-input blobs must be drained before this migration is reverted';
    END IF;
END
$guard$;
DROP INDEX runs_input_blob_idx;
ALTER TABLE runs DROP CONSTRAINT runs_input_blob_same_scope_fk;
ALTER TABLE runs DROP COLUMN input_blob_id;
ALTER TABLE blobs DROP CONSTRAINT blobs_owner_valid;
ALTER TABLE blobs DROP COLUMN purpose;
ALTER TABLE blobs ALTER COLUMN run_id SET NOT NULL;
ALTER TABLE blobs ALTER COLUMN step_id SET NOT NULL;
ALTER TABLE blobs ALTER COLUMN attempt SET NOT NULL;
ALTER TABLE blobs ALTER COLUMN generation SET NOT NULL;
