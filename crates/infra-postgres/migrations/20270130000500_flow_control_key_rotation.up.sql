-- Feature 022, release 0.3.0 slice (T022-095; FR-022-053, FR-022-056; D022-025, D022-042; G1190; ADR-0093, proposed): the
-- digest key is rotated per key, with live rows.
--
-- Before: a new key version was refused while any semaphore or window row used another one (T022-007, ADR-0056 decision
-- 7), so a rotation needed every queue drained. Now: activating a new version turns the current one RETIRING while rows
-- use it (RETIRED at once when none does). A deciding transaction reads the versions in force
-- (`flow_control_key_versions_in_force`), computes the key's digest under each, and uses the row that exists under the
-- retiring version, else the active one: a key is never split between two rows, because no row is ever created under a
-- retiring version (the trigger of 20270130000000) and an old row, once gone, never comes back.
--
-- Retirement is lazy (review of 2026-11-16): a retiring version stops being in force the moment no row uses it (the
-- function answers NULL for it, so no decider needs its key any more), and the next activation records it RETIRED. No
-- trigger and no lock is added to the paths that delete key rows. One rotation at a time: a third version is refused while
-- the retiring one is still used (a decider computes two digests, never three), and at most one version is RETIRING.
--
-- The command now records the database role that ran it (G1190), beside the operator name it was given.
--
-- Additive: one nullable column, one unique index, two functions; the command keeps its name and result shape. Versions
-- the earlier command left RETIRING have no row (it refused while any row used another version) and are retired here.

ALTER TABLE flow_control_key_versions ADD COLUMN activated_role text;
ALTER TABLE flow_control_key_versions ADD CONSTRAINT flow_control_key_versions_role_bounded
    CHECK (activated_role IS NULL OR char_length(activated_role) BETWEEN 1 AND 63);

UPDATE flow_control_key_versions k SET state = 'RETIRED', retired_at = clock_timestamp()
 WHERE k.state = 'RETIRING'
   AND NOT EXISTS (SELECT 1 FROM concurrency_semaphores s WHERE s.key_version = k.version)
   AND NOT EXISTS (SELECT 1 FROM rate_limit_windows w WHERE w.key_version = k.version);
CREATE UNIQUE INDEX flow_control_key_versions_one_retiring ON flow_control_key_versions ((true)) WHERE state = 'RETIRING';

-- How many key rows (semaphores and windows) use a version: what the operator's command reports.
CREATE FUNCTION flow_control_key_version_rows(p_version smallint) RETURNS bigint
    LANGUAGE sql SECURITY DEFINER SET search_path = pg_catalog, pg_temp
AS $$
    SELECT (SELECT count(*) FROM public.concurrency_semaphores WHERE key_version = p_version)
         + (SELECT count(*) FROM public.rate_limit_windows WHERE key_version = p_version)
$$;
REVOKE ALL ON FUNCTION flow_control_key_version_rows(smallint) FROM PUBLIC;

-- The versions a deciding transaction digests a key under: the active one, taken shared (a rotation waits for this
-- transaction, and a transaction that starts during one waits for it), and the retiring one while a row still uses it,
-- read without a lock: no row can be created under it, so once none uses it none ever will, and NULL is the answer from
-- then on. `active` is NULL when no version was ever activated: the decider refuses.
CREATE FUNCTION flow_control_key_versions_in_force(OUT active smallint, OUT retiring smallint)
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    active := public.flow_control_active_key_version();
    SELECT k.version INTO retiring FROM public.flow_control_key_versions k
     WHERE k.state = 'RETIRING'
       AND (EXISTS (SELECT 1 FROM public.concurrency_semaphores s WHERE s.key_version = k.version)
            OR EXISTS (SELECT 1 FROM public.rate_limit_windows w WHERE w.key_version = k.version));
END
$$;
REVOKE ALL ON FUNCTION flow_control_key_versions_in_force() FROM PUBLIC;

-- ---------------------------------------------------------------------------------------------------------------------
-- The operator's command, per-key. NOT executable by the runtime role.
--
-- outcome: ACTIVATED (out_rows: the rows the now-retiring version still has, 0 when it was retired at once)
--        | ALREADY_ACTIVE | RETIRED (the version was retired or replaced: never again)
--        | ROTATION_IN_PROGRESS (out_rows: the rows the retiring version still has)
-- ---------------------------------------------------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION flow_control_key_activate(p_version smallint, p_actor text)
RETURNS TABLE (out_outcome text, out_rows bigint)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    v_retiring smallint;
    v_active smallint;
    v_rows bigint;
BEGIN
    -- one rotation at a time; every decider that read the active version has committed, and none reads it until this
    -- commits. Key rows are not locked: none can be created under a version that is not active, so a count of the
    -- retiring version's rows can only be too high (a refusal the operator repeats), never too low.
    LOCK TABLE public.flow_control_key_versions IN EXCLUSIVE MODE;
    IF EXISTS (SELECT 1 FROM public.flow_control_key_versions WHERE version = p_version AND state = 'ACTIVE') THEN
        RETURN QUERY SELECT 'ALREADY_ACTIVE'::text, 0::bigint;
        RETURN;
    END IF;
    -- a retired version (for instance one whose key leaked) is never active again
    IF EXISTS (SELECT 1 FROM public.flow_control_key_versions WHERE version = p_version AND state = 'RETIRED') THEN
        RETURN QUERY SELECT 'RETIRED'::text, 0::bigint;
        RETURN;
    END IF;
    SELECT version INTO v_retiring FROM public.flow_control_key_versions WHERE state = 'RETIRING';
    IF v_retiring IS NOT NULL THEN
        v_rows := public.flow_control_key_version_rows(v_retiring);
        IF v_rows > 0 THEN
            RETURN QUERY SELECT 'ROTATION_IN_PROGRESS'::text, v_rows;
            RETURN;
        END IF;
        UPDATE public.flow_control_key_versions SET state = 'RETIRED', retired_at = clock_timestamp()
         WHERE version = v_retiring;
        -- a version that was replaced is never active again, like a retired one
        IF v_retiring = p_version THEN
            RETURN QUERY SELECT 'RETIRED'::text, 0::bigint;
            RETURN;
        END IF;
    END IF;
    SELECT version INTO v_active FROM public.flow_control_key_versions WHERE state = 'ACTIVE';
    v_rows := 0;
    IF v_active IS NOT NULL THEN
        v_rows := public.flow_control_key_version_rows(v_active);
        IF v_rows > 0 THEN
            UPDATE public.flow_control_key_versions SET state = 'RETIRING' WHERE version = v_active;
        ELSE
            UPDATE public.flow_control_key_versions SET state = 'RETIRED', retired_at = clock_timestamp()
             WHERE version = v_active;
        END IF;
    END IF;
    INSERT INTO public.flow_control_key_versions (version, state, activated_at, activated_by, activated_role)
    VALUES (p_version, 'ACTIVE', clock_timestamp(), p_actor, session_user);
    RETURN QUERY SELECT 'ACTIVATED'::text, v_rows;
END;
$$;
REVOKE ALL ON FUNCTION flow_control_key_activate(smallint, text) FROM PUBLIC;

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT EXECUTE ON FUNCTION flow_control_key_versions_in_force() TO sagat_app;
    END IF;
END
$grants$;
