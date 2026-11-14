-- Feature 018: business identifiers (FR-018-018..022, ADR-0050 decision 7). A start may carry a
-- caller's own identifier; at most one open execution holds an identifier of a namespace at any
-- commit. The binding is claimed in the transaction that creates the run, in the lock slot of the
-- run-creating aggregates; the transitions a run goes through never lock or write it, so whether
-- the holder is open is read from the holder (a run, or a chain by its initial run), never copied.
--
-- Additive: one table and one nullable column. Older code neither reads nor writes either.
CREATE TABLE business_ids (
    tenant_id      uuid        NOT NULL REFERENCES tenants (id),
    namespace_id   uuid        NOT NULL,
    -- 1 to 200 visible ASCII characters: customer content, charged to the tenant
    business_id    text        NOT NULL,
    -- the holder: a run, or a chain by its initial run. There is no foreign key: a holder purged by
    -- a retention policy leaves a binding that reads as free, and the purge below removes it.
    execution_id   uuid        NOT NULL,
    -- for display
    workflow_id    uuid        NOT NULL,
    bound_at       timestamptz NOT NULL,
    -- the window in force when the id was bound, so a later change of the setting does not move it
    reuse_window_s integer     NOT NULL,
    PRIMARY KEY (tenant_id, namespace_id, business_id),
    CONSTRAINT business_ids_namespace_same_tenant_fk
        FOREIGN KEY (tenant_id, namespace_id) REFERENCES namespaces (tenant_id, id),
    CONSTRAINT business_ids_visible_ascii CHECK (business_id ~ '^[!-~]{1,200}$'),
    CONSTRAINT business_ids_window_valid CHECK (reuse_window_s BETWEEN 1 AND 31622400)
);
-- The binding of a run, to read the one a holder has.
CREATE INDEX business_ids_holder_idx ON business_ids (tenant_id, execution_id);

-- Who an identifier belongs to never changes; a closed holder's binding is taken over by a new run
-- (the holder, the workflow, the instant and the window change), and nothing else is written.
CREATE FUNCTION business_ids_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    IF NEW.tenant_id IS DISTINCT FROM OLD.tenant_id
        OR NEW.namespace_id IS DISTINCT FROM OLD.namespace_id
        OR NEW.business_id IS DISTINCT FROM OLD.business_id THEN
        RAISE EXCEPTION 'which business identifier a binding is for never changes'
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER business_ids_guard BEFORE UPDATE OR DELETE ON business_ids
    FOR EACH ROW EXECUTE FUNCTION business_ids_guard();
CREATE TRIGGER business_ids_no_truncate BEFORE TRUNCATE ON business_ids
    FOR EACH STATEMENT EXECUTE FUNCTION execution_reject_mutation();
ALTER TABLE business_ids ENABLE ALWAYS TRIGGER business_ids_guard;
ALTER TABLE business_ids ENABLE ALWAYS TRIGGER business_ids_no_truncate;

-- The identifier is customer content, charged to the tenant by the ALWAYS accounting triggers.
CREATE TRIGGER business_ids_quota_delta AFTER INSERT OR UPDATE OR DELETE ON business_ids
    FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'octet_length_utf8', 'business_id');
ALTER TABLE business_ids ENABLE ALWAYS TRIGGER business_ids_quota_delta;
CREATE CONSTRAINT TRIGGER business_ids_quota_flush AFTER INSERT OR UPDATE OR DELETE ON business_ids
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE business_ids ENABLE ALWAYS TRIGGER business_ids_quota_flush;

-- A copy on the run, to list and filter by it and to show it. Null for every run that exists and for
-- every run that was started without one.
ALTER TABLE runs ADD COLUMN business_id text;
ALTER TABLE runs ADD CONSTRAINT runs_business_id_visible_ascii CHECK (
    business_id IS NULL OR business_id ~ '^[!-~]{1,200}$') NOT VALID;
