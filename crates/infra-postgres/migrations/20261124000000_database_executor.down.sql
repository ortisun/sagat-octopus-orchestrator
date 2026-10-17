DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM step_database_credentials) OR EXISTS (SELECT 1 FROM memberships WHERE role='database-runner') THEN
        RAISE EXCEPTION 'database attempt pins and executor identities must be retired before downgrade';
    END IF;
END $$;
DROP TABLE step_database_credentials;
DROP FUNCTION sagat_database_credential_immutable();
ALTER TABLE secret_provider_operations DROP CONSTRAINT secret_provider_scope_unique;
ALTER TABLE memberships DROP CONSTRAINT memberships_role_valid;
ALTER TABLE memberships ADD CONSTRAINT memberships_role_valid
    CHECK (role IN ('admin','developer','operator','viewer','worker','runner'));
