-- Durable collection progress. These rows are projections of the run journal and must be changed
-- in the same transaction as the corresponding journal event, run version, task and outbox hint.

CREATE TABLE run_instance_usage (
    tenant_id         uuid     NOT NULL,
    run_id            uuid     NOT NULL,
    max_instances     integer  NOT NULL DEFAULT 100000,
    max_depth         smallint NOT NULL DEFAULT 8,
    admitted_instances integer NOT NULL DEFAULT 0,
    reserved_instances integer NOT NULL DEFAULT 0,
    PRIMARY KEY (tenant_id, run_id),
    CONSTRAINT run_instance_usage_run_fk FOREIGN KEY (tenant_id, run_id)
        REFERENCES runs (tenant_id, id) ON DELETE CASCADE,
    CONSTRAINT run_instance_usage_limits CHECK (
        max_instances BETWEEN 1 AND 100000 AND max_depth BETWEEN 1 AND 8),
    CONSTRAINT run_instance_usage_counts CHECK (
        admitted_instances >= 0 AND reserved_instances >= 0)
);

-- Historic execution did not record the lifetime admission charge separately. Rows in active or
-- completed states are a conservative lower bound; CANCELLED is omitted because storage cannot
-- distinguish an activated instance from one cancelled while it was still pending.
INSERT INTO run_instance_usage
    (tenant_id, run_id, admitted_instances)
SELECT tenant_id, run_id, count(*)::integer
FROM steps
WHERE status IN ('READY', 'LEASED', 'RUNNING', 'WAITING', 'RETRY_WAIT', 'SUCCEEDED', 'FAILED')
GROUP BY tenant_id, run_id;

INSERT INTO run_instance_usage (tenant_id, run_id)
SELECT tenant_id, id FROM runs
ON CONFLICT DO NOTHING;

CREATE FUNCTION provision_run_instance_usage() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO run_instance_usage (tenant_id, run_id)
    VALUES (NEW.tenant_id, NEW.id);
    RETURN NEW;
END;
$$;

CREATE TRIGGER runs_instance_usage
    AFTER INSERT ON runs
    FOR EACH ROW EXECUTE FUNCTION provision_run_instance_usage();
ALTER TABLE runs ENABLE ALWAYS TRIGGER runs_instance_usage;

-- Root steps are charged once when their logical instance first leaves PENDING. Attempts, retries
-- and idempotent updates do not cross that boundary and therefore do not consume another unit.
CREATE FUNCTION account_root_instance_admission() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    should_charge boolean := false;
BEGIN
    IF cardinality(NEW.instance_path_nodes) = 0 THEN
        IF TG_OP = 'INSERT' THEN
            should_charge := NEW.status NOT IN ('PENDING', 'CANCELLED');
        ELSIF OLD.status = 'PENDING' AND NEW.status NOT IN ('PENDING', 'CANCELLED') THEN
            should_charge := true;
        END IF;
    END IF;
    IF should_charge THEN
        UPDATE public.run_instance_usage
           SET admitted_instances = admitted_instances + 1
         WHERE tenant_id = NEW.tenant_id AND run_id = NEW.run_id
           AND admitted_instances + reserved_instances < max_instances;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'run instance budget is unavailable'
                USING ERRCODE = 'check_violation';
        END IF;
    END IF;
    RETURN NULL;
END;
$$;
REVOKE ALL ON FUNCTION account_root_instance_admission() FROM PUBLIC;

CREATE TRIGGER steps_root_instance_admission
    AFTER INSERT OR UPDATE OF status ON steps
    FOR EACH ROW EXECUTE FUNCTION account_root_instance_admission();
ALTER TABLE steps ENABLE ALWAYS TRIGGER steps_root_instance_admission;

CREATE TABLE tenant_iteration_usage (
    tenant_id         uuid PRIMARY KEY REFERENCES tenants (id) ON DELETE CASCADE,
    active_iterations integer NOT NULL DEFAULT 0,
    CONSTRAINT tenant_iteration_usage_nonnegative CHECK (active_iterations >= 0)
);

