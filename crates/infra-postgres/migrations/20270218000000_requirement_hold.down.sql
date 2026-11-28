-- Reverses 20270218000000_requirement_hold. Refused while a design has registered a floor: that design's own down migration
-- removes its row and its function first, so a rollback never leaves a capability whose floor nobody enforces.
DO $refuse$
BEGIN
    IF EXISTS (SELECT 1 FROM platform_requirement_floors) THEN
        RAISE EXCEPTION 'platform_requirement_floors still holds a floor: roll back the migration that registered it first'
            USING ERRCODE = 'restrict_violation';
    END IF;
END
$refuse$;

DROP FUNCTION platform_requirement_hold(text);
DROP TRIGGER platform_requirement_floor ON platform_requirements;
DROP TABLE platform_requirement_floors;
DROP FUNCTION platform_requirement_floor();
DROP FUNCTION platform_requirement_floors_check();
DROP FUNCTION platform_requirement_floors_guard();
DROP FUNCTION platform_requirement_floor_admissible(regprocedure);

-- The operator's three functions and the registration, as `20270113000000` (and `20270208000200` for the bound of the removal)
-- left them: without the gate.
CREATE OR REPLACE FUNCTION platform_requirement_lock() RETURNS bigint
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    v_epoch bigint;
BEGIN
    SELECT e.epoch INTO STRICT v_epoch FROM public.platform_requirement_epoch e FOR UPDATE;
    RETURN v_epoch;
END;
$$;

CREATE OR REPLACE FUNCTION platform_requirement_activate(p_capability text, p_kinds text[], p_actor text)
RETURNS TABLE (out_outcome text, out_epoch bigint, out_missing integer, out_examples uuid[])
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    v_epoch    bigint;
    v_kinds    text[];
    v_existing text[];
    v_missing  integer;
    v_examples uuid[];
BEGIN
    v_kinds := coalesce(ARRAY(SELECT DISTINCT k FROM unnest(p_kinds) AS k ORDER BY k), '{}');
    SELECT e.epoch INTO STRICT v_epoch FROM public.platform_requirement_epoch e FOR UPDATE;
    SELECT r.kinds INTO v_existing FROM public.platform_requirements r WHERE r.capability = p_capability;
    IF FOUND THEN
        IF v_existing = v_kinds THEN
            RETURN QUERY SELECT 'ALREADY_RECORDED'::text, v_epoch, 0, '{}'::uuid[];
        ELSE
            RETURN QUERY SELECT 'KINDS_DIFFER'::text, v_epoch, 0, '{}'::uuid[];
        END IF;
        RETURN;
    END IF;
    SELECT count(*)::integer, (array_agg(s.instance_id ORDER BY s.instance_id))[1:16] INTO v_missing, v_examples
      FROM public.serving_instances s
     WHERE s.kind = ANY (v_kinds)
       AND s.last_seen > clock_timestamp() - interval '30 seconds'
       AND NOT (p_capability = ANY (s.capabilities));
    IF v_missing > 0 THEN
        RETURN QUERY SELECT 'INSTANCES_LACK_CAPABILITY'::text, v_epoch, v_missing, v_examples;
        RETURN;
    END IF;
    PERFORM set_config('sagat.registry_change', 'activation', true);
    INSERT INTO public.platform_requirements (capability, kinds, required_since, recorded_by)
    VALUES (p_capability, v_kinds, clock_timestamp(), p_actor);
    UPDATE public.platform_requirement_epoch SET epoch = epoch + 1 RETURNING epoch INTO v_epoch;
    INSERT INTO public.platform_requirement_history (action, capability, kinds, actor, database_user, epoch, at)
    VALUES ('ACTIVATED', p_capability, v_kinds, p_actor, session_user, v_epoch, clock_timestamp());
    PERFORM set_config('sagat.registry_change', '', true);
    RETURN QUERY SELECT 'RECORDED'::text, v_epoch, 0, '{}'::uuid[];
END;
$$;

CREATE OR REPLACE FUNCTION platform_requirement_remove(p_capability text, p_actor text)
RETURNS TABLE (out_outcome text, out_epoch bigint)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp SET lock_timeout = '1s' AS $$
DECLARE
    v_epoch bigint;
    v_kinds text[];
