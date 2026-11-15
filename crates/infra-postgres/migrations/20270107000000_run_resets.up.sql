-- Reset to a point (feature 018, FR-018-036/037): the register of resets. A reset creates a run from the prefix of
-- another; the entry says which run and point, what was asked, by whom and why, and what became of it. The
-- run's own journal (`run.reset`, `runs.reset_sequence`, `rerun_of`) says the same from the run's side; this
-- table is what the source is asked ("the resets made from this run") and what the API names by an id.
--
-- Additive: an older engine or control plane never reads this table.

CREATE TABLE run_resets (
    tenant_id         uuid        NOT NULL REFERENCES tenants (id),
    namespace_id      uuid        NOT NULL,
    id                uuid        NOT NULL,
    source_run_id     uuid        NOT NULL,
    -- the sequence of the last event of the source that the new run keeps
    source_sequence   bigint      NOT NULL,
    -- the SHA-256 of the canonical encoding of the rows 1..N, which `run.reset` carries too
    prefix_digest     bytea       NOT NULL,
    reapply           text        NOT NULL,
    business_id_policy text       NOT NULL,
    source_policy     text        NOT NULL,
    state             text        NOT NULL,
    failure_reason    text,
    new_run_id        uuid,
    actor             text        NOT NULL,
    -- the justification, customer text
    reason            text        NOT NULL,
    -- what the reapply did, a closed list of {sequence, kind, outcome, reason}
    reapply_report    jsonb       NOT NULL DEFAULT '[]',
    shard             smallint    GENERATED ALWAYS AS (run_shard(id)) STORED,
    created_at        timestamptz NOT NULL,
    completed_at      timestamptz,
    PRIMARY KEY (tenant_id, id),
    CONSTRAINT run_resets_namespace_same_tenant_fk
        FOREIGN KEY (tenant_id, namespace_id) REFERENCES namespaces (tenant_id, id),
    CONSTRAINT run_resets_source_same_tenant_fk
        FOREIGN KEY (tenant_id, source_run_id) REFERENCES runs (tenant_id, id),
    CONSTRAINT run_resets_new_run_same_tenant_fk
        FOREIGN KEY (tenant_id, new_run_id) REFERENCES runs (tenant_id, id),
    CONSTRAINT run_resets_sequence_valid CHECK (source_sequence >= 1),
    CONSTRAINT run_resets_prefix_digest_size CHECK (octet_length(prefix_digest) = 32),
    CONSTRAINT run_resets_reapply_valid CHECK (reapply IN ('NONE', 'SIGNALS', 'UPDATES', 'ALL')),
    CONSTRAINT run_resets_business_policy_valid CHECK (business_id_policy IN ('KEEP', 'DROP')),
    CONSTRAINT run_resets_source_policy_valid CHECK (source_policy IN ('REQUIRE_TERMINAL', 'CANCEL_SOURCE')),
    CONSTRAINT run_resets_state_valid CHECK (state IN ('PENDING_SOURCE', 'CREATED', 'FAILED')),
    CONSTRAINT run_resets_failure_reason_valid CHECK (failure_reason IS NULL OR failure_reason IN
        ('BOUNDARY_NO_LONGER_ELIGIBLE', 'ADMISSION_REFUSED', 'BUSINESS_ID_IN_USE',
         'CONCURRENCY_PERMIT_UNAVAILABLE', 'RUN_PURGED')),
    CONSTRAINT run_resets_failure_only_when_failed CHECK ((state = 'FAILED') = (failure_reason IS NOT NULL)),
    CONSTRAINT run_resets_run_only_when_created CHECK ((state = 'CREATED') = (new_run_id IS NOT NULL)),
    CONSTRAINT run_resets_completed_consistent CHECK ((state = 'PENDING_SOURCE') = (completed_at IS NULL)),
    CONSTRAINT run_resets_actor_bounded CHECK (char_length(actor) BETWEEN 1 AND 255),
    CONSTRAINT run_resets_reason_bounded CHECK (char_length(reason) BETWEEN 8 AND 2000),
    CONSTRAINT run_resets_report_is_array CHECK (jsonb_typeof(reapply_report) = 'array'),
    CONSTRAINT run_resets_report_bounded CHECK (
        jsonb_array_length(reapply_report) <= 256 AND octet_length(reapply_report::text) <= 65536)
);
-- The resets made from a run, newest first (ids are time-ordered).
CREATE INDEX run_resets_source_idx ON run_resets (tenant_id, source_run_id, id DESC);
-- A run is made by at most one reset, and the run says which.
CREATE UNIQUE INDEX run_resets_new_run_idx ON run_resets (tenant_id, new_run_id) WHERE new_run_id IS NOT NULL;
-- The resets that wait for their source to end, for the pass that completes them.
CREATE INDEX run_resets_pending_idx ON run_resets (shard, created_at) WHERE state = 'PENDING_SOURCE';

