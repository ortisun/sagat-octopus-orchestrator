-- Feature 022, release 0.3.0 slice (T022-069, T022-025, G1187; D022-041): the explicit groups each published version
-- declares, so that publication can warn `CONCURRENCY_LIMIT_MISMATCH` and `RATE_LIMIT_MISMATCH` when another published
-- definition of the namespace declares the same group with a different limit (or window) without parsing every other
-- document. Written once with the version, in the publication transaction; never changed, never deleted (a version is
-- immutable and never deleted either). A derived group (`=workflow`, `=workflow/node`) belongs to one definition and is
-- not listed.
--
-- Additive: a new table that no older build reads. It is filled only for versions published by this build; no version
-- published before it declares a gate (publication of a gate was refused, D022-037).
CREATE TABLE flow_control_group_declarations (
    tenant_id      uuid    NOT NULL,
    namespace_id   uuid    NOT NULL,
    workflow_id    uuid    NOT NULL,
    version_id     uuid    NOT NULL,
    gate           text    NOT NULL,
    -- '' for the run-level declaration, else the root node that declares it
    node           text    NOT NULL,
    group_name     text    NOT NULL,
    declared_limit integer NOT NULL,
    -- the window of a rate limit; NULL for concurrency
    period_ms      integer,
    PRIMARY KEY (tenant_id, version_id, gate, node),
    CONSTRAINT flow_control_group_declarations_version_fk FOREIGN KEY (tenant_id, version_id)
        REFERENCES workflow_versions (tenant_id, id),
    CONSTRAINT flow_control_group_declarations_workflow_fk FOREIGN KEY (tenant_id, workflow_id)
        REFERENCES workflows (tenant_id, id),
    CONSTRAINT flow_control_group_declarations_namespace_fk FOREIGN KEY (tenant_id, namespace_id)
        REFERENCES namespaces (tenant_id, id),
    CONSTRAINT flow_control_group_declarations_gate_valid CHECK (gate IN ('CONCURRENCY', 'RATE_LIMIT')),
    CONSTRAINT flow_control_group_declarations_node_shape CHECK (
        node = '' OR node ~ '^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$'),
    CONSTRAINT flow_control_group_declarations_group_explicit CHECK (group_name ~ '^[a-z][a-z0-9.-]{0,63}$'),
    CONSTRAINT flow_control_group_declarations_limit_valid CHECK (declared_limit BETWEEN 1 AND 10000),
    CONSTRAINT flow_control_group_declarations_period_shape CHECK (
        (gate = 'CONCURRENCY' AND period_ms IS NULL)
        OR (gate = 'RATE_LIMIT' AND period_ms IN (1000, 60000, 3600000)))
);
-- what publication reads: the declarations of one group in one namespace
CREATE INDEX flow_control_group_declarations_group_idx
    ON flow_control_group_declarations (tenant_id, namespace_id, gate, group_name);

-- A declaration is what the version said: written with it, never changed or removed.
CREATE FUNCTION flow_control_group_declarations_immutable() RETURNS trigger
    LANGUAGE plpgsql SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    RAISE EXCEPTION 'a declaration of a published version is never changed or removed'
        USING ERRCODE = 'restrict_violation';
END
$$;
CREATE TRIGGER flow_control_group_declarations_immutable BEFORE UPDATE OR DELETE ON flow_control_group_declarations
    FOR EACH ROW EXECUTE FUNCTION flow_control_group_declarations_immutable();
CREATE TRIGGER flow_control_group_declarations_no_truncate BEFORE TRUNCATE ON flow_control_group_declarations
    FOR EACH STATEMENT EXECUTE FUNCTION flow_control_group_declarations_immutable();
ALTER TABLE flow_control_group_declarations ENABLE ALWAYS TRIGGER flow_control_group_declarations_immutable;
ALTER TABLE flow_control_group_declarations ENABLE ALWAYS TRIGGER flow_control_group_declarations_no_truncate;

REVOKE ALL PRIVILEGES ON flow_control_group_declarations FROM PUBLIC;
DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT ON flow_control_group_declarations TO sagat_app;
    END IF;
END
$grants$;