BEGIN
    SELECT e.epoch INTO STRICT v_epoch FROM public.platform_requirement_epoch e FOR UPDATE;
    SELECT r.kinds INTO v_kinds FROM public.platform_requirements r WHERE r.capability = p_capability;
    IF NOT FOUND THEN
        RETURN QUERY SELECT 'NOT_RECORDED'::text, v_epoch;
        RETURN;
    END IF;
    PERFORM set_config('sagat.registry_change', 'activation', true);
    DELETE FROM public.platform_requirements WHERE capability = p_capability;
    UPDATE public.platform_requirement_epoch SET epoch = epoch + 1 RETURNING epoch INTO v_epoch;
    INSERT INTO public.platform_requirement_history (action, capability, kinds, actor, database_user, epoch, at)
    VALUES ('REMOVED', p_capability, v_kinds, p_actor, session_user, v_epoch, clock_timestamp());
    PERFORM set_config('sagat.registry_change', '', true);
    RETURN QUERY SELECT 'REMOVED'::text, v_epoch;
END;
$$;

CREATE OR REPLACE FUNCTION instance_register(
    p_instance uuid, p_token bytea, p_kind text, p_scheduler_owner text, p_version text, p_capabilities text[])
RETURNS TABLE (out_outcome text, out_epoch bigint, out_missing text[])
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    v_epoch   bigint;
    v_caps    text[];
    v_missing text[];
    v_count   integer;
    v_rows    integer;
    v_hash    bytea;
BEGIN
    IF p_token IS NULL OR octet_length(p_token) < 16 THEN
        RAISE EXCEPTION 'an instance registers with a token of at least 16 bytes'
            USING ERRCODE = 'invalid_parameter_value';
    END IF;
    v_hash := sha256(p_token);
    SELECT e.epoch INTO STRICT v_epoch FROM public.platform_requirement_epoch e FOR SHARE;
    v_caps := coalesce(ARRAY(SELECT DISTINCT c FROM unnest(p_capabilities) AS c ORDER BY c), '{}');
    SELECT coalesce(array_agg(r.capability ORDER BY r.capability), '{}') INTO v_missing
      FROM public.platform_requirements r
     WHERE p_kind = ANY (r.kinds) AND NOT (r.capability = ANY (v_caps));
    IF cardinality(v_missing) > 0 THEN
        RETURN QUERY SELECT 'CAPABILITY_MISSING'::text, v_epoch, v_missing;
        RETURN;
    END IF;

    IF NOT EXISTS (SELECT 1 FROM public.serving_instances s WHERE s.instance_id = p_instance) THEN
        -- registrations are counted one at a time, so that the table never holds more than its limit
        PERFORM pg_advisory_xact_lock(hashtextextended('sagat:serving_instances:capacity', 0));
        SELECT count(*) INTO v_count FROM public.serving_instances;
        IF v_count >= 1000 THEN
            -- the rows that are no longer live give way (a crash loop leaves one a start, and must not lock the
            -- fleet out); a table of live rows is full
            DELETE FROM public.serving_instances d WHERE d.instance_id IN (
                SELECT s.instance_id FROM public.serving_instances s
                 WHERE s.last_seen < clock_timestamp() - interval '30 seconds'
                 ORDER BY s.last_seen, s.instance_id
                 LIMIT v_count - 999
                 FOR UPDATE SKIP LOCKED);
            SELECT count(*) INTO v_count FROM public.serving_instances;
            IF v_count >= 1000 THEN
                RETURN QUERY SELECT 'FULL'::text, v_epoch, '{}'::text[];
                RETURN;
            END IF;
        END IF;
    END IF;

    INSERT INTO public.serving_instances AS s
        (instance_id, kind, scheduler_owner, binary_version, capabilities, started_at, last_seen, token_hash)
    VALUES (p_instance, p_kind, p_scheduler_owner, p_version, v_caps, clock_timestamp(), clock_timestamp(), v_hash)
    ON CONFLICT (instance_id) DO UPDATE
        SET scheduler_owner = EXCLUDED.scheduler_owner,
            binary_version  = EXCLUDED.binary_version,
            capabilities    = EXCLUDED.capabilities,
            last_seen       = EXCLUDED.last_seen
        WHERE s.kind = EXCLUDED.kind AND s.token_hash = EXCLUDED.token_hash;
    GET DIAGNOSTICS v_rows = ROW_COUNT;
    IF v_rows = 0 THEN
        -- the row is another process's, or the same process under another kind: it is not touched
        RAISE EXCEPTION 'an instance id belongs to the process that registered it, under the kind it registered with'
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN QUERY SELECT 'REGISTERED'::text, v_epoch, '{}'::text[];
END;
$$;

