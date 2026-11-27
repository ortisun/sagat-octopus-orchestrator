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
-- 4. The gate: a transaction advisory lock on one fixed key, before the epoch in the registry's lock order, taken exclusively by
--    the operator's three functions and shared by the hold and a registration. PostgreSQL 16 lets a new FOR SHARE of a row go
--    ahead of a waiting FOR UPDATE; its lock manager does not let a shared advisory request go ahead of a waiting exclusive one.
--    The three operator functions and the registration are rewritten in place with their bodies of `20270113000000` and the gate
--    as their first lock; `CREATE OR REPLACE` keeps their owner and grants, and restates their configuration.
--
-- Not covered (Gap 045-006): the beat (`instance_heartbeat_reporting_change`) reads the epoch FOR SHARE SKIP LOCKED and never waits,
-- and the automatic handover takes the epoch FOR UPDATE without the gate; neither breaks a guarantee of ADR-0112.

CREATE TABLE platform_requirement_floors (
    capability text         PRIMARY KEY,
    holds      regprocedure NOT NULL,
    CONSTRAINT platform_requirement_floors_capability_known CHECK (
        capability IN ('schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1',
                       'lease-withdrawal-v1', 'lease-withdrawal-v2'))
);

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
    EXECUTE format('SELECT %s', v_holds) INTO v_held;
    IF v_held IS DISTINCT FROM false THEN
        RAISE EXCEPTION 'FLOOR_HELD: % cannot be removed while its floor holds (%): a build without it cannot honour what depends on it', OLD.capability, v_holds
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN OLD;
END;
$$;

-- Admits a floor's function only with no argument, a boolean result and the owner of the floor trigger: the trigger calls it by
-- name, so nothing else may be named.
CREATE FUNCTION platform_requirement_floors_check() RETURNS trigger
LANGUAGE plpgsql SECURITY INVOKER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    v_args   smallint;
    v_result oid;
    v_owner  oid;
BEGIN
    SELECT p.pronargs, p.prorettype, p.proowner INTO v_args, v_result, v_owner FROM pg_proc p WHERE p.oid = NEW.holds;
    IF v_args <> 0 OR v_result <> 'pg_catalog.bool'::regtype
       OR v_owner <> (SELECT p.proowner FROM pg_proc p WHERE p.oid = 'public.platform_requirement_floor()'::regprocedure) THEN
        RAISE EXCEPTION 'a floor names a function with no argument, a boolean result and the owner of platform_requirement_floor()'
            USING ERRCODE = 'invalid_parameter_value';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER platform_requirement_floors_check BEFORE INSERT OR UPDATE ON platform_requirement_floors
    FOR EACH ROW EXECUTE FUNCTION platform_requirement_floors_check();
CREATE TRIGGER platform_requirement_floor BEFORE DELETE ON platform_requirements
    FOR EACH ROW EXECUTE FUNCTION platform_requirement_floor();
ALTER TABLE platform_requirement_floors ENABLE ALWAYS TRIGGER platform_requirement_floors_check;
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
    PERFORM pg_catalog.pg_advisory_xact_lock_shared(pg_catalog.hashtextextended('sagat:platform_requirement_epoch:gate', 0));
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
    PERFORM pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended('sagat:platform_requirement_epoch:gate', 0));
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
    PERFORM pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended('sagat:platform_requirement_epoch:gate', 0));
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
    PERFORM pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended('sagat:platform_requirement_epoch:gate', 0));
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
    PERFORM pg_catalog.pg_advisory_xact_lock_shared(pg_catalog.hashtextextended('sagat:platform_requirement_epoch:gate', 0));
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

-- Grants. The floors are the migration role's: the runtime role neither reads nor writes them, and calls only the hold.
REVOKE ALL PRIVILEGES ON platform_requirement_floors FROM PUBLIC;
REVOKE ALL ON FUNCTION platform_requirement_floor() FROM PUBLIC;
REVOKE ALL ON FUNCTION platform_requirement_floors_check() FROM PUBLIC;
REVOKE ALL ON FUNCTION platform_requirement_hold(text) FROM PUBLIC;
DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        REVOKE ALL PRIVILEGES ON platform_requirement_floors FROM sagat_app;
        GRANT EXECUTE ON FUNCTION platform_requirement_hold(text) TO sagat_app;
    END IF;
END $grants$;
