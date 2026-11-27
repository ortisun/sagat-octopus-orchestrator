-- Feature 022, release 0.3.0 slice (T022-095; FR-022-053, FR-022-056; ADR-0093 as accepted by OD-2026-11-19-2, O1170 (a)
-- with (b) for incidents): the operator's force, which ends a digest-key rotation at once.
--
-- An ordinary rotation (20270130000500) ends when the retiring version's last key row goes, and live rows of any tenant
-- decide when that is; a third version is refused meanwhile. For an emergency (a leaked key) the operator now forces the
-- retiring version RETIRED at once:
--   * no decider digests a key under it any more (`flow_control_key_versions_in_force` answers only a RETIRING version),
--     so a key decided after the force is filed under the active version: a key may have two rows until the old one
--     drains;
--   * the old row keeps its holders, grants and waiters; it gains no new waiter (a decider files a waiter only under the
--     row it resolved from the versions in force, and only the gate's two files write waiters, a rule of
--     scripts/check-architecture.py) and grants only its own waiters, within its own limit; it goes with its last holder,
--     grant and waiter, as every key row does. A key's limit is therefore over-admitted, at any moment, by at most what
--     its old row holds at that moment (its holders, or its counted grants), never more than its limit, until the old
--     row's holders and waiters drain. A rate window's grants drain only once their lease instants are recorded and
--     purged (G022-003, T022-074);
--   * one forced version drains at a time (FORCED_VERSION_DRAINING): a key has at most two rows;
--   * no process needs the retired version's key material any more, and the next rotation may start at once.
-- The force takes the version table EXCLUSIVE like the activation, so it waits for every deciding transaction in flight
-- (each holds the active version FOR SHARE) and every later one waits for it: a decider never sees the version both in
-- and out of force.
--
-- Only a RETIRING version is forced: the active one is replaced by activating a successor first (the two commands may
-- run in one transaction). Every force is recorded with the operator's name, the database role, a reason and the rows
-- it left, in an append-only history. The operator also gets the runs that hold a version's keys, to wait for or cancel
-- them before an ordinary rotation can end (O1170 (a)).
--
-- Additive: one table and three functions, none executable by the runtime role.

CREATE TABLE flow_control_key_forced_retirements (
    id            bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    version       smallint    NOT NULL REFERENCES flow_control_key_versions (version),
    -- the key rows (semaphores and windows) the version still had when it was forced: they drain on their own
    rows_left     bigint      NOT NULL,
    actor         text        NOT NULL,
    -- the database role of the session that forced it (`actor` is what the command says; this is what the database saw)
    database_user text        NOT NULL,
    reason        text        NOT NULL,
    at            timestamptz NOT NULL,
    CONSTRAINT flow_control_key_forced_rows_valid CHECK (rows_left >= 0),
    CONSTRAINT flow_control_key_forced_actor_bounded CHECK (
        char_length(actor) BETWEEN 1 AND 255 AND btrim(actor) <> '' AND actor !~ '[[:cntrl:]]'),
    CONSTRAINT flow_control_key_forced_user_bounded CHECK (char_length(database_user) BETWEEN 1 AND 128),
    CONSTRAINT flow_control_key_forced_reason_bounded CHECK (
        char_length(reason) BETWEEN 1 AND 1000 AND btrim(reason) <> '' AND reason !~ '[[:cntrl:]]')
);
CREATE TRIGGER flow_control_key_forced_retirements_append_only BEFORE UPDATE OR DELETE
    ON flow_control_key_forced_retirements FOR EACH ROW EXECUTE FUNCTION execution_reject_mutation();
CREATE TRIGGER flow_control_key_forced_retirements_no_truncate BEFORE TRUNCATE
    ON flow_control_key_forced_retirements FOR EACH STATEMENT EXECUTE FUNCTION execution_reject_mutation();
ALTER TABLE flow_control_key_forced_retirements
    ENABLE ALWAYS TRIGGER flow_control_key_forced_retirements_append_only;
ALTER TABLE flow_control_key_forced_retirements
    ENABLE ALWAYS TRIGGER flow_control_key_forced_retirements_no_truncate;
REVOKE ALL PRIVILEGES ON flow_control_key_forced_retirements FROM PUBLIC;

-- ---------------------------------------------------------------------------------------------------------------------
-- The operator's force. NOT executable by the runtime role.
--
-- outcome: FORCED (out_rows: the key rows the version still had; they drain on their own)
--        | ACTIVE (the version is the active one: activate a successor first; out_rows: its rows)
--        | FORCED_VERSION_DRAINING (an earlier forced version still has rows; out_rows: how many. A second force before
--          they drain would split a key into three rows: wait for them, or cancel the runs that hold them)
--        | ALREADY_RETIRED (out_rows: the rows it still has) | UNKNOWN
-- A reason or an operator name that is blank, longer than 1,000 (255) characters as typed, or holds a control character
-- is refused (22023).
-- ---------------------------------------------------------------------------------------------------------------------
CREATE FUNCTION flow_control_key_force_retire(p_version smallint, p_actor text, p_reason text)
RETURNS TABLE (out_outcome text, out_rows bigint)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    v_state text;
    v_rows bigint;
