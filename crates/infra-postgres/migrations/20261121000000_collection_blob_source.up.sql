-- T014-016 / FR-014-004: a collection whose items came from a blob keeps that blob referenced.
-- The scope row names the blob (same tenant and namespace) and the foreign key restricts its
-- deletion for as long as the scope exists, so neither the blob sweep nor a future retention job
-- can remove bytes a live membership snapshot was read from. A blob read this way is already
-- COMMITTED (the sweep never claims those); the constraint is what keeps that true if it changes.
ALTER TABLE iteration_scopes ADD COLUMN source_blob_id uuid;
ALTER TABLE iteration_scopes ADD CONSTRAINT iteration_scopes_source_blob_fk
    FOREIGN KEY (tenant_id, namespace_id, source_blob_id)
    REFERENCES blobs (tenant_id, namespace_id, id) ON DELETE RESTRICT;
CREATE INDEX iteration_scopes_source_blob_idx
    ON iteration_scopes (tenant_id, namespace_id, source_blob_id)
    WHERE source_blob_id IS NOT NULL;
