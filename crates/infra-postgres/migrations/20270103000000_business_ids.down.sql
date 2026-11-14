-- Rollback is an operator/quiesced-service operation. A binding says which execution holds an
-- identifier, and a run says which identifier it was started under: rolling back never destroys
-- either, so it refuses while any of them exists.
-- `runs` first: the test of the leaf certification rollback waits for the statement that takes this lock
LOCK TABLE runs, business_ids IN SHARE ROW EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM business_ids) OR EXISTS (SELECT 1 FROM runs WHERE business_id IS NOT NULL) THEN
        RAISE EXCEPTION 'business identifiers are retained; keep a compatible reader'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

DROP FUNCTION business_ids_purge(integer);

-- the correlation id is charged alone again
DROP TRIGGER runs_quota_delta ON runs;
CREATE TRIGGER runs_quota_delta AFTER INSERT OR UPDATE OR DELETE ON runs FOR EACH ROW
    EXECUTE FUNCTION sagat_quota_row_delta('content', 'octet_length_utf8', 'correlation_id');
ALTER TABLE runs ENABLE ALWAYS TRIGGER runs_quota_delta;

DROP TRIGGER runs_business_id ON runs;
DROP FUNCTION runs_business_id_guard();
DROP INDEX runs_business_id_idx;
ALTER TABLE runs
    DROP CONSTRAINT runs_business_id_visible_ascii,
    DROP COLUMN business_id;

DROP TABLE business_ids;
DROP FUNCTION business_ids_guard();
