-- FR-017-015: private immutable definitions and their durable run origin.
ALTER TABLE workflows ADD COLUMN internal boolean NOT NULL DEFAULT false;
ALTER TABLE workflows ADD CONSTRAINT workflows_internal_inactive
    CHECK (NOT internal OR active_version_id IS NULL);

CREATE FUNCTION workflows_internal_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.internal OR NEW.internal IS DISTINCT FROM OLD.internal THEN
        RAISE EXCEPTION 'private workflow is immutable' USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER workflows_internal_immutable BEFORE UPDATE ON workflows
    FOR EACH ROW EXECUTE FUNCTION workflows_internal_guard();
ALTER TABLE workflows ENABLE ALWAYS TRIGGER workflows_internal_immutable;

CREATE TABLE private_execution_definitions (
    tenant_id uuid NOT NULL,
    namespace_id uuid NOT NULL,
    actor text NOT NULL,
    digest text NOT NULL CHECK (digest ~ '^[0-9a-f]{64}$'),
    workflow_id uuid NOT NULL,
    version_id uuid NOT NULL,
    PRIMARY KEY (tenant_id, namespace_id, actor, digest),
    UNIQUE (tenant_id, workflow_id),
    UNIQUE (tenant_id, namespace_id, actor, workflow_id, version_id),
    FOREIGN KEY (tenant_id, namespace_id, workflow_id)
        REFERENCES workflows (tenant_id, namespace_id, id),
    FOREIGN KEY (tenant_id, workflow_id, version_id)
        REFERENCES workflow_versions (tenant_id, workflow_id, id)
);
CREATE FUNCTION private_execution_definition_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM workflows w JOIN workflow_versions v
          ON v.tenant_id=w.tenant_id AND v.workflow_id=w.id
        WHERE w.tenant_id=NEW.tenant_id AND w.namespace_id=NEW.namespace_id
          AND w.id=NEW.workflow_id AND w.internal AND v.id=NEW.version_id
          AND v.published_by=NEW.actor AND v.digest=NEW.digest AND v.version_number=1
    ) THEN
        RAISE EXCEPTION 'invalid private definition scope' USING ERRCODE = 'foreign_key_violation';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER private_execution_definition_scope BEFORE INSERT ON private_execution_definitions
    FOR EACH ROW EXECUTE FUNCTION private_execution_definition_guard();
CREATE TRIGGER private_execution_definition_immutable BEFORE UPDATE OR DELETE ON private_execution_definitions
    FOR EACH ROW EXECUTE FUNCTION workflow_versions_reject_mutation();
CREATE TRIGGER private_execution_definition_no_truncate BEFORE TRUNCATE ON private_execution_definitions
    FOR EACH STATEMENT EXECUTE FUNCTION workflow_versions_reject_mutation();
ALTER TABLE private_execution_definitions ENABLE ALWAYS TRIGGER private_execution_definition_scope;
ALTER TABLE private_execution_definitions ENABLE ALWAYS TRIGGER private_execution_definition_immutable;
ALTER TABLE private_execution_definitions ENABLE ALWAYS TRIGGER private_execution_definition_no_truncate;

ALTER TABLE runs ADD COLUMN origin text
    CHECK (origin IN ('AD_HOC_STEP', 'AD_HOC_WORKFLOW', 'CONNECTION_PROBE'));
CREATE INDEX runs_origin_listing_idx ON runs (tenant_id, namespace_id, origin, id DESC)
    WHERE origin IS NOT NULL;
CREATE FUNCTION runs_private_definition_guard() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE is_internal boolean;
BEGIN
    IF TG_OP='UPDATE' AND (NEW.origin, NEW.workflow_id, NEW.workflow_version_id, NEW.started_by,
        NEW.tenant_id, NEW.namespace_id) IS DISTINCT FROM
        (OLD.origin, OLD.workflow_id, OLD.workflow_version_id, OLD.started_by,
         OLD.tenant_id, OLD.namespace_id) THEN
        RAISE EXCEPTION 'run admission identity is immutable' USING ERRCODE='restrict_violation';
    END IF;
    SELECT internal INTO is_internal FROM workflows
        WHERE tenant_id=NEW.tenant_id AND namespace_id=NEW.namespace_id AND id=NEW.workflow_id;
    IF is_internal IS DISTINCT FROM (NEW.origin IS NOT NULL) OR
        (NEW.origin IS NOT NULL AND NOT EXISTS (
            SELECT 1 FROM private_execution_definitions d WHERE d.tenant_id=NEW.tenant_id
              AND d.namespace_id=NEW.namespace_id AND d.actor=NEW.started_by
              AND d.workflow_id=NEW.workflow_id AND d.version_id=NEW.workflow_version_id
        )) THEN
        RAISE EXCEPTION 'invalid private run scope' USING ERRCODE='foreign_key_violation';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER runs_private_definition_scope BEFORE INSERT OR UPDATE OF origin, workflow_id,
    workflow_version_id, started_by, tenant_id, namespace_id ON runs
    FOR EACH ROW EXECUTE FUNCTION runs_private_definition_guard();
ALTER TABLE runs ENABLE ALWAYS TRIGGER runs_private_definition_scope;

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname='sagat_app') THEN
        GRANT SELECT, INSERT ON private_execution_definitions TO sagat_app;
    END IF;
END
$grants$;
