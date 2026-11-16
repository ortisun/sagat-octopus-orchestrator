-- Undoes 20270130000500_flow_control_key_rotation.up.sql. Rollback is an operator/quiesced-service operation. The command
-- of 20270130000000 cannot finish a rotation that is in progress (it refuses while a row uses another version and has
-- no retiring state of its own to end), so rolling back refuses while a retiring version still has rows. The recorded
-- roles are dropped with their column: the operator names stay.
LOCK TABLE flow_control_key_versions IN EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM flow_control_key_versions k
                WHERE k.state = 'RETIRING' AND flow_control_key_version_rows(k.version) > 0) THEN
        RAISE EXCEPTION 'a digest key rotation is in progress; let its last rows go before rolling back'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;
-- a retiring version no row uses is retired, so that the restored command cannot make it active again
UPDATE flow_control_key_versions k SET state = 'RETIRED', retired_at = clock_timestamp()
 WHERE k.state = 'RETIRING' AND flow_control_key_version_rows(k.version) = 0;

DROP INDEX flow_control_key_versions_one_retiring;
DROP FUNCTION flow_control_key_versions_in_force();
DROP FUNCTION flow_control_key_activate(smallint, text);
-- the command as 20270130000100 left it
CREATE FUNCTION flow_control_key_activate(p_version smallint, p_actor text)
RETURNS TABLE (out_outcome text, out_rows bigint)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    v_rows bigint;
BEGIN
    LOCK TABLE public.flow_control_key_versions IN EXCLUSIVE MODE;
    LOCK TABLE public.concurrency_semaphores, public.rate_limit_windows IN SHARE MODE;
    IF EXISTS (SELECT 1 FROM public.flow_control_key_versions WHERE version = p_version AND state = 'ACTIVE') THEN
        RETURN QUERY SELECT 'ALREADY_ACTIVE'::text, 0::bigint;
        RETURN;
    END IF;
    -- a retired version (for instance one whose key leaked) is never active again
    IF EXISTS (SELECT 1 FROM public.flow_control_key_versions WHERE version = p_version AND state = 'RETIRED') THEN
        RETURN QUERY SELECT 'RETIRED'::text, 0::bigint;
        RETURN;
    END IF;
    SELECT (SELECT count(*) FROM public.concurrency_semaphores WHERE key_version <> p_version)
         + (SELECT count(*) FROM public.rate_limit_windows WHERE key_version <> p_version)
      INTO v_rows;
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
DROP FUNCTION flow_control_key_version_rows(smallint);
ALTER TABLE flow_control_key_versions DROP CONSTRAINT flow_control_key_versions_role_bounded;
ALTER TABLE flow_control_key_versions DROP COLUMN activated_role;
