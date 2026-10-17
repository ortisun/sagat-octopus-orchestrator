-- FR-017-010..014: a separate machine identity and durable credential evidence, never plaintext.
ALTER TABLE memberships DROP CONSTRAINT memberships_role_valid;
ALTER TABLE memberships ADD CONSTRAINT memberships_role_valid
    CHECK (role IN ('admin','developer','operator','viewer','worker','runner','database-runner'));
ALTER TABLE secret_provider_operations ADD CONSTRAINT secret_provider_scope_unique
    UNIQUE (tenant_id,namespace_id,id);
CREATE TABLE step_database_credentials (
    tenant_id uuid NOT NULL,
    namespace_id uuid NOT NULL,
    run_id uuid NOT NULL,
    node_id text NOT NULL CHECK (octet_length(node_id) BETWEEN 1 AND 64),
    attempt integer NOT NULL CHECK (attempt > 0),
    generation bigint NOT NULL CHECK (generation > 0),
    provider_operation uuid NOT NULL,
    secret_name text NOT NULL CHECK (secret_name ~ '^[a-z][a-z0-9._-]{0,63}$'),
    secret_version integer NOT NULL CHECK (secret_version > 0),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (tenant_id,run_id,node_id,attempt,generation),
    FOREIGN KEY (tenant_id,namespace_id,run_id) REFERENCES runs(tenant_id,namespace_id,id) ON DELETE CASCADE,
    FOREIGN KEY (tenant_id,run_id,node_id) REFERENCES steps(tenant_id,run_id,node) ON DELETE CASCADE,
    FOREIGN KEY (tenant_id,namespace_id,provider_operation) REFERENCES secret_provider_operations(tenant_id,namespace_id,id)
);
CREATE INDEX step_database_provider_pin ON step_database_credentials(tenant_id,namespace_id,provider_operation);
CREATE FUNCTION sagat_database_credential_immutable() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, pg_temp AS $$ BEGIN
    RAISE EXCEPTION 'database attempt credential pins are immutable' USING ERRCODE='check_violation';
END $$;
CREATE TRIGGER step_database_credential_immutable BEFORE UPDATE ON step_database_credentials
    FOR EACH ROW EXECUTE FUNCTION sagat_database_credential_immutable();
ALTER TABLE step_database_credentials ENABLE ALWAYS TRIGGER step_database_credential_immutable;
DO $grants$ BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname='sagat_app') THEN
        GRANT SELECT,INSERT,DELETE ON step_database_credentials TO sagat_app;
    END IF;
END $grants$;
