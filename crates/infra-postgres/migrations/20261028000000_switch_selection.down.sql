DO $rollback$
BEGIN
    IF EXISTS (SELECT 1 FROM steps WHERE switch_case_id IS NOT NULL)
       OR EXISTS (SELECT 1 FROM run_events WHERE event_type = 'switch.evaluated')
    THEN
        RAISE EXCEPTION 'switch selection is in use; keep a compatible reader until retained rows drain'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END
$rollback$;

ALTER TABLE steps
    DROP CONSTRAINT steps_switch_case_id_valid,
    DROP COLUMN switch_case_id;
