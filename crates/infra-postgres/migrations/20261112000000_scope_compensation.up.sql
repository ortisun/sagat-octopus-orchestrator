-- Compensation of the iterations a collection or loop scope completed (feature 014, D380..D389).
--
-- A scope that can be compensated is its own saga: once its drain has settled every iteration it
-- records one plan, runs the compensations through the same lease, fence, retry and manual-review
-- machinery as forward work, and ends with the outcome of how they ended. The tables below are the
-- projection of its two journal events (`scope.compensation_planned`, `scope.compensated`). They are
-- keyed by the owner *instance* (a path and a node) because the same body node runs once per
-- iteration; the nullable instance columns of `run_sagas` keep their meaning for root sagas.

-- The members a compensation waits for, as parallel arrays of iteration indices and node ids.
CREATE FUNCTION scope_saga_after_valid(iterations integer[], nodes text[]) RETURNS boolean
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
    SELECT iterations IS NOT NULL
       AND nodes IS NOT NULL
       AND cardinality(iterations) = cardinality(nodes)
       AND cardinality(iterations) <= 10000
       AND (cardinality(iterations) = 0
            OR (array_ndims(iterations) = 1 AND array_lower(iterations, 1) = 1
                AND array_ndims(nodes) = 1 AND array_lower(nodes, 1) = 1))
       AND NOT EXISTS (SELECT 1 FROM unnest(iterations) AS i(value)
                       WHERE value IS NULL OR value NOT BETWEEN 0 AND 9999)
       AND NOT EXISTS (SELECT 1 FROM unnest(nodes) AS n(value)
                       WHERE value IS NULL OR value !~ '^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$')
$$;

CREATE TABLE scope_sagas (
    tenant_id          uuid        NOT NULL,
    run_id             uuid        NOT NULL,
    owner_path_nodes   text[]      NOT NULL,
    owner_path_indices integer[]   NOT NULL,
    owner_node         text        NOT NULL,
    phase              text        NOT NULL,
    -- Why the scope compensates: the failure that drained it, or the stop of the run.
    cause_code         text        NOT NULL,
    outcome            text,
    -- It was planned, or ended, while the run was being stopped.
    during_stop        boolean     NOT NULL,
    planned_sequence   bigint      NOT NULL,
    ended_sequence     bigint,
    created_at         timestamptz NOT NULL,
    updated_at         timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, run_id, owner_path_nodes, owner_path_indices, owner_node),
    CONSTRAINT scope_sagas_run_same_tenant_fk FOREIGN KEY (tenant_id, run_id)
        REFERENCES runs (tenant_id, id),
    CONSTRAINT scope_sagas_owner_path CHECK (
        execution_instance_path_valid(owner_path_nodes, owner_path_indices)),
    CONSTRAINT scope_sagas_owner_node CHECK (owner_node ~ '^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$'),
    CONSTRAINT scope_sagas_phase CHECK (phase IN ('COMPENSATING', 'DONE')),
    CONSTRAINT scope_sagas_cause CHECK (cause_code ~ '^[A-Za-z0-9_.:-]{1,64}$'),
    CONSTRAINT scope_sagas_outcome CHECK (
        (phase = 'COMPENSATING' AND outcome IS NULL AND ended_sequence IS NULL)
        OR (phase = 'DONE'
            AND outcome IN ('COMPENSATED', 'PARTIAL', 'COMPENSATION_FAILED')
            AND ended_sequence > planned_sequence)),
    CONSTRAINT scope_sagas_sequence CHECK (planned_sequence >= 1)
);

-- What a scope planned: the forward instances it undoes (`kind = 'COMPENSATE'`, with the forward
-- instances each waits for) and the completed ones that cannot be undone (`kind = 'IRREVERSIBLE'`).
-- A member is an instance of one iteration of the owner: its path is the owner's path plus
-- `(owner node, iteration)`, so the iteration and the node name it.
CREATE TABLE scope_saga_entries (
    tenant_id          uuid      NOT NULL,
    run_id             uuid      NOT NULL,
    owner_path_nodes   text[]    NOT NULL,
    owner_path_indices integer[] NOT NULL,
    owner_node         text      NOT NULL,
    iteration_index    integer   NOT NULL,
    node               text      NOT NULL,
    kind               text      NOT NULL,
    -- Stable for the workflow version, the path and the node: the handle a provider can recognise
    -- across attempts and restarts (D382).
    compensation_id    uuid,
    -- The forward members this one waits for: (iteration, node) pairs of the same owner.
    after_iterations   integer[] NOT NULL DEFAULT '{}',
    after_nodes        text[]    NOT NULL DEFAULT '{}',
    PRIMARY KEY (tenant_id, run_id, owner_path_nodes, owner_path_indices, owner_node,
                 iteration_index, node),
    CONSTRAINT scope_saga_entries_saga_fk
        FOREIGN KEY (tenant_id, run_id, owner_path_nodes, owner_path_indices, owner_node)
        REFERENCES scope_sagas (tenant_id, run_id, owner_path_nodes, owner_path_indices, owner_node),
    CONSTRAINT scope_saga_entries_iteration CHECK (iteration_index BETWEEN 0 AND 9999),
    CONSTRAINT scope_saga_entries_node CHECK (node ~ '^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$'),
    CONSTRAINT scope_saga_entries_kind CHECK (
        (kind = 'COMPENSATE' AND compensation_id IS NOT NULL)
        OR (kind = 'IRREVERSIBLE' AND compensation_id IS NULL
            AND cardinality(after_iterations) = 0)),
    CONSTRAINT scope_saga_entries_after CHECK (
        scope_saga_after_valid(after_iterations, after_nodes))
);

CREATE INDEX scope_sagas_open_idx
    ON scope_sagas (tenant_id, run_id)
    WHERE phase = 'COMPENSATING';

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT, UPDATE ON scope_sagas TO sagat_app;
        GRANT SELECT, INSERT ON scope_saga_entries TO sagat_app;
    END IF;
END
$grants$;
