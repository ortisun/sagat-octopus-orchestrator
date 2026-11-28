-- The shared prerequisite of ADR-0112 (design 054's Task 054-18, built first by design 045 under its Task 045-21; Owner decisions
-- 2026-11-26-540 and 2026-11-26-564): the one publication hold of the fleet requirements, the one registry of their replay floors,
-- and the fairness gate of the amendment of 2026-11-27 that keeps publications from starving the operator.
--
-- 1. `platform_requirement_floors`: a row per capability that has a floor, naming the function that says whether the floor holds.
--    Only a migration writes it (no grant to the runtime role); its check trigger admits a function with no argument, a boolean
--    result and the owner of the floor trigger. A design adds one row and one function; it never replaces another's code.
-- 2. `platform_requirement_floor()`: `BEFORE DELETE` on `platform_requirements` (a requirement is deleted only inside
--    `platform_requirement_remove`), enabled ALWAYS. For a capability with a floor row it refuses outside READ COMMITTED (after
--    waiting for a publication's hold it must read the committed versions) and refuses with FLOOR_HELD when the floor's function
--    answers true. A capability with no floor row is removed as before.
-- 3. `platform_requirement_hold(capability)`: what a publication calls as its last statement before its commit. Its body is the
--    text of ADR-0112's amendment, compared by the drift guard of every later migration that finds it.
-- 4. The gate: a lock on the empty table `platform_requirement_gate`, before the epoch in the registry's lock order, taken IN
--    EXCLUSIVE MODE by the operator's three functions and IN SHARE MODE by the hold and a registration. PostgreSQL 16 lets a new
--    FOR SHARE of a row go ahead of a waiting FOR UPDATE; its lock manager does not let a SHARE request go ahead of a waiting
--    EXCLUSIVE one.
--    The three operator functions and the registration are rewritten in place with their bodies of `20270113000000` and the gate
--    as their first lock; `CREATE OR REPLACE` keeps their owner and grants, and restates their configuration.
--
-- The automatic handover (ADR-0091), which takes the epoch FOR UPDATE and then calls the activation, takes the gate exclusively
-- first, so that no function waits for the gate while it holds the epoch.
--
-- Not covered (Gap 045-006): the beat (`instance_heartbeat_reporting_change`) reads the epoch FOR SHARE SKIP LOCKED without the gate,
-- because it never waits; beats can still overlap an operator's wait, bounded by the beat rate.

CREATE TABLE platform_requirement_floors (
    capability text         PRIMARY KEY,
    holds      regprocedure NOT NULL,
    CONSTRAINT platform_requirement_floors_capability_known CHECK (
        capability IN ('schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1',
                       'lease-withdrawal-v1', 'lease-withdrawal-v2'))
);

-- The gate of ADR-0112's amendment: an empty table that no role but its owner may lock. The operator's functions and the automatic
-- handover lock it IN EXCLUSIVE MODE, the hold and a registration IN SHARE MODE, before the epoch; the lock manager queues a SHARE
-- request behind a waiting EXCLUSIVE one. A relation lock, not an advisory lock, because LOCK TABLE checks privileges: no other
-- role can take the gate, and no hashed advisory key of another feature can collide with it (security review of 2026-11-27).
CREATE TABLE platform_requirement_gate ();

-- Removes a requirement only when its floor allows it. SECURITY INVOKER: it runs as whoever deletes the row, which is the owner,
-- inside `platform_requirement_remove`.
CREATE FUNCTION platform_requirement_floor() RETURNS trigger
LANGUAGE plpgsql SECURITY INVOKER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    v_holds regprocedure;
    v_held  boolean;
BEGIN
    SELECT f.holds INTO v_holds FROM public.platform_requirement_floors f WHERE f.capability = OLD.capability;
    IF NOT FOUND THEN
        RETURN OLD;
    END IF;
    IF current_setting('transaction_isolation') <> 'read committed' THEN
        RAISE EXCEPTION '% is removed only under READ COMMITTED (this transaction runs under %): a stricter level keeps a snapshot that cannot see a version committed meanwhile', OLD.capability, current_setting('transaction_isolation')
            USING ERRCODE = 'restrict_violation';
    END IF;
    IF NOT public.platform_requirement_floor_admissible(v_holds) THEN
        RAISE EXCEPTION 'FLOOR_HELD: % cannot be removed: its floor (%) is no longer a function the registry admits', OLD.capability, v_holds::oid
            USING ERRCODE = 'restrict_violation';
    END IF;
    EXECUTE format('SELECT %s', v_holds) INTO v_held;
    IF v_held IS DISTINCT FROM false THEN
        RAISE EXCEPTION 'FLOOR_HELD: % cannot be removed while its floor holds (%): a build without it cannot honour what depends on it', OLD.capability, v_holds
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN OLD;
END;
$$;

-- Whether `p_holds` may be a floor: a plain function (not a procedure, an aggregate or a set-returning function) with no argument,
-- a boolean result, the owner of the floor trigger, run as that owner with the fixed search path, and the only function of its
-- schema with that name (an overload with defaults would make the trigger's call ambiguous). Read when a floor is registered and
-- again before each call, because a function can be dropped, replaced or given to another owner after it was registered.
CREATE FUNCTION platform_requirement_floor_admissible(p_holds regprocedure) RETURNS boolean
LANGUAGE sql STABLE SECURITY INVOKER SET search_path = pg_catalog, pg_temp AS $$
    SELECT EXISTS (
        SELECT 1 FROM pg_catalog.pg_proc p
         WHERE p.oid = p_holds
           AND p.prokind = 'f' AND NOT p.proretset AND p.pronargs = 0
           AND p.prorettype = 'pg_catalog.bool'::pg_catalog.regtype
           AND p.prosecdef
           AND p.proconfig @> ARRAY['search_path=pg_catalog, pg_temp']
           AND p.proowner = (SELECT t.proowner FROM pg_catalog.pg_proc t
                              WHERE t.oid = 'public.platform_requirement_floor()'::pg_catalog.regprocedure)
           AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_proc o
                            WHERE o.pronamespace = p.pronamespace AND o.proname = p.proname AND o.oid <> p.oid))
$$;

-- Admits a floor only through `platform_requirement_floor_admissible`.
CREATE FUNCTION platform_requirement_floors_check() RETURNS trigger
LANGUAGE plpgsql SECURITY INVOKER SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    IF NOT public.platform_requirement_floor_admissible(NEW.holds) THEN
        RAISE EXCEPTION 'a floor names a plain function with no argument and a boolean result, owned by the owner of platform_requirement_floor(), SECURITY DEFINER with search_path pg_catalog, pg_temp, and with no overload'
            USING ERRCODE = 'invalid_parameter_value';
    END IF;
    RETURN NEW;
END;
$$;

-- A floor is removed only by the down migration of the design that registered it, which sets `sagat.floor_change` for its own
-- transaction; a hand edit of the owner is refused, as the registry's own guards refuse one (a seat belt, not a secret).
CREATE FUNCTION platform_requirement_floors_guard() RETURNS trigger
LANGUAGE plpgsql SECURITY INVOKER SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    IF coalesce(current_setting('sagat.floor_change', true), '') <> 'migration' THEN
        RAISE EXCEPTION 'a floor is removed only by the down migration of the design that registered it'
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN OLD;
END;
$$;

CREATE TRIGGER platform_requirement_floors_check BEFORE INSERT OR UPDATE ON platform_requirement_floors
    FOR EACH ROW EXECUTE FUNCTION platform_requirement_floors_check();
CREATE TRIGGER platform_requirement_floors_guard BEFORE DELETE ON platform_requirement_floors
    FOR EACH ROW EXECUTE FUNCTION platform_requirement_floors_guard();
CREATE TRIGGER platform_requirement_floors_no_truncate BEFORE TRUNCATE ON platform_requirement_floors
    FOR EACH STATEMENT EXECUTE FUNCTION execution_reject_mutation();
CREATE TRIGGER platform_requirement_floor BEFORE DELETE ON platform_requirements
    FOR EACH ROW EXECUTE FUNCTION platform_requirement_floor();
ALTER TABLE platform_requirement_floors ENABLE ALWAYS TRIGGER platform_requirement_floors_check;
ALTER TABLE platform_requirement_floors ENABLE ALWAYS TRIGGER platform_requirement_floors_guard;
ALTER TABLE platform_requirement_floors ENABLE ALWAYS TRIGGER platform_requirement_floors_no_truncate;
ALTER TABLE platform_requirements ENABLE ALWAYS TRIGGER platform_requirement_floor;

CREATE FUNCTION public.platform_requirement_hold(capability text) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    v_kinds text[];
BEGIN
    IF NOT EXISTS (SELECT 1 FROM public.platform_requirement_floors f
                    WHERE f.capability = platform_requirement_hold.capability) THEN
        RAISE EXCEPTION 'no floor is registered for this capability' USING ERRCODE = 'invalid_parameter_value';
    END IF;
    LOCK TABLE public.platform_requirement_gate IN SHARE MODE;
    PERFORM 1 FROM public.platform_requirement_epoch e WHERE e.only_row FOR SHARE;
    SELECT r.kinds INTO v_kinds FROM public.platform_requirements r
     WHERE r.capability = platform_requirement_hold.capability;
    RETURN FOUND AND v_kinds @> ARRAY['CONTROL_PLANE', 'EXECUTION_ENGINE']::text[];
END;
$$;

-- The operator's three functions: the gate exclusively, then the epoch FOR UPDATE. Bodies otherwise those of `20270113000000`.
CREATE OR REPLACE FUNCTION platform_requirement_lock() RETURNS bigint
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    v_epoch bigint;
BEGIN
    LOCK TABLE public.platform_requirement_gate IN EXCLUSIVE MODE;
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
    LOCK TABLE public.platform_requirement_gate IN EXCLUSIVE MODE;
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
    LOCK TABLE public.platform_requirement_gate IN EXCLUSIVE MODE;
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

-- The automatic handover (ADR-0091) takes the epoch FOR UPDATE and then calls platform_requirement_activate, which now takes the
-- gate: without the gate first it would wait for the gate while holding the epoch, which deadlocks with a registration or a hold
-- that holds the gate shared and waits for the epoch. So it takes the gate exclusively first. Body otherwise that of
-- `20270117000000`.
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
    LOCK TABLE public.platform_requirement_gate IN EXCLUSIVE MODE;
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

-- The registration: the gate shared, then the epoch FOR SHARE. Body otherwise that of `20270113000000`.
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
    LOCK TABLE public.platform_requirement_gate IN SHARE MODE;
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

-- Grants. The floors and the gate are the migration role's: the runtime role cannot read, write or lock them, and calls only the hold.
REVOKE ALL PRIVILEGES ON platform_requirement_floors, platform_requirement_gate FROM PUBLIC;
REVOKE ALL ON FUNCTION platform_requirement_floor_admissible(regprocedure) FROM PUBLIC;
REVOKE ALL ON FUNCTION platform_requirement_floors_guard() FROM PUBLIC;
REVOKE ALL ON FUNCTION platform_requirement_floor() FROM PUBLIC;
REVOKE ALL ON FUNCTION platform_requirement_floors_check() FROM PUBLIC;
REVOKE ALL ON FUNCTION platform_requirement_hold(text) FROM PUBLIC;
DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        REVOKE ALL PRIVILEGES ON platform_requirement_floors, platform_requirement_gate FROM sagat_app;
        GRANT EXECUTE ON FUNCTION platform_requirement_hold(text) TO sagat_app;
    END IF;
END $grants$;
