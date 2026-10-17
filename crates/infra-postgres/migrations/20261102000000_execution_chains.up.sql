-- Chain metadata is a projection of the run journal. Handoff must update these rows,
-- both runs, journal, quota, audit and outbox in one transaction before it is enabled.
ALTER TABLE runs ADD CONSTRAINT runs_tenant_namespace_id_uniq
    UNIQUE (tenant_id, namespace_id, id);

CREATE TABLE execution_chains (
    tenant_id uuid NOT NULL,
    namespace_id uuid NOT NULL,
    chain_id uuid NOT NULL,
    current_run_id uuid NOT NULL,
    definition_version_id uuid NOT NULL,
    started_by text NOT NULL,
    max_segments integer NOT NULL,
    original_deadline_at timestamptz NOT NULL,
    segment_count integer NOT NULL DEFAULT 1,
    revision bigint NOT NULL DEFAULT 0,
    cancel_requested boolean NOT NULL DEFAULT false,
    final_outcome text,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, chain_id),
    CONSTRAINT execution_chains_namespace_unique UNIQUE (tenant_id, namespace_id, chain_id),
    CONSTRAINT execution_chains_namespace_fk FOREIGN KEY (tenant_id, namespace_id)
        REFERENCES namespaces (tenant_id, id),
    CONSTRAINT execution_chains_initial_run_fk FOREIGN KEY (tenant_id, namespace_id, chain_id)
        REFERENCES runs (tenant_id, namespace_id, id),
    CONSTRAINT execution_chains_current_run_fk FOREIGN KEY (tenant_id, namespace_id, current_run_id)
        REFERENCES runs (tenant_id, namespace_id, id),
    CONSTRAINT execution_chains_version_fk FOREIGN KEY (tenant_id, definition_version_id)
        REFERENCES workflow_versions (tenant_id, id),
    CONSTRAINT execution_chains_limits CHECK (
        max_segments BETWEEN 1 AND 10000
        AND segment_count BETWEEN 1 AND max_segments
        AND revision >= 0),
    CONSTRAINT execution_chains_outcome CHECK (final_outcome IS NULL OR final_outcome IN
        ('SUCCEEDED', 'FAILED', 'CANCELLED', 'TIMED_OUT', 'COMPENSATED', 'PARTIAL', 'COMPENSATION_FAILED'))
);

CREATE UNIQUE INDEX execution_chains_current_head_unique
    ON execution_chains (tenant_id, current_run_id);

CREATE TABLE chain_segments (
    tenant_id uuid NOT NULL,
    namespace_id uuid NOT NULL,
    chain_id uuid NOT NULL,
    segment_index integer NOT NULL,
    run_id uuid NOT NULL,
    predecessor_run_id uuid,
    successor_run_id uuid,
    completion_kind text,
    transfer_input_digest bytea,
    transfer_state_digest bytea,
    created_at timestamptz NOT NULL,
    completed_at timestamptz,
    PRIMARY KEY (tenant_id, chain_id, segment_index),
    CONSTRAINT chain_segments_chain_fk FOREIGN KEY (tenant_id, namespace_id, chain_id)
        REFERENCES execution_chains (tenant_id, namespace_id, chain_id),
    CONSTRAINT chain_segments_run_fk FOREIGN KEY (tenant_id, namespace_id, run_id)
        REFERENCES runs (tenant_id, namespace_id, id),
    CONSTRAINT chain_segments_predecessor_fk FOREIGN KEY (tenant_id, namespace_id, predecessor_run_id)
        REFERENCES runs (tenant_id, namespace_id, id),
    CONSTRAINT chain_segments_successor_fk FOREIGN KEY (tenant_id, namespace_id, successor_run_id)
        REFERENCES runs (tenant_id, namespace_id, id),
    CONSTRAINT chain_segments_input_fk FOREIGN KEY (tenant_id, transfer_input_digest)
        REFERENCES payloads (tenant_id, digest),
    CONSTRAINT chain_segments_state_fk FOREIGN KEY (tenant_id, transfer_state_digest)
        REFERENCES payloads (tenant_id, digest),
    CONSTRAINT chain_segments_index CHECK (segment_index BETWEEN 0 AND 9999),
    CONSTRAINT chain_segments_completion CHECK (
        (completion_kind IS NULL AND completed_at IS NULL AND successor_run_id IS NULL
            AND transfer_input_digest IS NULL AND transfer_state_digest IS NULL)
        OR (completion_kind = 'FINAL' AND completed_at IS NOT NULL AND successor_run_id IS NULL
            AND transfer_input_digest IS NULL AND transfer_state_digest IS NULL)
        OR (completion_kind = 'CONTINUED' AND completed_at IS NOT NULL AND successor_run_id IS NOT NULL
            AND transfer_input_digest IS NOT NULL)),
    CONSTRAINT chain_segments_predecessor CHECK ((segment_index = 0) = (predecessor_run_id IS NULL)),
    CONSTRAINT chain_segments_distinct_runs CHECK (
        (predecessor_run_id IS NULL OR predecessor_run_id <> run_id)
        AND (successor_run_id IS NULL OR successor_run_id <> run_id))
);