ALTER TABLE runs VALIDATE CONSTRAINT runs_business_id_visible_ascii;
-- By id, and by prefix (`text_pattern_ops` makes `LIKE 'prefix%'` an index range whatever the collation).
CREATE INDEX runs_business_id_idx ON runs (tenant_id, namespace_id, business_id text_pattern_ops)
    WHERE business_id IS NOT NULL;

-- What a run was started under never changes after it is created. The trigger fires only for an UPDATE
-- that names the column, so ordinary updates of a run pay nothing.
CREATE FUNCTION runs_business_id_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.business_id IS DISTINCT FROM OLD.business_id THEN
        RAISE EXCEPTION 'the business identifier of a run never changes'
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER runs_business_id BEFORE UPDATE OF business_id ON runs
    FOR EACH ROW EXECUTE FUNCTION runs_business_id_guard();
ALTER TABLE runs ENABLE ALWAYS TRIGGER runs_business_id;

-- The copy is charged with the correlation id it sits beside.
DROP TRIGGER runs_quota_delta ON runs;
CREATE TRIGGER runs_quota_delta AFTER INSERT OR UPDATE OR DELETE ON runs FOR EACH ROW
    EXECUTE FUNCTION sagat_quota_row_delta('content', 'octet_length_utf8', 'business_id', 'content', 'octet_length_utf8', 'correlation_id');
ALTER TABLE runs ENABLE ALWAYS TRIGGER runs_quota_delta;

-- The runtime role reads and writes what the store needs. UPDATE is what lets it take the row lock of a
-- binding and take over one whose holder closed. It deletes nothing: retention runs through the bounded
-- function below.
REVOKE ALL PRIVILEGES ON business_ids FROM PUBLIC;
DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        REVOKE ALL PRIVILEGES ON business_ids FROM sagat_app;
        GRANT SELECT, INSERT, UPDATE ON business_ids TO sagat_app;
    END IF;
END $grants$;

-- Removes up to `p_rows` bindings whose holder closed and whose window has passed; returns how many.
-- A holder that is gone (purged by a retention policy) counts as closed long ago. A chain is closed when
-- it has a final outcome; a run, when it is terminal and has a finish time.
CREATE FUNCTION business_ids_purge(p_rows integer) RETURNS bigint
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public, pg_temp AS $$
DECLARE
    v_deleted bigint;
BEGIN
    IF p_rows < 1 THEN
        RETURN 0;
    END IF;
    WITH expired AS (
        SELECT b.tenant_id, b.namespace_id, b.business_id
          FROM business_ids b
          LEFT JOIN runs r ON r.tenant_id = b.tenant_id AND r.id = b.execution_id
          LEFT JOIN execution_chains c ON c.tenant_id = b.tenant_id AND c.chain_id = b.execution_id
          LEFT JOIN runs cr ON cr.tenant_id = c.tenant_id AND cr.id = c.current_run_id
         WHERE r.id IS NULL
            OR (c.chain_id IS NULL
                AND r.finished_at IS NOT NULL
                AND r.finished_at + make_interval(secs => b.reuse_window_s) < now())
            OR (c.chain_id IS NOT NULL
                AND c.final_outcome IS NOT NULL
                AND cr.finished_at IS NOT NULL
                AND cr.finished_at + make_interval(secs => b.reuse_window_s) < now())
         ORDER BY b.bound_at
         LIMIT p_rows
           FOR UPDATE OF b SKIP LOCKED),
    gone AS (
        DELETE FROM business_ids d USING expired e
         WHERE d.tenant_id = e.tenant_id AND d.namespace_id = e.namespace_id AND d.business_id = e.business_id
        RETURNING 1)
    SELECT count(*) INTO v_deleted FROM gone;
    RETURN v_deleted;
END;
$$;
REVOKE ALL ON FUNCTION business_ids_purge(integer) FROM PUBLIC;
DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT EXECUTE ON FUNCTION business_ids_purge(integer) TO sagat_app;
    END IF;
END $grants$;
