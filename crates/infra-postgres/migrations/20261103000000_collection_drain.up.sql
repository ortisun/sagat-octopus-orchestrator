ALTER TABLE iteration_scopes
    DROP CONSTRAINT iteration_scopes_terminal_result,
    ADD COLUMN drain_kind text,
    ADD COLUMN drain_iteration integer,
    ADD COLUMN drain_failure_code text,
    ADD COLUMN drain_sequence bigint,
    ADD COLUMN drain_outbox_id uuid,
    ADD CONSTRAINT iteration_scopes_drain_outbox_uniq UNIQUE (drain_outbox_id),
    ADD CONSTRAINT iteration_scopes_drain_reason CHECK (
        (lifecycle IN ('OPEN', 'SUCCEEDED')
            AND drain_kind IS NULL AND drain_iteration IS NULL
            AND drain_failure_code IS NULL AND drain_sequence IS NULL
            AND drain_outbox_id IS NULL)
        OR (lifecycle IN ('DRAINING', 'FAILED') AND drain_kind = 'FAILED'
            AND drain_iteration BETWEEN 0 AND 9999
            AND char_length(drain_failure_code) BETWEEN 1 AND 128
            AND drain_sequence IS NOT NULL AND drain_outbox_id IS NOT NULL)
        OR (lifecycle IN ('DRAINING', 'CANCELLED') AND drain_kind = 'CANCELLED'
            AND drain_iteration IS NULL AND drain_failure_code IS NULL
            AND drain_sequence IS NOT NULL AND drain_outbox_id IS NOT NULL)),
    ADD CONSTRAINT iteration_scopes_terminal_result CHECK (
        (lifecycle IN ('OPEN', 'DRAINING') AND result_digest IS NULL
            AND finish_sequence IS NULL AND finish_outbox_id IS NULL)
        OR (lifecycle = 'SUCCEEDED' AND completed_count = item_count AND active_count = 0
            AND result_digest IS NOT NULL AND finish_sequence IS NOT NULL
            AND finish_outbox_id IS NOT NULL)
        OR (lifecycle IN ('FAILED', 'CANCELLED') AND active_count = 0
            AND result_digest IS NULL AND finish_sequence IS NOT NULL
            AND finish_outbox_id IS NOT NULL));

ALTER TABLE collection_iterations
    ADD COLUMN drained_sequence bigint,
    ADD COLUMN drain_outbox_id uuid,
    ADD CONSTRAINT collection_iterations_drain_outbox_uniq UNIQUE (drain_outbox_id),
    ADD CONSTRAINT collection_iterations_drain_receipt CHECK (
        (lifecycle = 'DRAINED' AND drained_sequence IS NOT NULL AND drain_outbox_id IS NOT NULL)
        OR (lifecycle <> 'DRAINED' AND drained_sequence IS NULL AND drain_outbox_id IS NULL));
