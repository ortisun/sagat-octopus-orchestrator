-- Feature 018: retention of the actions of a schedule (FR-018-013). An action is kept while it is among
-- the last 1,000 of its schedule or younger than 30 days; a waiting (BUFFERED) action is never deleted.
-- The runtime role may not delete from the table (the actions are the evidence of what a schedule did),
-- so retention is one bounded function it may run.
--
-- `actions_purged_at` is when the purge last finished with the schedule. The purge visits the schedules
-- in the order of it, never visited first, so a schedule that has nothing to purge (fewer than 1,000
-- actions, however old) is looked at once per cycle and cannot keep the others from being reached.
-- Additive: older code neither reads nor writes the column.
ALTER TABLE schedules ADD COLUMN actions_purged_at timestamptz;
CREATE INDEX schedules_purge_idx ON schedules (actions_purged_at NULLS FIRST, tenant_id, id);

-- Deletes up to `p_rows` actions, looking at up to `p_schedules` schedules; returns how many. A
-- schedule is marked visited only when its purgeable actions are all gone, so one that is cut short by
-- the row budget is the first to be looked at again.
CREATE FUNCTION schedule_purge_actions(p_schedules integer, p_rows integer) RETURNS bigint
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public, pg_temp AS $$
DECLARE
    v_schedule record;
    v_cut      timestamptz;
    v_budget   bigint;
    v_gone     bigint;
    v_deleted  bigint := 0;
BEGIN
    IF p_schedules < 1 OR p_rows < 1 THEN
        RETURN 0;
    END IF;
    FOR v_schedule IN
        SELECT s.tenant_id, s.id FROM schedules s
        ORDER BY s.actions_purged_at NULLS FIRST, s.tenant_id, s.id
        LIMIT p_schedules
    LOOP
        -- the time of the thousandth newest action: what is older than it is beyond the last thousand
        SELECT a.decided_at INTO v_cut FROM schedule_actions a
         WHERE a.tenant_id = v_schedule.tenant_id AND a.schedule_id = v_schedule.id
         ORDER BY a.decided_at DESC OFFSET 999 LIMIT 1;
        v_gone := 0;
        v_budget := p_rows - v_deleted;
        IF v_cut IS NOT NULL THEN
            WITH gone AS (
                DELETE FROM schedule_actions d WHERE (d.tenant_id, d.schedule_id, d.origin, d.nominal_at) IN (
                    SELECT a.tenant_id, a.schedule_id, a.origin, a.nominal_at FROM schedule_actions a
                     WHERE a.tenant_id = v_schedule.tenant_id AND a.schedule_id = v_schedule.id
                       AND a.outcome <> 'BUFFERED'
                       AND a.decided_at < LEAST(v_cut, now() - interval '30 days')
                     ORDER BY a.decided_at LIMIT v_budget)
                RETURNING 1)
            SELECT count(*) INTO v_gone FROM gone;
            v_deleted := v_deleted + v_gone;
        END IF;
        IF v_gone >= v_budget THEN
            -- the budget ran out before this schedule was done: it stays first in line
            EXIT;
        END IF;
        UPDATE schedules SET actions_purged_at = clock_timestamp()
         WHERE tenant_id = v_schedule.tenant_id AND id = v_schedule.id;
        EXIT WHEN v_deleted >= p_rows;
    END LOOP;
    RETURN v_deleted;
END;
$$;

REVOKE ALL ON FUNCTION schedule_purge_actions(integer, integer) FROM PUBLIC;
DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT EXECUTE ON FUNCTION schedule_purge_actions(integer, integer) TO sagat_app;
    END IF;
END $grants$;