-- No default is provisioned: collection admission remains closed until an operator records an
-- explicit tenant capacity. The row is locked together with usage by every admission.
CREATE TABLE tenant_collection_policies (
    tenant_id             uuid        PRIMARY KEY REFERENCES tenants (id) ON DELETE CASCADE,
    max_active_iterations integer     NOT NULL,
    updated_by            text        NOT NULL,
    updated_at            timestamptz NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT tenant_collection_policies_limit CHECK (
        max_active_iterations BETWEEN 1 AND 1000000000),
    CONSTRAINT tenant_collection_policies_actor CHECK (
        char_length(updated_by) BETWEEN 1 AND 255 AND updated_by !~ '[[:cntrl:]]')
);

INSERT INTO tenant_iteration_usage (tenant_id)
SELECT id FROM tenants
ON CONFLICT DO NOTHING;

CREATE FUNCTION provision_tenant_iteration_usage() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO tenant_iteration_usage (tenant_id) VALUES (NEW.id)
    ON CONFLICT DO NOTHING;
    RETURN NEW;
END;
$$;

CREATE TRIGGER tenants_iteration_usage
    AFTER INSERT ON tenants
    FOR EACH ROW EXECUTE FUNCTION provision_tenant_iteration_usage();
ALTER TABLE tenants ENABLE ALWAYS TRIGGER tenants_iteration_usage;

REVOKE ALL ON FUNCTION provision_run_instance_usage(),
    provision_tenant_iteration_usage(), account_root_instance_admission() FROM PUBLIC;

CREATE TABLE iteration_scopes (
    tenant_id             uuid        NOT NULL,
    run_id                uuid        NOT NULL,
    scope_id              uuid        NOT NULL,
    namespace_id          uuid        NOT NULL,
    owner_path_nodes      text[]      NOT NULL,
    owner_path_indices    integer[]   NOT NULL,
    owner_node            text        NOT NULL,
    kind                  text        NOT NULL,
    lifecycle             text        NOT NULL,
    snapshot_digest       bytea       NOT NULL,
    item_count            integer     NOT NULL,
    max_items             integer     NOT NULL,
    max_concurrency       integer     NOT NULL,
    next_admission_index  integer     NOT NULL DEFAULT 0,
    admitted_count        integer     NOT NULL DEFAULT 0,
    completed_count       integer     NOT NULL DEFAULT 0,
    active_count          integer     NOT NULL DEFAULT 0,
    result_digest         bytea,
    finish_sequence       bigint,
    finish_outbox_id      uuid,
    created_at            timestamptz NOT NULL,
    updated_at            timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, run_id, scope_id),
    CONSTRAINT iteration_scopes_owner_uniq UNIQUE
        (tenant_id, run_id, owner_path_nodes, owner_path_indices, owner_node),
    CONSTRAINT iteration_scopes_run_fk FOREIGN KEY (tenant_id, run_id)
        REFERENCES runs (tenant_id, id) ON DELETE CASCADE,
    CONSTRAINT iteration_scopes_namespace_fk FOREIGN KEY (tenant_id, namespace_id)
        REFERENCES namespaces (tenant_id, id),
    CONSTRAINT iteration_scopes_snapshot_payload_fk FOREIGN KEY (tenant_id, snapshot_digest)
        REFERENCES payloads (tenant_id, digest),
    CONSTRAINT iteration_scopes_result_payload_fk FOREIGN KEY (tenant_id, result_digest)
        REFERENCES payloads (tenant_id, digest),
    CONSTRAINT iteration_scopes_finish_outbox_uniq UNIQUE (finish_outbox_id),
    CONSTRAINT iteration_scopes_owner_path CHECK (
        execution_instance_path_valid(owner_path_nodes, owner_path_indices)),
    CONSTRAINT iteration_scopes_owner_node CHECK (
        owner_node ~ '^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$'),
    CONSTRAINT iteration_scopes_kind CHECK (kind IN ('COLLECTION')),
    CONSTRAINT iteration_scopes_lifecycle CHECK (
        lifecycle IN ('OPEN', 'DRAINING', 'SUCCEEDED', 'FAILED', 'CANCELLED')),
    CONSTRAINT iteration_scopes_snapshot_digest CHECK (octet_length(snapshot_digest) = 32),
    CONSTRAINT iteration_scopes_result_digest CHECK (
        result_digest IS NULL OR octet_length(result_digest) = 32),
    CONSTRAINT iteration_scopes_limits CHECK (
        max_items BETWEEN 1 AND 10000
        AND max_concurrency BETWEEN 1 AND 256
        AND item_count BETWEEN 0 AND max_items),
    CONSTRAINT iteration_scopes_progress CHECK (
        next_admission_index BETWEEN 0 AND item_count
        AND admitted_count = next_admission_index
        AND completed_count BETWEEN 0 AND admitted_count
        AND active_count BETWEEN 0 AND max_concurrency
        AND active_count <= admitted_count - completed_count),
    CONSTRAINT iteration_scopes_terminal_result CHECK (
        (lifecycle <> 'SUCCEEDED' AND result_digest IS NULL
            AND finish_sequence IS NULL AND finish_outbox_id IS NULL)
        OR (lifecycle = 'SUCCEEDED' AND completed_count = item_count AND active_count = 0
            AND result_digest IS NOT NULL AND finish_sequence IS NOT NULL
            AND finish_outbox_id IS NOT NULL))
);

