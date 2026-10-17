-- Stop cleanup jobs before rollback. Abandoned objects remain unavailable and recoverable.
DROP INDEX runs_database_credential_cleanup;
DROP INDEX secret_provider_cleanup_due;
ALTER TABLE secret_provider_operations
    DROP CONSTRAINT secret_provider_cleanup_abandoned,
    DROP CONSTRAINT secret_provider_cleanup_lease_pair,
    DROP COLUMN cleanup_lease,
    DROP COLUMN cleanup_expires_at;
