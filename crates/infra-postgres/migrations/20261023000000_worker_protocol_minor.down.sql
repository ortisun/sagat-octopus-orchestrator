DROP INDEX blobs_committed_step_output_run_idx;

ALTER TABLE worker_sessions
    DROP COLUMN accepted_protocol_minor;
