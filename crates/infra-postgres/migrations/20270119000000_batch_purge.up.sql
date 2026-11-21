-- Feature 018: retention of batch jobs and their items (T018-051, the purge half; FR-018-033, G2317). A job and its
-- items are the evidence of what was done to which runs, by whom and why. They are kept for the window the job was
-- stamped with when it was created (`expires_at`: 30 days by default, at most 90, at least 14) and then purged. The
-- runtime role may not delete from either table (`20270102000000_batch_operations` grants it no DELETE, and the guard
-- of `batch_jobs` refuses the delete of a job that has not finished), so retention is one bounded function it may run,
-- as it is for the actions of a schedule (`schedule_purge_actions`) and the bindings of business identifiers
-- (`business_ids_purge`).
--
-- Additive: older code neither reads nor calls the function. Until it is called, nothing changes (the tables only grow,
-- as before). The index takes a SHARE lock on `batch_jobs` while it is built, so a job that is created or moves on waits
-- for it; the table holds the jobs of the retention window and the build takes a moment on thousands of rows.

-- The jobs that have finished, oldest window first: what the purge reads. `IF NOT EXISTS` lets an operator build it CONCURRENTLY
-- first on a large database (the recipe is in `docs/runbooks/batch-operations.md`, section 6); a build that failed leaves an index
-- that exists and is INVALID, which the planner never uses, so it is refused here (the pattern of `20261017000000_scheduler_shards`).
CREATE INDEX IF NOT EXISTS batch_jobs_purge_idx ON batch_jobs (expires_at, tenant_id, id)
    WHERE state IN ('COMPLETED', 'CANCELLED', 'FAILED');
DO $valid$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_index i JOIN pg_class c ON c.oid = i.indexrelid
                WHERE NOT i.indisvalid AND c.relname = 'batch_jobs_purge_idx') THEN
        RAISE EXCEPTION 'invalid index left by a failed concurrent build, drop it and run again: batch_jobs_purge_idx';
    END IF;
END
$valid$;

-- Removes the items and then the rows of the jobs that are past their retention, up to `p_jobs` jobs and `p_rows` rows
-- (items and jobs together); returns how many jobs and how many items. A job goes when it has finished (one of the three
-- terminal states: the table's check gives each an end, `finished_at`, which is null for a job that has not finished and
-- so never passes the test of its age below), its window has ended (`expires_at`, which a person was told in the read of
-- the job), and it ended at least 14 days ago, so a job that ended after its own window (it waited for an engine, or ran
-- long) is still there to be read, and the key of the request that made it (kept for 7 days) is forgotten before the job
-- is. A job that has not finished is never purged, whatever its age; the guard of the table refuses it besides.
--
-- A bound that is not at least 1 removes nothing, a call with no bound (NULL) answers nothing (the function is STRICT, so that a
-- caller cannot lift the bound by leaving it out), and a bound above the ceilings of one call, 1,000 jobs and 100,000 rows, is
-- held to them: the function is the boundary, and the runtime role can call it with any number. One call is one statement of the
-- engine, under its statement timeout, and each row it deletes fires the guard of its table and the maintenance of four indexes;
-- the engine asks for far less than the ceilings (`passes.rs`).
--
-- One job at a time, the oldest window first: its row is locked with `SKIP LOCKED`, so a job another transaction holds
-- (a second replica purging it, a cancel that reads it) is passed over, never waited for, and the lock is held only to
-- the end of this call, which is bounded by `p_rows`. The items go first, in a chunk that is the rows the call has left,
-- and the row of the job goes last and only when its items are all gone and the call has a row left for it: a call that
-- the budget cuts short leaves the job with its end and the items it has left, and the next call takes the same job
-- (it is still the oldest) and goes on. So an item is never without its job (the foreign key says so) and a job is never
-- without its end (the state of a finished job does not change). The items are read and deleted by tenant and job, never
-- by the id of the job alone: the id of a job is its tenant's.
CREATE FUNCTION batch_jobs_purge(p_jobs integer, p_rows integer)
RETURNS TABLE (jobs_removed bigint, items_removed bigint)
LANGUAGE plpgsql STRICT SECURITY DEFINER SET search_path = pg_catalog, public, pg_temp AS $$
DECLARE
    v_job  record;
    v_left bigint;
    v_cut  integer;
    v_gone bigint;
BEGIN
    jobs_removed := 0;
    items_removed := 0;
    p_jobs := LEAST(p_jobs, 1000);
    p_rows := LEAST(p_rows, 100000);
    LOOP
        v_left := p_rows - jobs_removed - items_removed;
        EXIT WHEN v_left < 1 OR jobs_removed >= p_jobs;
        SELECT j.tenant_id, j.id INTO v_job
          FROM batch_jobs j
         WHERE j.state IN ('COMPLETED', 'CANCELLED', 'FAILED')
           AND j.expires_at < now()
           AND j.finished_at < now() - interval '14 days'
         ORDER BY j.expires_at, j.tenant_id, j.id
         LIMIT 1
           FOR UPDATE OF j SKIP LOCKED;
        EXIT WHEN NOT FOUND;
        -- the chunk is a range of the job's own items, by their order: everything up to the `v_left`th of them (the ordinals of a
        -- job are distinct), or all of them when there are fewer. A delete that joins the items to a list of the ordinals it
        -- chose is quadratic in the chunk (5,000 rows of one job took 3.2 s), and this one is a range of the primary key.
        SELECT i.ordinal INTO v_cut FROM batch_items i
         WHERE i.tenant_id = v_job.tenant_id AND i.job_id = v_job.id
         ORDER BY i.ordinal
        OFFSET v_left - 1 LIMIT 1;
        DELETE FROM batch_items d
         WHERE d.tenant_id = v_job.tenant_id AND d.job_id = v_job.id AND (v_cut IS NULL OR d.ordinal <= v_cut);
        GET DIAGNOSTICS v_gone = ROW_COUNT;
        items_removed := items_removed + v_gone;
        IF v_gone >= v_left THEN
            -- the budget went on the items: the job keeps its end and what is left of its items, and is the first in line
            -- at the next call
            EXIT;
        END IF;
        DELETE FROM batch_jobs d WHERE d.tenant_id = v_job.tenant_id AND d.id = v_job.id;
        jobs_removed := jobs_removed + 1;
    END LOOP;
    RETURN NEXT;
END;
$$;

REVOKE ALL ON FUNCTION batch_jobs_purge(integer, integer) FROM PUBLIC;
DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT EXECUTE ON FUNCTION batch_jobs_purge(integer, integer) TO sagat_app;
    END IF;
END $grants$;
