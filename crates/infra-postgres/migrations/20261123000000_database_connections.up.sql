-- FR-017-001..003: stable identities, immutable settings and retained run bindings.
CREATE TABLE database_connections (
    id uuid PRIMARY KEY,
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    namespace_id uuid NOT NULL,
    name text NOT NULL CHECK (name ~ '^[a-z][a-z0-9-]{1,61}[a-z0-9]$' AND name NOT LIKE '%--%'),
    current_revision integer NOT NULL CHECK (current_revision > 0),
    status text NOT NULL CHECK (status IN ('ACTIVE','PAUSED','DELETED')),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (tenant_id,namespace_id,id),
    UNIQUE (tenant_id,namespace_id,name),
    FOREIGN KEY (tenant_id,namespace_id) REFERENCES namespaces(tenant_id,id)
);
CREATE TABLE database_connection_revisions (
    tenant_id uuid NOT NULL,
    namespace_id uuid NOT NULL,
    connection_id uuid NOT NULL,
    revision integer NOT NULL CHECK (revision > 0),
    config jsonb NOT NULL CHECK (jsonb_typeof(config)='object' AND octet_length(config::text) <= 131072),
    status text NOT NULL CHECK (status IN ('ACTIVE','PAUSED','DELETED')),
    actor text NOT NULL CHECK (char_length(actor) BETWEEN 1 AND 255 AND actor !~ '[[:cntrl:]]'),
    request_id uuid NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (tenant_id,namespace_id,connection_id,revision),
    UNIQUE (tenant_id,namespace_id,actor,request_id),
    FOREIGN KEY (tenant_id,namespace_id,connection_id) REFERENCES database_connections(tenant_id,namespace_id,id)
);
ALTER TABLE database_connections ADD CONSTRAINT database_connection_head_fk
    FOREIGN KEY (tenant_id,namespace_id,id,current_revision)
    REFERENCES database_connection_revisions(tenant_id,namespace_id,connection_id,revision)
    DEFERRABLE INITIALLY DEFERRED;
CREATE FUNCTION sagat_database_revision_immutable() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog,pg_temp AS $$
BEGIN
    RAISE EXCEPTION 'database connection revisions are immutable' USING ERRCODE='check_violation';
END $$;
CREATE TRIGGER database_revision_immutable BEFORE UPDATE ON database_connection_revisions
    FOR EACH ROW EXECUTE FUNCTION sagat_database_revision_immutable();
ALTER TABLE database_connection_revisions ENABLE ALWAYS TRIGGER database_revision_immutable;
CREATE FUNCTION sagat_database_identity_immutable() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog,pg_temp AS $$
BEGIN
    IF ROW(NEW.id,NEW.tenant_id,NEW.namespace_id,NEW.name,NEW.created_at)
       IS DISTINCT FROM ROW(OLD.id,OLD.tenant_id,OLD.namespace_id,OLD.name,OLD.created_at)
       OR OLD.status='DELETED' OR NEW.current_revision <> OLD.current_revision + 1 THEN
        RAISE EXCEPTION 'database connection identity or revision transition is invalid' USING ERRCODE='check_violation';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER database_identity_immutable BEFORE UPDATE ON database_connections
    FOR EACH ROW EXECUTE FUNCTION sagat_database_identity_immutable();
ALTER TABLE database_connections ENABLE ALWAYS TRIGGER database_identity_immutable;

ALTER TABLE runs ADD CONSTRAINT runs_database_scope_unique UNIQUE (tenant_id,namespace_id,id);
CREATE TABLE run_database_connections (
    tenant_id uuid NOT NULL,
    namespace_id uuid NOT NULL,
    run_id uuid NOT NULL,
    node_id text NOT NULL CHECK (octet_length(node_id) BETWEEN 1 AND 64),
    connection_id uuid NOT NULL,
    revision integer NOT NULL,
    PRIMARY KEY (tenant_id,run_id,node_id),
    FOREIGN KEY (tenant_id,namespace_id,run_id) REFERENCES runs(tenant_id,namespace_id,id) ON DELETE CASCADE,
    FOREIGN KEY (tenant_id,namespace_id,connection_id,revision)
        REFERENCES database_connection_revisions(tenant_id,namespace_id,connection_id,revision)
);
CREATE TRIGGER run_database_binding_immutable BEFORE UPDATE ON run_database_connections
    FOR EACH ROW EXECUTE FUNCTION sagat_database_revision_immutable();
ALTER TABLE run_database_connections ENABLE ALWAYS TRIGGER run_database_binding_immutable;
CREATE TRIGGER database_connections_quota_delta AFTER INSERT OR UPDATE OR DELETE ON database_connections
    FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content','octet_length_utf8','name');
ALTER TABLE database_connections ENABLE ALWAYS TRIGGER database_connections_quota_delta;
CREATE CONSTRAINT TRIGGER database_connections_quota_flush AFTER INSERT OR UPDATE OR DELETE ON database_connections
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE database_connections ENABLE ALWAYS TRIGGER database_connections_quota_flush;
CREATE TRIGGER database_connection_revisions_quota_delta AFTER INSERT OR UPDATE OR DELETE ON database_connection_revisions
    FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content','canonical_json_utf8','config');
ALTER TABLE database_connection_revisions ENABLE ALWAYS TRIGGER database_connection_revisions_quota_delta;
CREATE CONSTRAINT TRIGGER database_connection_revisions_quota_flush AFTER INSERT OR UPDATE OR DELETE ON database_connection_revisions
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE database_connection_revisions ENABLE ALWAYS TRIGGER database_connection_revisions_quota_flush;
DO $grants$ BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname='sagat_app') THEN
        GRANT SELECT,INSERT,UPDATE,DELETE ON database_connections TO sagat_app;
        GRANT SELECT,INSERT,DELETE ON database_connection_revisions,run_database_connections TO sagat_app;
    END IF;
END $grants$;