CREATE TABLE collection_scope_items (
    tenant_id      uuid    NOT NULL,
    run_id         uuid    NOT NULL,
    scope_id       uuid    NOT NULL,
    iteration_index integer NOT NULL,
    item_digest    bytea   NOT NULL,
    PRIMARY KEY (tenant_id, run_id, scope_id, iteration_index),
    CONSTRAINT collection_scope_items_identity_uniq UNIQUE
        (tenant_id, run_id, scope_id, iteration_index, item_digest),
    CONSTRAINT collection_scope_items_scope_fk FOREIGN KEY (tenant_id, run_id, scope_id)
        REFERENCES iteration_scopes (tenant_id, run_id, scope_id) ON DELETE CASCADE,
    CONSTRAINT collection_scope_items_index CHECK (iteration_index BETWEEN 0 AND 9999),
    CONSTRAINT collection_scope_items_digest CHECK (octet_length(item_digest) = 32)
);

CREATE TABLE collection_iterations (
    tenant_id              uuid        NOT NULL,
    run_id                 uuid        NOT NULL,
    scope_id               uuid        NOT NULL,
    iteration_index        integer     NOT NULL,
    entry_path_nodes       text[]      NOT NULL,
    entry_path_indices     integer[]   NOT NULL,
    entry_node             text        NOT NULL,
    entry_instance_id      uuid        NOT NULL,
    activity               text        NOT NULL,
    item_digest            bytea       NOT NULL,
    lifecycle              text        NOT NULL,
    slot_held              boolean     NOT NULL,
    reserved_instances     integer     NOT NULL,
    remaining_reservations integer     NOT NULL,
    return_path_nodes      text[],
    return_path_indices    integer[],
    return_node            text,
    return_instance_id     uuid,
    result_digest          bytea,
    admitted_sequence      bigint      NOT NULL,
    outbox_id              uuid        NOT NULL,
    checkpoint_sequence    bigint,
    created_at             timestamptz NOT NULL,
    updated_at             timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, run_id, scope_id, iteration_index),
    CONSTRAINT collection_iterations_scope_fk FOREIGN KEY (tenant_id, run_id, scope_id)
        REFERENCES iteration_scopes (tenant_id, run_id, scope_id) ON DELETE CASCADE,
    CONSTRAINT collection_iterations_membership_fk FOREIGN KEY
        (tenant_id, run_id, scope_id, iteration_index, item_digest)
        REFERENCES collection_scope_items
        (tenant_id, run_id, scope_id, iteration_index, item_digest),
    CONSTRAINT collection_iterations_item_payload_fk FOREIGN KEY (tenant_id, item_digest)
        REFERENCES payloads (tenant_id, digest),
    CONSTRAINT collection_iterations_result_payload_fk FOREIGN KEY (tenant_id, result_digest)
        REFERENCES payloads (tenant_id, digest),
    CONSTRAINT collection_iterations_entry_uniq UNIQUE (tenant_id, entry_instance_id),
    CONSTRAINT collection_iterations_outbox_uniq UNIQUE (outbox_id),
    CONSTRAINT collection_iterations_entry_path CHECK (
        execution_instance_path_valid(entry_path_nodes, entry_path_indices)),
    CONSTRAINT collection_iterations_index CHECK (iteration_index BETWEEN 0 AND 9999),
    CONSTRAINT collection_iterations_entry_node CHECK (
        entry_node ~ '^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$'),
    CONSTRAINT collection_iterations_activity CHECK (
        char_length(activity) BETWEEN 1 AND 128 AND activity !~ '[[:cntrl:]]'),
    CONSTRAINT collection_iterations_item_digest CHECK (octet_length(item_digest) = 32),
    CONSTRAINT collection_iterations_lifecycle CHECK (
        lifecycle IN ('ADMITTED', 'CHECKPOINTED', 'DRAINED')),
    CONSTRAINT collection_iterations_reservations CHECK (
        reserved_instances >= 1
        AND remaining_reservations BETWEEN 0 AND reserved_instances),
    CONSTRAINT collection_iterations_sequences CHECK (
        admitted_sequence >= 1
        AND (checkpoint_sequence IS NULL OR checkpoint_sequence > admitted_sequence)),
    CONSTRAINT collection_iterations_return_consistent CHECK (
        (return_path_nodes IS NULL AND return_path_indices IS NULL AND return_node IS NULL
            AND return_instance_id IS NULL AND result_digest IS NULL
            AND checkpoint_sequence IS NULL)
        OR (execution_instance_path_valid(return_path_nodes, return_path_indices)
            AND return_node ~ '^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$'
            AND return_instance_id IS NOT NULL
            AND octet_length(result_digest) = 32
            AND checkpoint_sequence IS NOT NULL)),
    CONSTRAINT collection_iterations_slot_consistent CHECK (
        (lifecycle = 'ADMITTED' AND slot_held)
        OR (lifecycle IN ('CHECKPOINTED', 'DRAINED') AND NOT slot_held))
);

