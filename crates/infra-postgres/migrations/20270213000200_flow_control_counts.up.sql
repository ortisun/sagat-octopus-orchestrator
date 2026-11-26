-- Feature 022, release 0.3.0 slice (T022-105; FR-022-058; decisions D022-026, D022-051, D022-084): the namespace's queued
-- runs are read from counters, not counted.
--
-- A run's start that waits for its permit or its slot is a run-scope waiter row in `concurrency_waiters` or
-- `rate_limit_waiters`, and a run that waits for both has one in each (FR-022-044: they are queued and leave together,
-- in one transaction of the run). The `queued_runs` ceiling counted them with a scan of the namespace's waiters, under the
-- namespace's admission lock (D022-051: exact). The count is now kept by the database, in the transaction that adds or
-- removes the rows, in sixteen slot rows per namespace by the run's identifier (the slots of `run_active_counts`), and
-- the check reads at most sixteen rows. A run counts once whatever its waiter rows: the trigger of a row adds one only when
-- the run has no run-scope waiter row in the other table, and takes one away only when none is left there.
--
-- Lock order (ADR-0050 decision 7, ADR-0056 decision 2, D022-038): a slot row is a usage row of slot 8b. It is locked
-- when a run's own waiter row is written or deleted, which every path does after the run's active-run counter row (slot
-- 6: the admission inserts the run first; a commit that ends a run locks it first, `run_active_counts_lock_slot`; the
-- grant of a run's start locks it before its keys, since the grant can end the run in its own transaction) and after
-- the start's keys are locked (semaphores, then windows). The only keys a transaction locks after it are the STEP keys of
-- the run's entry steps when the grant routine starts a buffered run, and no transaction that holds a step key writes a
-- run waiter afterwards (a run that waits for its start has no step). The counters of live keys are not kept here
-- (decision D022-084: such a counter would be locked in the middle of a sorted pass over the keys, or at the commit beside
-- the deferred quota flush, where two transactions can wait on each other).
--
-- SECURITY DEFINER with `search_path = pg_catalog, pg_temp` and every name schema-qualified: the runtime role reads the
-- counters and cannot write them; the only way to change one is to change a waiter row. Additive: an older engine never
-- reads the table and its writes of waiter rows keep it exact through the triggers.

-- No waiter row may be written between the count below and the triggers being in place. The rate-limit waiters first, as a
-- dual-gated admission writes them (batch 3 correctness minor 6), so that the migration and an admission never wait on
-- each other in opposite orders.
LOCK TABLE rate_limit_waiters, concurrency_waiters IN SHARE ROW EXCLUSIVE MODE;

CREATE TABLE flow_control_counts (
    tenant_id    uuid     NOT NULL,
    namespace_id uuid     NOT NULL,
    slot         smallint NOT NULL,
    queued_runs  bigint   NOT NULL,
    PRIMARY KEY (tenant_id, namespace_id, slot),
    CONSTRAINT flow_control_counts_slot_valid CHECK (slot BETWEEN 0 AND 15),
    CONSTRAINT flow_control_counts_queued_runs_valid CHECK (queued_runs >= 0),
    CONSTRAINT flow_control_counts_namespace_fk FOREIGN KEY (tenant_id, namespace_id)
        REFERENCES namespaces (tenant_id, id)
);

INSERT INTO flow_control_counts (tenant_id, namespace_id, slot, queued_runs)
SELECT tenant_id, namespace_id, (get_byte(uuid_send(run_id), 15) & 15)::smallint, count(*)
  FROM (SELECT tenant_id, namespace_id, run_id FROM concurrency_waiters WHERE permit_scope = 'RUN'
        UNION
        SELECT tenant_id, namespace_id, run_id FROM rate_limit_waiters WHERE limit_scope = 'RUN') waiting
 GROUP BY 1, 2, 3;

CREATE FUNCTION flow_control_counts_track() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    changed record;
    other   boolean;
    delta   integer;
BEGIN
    IF TG_OP = 'INSERT' THEN
        changed := NEW;
        delta := 1;
    ELSE
        changed := OLD;
        delta := -1;
    END IF;
    -- a step's waiter is not a queued run (it is bounded per run and gated node, plan "Ceilings, revised")
    IF changed.node <> '' THEN
        RETURN NULL;
    END IF;
    -- the run's run-scope waiter row in the other table, as this transaction sees it: the run counts once
    IF TG_TABLE_NAME = 'concurrency_waiters' THEN
        other := EXISTS (SELECT 1 FROM public.rate_limit_waiters w
                          WHERE w.tenant_id = changed.tenant_id AND w.run_id = changed.run_id AND w.node = ''
                            AND w.limit_scope = 'RUN');
    ELSE
        other := EXISTS (SELECT 1 FROM public.concurrency_waiters w
                          WHERE w.tenant_id = changed.tenant_id AND w.run_id = changed.run_id AND w.node = ''
                            AND w.permit_scope = 'RUN');
    END IF;
    IF other THEN
        RETURN NULL;
    END IF;
    INSERT INTO public.flow_control_counts AS c (tenant_id, namespace_id, slot, queued_runs)
    VALUES (changed.tenant_id, changed.namespace_id,
            (get_byte(uuid_send(changed.run_id), 15) & 15)::smallint, greatest(delta, 0))
    ON CONFLICT (tenant_id, namespace_id, slot) DO UPDATE SET queued_runs = c.queued_runs + delta;
    RETURN NULL;
END
$$;
-- Only the triggers below may run it: a role that can execute it could attach it to a table of its own (a temporary one)
-- and move any tenant's counters with the rights of the owner.
REVOKE ALL ON FUNCTION flow_control_counts_track() FROM PUBLIC;

-- A run's waiter only (`node = ''`): a step's waiter row, the common case of a gated node, fires no trigger at all (batch 3
-- correctness nit). One trigger per operation, since a `WHEN` of an `INSERT OR DELETE` trigger cannot name both rows.
CREATE TRIGGER concurrency_waiters_queued_runs AFTER INSERT ON concurrency_waiters
    FOR EACH ROW WHEN (NEW.node = '') EXECUTE FUNCTION flow_control_counts_track();
CREATE TRIGGER concurrency_waiters_queued_runs_out AFTER DELETE ON concurrency_waiters
    FOR EACH ROW WHEN (OLD.node = '') EXECUTE FUNCTION flow_control_counts_track();
CREATE TRIGGER rate_limit_waiters_queued_runs AFTER INSERT ON rate_limit_waiters
    FOR EACH ROW WHEN (NEW.node = '') EXECUTE FUNCTION flow_control_counts_track();
CREATE TRIGGER rate_limit_waiters_queued_runs_out AFTER DELETE ON rate_limit_waiters
    FOR EACH ROW WHEN (OLD.node = '') EXECUTE FUNCTION flow_control_counts_track();
-- a counter follows its rows whatever replication role a session runs with
ALTER TABLE concurrency_waiters ENABLE ALWAYS TRIGGER concurrency_waiters_queued_runs;
ALTER TABLE concurrency_waiters ENABLE ALWAYS TRIGGER concurrency_waiters_queued_runs_out;
ALTER TABLE rate_limit_waiters ENABLE ALWAYS TRIGGER rate_limit_waiters_queued_runs;
ALTER TABLE rate_limit_waiters ENABLE ALWAYS TRIGGER rate_limit_waiters_queued_runs_out;

REVOKE ALL PRIVILEGES ON flow_control_counts FROM PUBLIC;
DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT ON flow_control_counts TO sagat_app;
    END IF;
END
$grants$;
