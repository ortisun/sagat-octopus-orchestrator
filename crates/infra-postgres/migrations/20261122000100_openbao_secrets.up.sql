-- FR-017-005..009: values live in OpenBao, metadata/authority and recoverable intents in Sagat.
ALTER TABLE secrets ADD COLUMN authority text NOT NULL DEFAULT 'local'
    CHECK (authority IN ('local', 'openbao'));
CREATE TABLE secret_provider_operations (
    id uuid PRIMARY KEY,
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    namespace_id uuid NOT NULL,
    secret_id uuid NOT NULL,
    name text NOT NULL CHECK (name ~ '^[a-z][a-z0-9._-]{0,63}$'),
    actor text NOT NULL CHECK (char_length(actor) BETWEEN 1 AND 255 AND actor !~ '[[:cntrl:]]'),
    request_id uuid NOT NULL,
    expected_version integer NOT NULL CHECK (expected_version BETWEEN 0 AND 2147483646),
    binding bytea NOT NULL CHECK (octet_length(binding) = 32),
    value_bytes bigint NOT NULL CHECK (value_bytes BETWEEN 1 AND 16384),
    state text NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'active', 'abandoned')),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    activated_at timestamptz,
    source_version uuid,
    pools text[] CHECK (cardinality(pools) <= 16),
    FOREIGN KEY (tenant_id, namespace_id) REFERENCES namespaces(tenant_id,id),
    UNIQUE (tenant_id, namespace_id, actor, request_id),
    UNIQUE (tenant_id, secret_id, id)
);
CREATE UNIQUE INDEX secret_provider_pending_name ON secret_provider_operations(tenant_id,namespace_id,name) WHERE state = 'pending';
CREATE INDEX secret_provider_cleanup ON secret_provider_operations(created_at, id);
CREATE FUNCTION sagat_secret_provider_immutable() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    IF ROW(NEW.id,NEW.tenant_id,NEW.namespace_id,NEW.secret_id,NEW.name,NEW.actor,NEW.request_id,NEW.expected_version,NEW.binding,NEW.value_bytes,NEW.created_at,NEW.source_version,NEW.pools)
       IS DISTINCT FROM ROW(OLD.id,OLD.tenant_id,OLD.namespace_id,OLD.secret_id,OLD.name,OLD.actor,OLD.request_id,OLD.expected_version,OLD.binding,OLD.value_bytes,OLD.created_at,OLD.source_version,OLD.pools)
       OR (NEW.state <> OLD.state AND NOT ((OLD.state='pending' AND NEW.state IN ('active','abandoned')) OR (OLD.state='active' AND NEW.state='abandoned'))) THEN
        RAISE EXCEPTION 'provider intent identity and content binding are immutable' USING ERRCODE='check_violation';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER secret_provider_immutable BEFORE UPDATE ON secret_provider_operations
    FOR EACH ROW EXECUTE FUNCTION sagat_secret_provider_immutable();
ALTER TABLE secret_provider_operations ENABLE ALWAYS TRIGGER secret_provider_immutable;

ALTER TABLE secret_versions
    ADD COLUMN provider_operation uuid,
    ALTER COLUMN key_id DROP NOT NULL,
    ALTER COLUMN nonce DROP NOT NULL,
    ALTER COLUMN ciphertext DROP NOT NULL,
    ADD CONSTRAINT secret_versions_representation CHECK (
        (provider_operation IS NULL AND key_id IS NOT NULL AND nonce IS NOT NULL AND ciphertext IS NOT NULL)
        OR (provider_operation IS NOT NULL AND key_id IS NULL AND nonce IS NULL AND ciphertext IS NULL)),
    ADD CONSTRAINT secret_versions_provider_fk FOREIGN KEY (tenant_id,secret_id,provider_operation)
        REFERENCES secret_provider_operations(tenant_id,secret_id,id);

-- Reserve the actual value size at intent admission, including receipts retained for recovery.
-- The provider cannot bypass tenant content limits simply because its bytes are off-platform.
CREATE FUNCTION sagat_secret_provider_quota_delta() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    IF TG_OP <> 'INSERT' THEN
        PERFORM public.sagat_quota_add_delta(OLD.tenant_id, -OLD.value_bytes, 0, 0);
    END IF;
    IF TG_OP <> 'DELETE' THEN
        PERFORM public.sagat_quota_add_delta(NEW.tenant_id, NEW.value_bytes, 0, 0);
    END IF;
    RETURN NULL;
END
$$;
REVOKE ALL ON FUNCTION sagat_secret_provider_quota_delta() FROM PUBLIC;
CREATE TRIGGER secret_provider_quota_delta AFTER INSERT OR DELETE ON secret_provider_operations
    FOR EACH ROW EXECUTE FUNCTION sagat_secret_provider_quota_delta();
ALTER TABLE secret_provider_operations ENABLE ALWAYS TRIGGER secret_provider_quota_delta;
CREATE CONSTRAINT TRIGGER secret_provider_quota_flush AFTER INSERT OR DELETE ON secret_provider_operations
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE secret_provider_operations ENABLE ALWAYS TRIGGER secret_provider_quota_flush;
DO $grants$ BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON secret_provider_operations TO sagat_app;
    END IF;
END $grants$;
