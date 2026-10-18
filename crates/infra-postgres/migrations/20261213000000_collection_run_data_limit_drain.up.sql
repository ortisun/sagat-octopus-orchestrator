ALTER TABLE iteration_scopes
    DROP CONSTRAINT iteration_scopes_drain_reason,
    ADD CONSTRAINT iteration_scopes_drain_reason CHECK ((
        (lifecycle IN ('OPEN', 'SUCCEEDED')
            AND drain_kind IS NULL AND drain_iteration IS NULL
            AND drain_failure_code IS NULL AND drain_sequence IS NULL
            AND drain_outbox_id IS NULL)
        OR (lifecycle IN ('DRAINING', 'FAILED') AND drain_kind = 'FAILED'
            AND drain_iteration BETWEEN 0 AND 9999
            AND char_length(drain_failure_code) BETWEEN 1 AND 128
            AND drain_sequence IS NOT NULL AND drain_outbox_id IS NOT NULL)
        OR (lifecycle IN ('DRAINING', 'FAILED') AND drain_kind = 'STORAGE_QUOTA'
            AND drain_iteration IS NULL
            AND drain_failure_code = 'STORAGE_QUOTA_EXCEEDED'
            AND drain_sequence IS NOT NULL AND drain_outbox_id IS NOT NULL)
        OR (lifecycle IN ('DRAINING', 'CANCELLED') AND drain_kind = 'CANCELLED'
            AND drain_iteration IS NULL AND drain_failure_code IS NULL
            AND drain_sequence IS NOT NULL AND drain_outbox_id IS NOT NULL)
        OR (lifecycle IN ('DRAINING', 'FAILED') AND drain_kind = 'RUN_DATA_LIMIT'
            AND drain_iteration IS NULL
            AND drain_failure_code = 'RUN_DATA_LIMIT'
            AND drain_sequence > 0
            AND drain_outbox_id IS NOT NULL)
    ) IS TRUE);
