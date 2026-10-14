ALTER TABLE worker_sessions
    ADD COLUMN accepted_protocol_minor integer NOT NULL DEFAULT 0,
    ADD CONSTRAINT worker_sessions_protocol_minor_valid
        CHECK (accepted_protocol_minor >= 0);

CREATE INDEX blobs_committed_step_output_run_idx
    ON blobs (tenant_id, namespace_id, run_id)
    WHERE purpose = 'STEP_OUTPUT' AND state = 'COMMITTED';
