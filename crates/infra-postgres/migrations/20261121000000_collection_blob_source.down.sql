DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM iteration_scopes WHERE source_blob_id IS NOT NULL) THEN
        RAISE EXCEPTION 'collections whose items came from a blob are retained';
    END IF;
END $$;

DROP INDEX IF EXISTS iteration_scopes_source_blob_idx;
ALTER TABLE iteration_scopes DROP CONSTRAINT IF EXISTS iteration_scopes_source_blob_fk;
ALTER TABLE iteration_scopes DROP COLUMN IF EXISTS source_blob_id;