-- What a reset is, and why it exists, never changes; only a pending one is completed, once.
CREATE FUNCTION run_resets_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        IF OLD.state = 'PENDING_SOURCE' THEN
            RAISE EXCEPTION 'a reset that waits for its source is never purged'
                USING ERRCODE = 'restrict_violation';
        END IF;
        RETURN OLD;
    END IF;
    IF NEW.tenant_id IS DISTINCT FROM OLD.tenant_id
        OR NEW.namespace_id IS DISTINCT FROM OLD.namespace_id
        OR NEW.id IS DISTINCT FROM OLD.id
        OR NEW.source_run_id IS DISTINCT FROM OLD.source_run_id
        OR NEW.source_sequence IS DISTINCT FROM OLD.source_sequence
        OR NEW.prefix_digest IS DISTINCT FROM OLD.prefix_digest
        OR NEW.reapply IS DISTINCT FROM OLD.reapply
        OR NEW.business_id_policy IS DISTINCT FROM OLD.business_id_policy
        OR NEW.source_policy IS DISTINCT FROM OLD.source_policy
        OR NEW.actor IS DISTINCT FROM OLD.actor
        OR NEW.reason IS DISTINCT FROM OLD.reason
        OR NEW.created_at IS DISTINCT FROM OLD.created_at THEN
        RAISE EXCEPTION 'what a reset is never changes; only its outcome does'
            USING ERRCODE = 'restrict_violation';
    END IF;
    IF OLD.state <> 'PENDING_SOURCE' AND (
        NEW.state IS DISTINCT FROM OLD.state
        OR NEW.failure_reason IS DISTINCT FROM OLD.failure_reason
        OR NEW.new_run_id IS DISTINCT FROM OLD.new_run_id
        OR NEW.completed_at IS DISTINCT FROM OLD.completed_at) THEN
        RAISE EXCEPTION 'a reset that has completed stays as it completed'
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER run_resets_guard BEFORE UPDATE OR DELETE ON run_resets
    FOR EACH ROW EXECUTE FUNCTION run_resets_guard();
CREATE TRIGGER run_resets_no_truncate BEFORE TRUNCATE ON run_resets
    FOR EACH STATEMENT EXECUTE FUNCTION execution_reject_mutation();
ALTER TABLE run_resets ENABLE ALWAYS TRIGGER run_resets_guard;
ALTER TABLE run_resets ENABLE ALWAYS TRIGGER run_resets_no_truncate;

-- The justification is customer content: charged to the tenant by the ALWAYS accounting triggers, as the
-- manifest of storage classes says.
CREATE TRIGGER run_resets_quota_delta AFTER INSERT OR UPDATE OR DELETE ON run_resets
    FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'octet_length_utf8', 'reason');
ALTER TABLE run_resets ENABLE ALWAYS TRIGGER run_resets_quota_delta;
CREATE CONSTRAINT TRIGGER run_resets_quota_flush AFTER INSERT OR UPDATE OR DELETE ON run_resets
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE run_resets ENABLE ALWAYS TRIGGER run_resets_quota_flush;

-- The runtime role reads and writes what the stores need. Nothing is deleted by the role: retention runs
-- through a bounded purge function.
REVOKE ALL PRIVILEGES ON run_resets FROM PUBLIC;
DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        REVOKE ALL PRIVILEGES ON run_resets FROM sagat_app;
        GRANT SELECT, INSERT, UPDATE ON run_resets TO sagat_app;
    END IF;
END $grants$;
