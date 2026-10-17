-- FR-017-007/009: cleanup ownership survives crashes without storing credentials.
ALTER TABLE secret_provider_operations
    ADD COLUMN cleanup_lease uuid,
    ADD COLUMN cleanup_expires_at timestamptz,
    ADD CONSTRAINT secret_provider_cleanup_lease_pair CHECK (
        (cleanup_lease IS NULL) = (cleanup_expires_at IS NULL)),
    ADD CONSTRAINT secret_provider_cleanup_abandoned CHECK (
        cleanup_lease IS NULL OR state='abandoned');
CREATE INDEX secret_provider_cleanup_due ON secret_provider_operations
    (coalesce(cleanup_expires_at,'-infinity'::timestamptz),created_at,id);
CREATE INDEX runs_database_credential_cleanup ON runs(finished_at,id)
    WHERE finished_at IS NOT NULL;
