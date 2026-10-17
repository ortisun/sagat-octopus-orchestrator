-- A down migration must not silently erase authoritative provider references.
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM secrets WHERE authority = 'openbao')
       OR EXISTS (SELECT 1 FROM secret_provider_operations) THEN
        RAISE EXCEPTION 'retire OpenBao secrets and provider receipts before rolling back';
    END IF;
END $$;
ALTER TABLE secret_versions DROP CONSTRAINT secret_versions_provider_fk,
    DROP CONSTRAINT secret_versions_representation, DROP COLUMN provider_operation,
    ALTER COLUMN key_id SET NOT NULL, ALTER COLUMN nonce SET NOT NULL, ALTER COLUMN ciphertext SET NOT NULL;
DROP TABLE secret_provider_operations;
DROP FUNCTION sagat_secret_provider_quota_delta();
DROP FUNCTION sagat_secret_provider_immutable();
ALTER TABLE secrets DROP COLUMN authority;
