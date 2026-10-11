-- Feature 010: the quota of active runs is read from counters, not counted.
--
-- Every start of a run checked the quotas of its namespace and of its tenant by counting the runs that
-- have not finished (`SELECT count(*) FROM runs WHERE ... AND finished_at IS NULL`), under a lock that
-- serialises the starts of a namespace. The count reads one index entry per active run, so the cost of
-- a start grew with the number of runs waiting: starting the hundred-thousandth wait took a hundred
-- times what starting the thousandth did, and a million waits could not be made at all. The count is
-- now kept by the database, in the same transaction as the change it counts, and the check reads at
-- most sixteen rows.
--
-- Sixteen slots per namespace (by the run's identifier) so that the runs that finish at the same time
-- do not all wait for one row. The sum of the slots is the number of unfinished runs; a trigger on
-- `runs` adds one when an unfinished run is inserted and takes one away when a run finishes or an
-- unfinished one is deleted. The function is SECURITY DEFINER: the runtime role can read the counters
-- and cannot write them, and the only way to change one is to change a run.

-- No write to `runs` may slip between the count below and the triggers being in place.
LOCK TABLE runs IN SHARE ROW EXCLUSIVE MODE;

CREATE TABLE run_active_counts (
    tenant_id    uuid     NOT NULL,
    namespace_id uuid     NOT NULL,
    slot         smallint NOT NULL CHECK (slot BETWEEN 0 AND 15),
    active       bigint   NOT NULL,
    PRIMARY KEY (tenant_id, namespace_id, slot)
);

INSERT INTO run_active_counts (tenant_id, namespace_id, slot, active)
SELECT tenant_id, namespace_id, (get_byte(uuid_send(id), 15) & 15)::smallint, count(*)
  FROM runs WHERE finished_at IS NULL GROUP BY 1, 2, 3;

CREATE FUNCTION run_active_counts_track() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public, pg_temp
AS $$
DECLARE
    delta   integer;
    changed public.runs;
BEGIN
    IF TG_OP = 'INSERT' THEN
        delta := 1;
        changed := NEW;
    ELSIF TG_OP = 'DELETE' THEN
        delta := -1;
        changed := OLD;
    ELSIF OLD.finished_at IS NULL AND NEW.finished_at IS NOT NULL THEN
        -- Finished now.
        delta := -1;
        changed := NEW;
    ELSIF OLD.finished_at IS NOT NULL AND NEW.finished_at IS NULL THEN
        -- Opened again (nothing does it; the count follows if something ever does).
        delta := 1;
        changed := NEW;
    ELSE
        -- The time it finished at was corrected: still finished.
        RETURN NULL;
    END IF;
    -- Every name is qualified and pg_temp is last in the path: a function that runs with its owner's
    -- rights must not find a table of the same name that the caller made in its own temporary schema
    -- (which is searched first for relations unless the path says otherwise).
    INSERT INTO public.run_active_counts AS c (tenant_id, namespace_id, slot, active)
    VALUES (changed.tenant_id, changed.namespace_id,
            (get_byte(uuid_send(changed.id), 15) & 15)::smallint, greatest(delta, 0))
    ON CONFLICT (tenant_id, namespace_id, slot)
    DO UPDATE SET active = greatest(c.active + delta, 0);
    RETURN NULL;
END
$$;

-- Only the triggers below may run it: a role that can execute it could attach it to a table of its own
-- (a temporary one) and move any tenant's counters with the rights of the owner.
REVOKE ALL ON FUNCTION run_active_counts_track() FROM PUBLIC;

CREATE TRIGGER runs_count_started AFTER INSERT ON runs
    FOR EACH ROW WHEN (NEW.finished_at IS NULL) EXECUTE FUNCTION run_active_counts_track();
CREATE TRIGGER runs_count_finished AFTER UPDATE OF finished_at ON runs
    FOR EACH ROW WHEN (OLD.finished_at IS DISTINCT FROM NEW.finished_at)
    EXECUTE FUNCTION run_active_counts_track();
CREATE TRIGGER runs_count_deleted AFTER DELETE ON runs
    FOR EACH ROW WHEN (OLD.finished_at IS NULL) EXECUTE FUNCTION run_active_counts_track();

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT ON run_active_counts TO sagat_app;
    END IF;
END
$grants$;
