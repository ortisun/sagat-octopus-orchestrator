DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM loop_checkpoints WHERE continuation = 'INSTANCE_LIMIT') THEN
        RAISE EXCEPTION 'loop instance-limit checkpoints are retained';
    END IF;
END $$;

ALTER TABLE loop_checkpoints
    DROP CONSTRAINT loop_checkpoints_continuation,
    ADD CONSTRAINT loop_checkpoints_continuation CHECK (
        continuation IN ('CONTINUE', 'DELAY', 'COMPLETE', 'ITERATION_LIMIT'));
