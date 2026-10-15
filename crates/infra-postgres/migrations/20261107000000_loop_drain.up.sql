DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM loop_iterations WHERE lifecycle = 'DRAINED') THEN
        RAISE EXCEPTION 'legacy loop drain rows require a compatible reader before upgrade';
    END IF;
END $$;

ALTER TABLE loop_scopes
    DROP CONSTRAINT loop_scopes_active,
    ADD COLUMN drain_kind text,
    ADD COLUMN drain_iteration integer,
    ADD COLUMN drain_failure_code text,
    ADD COLUMN drain_sequence bigint,
    ADD COLUMN drain_outbox_id uuid,
    ADD COLUMN finish_sequence bigint,
    ADD COLUMN finish_outbox_id uuid,
    ADD CONSTRAINT loop_scopes_drain_outbox_uniq UNIQUE (drain_outbox_id),
    ADD CONSTRAINT loop_scopes_finish_outbox_uniq UNIQUE (finish_outbox_id),
    ADD CONSTRAINT loop_scopes_active CHECK (
        (lifecycle = 'RUNNING' AND active_iteration IS NOT NULL
            AND active_iteration >= 0 AND active_iteration < max_iterations
            AND delay_due_at IS NULL)
        OR (lifecycle = 'DELAYING' AND active_iteration IS NULL AND delay_due_at IS NOT NULL)
        OR (lifecycle = 'DRAINING' AND delay_due_at IS NULL
            AND (active_iteration IS NULL OR
                 (active_iteration >= 0 AND active_iteration < max_iterations)))
        OR (lifecycle NOT IN ('RUNNING', 'DELAYING', 'DRAINING')
            AND active_iteration IS NULL AND delay_due_at IS NULL)),
    ADD CONSTRAINT loop_scopes_drain_reason CHECK (
        (drain_kind IS NULL AND drain_iteration IS NULL AND drain_failure_code IS NULL
            AND drain_sequence IS NULL AND drain_outbox_id IS NULL
            AND finish_sequence IS NULL AND finish_outbox_id IS NULL
            AND lifecycle IN ('READY', 'RUNNING', 'DELAYING', 'SUCCEEDED', 'FAILED'))
        OR (drain_kind = 'FAILED' AND drain_iteration BETWEEN 0 AND 9999
            AND char_length(drain_failure_code) BETWEEN 1 AND 128
            AND drain_sequence IS NOT NULL AND drain_outbox_id IS NOT NULL
            AND ((lifecycle = 'DRAINING' AND finish_sequence IS NULL AND finish_outbox_id IS NULL)
                OR (lifecycle = 'FAILED' AND finish_sequence IS NOT NULL
                    AND finish_outbox_id IS NOT NULL)))
        OR (drain_kind = 'TIMED_OUT' AND drain_iteration IS NULL
            AND drain_failure_code = 'RUN_TIMED_OUT'
            AND drain_sequence IS NOT NULL AND drain_outbox_id IS NOT NULL
            AND ((lifecycle = 'DRAINING' AND finish_sequence IS NULL AND finish_outbox_id IS NULL)
                OR (lifecycle = 'FAILED' AND finish_sequence IS NOT NULL
                    AND finish_outbox_id IS NOT NULL)))
        OR (drain_kind = 'CANCELLED' AND drain_iteration IS NULL
            AND drain_failure_code IS NULL
            AND drain_sequence IS NOT NULL AND drain_outbox_id IS NOT NULL
            AND ((lifecycle = 'DRAINING' AND finish_sequence IS NULL AND finish_outbox_id IS NULL)
                OR (lifecycle = 'CANCELLED' AND finish_sequence IS NOT NULL
                    AND finish_outbox_id IS NOT NULL))));

ALTER TABLE loop_iterations
    DROP CONSTRAINT loop_iterations_checkpoint,
    ADD COLUMN drained_sequence bigint,
    ADD COLUMN drain_outbox_id uuid,
    ADD CONSTRAINT loop_iterations_drain_outbox_uniq UNIQUE (drain_outbox_id),
    ADD CONSTRAINT loop_iterations_checkpoint CHECK (
        (lifecycle = 'ADMITTED' AND slot_held AND return_instance_id IS NULL
            AND return_node IS NULL AND result_digest IS NULL AND carry_digest IS NULL
            AND condition IS NULL AND checked_at IS NULL AND checkpoint_sequence IS NULL
            AND drained_sequence IS NULL AND drain_outbox_id IS NULL)
        OR (lifecycle = 'CHECKPOINTED' AND NOT slot_held
            AND return_instance_id IS NOT NULL
            AND return_node ~ '^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$'
            AND octet_length(result_digest) = 32 AND octet_length(carry_digest) = 32
            AND condition IS NOT NULL AND checked_at IS NOT NULL
            AND checkpoint_sequence IS NOT NULL
            AND drained_sequence IS NULL AND drain_outbox_id IS NULL)
        OR (lifecycle = 'DRAINED' AND NOT slot_held
            AND return_instance_id IS NULL AND return_node IS NULL
            AND result_digest IS NULL AND carry_digest IS NULL
            AND condition IS NULL AND checked_at IS NULL AND checkpoint_sequence IS NULL
            AND drained_sequence IS NOT NULL AND drain_outbox_id IS NOT NULL));
