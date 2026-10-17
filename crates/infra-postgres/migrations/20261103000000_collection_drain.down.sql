DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM iteration_scopes
        WHERE lifecycle IN ('DRAINING', 'FAILED', 'CANCELLED')
    ) OR EXISTS (
        SELECT 1 FROM collection_iterations WHERE lifecycle = 'DRAINED'
    ) THEN
        RAISE EXCEPTION 'collection drain progress is retained';
    END IF;
END $$;

ALTER TABLE collection_iterations
    DROP CONSTRAINT collection_iterations_drain_receipt,
    DROP CONSTRAINT collection_iterations_drain_outbox_uniq,
    DROP COLUMN drain_outbox_id,
    DROP COLUMN drained_sequence;

ALTER TABLE iteration_scopes
    DROP CONSTRAINT iteration_scopes_terminal_result,
    DROP CONSTRAINT iteration_scopes_drain_reason,
    DROP CONSTRAINT iteration_scopes_drain_outbox_uniq,
    DROP COLUMN drain_outbox_id,
    DROP COLUMN drain_sequence,
    DROP COLUMN drain_failure_code,
    DROP COLUMN drain_iteration,
    DROP COLUMN drain_kind,
    ADD CONSTRAINT iteration_scopes_terminal_result CHECK (
        (lifecycle <> 'SUCCEEDED' AND result_digest IS NULL
            AND finish_sequence IS NULL AND finish_outbox_id IS NULL)
        OR (lifecycle = 'SUCCEEDED' AND completed_count = item_count AND active_count = 0
            AND result_digest IS NOT NULL AND finish_sequence IS NOT NULL
            AND finish_outbox_id IS NOT NULL));
