-- The shared prerequisite of ADR-0112 (design 054's Task 054-18, built first by design 045 under its Task 045-21; Owner decisions
-- 2026-11-26-540 and 2026-11-26-564): the one publication hold of the fleet requirements, the one registry of their replay floors,
-- and, in the step after this one, the fairness gate of the amendment of 2026-11-27 that keeps publications from starving the operator.
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
-- 4. Not yet the gate: this step holds the epoch FOR SHARE alone, as design 054's text does, so that the held-lock
--    tests of Task 045-21 are seen failing before the gate is added.
--

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
    PERFORM 1 FROM public.platform_requirement_epoch e WHERE e.only_row FOR SHARE;
    SELECT r.kinds INTO v_kinds FROM public.platform_requirements r
     WHERE r.capability = platform_requirement_hold.capability;
    RETURN FOUND AND v_kinds @> ARRAY['CONTROL_PLANE', 'EXECUTION_ENGINE']::text[];
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
