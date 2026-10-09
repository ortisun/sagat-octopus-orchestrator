-- Feature 009: the egress policy of a namespace (FR-009-008, US03). One row per namespace; a
-- namespace without a row may reach nothing. The rules are stored in canonical text and parsed by the
-- domain, which refuses what no policy can allow (loopback, link-local and metadata addresses, ...).

CREATE TABLE egress_policies (
    tenant_id    uuid        NOT NULL REFERENCES tenants (id),
    namespace_id uuid        NOT NULL,
    rules        text[]      NOT NULL,
    ports        integer[]   NOT NULL,
    allow_http   boolean     NOT NULL,
    updated_by   text        NOT NULL,
    updated_at   timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (tenant_id, namespace_id),
    CONSTRAINT egress_policies_namespace_fk FOREIGN KEY (tenant_id, namespace_id)
        REFERENCES namespaces (tenant_id, id),
    CONSTRAINT egress_policies_rule_count CHECK (cardinality(rules) <= 256),
    CONSTRAINT egress_policies_port_count CHECK (cardinality(ports) <= 32)
);

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON egress_policies TO sagat_app;
    END IF;
END
$grants$;
