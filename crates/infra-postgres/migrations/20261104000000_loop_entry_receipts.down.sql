DO $rollback$
BEGIN
    IF EXISTS (SELECT 1 FROM loop_scopes WHERE entry_outbox_id IS NOT NULL)
    THEN
        RAISE EXCEPTION 'loop entry receipts are retained; keep a compatible reader until affected runs drain'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END
$rollback$;

DROP INDEX loop_scopes_entry_outbox_uniq;
ALTER TABLE loop_scopes
    DROP CONSTRAINT loop_scopes_initial_condition,
    DROP COLUMN entry_outbox_id,
    DROP COLUMN initial_condition;