CREATE UNIQUE INDEX chain_segments_run_unique ON chain_segments (tenant_id, run_id);
ALTER TABLE chain_segments ADD CONSTRAINT chain_segments_chain_run_unique
    UNIQUE (tenant_id, namespace_id, chain_id, run_id);
ALTER TABLE chain_segments ADD CONSTRAINT chain_segments_predecessor_member_fk
    FOREIGN KEY (tenant_id, namespace_id, chain_id, predecessor_run_id)
    REFERENCES chain_segments (tenant_id, namespace_id, chain_id, run_id)
    DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE chain_segments ADD CONSTRAINT chain_segments_successor_member_fk
    FOREIGN KEY (tenant_id, namespace_id, chain_id, successor_run_id)
    REFERENCES chain_segments (tenant_id, namespace_id, chain_id, run_id)
    DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE execution_chains ADD CONSTRAINT execution_chains_head_segment_fk
    FOREIGN KEY (tenant_id, namespace_id, chain_id, current_run_id)
    REFERENCES chain_segments (tenant_id, namespace_id, chain_id, run_id)
    DEFERRABLE INITIALLY DEFERRED;
CREATE UNIQUE INDEX chain_segments_predecessor_unique
    ON chain_segments (tenant_id, predecessor_run_id) WHERE predecessor_run_id IS NOT NULL;
CREATE UNIQUE INDEX chain_segments_successor_unique
    ON chain_segments (tenant_id, successor_run_id) WHERE successor_run_id IS NOT NULL;

CREATE TABLE child_chain_links (
    tenant_id uuid NOT NULL,
    namespace_id uuid NOT NULL,
    parent_run_id uuid NOT NULL,
    parent_instance_id uuid NOT NULL,
    child_chain_id uuid NOT NULL,
    cancel_requested boolean NOT NULL DEFAULT false,
    final_outcome text,
    settled boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, parent_run_id, parent_instance_id),
    CONSTRAINT child_chain_links_parent_fk FOREIGN KEY (tenant_id, namespace_id, parent_run_id)
        REFERENCES runs (tenant_id, namespace_id, id),
    CONSTRAINT child_chain_links_chain_fk FOREIGN KEY (tenant_id, namespace_id, child_chain_id)
        REFERENCES execution_chains (tenant_id, namespace_id, chain_id),
    CONSTRAINT child_chain_links_outcome CHECK (final_outcome IS NULL OR final_outcome IN
        ('SUCCEEDED', 'FAILED', 'CANCELLED', 'TIMED_OUT', 'COMPENSATED', 'PARTIAL', 'COMPENSATION_FAILED')),
    CONSTRAINT child_chain_links_settlement CHECK (NOT settled OR final_outcome IS NOT NULL),
    CONSTRAINT child_chain_links_distinct_runs CHECK (parent_run_id <> child_chain_id)
);