CREATE INDEX collection_iterations_active_idx
    ON collection_iterations (tenant_id, run_id, scope_id, iteration_index)
    WHERE slot_held;

CREATE TABLE iteration_checkpoints (
    tenant_id          uuid        NOT NULL,
    run_id             uuid        NOT NULL,
    scope_id           uuid        NOT NULL,
    iteration_index    integer     NOT NULL,
    return_instance_id uuid        NOT NULL,
    result_digest      bytea       NOT NULL,
    journal_sequence   bigint      NOT NULL,
    outbox_id          uuid        NOT NULL,
    created_at         timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, run_id, scope_id, iteration_index),
    CONSTRAINT iteration_checkpoints_iteration_fk
        FOREIGN KEY (tenant_id, run_id, scope_id, iteration_index)
        REFERENCES collection_iterations (tenant_id, run_id, scope_id, iteration_index)
        ON DELETE CASCADE,
    CONSTRAINT iteration_checkpoints_result_digest CHECK (octet_length(result_digest) = 32),
    CONSTRAINT iteration_checkpoints_result_payload_fk FOREIGN KEY (tenant_id, result_digest)
        REFERENCES payloads (tenant_id, digest),
    CONSTRAINT iteration_checkpoints_outbox_uniq UNIQUE (outbox_id),
    CONSTRAINT iteration_checkpoints_sequence CHECK (journal_sequence >= 2)
);

-- A collection owner parks without a process. The existing waiting invariant still applies.
ALTER TABLE steps DROP CONSTRAINT steps_waiting_kind_valid;
ALTER TABLE steps ADD CONSTRAINT steps_waiting_kind_valid CHECK (
    waiting_kind IS NULL OR waiting_kind IN
    ('TIMER', 'EVENT', 'BRANCHES', 'CHILD', 'SAGA', 'RECONCILIATION', 'OPERATOR', 'COLLECTION')) NOT VALID;

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT, UPDATE ON run_instance_usage, tenant_iteration_usage,
            iteration_scopes, collection_scope_items, collection_iterations,
            iteration_checkpoints TO sagat_app;
        GRANT SELECT ON tenant_collection_policies TO sagat_app;
        REVOKE ALL ON FUNCTION provision_run_instance_usage(),
            provision_tenant_iteration_usage(), account_root_instance_admission() FROM sagat_app;
    END IF;
END
$grants$;
