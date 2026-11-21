-- Rollback is an operator/quiesced-service operation. A window, a grant, a waiter or a journal that holds a rate-limit
-- event says what the engine of this version understood: rolling back never destroys any of them, so it refuses while
-- one exists.
LOCK TABLE rate_limit_windows, rate_limit_grants, rate_limit_waiters, run_events IN SHARE ROW EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM rate_limit_windows)
       OR EXISTS (SELECT 1 FROM rate_limit_grants)
       OR EXISTS (SELECT 1 FROM rate_limit_waiters)
       OR EXISTS (SELECT 1 FROM run_events WHERE event_type LIKE 'rate\_limit.%') THEN
        RAISE EXCEPTION 'rate-limit windows, grants, waiters or their journal events are retained; keep a compatible reader'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

-- the rotation refusal as the concurrency migration wrote it
CREATE OR REPLACE FUNCTION flow_control_key_activate(p_version smallint, p_actor text)
RETURNS TABLE (out_outcome text, out_rows bigint)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    v_rows bigint;
BEGIN
    LOCK TABLE public.flow_control_key_versions IN EXCLUSIVE MODE;
    LOCK TABLE public.concurrency_semaphores IN SHARE MODE;
    IF EXISTS (SELECT 1 FROM public.flow_control_key_versions WHERE version = p_version AND state = 'ACTIVE') THEN
        RETURN QUERY SELECT 'ALREADY_ACTIVE'::text, 0::bigint;
        RETURN;
    END IF;
    -- a retired version (for instance one whose key leaked) is never active again
    IF EXISTS (SELECT 1 FROM public.flow_control_key_versions WHERE version = p_version AND state = 'RETIRED') THEN
        RETURN QUERY SELECT 'RETIRED'::text, 0::bigint;
        RETURN;
    END IF;
    SELECT count(*) INTO v_rows FROM public.concurrency_semaphores WHERE key_version <> p_version;
    IF v_rows > 0 THEN
        RETURN QUERY SELECT 'ROWS_UNDER_OTHER_VERSION'::text, v_rows;
        RETURN;
    END IF;
    UPDATE public.flow_control_key_versions SET state = 'RETIRING' WHERE state = 'ACTIVE';
    INSERT INTO public.flow_control_key_versions (version, state, activated_at, activated_by)
    VALUES (p_version, 'ACTIVE', clock_timestamp(), p_actor)
    ON CONFLICT (version) DO UPDATE
        SET state = 'ACTIVE', activated_at = EXCLUDED.activated_at, activated_by = EXCLUDED.activated_by,
            retired_at = NULL;
    RETURN QUERY SELECT 'ACTIVATED'::text, 0::bigint;
END;
$$;
REVOKE ALL ON FUNCTION flow_control_key_activate(smallint, text) FROM PUBLIC;

DROP TABLE rate_limit_waiters;
DROP TABLE rate_limit_grants;
DROP TABLE rate_limit_windows;
DROP FUNCTION rate_limit_rows_guard();
DROP FUNCTION rate_limit_window_prune();
DROP FUNCTION rate_limit_window_count();