CREATE UNIQUE INDEX child_chain_links_child_unique
    ON child_chain_links (tenant_id, child_chain_id);

CREATE FUNCTION execution_chain_identity_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_TABLE_NAME = 'execution_chains' THEN
        IF (OLD.tenant_id, OLD.namespace_id, OLD.chain_id, OLD.definition_version_id,
            OLD.started_by, OLD.max_segments, OLD.original_deadline_at)
            IS DISTINCT FROM
           (NEW.tenant_id, NEW.namespace_id, NEW.chain_id, NEW.definition_version_id,
            NEW.started_by, NEW.max_segments, NEW.original_deadline_at) THEN
            RAISE EXCEPTION 'execution chain identity is immutable' USING ERRCODE = '23514';
        END IF;
    ELSIF TG_TABLE_NAME = 'chain_segments' THEN
        IF (OLD.tenant_id, OLD.namespace_id, OLD.chain_id, OLD.segment_index,
            OLD.run_id, OLD.predecessor_run_id, OLD.created_at)
            IS DISTINCT FROM
           (NEW.tenant_id, NEW.namespace_id, NEW.chain_id, NEW.segment_index,
            NEW.run_id, NEW.predecessor_run_id, NEW.created_at) THEN
            RAISE EXCEPTION 'chain segment identity is immutable' USING ERRCODE = '23514';
        END IF;
        IF OLD.completion_kind IS NOT NULL AND
           (OLD.completion_kind, OLD.successor_run_id, OLD.transfer_input_digest,
            OLD.transfer_state_digest, OLD.completed_at) IS DISTINCT FROM
           (NEW.completion_kind, NEW.successor_run_id, NEW.transfer_input_digest,
            NEW.transfer_state_digest, NEW.completed_at) THEN
            RAISE EXCEPTION 'chain segment completion is immutable' USING ERRCODE = '23514';
        END IF;
    ELSIF TG_TABLE_NAME = 'child_chain_links' THEN
        IF (OLD.tenant_id, OLD.namespace_id, OLD.parent_run_id,
            OLD.parent_instance_id, OLD.child_chain_id, OLD.created_at)
            IS DISTINCT FROM
           (NEW.tenant_id, NEW.namespace_id, NEW.parent_run_id,
            NEW.parent_instance_id, NEW.child_chain_id, NEW.created_at) THEN
            RAISE EXCEPTION 'child chain link identity is immutable' USING ERRCODE = '23514';
        END IF;
    END IF;
    RETURN NEW;
END
$$;

CREATE TRIGGER execution_chains_identity_guard BEFORE UPDATE ON execution_chains
    FOR EACH ROW EXECUTE FUNCTION execution_chain_identity_guard();
CREATE TRIGGER chain_segments_identity_guard BEFORE UPDATE ON chain_segments
    FOR EACH ROW EXECUTE FUNCTION execution_chain_identity_guard();
CREATE TRIGGER child_chain_links_identity_guard BEFORE UPDATE ON child_chain_links
    FOR EACH ROW EXECUTE FUNCTION execution_chain_identity_guard();
ALTER TABLE execution_chains ENABLE ALWAYS TRIGGER execution_chains_identity_guard;
ALTER TABLE chain_segments ENABLE ALWAYS TRIGGER chain_segments_identity_guard;
ALTER TABLE child_chain_links ENABLE ALWAYS TRIGGER child_chain_links_identity_guard;

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT, UPDATE ON execution_chains TO sagat_app;
        GRANT SELECT, INSERT, UPDATE ON chain_segments TO sagat_app;
        GRANT SELECT, INSERT, UPDATE ON child_chain_links TO sagat_app;
    END IF;
END
$grants$;