-- The automatic handover, as `20270117000000` left it.
CREATE OR REPLACE FUNCTION schedule_trigger_handover_auto(p_run uuid)
RETURNS TABLE (out_outcome text, out_reason text, out_epoch bigint, out_live integer, out_tenants integer)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    v_epoch   bigint;
    v_state   text;
    v_now     timestamptz;
    v_actor   text;
    v_tenants integer;
    a         public.schedule_trigger_handover_attempts%ROWTYPE;
    c         record;
    r         record;
BEGIN
    IF session_user::text <> 'sagat_handover' THEN
        RAISE EXCEPTION 'the automatic handover is made only by the role sagat_handover, logged in as itself'
            USING ERRCODE = 'insufficient_privilege';
    END IF;
    IF p_run IS NULL THEN
        RAISE EXCEPTION 'the automatic handover names its run' USING ERRCODE = 'invalid_parameter_value';
    END IF;
    v_actor := 'system:handover-job:' || p_run::text;
    SELECT e.epoch INTO STRICT v_epoch FROM public.platform_requirement_epoch e FOR UPDATE;
    SELECT h.state INTO STRICT v_state FROM public.schedule_trigger_handover h;
    IF v_state = 'HANDED_OVER' THEN
        RETURN QUERY SELECT 'ALREADY_HANDED_OVER'::text, NULL::text,
                            (SELECT h.changed_epoch FROM public.schedule_trigger_handover h), 0, 0;
        RETURN;
    END IF;
    IF v_state <> 'LEGACY' THEN
        RETURN QUERY SELECT 'UNKNOWN_STATE'::text, NULL::text, v_epoch, 0, 0;
        RETURN;
    END IF;
    SELECT * INTO a FROM public.schedule_trigger_handover_attempts t WHERE t.run_id = p_run;
    IF NOT FOUND THEN
        RETURN QUERY SELECT 'REFUSED'::text, 'FIRST_HORIZON'::text, v_epoch, 0, 0;
        RETURN;
    END IF;
    v_now := clock_timestamp();
    IF v_now >= a.deadline THEN
        RETURN QUERY SELECT 'REFUSED'::text, 'DEADLINE'::text, v_epoch, 0, 0;
        RETURN;
    END IF;
    IF v_now < a.first_round_at + a.horizon THEN
        RETURN QUERY SELECT 'REFUSED'::text, 'FIRST_HORIZON'::text, v_epoch, 0, 0;
        RETURN;
    END IF;
    -- too few stable rounds, or a stable count from long ago: the fleet has not been seen still, now
    IF a.stable_count < a.stable_wanted OR a.last_stable_at IS NULL
       OR a.last_stable_at <= v_now - interval '12 seconds' THEN
        RETURN QUERY SELECT 'REFUSED'::text, 'FLEET_CHANGING'::text, v_epoch, 0, 0;
        RETURN;
    END IF;
    PERFORM public.schedule_trigger_handover_lock();
    PERFORM 1 FROM public.schedule_trigger_handover h FOR UPDATE;
    SELECT * INTO STRICT c
      FROM public.schedule_trigger_handover_conditions(a.horizon, public.schedule_trigger_handover_min_live(a.min_live));
    IF c.out_reason IS NOT NULL THEN
        RETURN QUERY SELECT 'REFUSED'::text, c.out_reason, v_epoch, c.out_live, 0;
        RETURN;
    END IF;
    IF a.stable_set IS DISTINCT FROM c.out_set THEN
        RETURN QUERY SELECT 'REFUSED'::text, 'FLEET_CHANGING'::text, v_epoch, c.out_live, 0;
        RETURN;
    END IF;
    SELECT * INTO STRICT r FROM public.platform_requirement_activate('schedules-v1', ARRAY['EXECUTION_ENGINE'], v_actor);
    IF r.out_outcome = 'INSTANCES_LACK_CAPABILITY' THEN
        RETURN QUERY SELECT 'REFUSED'::text, 'ENGINE_LACKS_CAPABILITY'::text, v_epoch, c.out_live, 0;
        RETURN;
    END IF;
    IF r.out_outcome = 'KINDS_DIFFER' THEN
        RETURN QUERY SELECT 'KINDS_DIFFER'::text, NULL::text, v_epoch, c.out_live, 0;
        RETURN;
    END IF;
    v_tenants := public.schedule_trigger_handover_record(v_actor, 'AUTOMATIC', a.horizon);
    RETURN QUERY SELECT 'HANDED_OVER'::text, NULL::text, (SELECT h.changed_epoch FROM public.schedule_trigger_handover h),
                        c.out_live, coalesce(v_tenants, 0);
END;
$$;

-- The gate, once no function names it.
DROP TABLE platform_requirement_gate;
