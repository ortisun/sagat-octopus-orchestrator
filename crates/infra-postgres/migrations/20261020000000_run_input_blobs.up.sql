-- T002-040 / FR-002-008: a pre-run input has a real namespace/uploader owner, not a fake lease.
ALTER TABLE blobs ADD COLUMN purpose text NOT NULL DEFAULT 'STEP_OUTPUT';
ALTER TABLE blobs ALTER COLUMN run_id DROP NOT NULL;
ALTER TABLE blobs ALTER COLUMN step_id DROP NOT NULL;
ALTER TABLE blobs ALTER COLUMN attempt DROP NOT NULL;
ALTER TABLE blobs ALTER COLUMN generation DROP NOT NULL;
ALTER TABLE blobs ADD CONSTRAINT blobs_owner_valid CHECK (
    (purpose = 'STEP_OUTPUT' AND run_id IS NOT NULL AND step_id IS NOT NULL
        AND attempt IS NOT NULL AND generation IS NOT NULL)
    OR (purpose = 'RUN_INPUT' AND step_id IS NULL AND attempt IS NULL AND generation IS NULL
        AND ((state = 'COMMITTED') = (run_id IS NOT NULL)))
);

ALTER TABLE runs ADD COLUMN input_blob_id uuid;
ALTER TABLE runs ADD CONSTRAINT runs_input_blob_same_scope_fk
    FOREIGN KEY (tenant_id, namespace_id, input_blob_id)
    REFERENCES blobs (tenant_id, namespace_id, id);
CREATE INDEX runs_input_blob_idx ON runs (tenant_id, namespace_id, input_blob_id)
    WHERE input_blob_id IS NOT NULL;
