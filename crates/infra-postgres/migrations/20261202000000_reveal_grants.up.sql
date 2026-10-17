-- Namespace-scoped grants for explicit PII payload reveal (feature 016, FR-016-011).
--
-- Membership is the authority for the subject. Removing that membership revokes every reveal
-- grant through the composite cascading foreign key. The namespace and subject always carry the
-- tenant, so a cross-tenant grant cannot be represented.

CREATE TABLE payload_reveal_grants (
    tenant_id   uuid        NOT NULL,
    namespace_id uuid       NOT NULL,
    subject     text        NOT NULL,
    granted_by  text        NOT NULL,
    granted_at  timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, namespace_id, subject),
    CONSTRAINT payload_reveal_grants_subject_valid
        CHECK (char_length(subject) BETWEEN 1 AND 255 AND subject !~ '[[:cntrl:]]'),
    CONSTRAINT payload_reveal_grants_granted_by_valid
        CHECK (char_length(granted_by) BETWEEN 1 AND 255 AND granted_by !~ '[[:cntrl:]]'),
    CONSTRAINT payload_reveal_grants_namespace_same_tenant_fk
        FOREIGN KEY (tenant_id, namespace_id) REFERENCES namespaces (tenant_id, id),
    CONSTRAINT payload_reveal_grants_membership_same_tenant_fk
        FOREIGN KEY (tenant_id, subject) REFERENCES memberships (tenant_id, subject)
        ON DELETE CASCADE
);

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT, DELETE ON payload_reveal_grants TO sagat_app;
    END IF;
END
$grants$;
