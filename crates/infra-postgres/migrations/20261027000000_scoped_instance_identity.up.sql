-- Root-compatible storage identity for nodes instantiated inside bounded scopes.
-- Existing projections become empty-path roots. Historic journal rows stay byte-for-byte events
-- with a NULL instance identity and are decoded by compatible readers as roots.

CREATE FUNCTION execution_instance_path_valid(nodes text[], indices integer[]) RETURNS boolean
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
    SELECT nodes IS NOT NULL
       AND indices IS NOT NULL
       AND cardinality(nodes) = cardinality(indices)
       AND cardinality(nodes) <= 8
       AND (cardinality(nodes) = 0 OR (array_ndims(nodes) = 1 AND array_lower(nodes, 1) = 1))
       AND (cardinality(indices) = 0 OR (array_ndims(indices) = 1 AND array_lower(indices, 1) = 1))
       AND NOT EXISTS (
            SELECT 1 FROM unnest(nodes) AS n(value)
            WHERE value IS NULL OR value !~ '^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$')
       AND NOT EXISTS (
            SELECT 1 FROM unnest(indices) AS i(value)
            WHERE value IS NULL OR value NOT BETWEEN 0 AND 9999)
$$;

-- `step_id` remains the opaque instance id. The path and definition node make its independently
-- checkable logical identity; attempt and generation remain separate fencing dimensions.
ALTER TABLE steps
    ADD COLUMN instance_path_nodes text[] NOT NULL DEFAULT '{}',
    ADD COLUMN instance_path_indices integer[] NOT NULL DEFAULT '{}',
    ADD CONSTRAINT steps_instance_path_valid
        CHECK (execution_instance_path_valid(instance_path_nodes, instance_path_indices));

ALTER TABLE steps DROP CONSTRAINT steps_pkey;
ALTER TABLE steps ADD CONSTRAINT steps_pkey PRIMARY KEY
    (tenant_id, run_id, instance_path_nodes, instance_path_indices, node);

-- Journal history is append-only. No UPDATE backfills old events: NULL is the legacy root encoding,
-- while new events can persist the complete identity used to replay a scoped node.
ALTER TABLE run_events
    ADD COLUMN instance_id uuid,
    ADD COLUMN instance_path_nodes text[],
    ADD COLUMN instance_path_indices integer[],
    ADD CONSTRAINT run_events_instance_identity_consistent CHECK (
        (instance_id IS NULL AND instance_path_nodes IS NULL AND instance_path_indices IS NULL)
        OR (instance_id IS NOT NULL
            AND node IS NOT NULL
            AND execution_instance_path_valid(instance_path_nodes, instance_path_indices)));

CREATE INDEX run_events_instance_idx
    ON run_events (tenant_id, run_id, instance_id, seq)
    WHERE instance_id IS NOT NULL;

-- A wait's durable delivery evidence must eventually distinguish equal definition nodes in separate
-- instances. The existing consumer key remains authoritative until the negotiated scoped protocol is
-- enabled; these nullable fields are additive and do not reinterpret old deliveries.
ALTER TABLE event_deliveries
    ADD COLUMN instance_id uuid,
    ADD COLUMN instance_path_nodes text[],
    ADD COLUMN instance_path_indices integer[],
    ADD CONSTRAINT event_deliveries_instance_identity_consistent CHECK (
        (instance_id IS NULL AND instance_path_nodes IS NULL AND instance_path_indices IS NULL)
        OR (instance_id IS NOT NULL
            AND node IS NOT NULL
            AND execution_instance_path_valid(instance_path_nodes, instance_path_indices)));

CREATE INDEX event_deliveries_instance_idx
    ON event_deliveries (tenant_id, run_id, instance_id)
    WHERE instance_id IS NOT NULL;

-- Saga and script rows keep their legacy keys in this foundation migration because the current
-- adapter still names those conflict targets. Nullable identity prepares an additive dual reader;
-- their key transition belongs with the instance-aware adapter that writes the new fields.
ALTER TABLE run_sagas
    ADD COLUMN instance_id uuid,
    ADD COLUMN instance_path_nodes text[],
    ADD COLUMN instance_path_indices integer[],
    ADD CONSTRAINT run_sagas_instance_identity_consistent CHECK (
        (instance_id IS NULL AND instance_path_nodes IS NULL AND instance_path_indices IS NULL)
        OR (instance_id IS NOT NULL
            AND execution_instance_path_valid(instance_path_nodes, instance_path_indices)));

CREATE INDEX run_sagas_instance_idx
    ON run_sagas (tenant_id, run_id, instance_id)
    WHERE instance_id IS NOT NULL;

ALTER TABLE script_executions
    ADD COLUMN instance_id uuid,
    ADD COLUMN instance_path_nodes text[],
    ADD COLUMN instance_path_indices integer[],
    ADD CONSTRAINT script_executions_instance_identity_consistent CHECK (
        (instance_id IS NULL AND instance_path_nodes IS NULL AND instance_path_indices IS NULL)
        OR (instance_id IS NOT NULL
            AND run_id IS NOT NULL
            AND node IS NOT NULL
            AND execution_instance_path_valid(instance_path_nodes, instance_path_indices)));

CREATE INDEX script_executions_instance_idx
    ON script_executions (tenant_id, run_id, instance_id, attempt)
    WHERE instance_id IS NOT NULL;
