-- Feature 018, T018-060 and T018-061 (FR-018-017, D1034): the audited handover of the cron triggers from the legacy cron
-- pass to the schedule pass.
--
-- The singleton `schedule_trigger_handover` (migration 20261230000000) already says whether the handover happened. What
-- is added here is the only way to change it:
--
--   * `schedule_trigger_handover_complete(actor)` moves it from LEGACY to HANDED_OVER and records who did it (the
--     operator's name as the command line gave it) and the database role of the session that did it, on the database's
--     clock. It is `SECURITY DEFINER` and executable by nobody but its owner: the operator command runs it, with the
--     schema-owning role, in the transaction that records the `schedules-v1` requirement (`platform_requirement_activate`),
--     so that the state and the requirement are one commit. It refuses to run unless that requirement is recorded.
--   * a guard (ENABLE ALWAYS) refuses every other change of the row and every change that moves a handed-over row back:
--     the handover has no reverse (G1011 says why a rollback below the registry release is a hazard, not a command).
--
-- Additive: an older binary never reads the new column or calls the function. The runtime role keeps SELECT on the table
-- and gets nothing on the function.

ALTER TABLE schedule_trigger_handover ADD COLUMN changed_db_role text;
ALTER TABLE schedule_trigger_handover ADD CONSTRAINT schedule_trigger_handover_db_role_consistent
    CHECK ((state = 'LEGACY') = (changed_db_role IS NULL));
ALTER TABLE schedule_trigger_handover ADD CONSTRAINT schedule_trigger_handover_db_role_bounded
    CHECK (changed_db_role IS NULL OR char_length(changed_db_role) BETWEEN 1 AND 128);
ALTER TABLE schedule_trigger_handover ADD CONSTRAINT schedule_trigger_handover_changed_by_nonempty
    CHECK (changed_by IS NULL OR char_length(changed_by) >= 1);
-- the record must read as what it says wherever it is shown (D2052, D2076): no control, bidirectional or zero-width character
ALTER TABLE schedule_trigger_handover ADD CONSTRAINT schedule_trigger_handover_changed_by_printable
    CHECK (changed_by IS NULL OR changed_by !~ '[\u0001-\u001F\u007F-\u009F\u00AD\u061C\u180E\u200B-\u200F\u202A-\u202E\u2060-\u2064\u2066-\u206F\uFEFF\uFFF9-\uFFFB]');

-- Where the schedule of a cron trigger is not equal to the trigger (D2046, D2069): the ONE definition, read by the
-- reconciliation (to repair) and by the handover and the gauge (to count), so that "nothing to repair" and "converged" are the
-- same sentence. A trigger is listed with the kind of difference when it has:
--   MISSING       no schedule at all
--   REVIVED       a tombstone (its trigger is in force again)
--   RESPEC        a schedule whose head revision is not the trigger's expression and zone
--   PAUSED        a paused schedule while the state is LEGACY: before the handover the API refuses every pause, so a pause can only be
--                 an accident, and a trigger the legacy pass would still fire must not be left paused when the pass stops. After
--                 the handover a pause is the operator's and is not listed.
--   NO_NEXT_TIME  an active schedule that is not planned: it lacks the nominal or the actual time the schedule pass acts by, and it
--                 is not exhausted. A cron whose calendar has no occurrence at all (`0 0 31 2 *`, accepted at publication as feature
--                 004 always accepted it) is planned as exhausted: no next time and no actions left, which is converged (D2069).
-- Invoker's rights: the runtime role reads it as it reads the tables.
--
-- Reading it must not depend on the plan the statistics give it (G2049, D2117). The schedule of a trigger is looked up for
-- each trigger (LEFT JOIN LATERAL ... LIMIT 1 over the partial unique index `schedules_trigger_uniq`: a trigger has at most one),
-- and the singleton `schedule_trigger_handover` is read by a scalar subquery, so that it multiplies no estimate. A plain join let
-- the planner choose, for the `EXISTS` the handover asks under its lock, a nested loop that scanned `schedules` again for every
-- trigger: an `EXISTS` is planned for its first row, the filter of this view (`kind IS NOT NULL`) is estimated to keep nearly
-- every trigger, and the singleton, which autovacuum never analyzes, is estimated at hundreds of rows, so a plan with no
-- start-up cost wins whatever its total. The form gives the planner a probe for each trigger and no join to rescan; it does not
-- by itself rule out every other plan (PostgreSQL does not take a partial unique index as proof of uniqueness), so the plan is
-- held by `trigger_plan.rs`, and the measurements are in ADR-0083 and in the benchmark report, not here.
--
-- One behaviour differs from the definition this replaced, and cannot be reached: with the singleton row missing (the table
-- has a primary key on `only_row` and ENABLE ALWAYS triggers that refuse DELETE and TRUNCATE) the old CROSS JOIN listed
-- nothing at all, and the scalar subquery lists MISSING, REVIVED and RESPEC and never PAUSED.
CREATE VIEW schedule_trigger_drift WITH (security_invoker = true) AS
SELECT d.* FROM (
    SELECT t.id AS trigger_id, t.tenant_id, t.namespace_id, t.workflow_id, t.version_id, t.cron, t.timezone, t.next_fire_at,
           CASE WHEN s.id IS NULL THEN 'MISSING'
                WHEN s.state = 'DELETED' THEN 'REVIVED'
                WHEN s.spec #>> '{cron,0}' IS DISTINCT FROM t.cron
                  OR s.spec ->> 'timezone' IS DISTINCT FROM t.timezone THEN 'RESPEC'
                WHEN s.state = 'PAUSED' THEN
                     CASE WHEN (SELECT h.state FROM public.schedule_trigger_handover h) = 'LEGACY' THEN 'PAUSED' END
                WHEN s.state = 'ACTIVE'
                 AND (s.next_action_at IS NULL OR s.next_actual_at IS NULL)
                 AND s.remaining_actions IS DISTINCT FROM 0 THEN 'NO_NEXT_TIME'
           END AS kind,
           s.id AS schedule_id, s.state_version AS state_version, s.head_revision AS revision
      FROM public.workflow_triggers t
      LEFT JOIN LATERAL (
            SELECT x.id, x.state, x.state_version, x.head_revision, x.next_action_at, x.next_actual_at, x.remaining_actions,
                   r.spec
              FROM public.schedules x
              LEFT JOIN public.schedule_revisions r
                     ON r.tenant_id = x.tenant_id AND r.schedule_id = x.id AND r.revision = x.head_revision
             WHERE x.trigger_id = t.id
             LIMIT 1) s ON true
     WHERE t.kind = 'cron') d
 WHERE d.kind IS NOT NULL;

-- How many cron triggers are listed by the view (at most `p_limit`), and up to sixteen of their ids (an id is not content): the
-- gauge's number and the dry run's words. The handover itself asks only whether there is one (`schedule_trigger_unconverged`).
CREATE FUNCTION schedule_trigger_convergence(p_limit integer)
RETURNS TABLE (out_count integer, out_examples uuid[])
LANGUAGE sql STABLE SET search_path = pg_catalog, pg_temp AS $$
    WITH unconverged AS (SELECT d.trigger_id FROM public.schedule_trigger_drift d LIMIT p_limit)
    SELECT count(*)::integer, coalesce((array_agg(u.trigger_id ORDER BY u.trigger_id))[1:16], '{}'::uuid[]) FROM unconverged u
$$;

-- Whether any cron trigger is listed: what is asked under the table lock, where a count would be work done while publications wait.
CREATE FUNCTION schedule_trigger_unconverged() RETURNS boolean
LANGUAGE sql STABLE SET search_path = pg_catalog, pg_temp AS $$
    SELECT EXISTS (SELECT 1 FROM public.schedule_trigger_drift)
$$;

-- The partial indexes the reconciliation's one query per round reads by shard (D2047): the cron triggers by the shard of their
-- id, the trigger-owned schedules by the shard of their trigger, and the cron triggers by the last time a legacy pass fired
-- them (the tripwire of an old binary that goes on firing after the handover, G2005).
CREATE INDEX workflow_triggers_cron_shard_idx ON workflow_triggers (run_shard(id), id) WHERE kind = 'cron';
CREATE INDEX schedules_trigger_shard_idx ON schedules (run_shard(trigger_id), trigger_id) WHERE trigger_id IS NOT NULL;
CREATE INDEX workflow_triggers_cron_last_fire_idx ON workflow_triggers (last_fire_at)
    WHERE kind = 'cron' AND last_fire_at IS NOT NULL;

CREATE FUNCTION schedule_trigger_handover_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF coalesce(current_setting('sagat.handover_change', true), '') <> 'handover' THEN
        RAISE EXCEPTION 'the trigger handover changes only through schedule_trigger_handover_complete'
            USING ERRCODE = 'restrict_violation';
    END IF;
    IF OLD.state = 'HANDED_OVER' THEN
        RAISE EXCEPTION 'a handed-over trigger handover is never changed or moved back'
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER schedule_trigger_handover_guard BEFORE UPDATE ON schedule_trigger_handover
    FOR EACH ROW EXECUTE FUNCTION schedule_trigger_handover_guard();
ALTER TABLE schedule_trigger_handover ENABLE ALWAYS TRIGGER schedule_trigger_handover_guard;

-- Takes the lock that keeps a publication from committing between the count and the commit of the state: SHARE ROW EXCLUSIVE on the
-- cron triggers. It waits for the writers in flight and, while it waits, holds the later ones back, so the wait is short and its
-- end is a refusal in words ("try again") and not a long stall: at most `lock_timeout` (2 s) for anything that writes the triggers
-- (a publication, the legacy cron pass), and nothing is recorded (D2070).
CREATE FUNCTION schedule_trigger_handover_lock() RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    v_previous text := current_setting('lock_timeout');
BEGIN
    PERFORM set_config('lock_timeout', '2s', true);
    BEGIN
        LOCK TABLE public.workflow_triggers IN SHARE ROW EXCLUSIVE MODE;
    EXCEPTION WHEN lock_not_available THEN
        RAISE EXCEPTION 'HANDOVER_BUSY: a publication or a cron pass is writing the triggers; nothing was changed, run the command again'
            USING ERRCODE = 'lock_not_available';
    END;
    PERFORM set_config('lock_timeout', v_previous, true);
END;
$$;

-- Hands the cron triggers over to the schedule pass. outcome: HANDED_OVER | ALREADY_HANDED_OVER.
--
-- The requirement of `schedules-v1` for the execution engines must be recorded (by `platform_requirement_activate`, in
-- the same transaction as this call, or before): without it nothing stops an engine that cannot fire a schedule from
-- being started after the legacy pass stopped, which is the hazard the handover exists to close.
--
-- Locks, in this order and no other (D2070): the requirement epoch (the caller's), the cron triggers' table, the handover row.
-- The census takes the same first two, so two operators can never wait for each other. Only the state `LEGACY` is moved; any other
-- state (one a newer build wrote) is refused, and `HANDED_OVER` is an answer.
CREATE FUNCTION schedule_trigger_handover_complete(p_actor text) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    v_state text;
BEGIN
    IF p_actor IS NULL OR char_length(p_actor) NOT BETWEEN 1 AND 255 THEN
        RAISE EXCEPTION 'the handover names the operator that did it, 1 to 255 characters'
            USING ERRCODE = 'invalid_parameter_value';
    END IF;
    -- a handover is never undone, so reading it without a lock is an answer
    SELECT h.state INTO STRICT v_state FROM public.schedule_trigger_handover h;
    IF v_state = 'HANDED_OVER' THEN
        RETURN 'ALREADY_HANDED_OVER';
    END IF;
    PERFORM public.schedule_trigger_handover_lock();
    SELECT h.state INTO STRICT v_state FROM public.schedule_trigger_handover h FOR UPDATE;
    IF v_state = 'HANDED_OVER' THEN
        RETURN 'ALREADY_HANDED_OVER';
    END IF;
    IF v_state <> 'LEGACY' THEN
        RAISE EXCEPTION 'the handover state is %, which this build does not know how to move', v_state
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM public.platform_requirements r
                    WHERE r.capability = 'schedules-v1' AND 'EXECUTION_ENGINE' = ANY (r.kinds)) THEN
        RAISE EXCEPTION 'the requirement schedules-v1 for the execution engines is not recorded'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
    -- No publication commits between this check and the commit of the state, and under the lock the check is whether there is
    -- one, not how many (the number and the names are asked for outside the lock, after a refusal).
    IF public.schedule_trigger_unconverged() THEN
        RAISE EXCEPTION 'HANDOVER_UNCONVERGED: cron trigger(s) have no schedule equal to them'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
    PERFORM set_config('sagat.handover_change', 'handover', true);
    UPDATE public.schedule_trigger_handover
       SET state = 'HANDED_OVER', changed_by = p_actor, changed_at = clock_timestamp(),
           changed_db_role = session_user::text;
    PERFORM set_config('sagat.handover_change', '', true);
    RETURN 'HANDED_OVER';
END;
$$;

-- What the operator is told before and during the handover: how many execution engines are live (seen within 30 seconds),
-- how many of them do not list `schedules-v1`, and up to sixteen of their ids (an id is not content); and the cron triggers
-- whose schedule is not equal to them. Without the lock (the dry run, outside any activation) that is a count and up to sixteen
-- ids; with it (inside the activation, where publications wait) it is only whether there is one: 0 or 1, no ids (D2071). The
-- registry table is not the runtime role's to read whole (its token column is not), so the owner reads it through this
-- function; it is the operator command's and nobody else's.
CREATE FUNCTION schedule_trigger_handover_census(p_lock boolean)
RETURNS TABLE (out_live integer, out_lacking integer, out_examples uuid[], out_unconverged integer, out_unconverged_examples uuid[])
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    IF p_lock THEN
        -- held to the end of the activation's transaction: no publication commits meanwhile (see complete)
        PERFORM public.schedule_trigger_handover_lock();
    END IF;
    RETURN QUERY
    SELECT e.live, e.lacking, e.examples,
           CASE WHEN p_lock THEN (public.schedule_trigger_unconverged())::integer ELSE c.out_count END,
           CASE WHEN p_lock THEN '{}'::uuid[] ELSE c.out_examples END
      FROM (SELECT count(*)::integer AS live,
                   (count(*) FILTER (WHERE NOT ('schedules-v1' = ANY (s.capabilities))))::integer AS lacking,
                   coalesce((array_agg(s.instance_id ORDER BY s.instance_id)
                               FILTER (WHERE NOT ('schedules-v1' = ANY (s.capabilities))))[1:16], '{}'::uuid[]) AS examples
              FROM public.serving_instances s
             WHERE s.kind = 'EXECUTION_ENGINE' AND s.last_seen > clock_timestamp() - interval '30 seconds') e
      LEFT JOIN LATERAL (SELECT * FROM public.schedule_trigger_convergence(100000)) c ON NOT p_lock;
END;
$$;

-- No requirement of `schedules-v1` can be removed once the cron triggers were handed over (D2048): an engine without the
-- schedule pass could register and nothing would fire the triggers.
CREATE FUNCTION platform_requirements_handover_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.capability = 'schedules-v1'
       AND EXISTS (SELECT 1 FROM public.schedule_trigger_handover h WHERE h.state = 'HANDED_OVER') THEN
        RAISE EXCEPTION 'schedules-v1 cannot be removed after the cron triggers were handed over to the schedule pass'
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN OLD;
END;
$$;
CREATE TRIGGER platform_requirements_handover_guard BEFORE DELETE ON platform_requirements
    FOR EACH ROW EXECUTE FUNCTION platform_requirements_handover_guard();
ALTER TABLE platform_requirements ENABLE ALWAYS TRIGGER platform_requirements_handover_guard;

REVOKE ALL ON FUNCTION schedule_trigger_handover_complete(text) FROM PUBLIC;
REVOKE ALL ON FUNCTION schedule_trigger_handover_census(boolean) FROM PUBLIC;
REVOKE ALL ON FUNCTION schedule_trigger_handover_lock() FROM PUBLIC;
DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        REVOKE ALL ON FUNCTION schedule_trigger_handover_complete(text) FROM sagat_app;
        REVOKE ALL ON FUNCTION schedule_trigger_handover_census(boolean) FROM sagat_app;
        REVOKE ALL ON FUNCTION schedule_trigger_handover_lock() FROM sagat_app;
        GRANT SELECT ON schedule_trigger_drift TO sagat_app;
        GRANT EXECUTE ON FUNCTION schedule_trigger_convergence(integer) TO sagat_app;
        GRANT EXECUTE ON FUNCTION schedule_trigger_unconverged() TO sagat_app;
    END IF;
END $grants$;