BEGIN
    IF p_reason IS NULL OR char_length(p_reason) NOT BETWEEN 1 AND 1000 OR btrim(p_reason) = ''
       OR p_reason ~ '[[:cntrl:]]' THEN
        RAISE EXCEPTION 'a forced retirement needs a reason of 1 to 1000 characters, on one line'
            USING ERRCODE = 'invalid_parameter_value';
    END IF;
    IF p_actor IS NULL OR char_length(p_actor) NOT BETWEEN 1 AND 255 OR btrim(p_actor) = ''
       OR p_actor ~ '[[:cntrl:]]' THEN
        RAISE EXCEPTION 'a forced retirement needs the operator''s name, 1 to 255 characters on one line'
            USING ERRCODE = 'invalid_parameter_value';
    END IF;
    -- as the activation: every decider that read the versions has committed, and none reads them until this commits
    LOCK TABLE public.flow_control_key_versions IN EXCLUSIVE MODE;
    SELECT k.state INTO v_state FROM public.flow_control_key_versions k WHERE k.version = p_version;
    IF NOT FOUND THEN
        RETURN QUERY SELECT 'UNKNOWN'::text, 0::bigint;
        RETURN;
    END IF;
    IF v_state = 'ACTIVE' THEN
        RETURN QUERY SELECT 'ACTIVE'::text, public.flow_control_key_version_rows(p_version);
        RETURN;
    END IF;
    IF v_state = 'RETIRED' THEN
        RETURN QUERY SELECT 'ALREADY_RETIRED'::text, public.flow_control_key_version_rows(p_version);
        RETURN;
    END IF;
    -- one forced version draining at a time: a key then has at most two rows, its old one and its active one
    SELECT coalesce(sum(public.flow_control_key_version_rows(f.version)), 0)::bigint INTO v_rows
      FROM (SELECT DISTINCT version FROM public.flow_control_key_forced_retirements) f;
    IF v_rows > 0 THEN
        RETURN QUERY SELECT 'FORCED_VERSION_DRAINING'::text, v_rows;
        RETURN;
    END IF;
    -- RETIRING: out of force at once; its rows (if any) live on under their old digest until they drain
    v_rows := public.flow_control_key_version_rows(p_version);
    UPDATE public.flow_control_key_versions SET state = 'RETIRED', retired_at = clock_timestamp()
     WHERE version = p_version;
    INSERT INTO public.flow_control_key_forced_retirements (version, rows_left, actor, database_user, reason, at)
    VALUES (p_version, v_rows, p_actor, session_user, p_reason, clock_timestamp());
    RETURN QUERY SELECT 'FORCED'::text, v_rows;
END;
$$;
REVOKE ALL ON FUNCTION flow_control_key_force_retire(smallint, text, text) FROM PUBLIC;

-- The runs whose holders, waiters or grants use a key row of `p_version`: what an ordinary rotation waits for. A run of
-- another tenant is listed by its identifiers only (no key, digest or label). NOT executable by the runtime role.
CREATE FUNCTION flow_control_key_version_runs(p_version smallint)
RETURNS TABLE (tenant_id uuid, namespace_id uuid, run_id uuid, holders bigint, waiters bigint, grants bigint)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
    WITH uses AS (
        SELECT h.tenant_id, h.namespace_id, h.run_id, 1 AS holder, 0 AS waiter, 0 AS grant_
          FROM public.concurrency_holders h
          JOIN public.concurrency_semaphores s
            ON (s.tenant_id, s.namespace_id, s.pool, s.permit_scope, s.group_name, s.key_digest)
             = (h.tenant_id, h.namespace_id, h.pool, h.holder_scope, h.group_name, h.key_digest)
         WHERE s.key_version = p_version
        UNION ALL
        SELECT w.tenant_id, w.namespace_id, w.run_id, 0, 1, 0
          FROM public.concurrency_waiters w
          JOIN public.concurrency_semaphores s
            ON (s.tenant_id, s.namespace_id, s.pool, s.permit_scope, s.group_name, s.key_digest)
             = (w.tenant_id, w.namespace_id, w.pool, w.permit_scope, w.group_name, w.key_digest)
         WHERE s.key_version = p_version
        UNION ALL
        SELECT g.tenant_id, g.namespace_id, g.run_id, 0, 0, 1
          FROM public.rate_limit_grants g
          JOIN public.rate_limit_windows r
            ON (r.tenant_id, r.namespace_id, r.pool, r.limit_scope, r.group_name, r.key_digest)
             = (g.tenant_id, g.namespace_id, g.pool, g.limit_scope, g.group_name, g.key_digest)
         WHERE r.key_version = p_version
        UNION ALL
        SELECT w.tenant_id, w.namespace_id, w.run_id, 0, 1, 0
          FROM public.rate_limit_waiters w
          JOIN public.rate_limit_windows r
            ON (r.tenant_id, r.namespace_id, r.pool, r.limit_scope, r.group_name, r.key_digest)
             = (w.tenant_id, w.namespace_id, w.pool, w.limit_scope, w.group_name, w.key_digest)
         WHERE r.key_version = p_version
    )
    SELECT tenant_id, namespace_id, run_id, sum(holder)::bigint, sum(waiter)::bigint, sum(grant_)::bigint
      FROM uses GROUP BY tenant_id, namespace_id, run_id ORDER BY tenant_id, namespace_id, run_id
$$;
REVOKE ALL ON FUNCTION flow_control_key_version_runs(smallint) FROM PUBLIC;
