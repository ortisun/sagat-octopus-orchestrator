-- Dormant immutable metadata; no source writer or production caller is enabled.
-- D298 retains certificates and their original sources without expiry.
CREATE TABLE leaf_collection_success_certifications (
    tenant_id uuid NOT NULL,
    namespace_id uuid NOT NULL,
    run_id uuid NOT NULL,
    owner_node text NOT NULL,
    iteration_index integer NOT NULL,
    profile text NOT NULL,
    definition_version_id uuid NOT NULL,
    scope_id uuid NOT NULL,
    body_node text NOT NULL,
    return_node text NOT NULL,
    admitted_sequence bigint NOT NULL,
    returned_sequence bigint NOT NULL,
    checkpoint_sequence bigint NOT NULL,
    checkpoint_result_digest bytea NOT NULL,
    finished_sequence bigint NOT NULL,
    aggregate_result_digest bytea NOT NULL,
    owner_routed_sequence bigint NOT NULL,
    certified_run_revision bigint NOT NULL,
    certified_at timestamptz NOT NULL,
    audit_id uuid NOT NULL,
    PRIMARY KEY (tenant_id,namespace_id,run_id,owner_node,iteration_index),
    CONSTRAINT leaf_success_original_path_unique UNIQUE (tenant_id,run_id,owner_node,iteration_index),
    CONSTRAINT leaf_success_audit_unique UNIQUE (audit_id),
    CONSTRAINT leaf_success_run_fk FOREIGN KEY (tenant_id,namespace_id,run_id)
        REFERENCES runs (tenant_id,namespace_id,id),
    -- These scalar references are checked against actual source facts by the future adapter.
    -- No child/source/audit/outbox FK and no claim that an INSERT establishes source closure.
    CONSTRAINT leaf_success_owner CHECK (owner_node ~ '^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$'),
    CONSTRAINT leaf_success_body CHECK (body_node ~ '^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$'),
    CONSTRAINT leaf_success_return CHECK (return_node ~ '^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$'),
    CONSTRAINT leaf_success_index CHECK (iteration_index BETWEEN 0 AND 9999),
    CONSTRAINT leaf_success_profile CHECK (profile='ORDINARY_LEAF_V1'),
    CONSTRAINT leaf_success_digests CHECK (
        octet_length(checkpoint_result_digest)=32 AND octet_length(aggregate_result_digest)=32),
    CONSTRAINT leaf_success_sequences CHECK (
        admitted_sequence>=1 AND returned_sequence>=1 AND checkpoint_sequence>=1
        AND finished_sequence>=1 AND owner_routed_sequence>=1 AND certified_run_revision>=1
        AND admitted_sequence<returned_sequence
        AND checkpoint_sequence-returned_sequence=1
        AND checkpoint_sequence<finished_sequence
        AND owner_routed_sequence-finished_sequence=1
        AND owner_routed_sequence<=certified_run_revision),
    CONSTRAINT leaf_success_finite_time CHECK (isfinite(certified_at))
);
CREATE TRIGGER leaf_success_append_only BEFORE UPDATE OR DELETE ON leaf_collection_success_certifications
    FOR EACH ROW EXECUTE FUNCTION execution_reject_mutation();
CREATE TRIGGER leaf_success_no_truncate BEFORE TRUNCATE ON leaf_collection_success_certifications
    FOR EACH STATEMENT EXECUTE FUNCTION execution_reject_mutation();
ALTER TABLE leaf_collection_success_certifications ENABLE ALWAYS TRIGGER leaf_success_append_only;
ALTER TABLE leaf_collection_success_certifications ENABLE ALWAYS TRIGGER leaf_success_no_truncate;

-- Unconditional erasure rejection avoids certificate-lookup races and introduces no row-lock edge.
-- INSERT/UPDATE producer preservation is deliberately NOT established by these retention guards.
DO $retention$
DECLARE source_table text;
BEGIN
    FOREACH source_table IN ARRAY ARRAY['runs','steps','iteration_scopes','collection_scope_items',
        'collection_iterations','iteration_checkpoints','scope_sagas','scope_saga_entries'] LOOP
        EXECUTE format('CREATE TRIGGER leaf_source_no_delete BEFORE DELETE ON %I FOR EACH ROW EXECUTE FUNCTION execution_reject_mutation()',source_table);
        EXECUTE format('CREATE TRIGGER leaf_source_no_truncate BEFORE TRUNCATE ON %I FOR EACH STATEMENT EXECUTE FUNCTION execution_reject_mutation()',source_table);
        EXECUTE format('ALTER TABLE %I ENABLE ALWAYS TRIGGER leaf_source_no_delete',source_table);
        EXECUTE format('ALTER TABLE %I ENABLE ALWAYS TRIGGER leaf_source_no_truncate',source_table);
        EXECUTE format('REVOKE DELETE, TRUNCATE ON %I FROM PUBLIC',source_table);
        IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname='sagat_app') THEN
            EXECUTE format('REVOKE DELETE, TRUNCATE ON %I FROM sagat_app',source_table);
        END IF;
    END LOOP;
END $retention$;

-- Ordinary FK checks can be disabled in replica sessions: retain the original run key explicitly.
CREATE FUNCTION leaf_source_run_identity_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.id IS DISTINCT FROM NEW.id THEN
        RAISE EXCEPTION 'original run identity is immutable' USING ERRCODE='23001';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER leaf_source_run_identity BEFORE UPDATE OF id ON runs
    FOR EACH ROW EXECUTE FUNCTION leaf_source_run_identity_guard();
ALTER TABLE runs ENABLE ALWAYS TRIGGER leaf_source_run_identity;
REVOKE ALL ON FUNCTION leaf_source_run_identity_guard() FROM PUBLIC;
REVOKE ALL PRIVILEGES ON leaf_collection_success_certifications FROM PUBLIC;
DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname='sagat_app') THEN
        REVOKE ALL PRIVILEGES ON leaf_collection_success_certifications FROM sagat_app;
        -- INSERT is a separate adapter/proof increment; this dormant schema is read-only at runtime.
        GRANT SELECT ON leaf_collection_success_certifications TO sagat_app;
    END IF;
END $grants$;
