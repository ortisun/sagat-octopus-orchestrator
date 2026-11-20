-- Feature 022 (T022-022; FR-022-018, FR-022-021; D022-031, D022-049): the projection of the gates of a run's start on
-- `runs`, as `steps.flow_control_gates` is for a step (20270130000600). A run of a definition that declares a top-level
-- `concurrency` holds a permit or waits for one from its admission; `flow_control_gates` holds the state of each gate of
-- its start as the reducer left it (phase, pool, group, the key's 12-hex label and a wait's deadline; never a key's
-- value), so that a transaction restores the run as a replay of its journal would, and a finished run keeps what its
-- gates ended as. The holder and waiter rows stay the truth of the keys; `verify_run` checks the two agree.
--
-- Additive: one nullable column (no rewrite) and a CHECK added `NOT VALID`, validated by
-- `20270203000100_flow_control_run_gates_validate` (D043-145). No row has a value until a run of a definition with a
-- start gate is admitted, which no process of this build lists the capability for (D022-037). `sagat_app` already updates
-- `runs`; no grant changes.
ALTER TABLE runs ADD COLUMN flow_control_gates jsonb;
ALTER TABLE runs ADD CONSTRAINT runs_flow_control_gates_bounded CHECK (
    flow_control_gates IS NULL
    OR (jsonb_typeof(flow_control_gates) = 'object' AND octet_length(flow_control_gates::text) <= 1024)) NOT VALID;

-- ADR-0050's merged order puts a run's active-run counter row (slot 6, kept by `run_active_counts_track` on the run's
-- insert and on its `finished_at`) before the semaphores (slot 7). An admission takes them in that order; a commit that
-- ends a run gives its permits back before it sets `finished_at`, so it takes the semaphore first and the counter row
-- last, and the two can wait on each other in a cycle when the runs share a counter slot (correctness review M1 of
-- 2026-11-20). `persist` calls this first in a commit that ends a run of a gated definition, so the counter row is locked
-- before any key. It locks only the slot row of the run it names, in that run's own tenant and namespace; `sagat_app`
-- holds `SELECT` only on the counters, so the lock is taken with the owner's rights, and every name is qualified.
CREATE FUNCTION run_active_counts_lock_slot(p_tenant uuid, p_run uuid) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    PERFORM 1
       FROM public.run_active_counts c
       JOIN public.runs r ON r.tenant_id = c.tenant_id AND r.namespace_id = c.namespace_id
      WHERE r.tenant_id = p_tenant AND r.id = p_run
        AND c.slot = (get_byte(uuid_send(p_run), 15) & 15)::smallint
        FOR UPDATE OF c;
END
$$;
REVOKE ALL ON FUNCTION run_active_counts_lock_slot(uuid, uuid) FROM PUBLIC;
DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT EXECUTE ON FUNCTION run_active_counts_lock_slot(uuid, uuid) TO sagat_app;
    END IF;
END
$grants$;
