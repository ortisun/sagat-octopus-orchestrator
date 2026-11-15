-- A handover that happened is one-way (ADR-0091 decision 10): while the row records a mode, the cron triggers were handed over and
-- its census copy, tenant set and audit ids are the evidence of it. Rolling back is refused; keep a compatible reader.
LOCK TABLE schedule_trigger_handover IN ACCESS EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM schedule_trigger_handover WHERE changed_mode IS NOT NULL OR state <> 'LEGACY') THEN
        RAISE EXCEPTION 'the cron triggers were handed over to the schedule pass; keep a compatible reader'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

-- the operator's function as migration 20270114000000 wrote it
CREATE OR REPLACE FUNCTION schedule_trigger_handover_complete(p_actor text) RETURNS text
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
REVOKE ALL ON FUNCTION schedule_trigger_handover_complete(text) FROM PUBLIC;

-- The role `sagat_handover` is a role of the cluster, not of this schema, and may serve another database: it is left as it is,
-- with nothing to execute here once the functions are gone.
ALTER FUNCTION platform_requirements_epoch_moved() SECURITY INVOKER;
ALTER FUNCTION platform_requirements_epoch_moved() RESET search_path;
DROP FUNCTION schedule_trigger_handover_role_grants();
DROP FUNCTION schedule_trigger_handover_role_ensure();
DROP FUNCTION schedule_trigger_handover_retire();
DROP FUNCTION schedule_trigger_handover_audit();
DROP FUNCTION schedule_trigger_handover_auto(uuid);
DROP FUNCTION schedule_trigger_handover_record(text, text, interval);
DROP FUNCTION schedule_trigger_handover_round(uuid, integer, interval, integer, interval, text);
DROP FUNCTION schedule_trigger_handover_conditions(interval, integer);
DROP INDEX schedules_reconciled_at_idx;
ALTER TABLE schedules DROP COLUMN reconciled_at;
DROP TABLE schedule_trigger_handover_attempts;
DROP FUNCTION schedule_trigger_handover_reasons();
DROP TABLE schedule_trigger_handover_retirement;
DROP TABLE schedule_trigger_handover_tenants;
DROP FUNCTION schedule_trigger_handover_tenants_guard();
DROP TABLE schedule_trigger_handover_census;
ALTER TABLE schedule_trigger_handover DROP CONSTRAINT schedule_trigger_handover_epoch_consistent;
ALTER TABLE schedule_trigger_handover DROP CONSTRAINT schedule_trigger_handover_mode_consistent;
ALTER TABLE schedule_trigger_handover DROP CONSTRAINT schedule_trigger_handover_mode_valid;
ALTER TABLE schedule_trigger_handover DROP COLUMN changed_epoch;
ALTER TABLE schedule_trigger_handover DROP COLUMN changed_mode;
