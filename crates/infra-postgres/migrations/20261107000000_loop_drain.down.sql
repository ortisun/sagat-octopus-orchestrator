DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM loop_scopes
        WHERE drain_sequence IS NOT NULL OR finish_sequence IS NOT NULL
    ) OR EXISTS (
        SELECT 1 FROM loop_iterations WHERE drained_sequence IS NOT NULL
    ) THEN
        RAISE EXCEPTION 'loop drain progress is retained';
    END IF;
END $$;

ALTER TABLE loop_iterations
    DROP CONSTRAINT loop_iterations_checkpoint,
    DROP CONSTRAINT loop_iterations_drain_outbox_uniq,
    DROP COLUMN drain_outbox_id,
    DROP COLUMN drained_sequence,
    ADD CONSTRAINT loop_iterations_checkpoint CHECK (
        (lifecycle = 'ADMITTED' AND slot_held AND return_instance_id IS NULL
            AND return_node IS NULL AND result_digest IS NULL AND carry_digest IS NULL
            AND condition IS NULL AND checked_at IS NULL AND checkpoint_sequence IS NULL)
        OR (lifecycle IN ('CHECKPOINTED', 'DRAINED') AND NOT slot_held
            AND return_instance_id IS NOT NULL
            AND return_node ~ '^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$'
            AND octet_length(result_digest) = 32 AND octet_length(carry_digest) = 32
            AND condition IS NOT NULL AND checked_at IS NOT NULL
            AND checkpoint_sequence IS NOT NULL));

ALTER TABLE loop_scopes
    DROP CONSTRAINT loop_scopes_drain_reason,
    DROP CONSTRAINT loop_scopes_active,
    DROP CONSTRAINT loop_scopes_finish_outbox_uniq,
    DROP CONSTRAINT loop_scopes_drain_outbox_uniq,
    DROP COLUMN finish_outbox_id,
    DROP COLUMN finish_sequence,
    DROP COLUMN drain_outbox_id,
    DROP COLUMN drain_sequence,
    DROP COLUMN drain_failure_code,
    DROP COLUMN drain_iteration,
    DROP COLUMN drain_kind,
    ADD CONSTRAINT loop_scopes_active CHECK (
        (lifecycle = 'RUNNING' AND active_iteration IS NOT NULL
            AND active_iteration >= 0 AND active_iteration < max_iterations
            AND delay_due_at IS NULL)
        OR (lifecycle = 'DELAYING' AND active_iteration IS NULL AND delay_due_at IS NOT NULL)
        OR (lifecycle NOT IN ('RUNNING', 'DELAYING')
            AND active_iteration IS NULL AND delay_due_at IS NULL));
