-- Feature 018, T018-080 (FR-018-052, SC-018-017): the instance-capability registry (ADR-0050 decision 9;
-- `specs/018-schedules-bulk-operations/data-model.md`, "Instance-capability registry"; shared with features 029 and 031).
--
-- Every control-plane and execution-engine process announces what its binary can do before it serves
-- (`serving_instances`), keeps announcing that it is alive, and refuses to serve while the platform records a
-- requirement for its kind that its binary cannot honour (`platform_requirements`). An operator records a
-- requirement only through `platform_requirement_activate`, in one transaction that locks the requirement epoch,
-- verifies every live instance of the required kinds and bumps the epoch; a process learns of it when its heartbeat
-- reports that the epoch moved.
--
-- Lock order, in every function below: the requirement epoch row first (FOR SHARE to register, FOR UPDATE to change a
-- requirement), then the advisory lock that serialises the count of the table, then rows. A heartbeat takes only
-- rows of its own and rows it can skip. No function waits for a lock while it holds a later one.
--
-- Rule for a feature that activates through the registry: its activation step runs while the epoch is held FOR UPDATE, and
-- a transaction that reads the epoch FOR SHARE (a process registering, or a request path that fences on it) waits for the
-- whole step. So the step takes no lock that a transaction reading the epoch may hold: it reads, it does not lock. The
-- operator's transaction is bounded (lock and statement timeouts, set by the adapter) so that a slow step cannot hold the
-- epoch longer than a process waits for it.
--
-- Additive: nothing existing reads or writes any of it. A binary older than this release does not register, so no
-- activation can see it (G1011, G1607): the registry ships in a release before any feature activates through it.

-- The processes that serve. One row per process start; a restart is a new row and the old one expires.
CREATE TABLE serving_instances (
    instance_id     uuid        PRIMARY KEY,
    kind            text        NOT NULL,
    -- an engine's scheduler_members.owner, for correlation only; scheduler_members itself is untouched, because the
    -- keeper divides the shards by the replicas it lists (ADR-0020 decision 2)
    scheduler_owner text,
    binary_version  text        NOT NULL,
    capabilities    text[]      NOT NULL DEFAULT '{}',
    started_at      timestamptz NOT NULL,
    last_seen       timestamptz NOT NULL,
    -- sha256 of a random token that only the process that registered the row holds. The functions touch a row only for
    -- the token that matches; the runtime role cannot read this column, so it cannot learn what to send.
    token_hash      bytea       NOT NULL,
    CONSTRAINT serving_instances_token_hash_sha256 CHECK (octet_length(token_hash) = 32),
    CONSTRAINT serving_instances_kind_valid CHECK (kind IN ('CONTROL_PLANE', 'EXECUTION_ENGINE')),
    CONSTRAINT serving_instances_owner_bounded CHECK (scheduler_owner IS NULL OR char_length(scheduler_owner) BETWEEN 1 AND 255),
    CONSTRAINT serving_instances_version_bounded CHECK (char_length(binary_version) BETWEEN 1 AND 64),
    CONSTRAINT serving_instances_capabilities_closed CHECK (
        capabilities <@ ARRAY['schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1']::text[]
        AND cardinality(capabilities) <= 32)
);
CREATE INDEX serving_instances_last_seen_idx ON serving_instances (last_seen);

-- What the platform requires of a kind. Changed only by the activation functions.
CREATE TABLE platform_requirements (
    capability     text        PRIMARY KEY,
    kinds          text[]      NOT NULL,
    required_since timestamptz NOT NULL,
    recorded_by    text        NOT NULL,
    CONSTRAINT platform_requirements_capability_valid CHECK (
        capability IN ('schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1')),
    CONSTRAINT platform_requirements_kinds_valid CHECK (
        kinds <@ ARRAY['CONTROL_PLANE', 'EXECUTION_ENGINE']::text[] AND cardinality(kinds) >= 1),
    CONSTRAINT platform_requirements_recorded_by_bounded CHECK (char_length(recorded_by) BETWEEN 1 AND 255)
);

-- The singleton that every change of a requirement increments. `changed_xact` is the transaction that last moved it:
-- the deferred check below needs it to know that a requirement did not change behind the epoch's back.
CREATE TABLE platform_requirement_epoch (
    only_row     boolean PRIMARY KEY DEFAULT true CHECK (only_row),
    epoch        bigint  NOT NULL DEFAULT 0 CHECK (epoch >= 0),
    changed_xact xid8    NOT NULL DEFAULT '0'::xid8
);
INSERT INTO platform_requirement_epoch (only_row, epoch) VALUES (true, 0);

-- Who recorded or removed what, and when. The activation is deployment-level, so it has no tenant and cannot be an
-- `audit_records` row (that table is tenant-scoped); this is its append-only evidence.
CREATE TABLE platform_requirement_history (
    id         bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    action     text        NOT NULL,
    capability text        NOT NULL,
    kinds      text[]      NOT NULL,
    actor      text        NOT NULL,
    -- the database role of the session that did it (`actor` is what the command says; this is what the database saw)
    database_user text     NOT NULL,
    epoch      bigint      NOT NULL,
    at         timestamptz NOT NULL,
    CONSTRAINT platform_requirement_history_action_valid CHECK (action IN ('ACTIVATED', 'REMOVED')),
    CONSTRAINT platform_requirement_history_capability_valid CHECK (
        capability IN ('schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1')),
    CONSTRAINT platform_requirement_history_actor_bounded CHECK (char_length(actor) BETWEEN 1 AND 255),
    CONSTRAINT platform_requirement_history_user_bounded CHECK (char_length(database_user) BETWEEN 1 AND 128)
);

-- ---------------------------------------------------------------------------------------------------------------------
-- Guards (ENABLE ALWAYS: they hold under session_replication_role = replica as well).
--
-- A requirement and the epoch change only inside the activation functions, which set a transaction-local flag; the
-- epoch moves by exactly one; and a requirement that was inserted or deleted without the epoch having moved in the
-- same transaction is refused at commit. The flag is a seat belt against a hand edit, not a secret: the owner can
-- set it, and then the deferred check still holds.
-- ---------------------------------------------------------------------------------------------------------------------

CREATE FUNCTION platform_requirements_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF coalesce(current_setting('sagat.registry_change', true), '') <> 'activation' THEN
        RAISE EXCEPTION 'platform requirements change only through platform_requirement_activate and platform_requirement_remove'
            USING ERRCODE = 'restrict_violation';
    END IF;
    IF TG_OP = 'UPDATE' THEN
        RAISE EXCEPTION 'a requirement is removed and recorded again, never changed'
            USING ERRCODE = 'restrict_violation';
    END IF;
    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$;

CREATE FUNCTION platform_requirement_epoch_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP <> 'UPDATE' THEN
        RAISE EXCEPTION 'the requirement epoch is one row, made by the migration that created it'
            USING ERRCODE = 'restrict_violation';
    END IF;
    IF coalesce(current_setting('sagat.registry_change', true), '') <> 'activation' THEN
        RAISE EXCEPTION 'the requirement epoch moves only through platform_requirement_activate and platform_requirement_remove'
            USING ERRCODE = 'restrict_violation';
    END IF;
    IF NEW.epoch <> OLD.epoch + 1 THEN
        RAISE EXCEPTION 'the requirement epoch moves by one'
            USING ERRCODE = 'restrict_violation';
    END IF;
    NEW.changed_xact := pg_current_xact_id();
    RETURN NEW;
END;
$$;

CREATE FUNCTION platform_requirements_epoch_moved() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM public.platform_requirement_epoch WHERE changed_xact = pg_current_xact_id()) THEN
        RAISE EXCEPTION 'a requirement changed without the requirement epoch moving in the same transaction'
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN NULL;
END;
$$;

CREATE TRIGGER platform_requirements_guard BEFORE INSERT OR UPDATE OR DELETE ON platform_requirements
    FOR EACH ROW EXECUTE FUNCTION platform_requirements_guard();
CREATE TRIGGER platform_requirements_no_truncate BEFORE TRUNCATE ON platform_requirements
    FOR EACH STATEMENT EXECUTE FUNCTION execution_reject_mutation();
CREATE CONSTRAINT TRIGGER platform_requirements_epoch_moved AFTER INSERT OR DELETE ON platform_requirements
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION platform_requirements_epoch_moved();
CREATE TRIGGER platform_requirement_epoch_guard BEFORE INSERT OR UPDATE OR DELETE ON platform_requirement_epoch
    FOR EACH ROW EXECUTE FUNCTION platform_requirement_epoch_guard();
CREATE TRIGGER platform_requirement_epoch_no_truncate BEFORE TRUNCATE ON platform_requirement_epoch
    FOR EACH STATEMENT EXECUTE FUNCTION execution_reject_mutation();
CREATE TRIGGER platform_requirement_history_append_only BEFORE UPDATE OR DELETE ON platform_requirement_history
    FOR EACH ROW EXECUTE FUNCTION execution_reject_mutation();
CREATE TRIGGER platform_requirement_history_no_truncate BEFORE TRUNCATE ON platform_requirement_history
    FOR EACH STATEMENT EXECUTE FUNCTION execution_reject_mutation();

ALTER TABLE platform_requirements ENABLE ALWAYS TRIGGER platform_requirements_guard;
ALTER TABLE platform_requirements ENABLE ALWAYS TRIGGER platform_requirements_no_truncate;
ALTER TABLE platform_requirements ENABLE ALWAYS TRIGGER platform_requirements_epoch_moved;
ALTER TABLE platform_requirement_epoch ENABLE ALWAYS TRIGGER platform_requirement_epoch_guard;
ALTER TABLE platform_requirement_epoch ENABLE ALWAYS TRIGGER platform_requirement_epoch_no_truncate;
ALTER TABLE platform_requirement_history ENABLE ALWAYS TRIGGER platform_requirement_history_append_only;
ALTER TABLE platform_requirement_history ENABLE ALWAYS TRIGGER platform_requirement_history_no_truncate;

-- ---------------------------------------------------------------------------------------------------------------------
-- The functions of a process (executable by the runtime role). SECURITY DEFINER, because the runtime role takes no
-- row lock on a table it cannot UPDATE, and because a row is touched only by instance_id inside the function.
-- ---------------------------------------------------------------------------------------------------------------------

-- Registers, or refreshes, the instance's own row. One transaction: the epoch FOR SHARE (an activation that holds it
-- FOR UPDATE finishes first, and one that comes later waits for this one), the requirements for the kind, the room in
-- the table, then the row. A refusal writes nothing.
--
-- outcome: REGISTERED | CAPABILITY_MISSING (missing names them) | FULL
CREATE FUNCTION instance_register(
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

-- Says the instance is alive, on the database's clock, and touches no other instance's row. Rows silent for more than
-- a day are deleted (rows another heartbeat holds are left to it). Returns whether the row was there, the epoch, and the
-- live count per kind (seen within 30 seconds).
CREATE FUNCTION instance_heartbeat(p_instance uuid, p_token bytea)
RETURNS TABLE (out_found boolean, out_epoch bigint, out_live_kinds text[], out_live_counts integer[])
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    v_found  boolean;
    v_epoch  bigint;
    v_kinds  text[];
    v_counts integer[];
BEGIN
    -- another process's row, or a token that does not match, is a row that is not found
    UPDATE public.serving_instances SET last_seen = clock_timestamp()
     WHERE instance_id = p_instance AND token_hash = sha256(coalesce(p_token, ''::bytea));
    v_found := FOUND;
    DELETE FROM public.serving_instances d WHERE d.instance_id IN (
        SELECT s.instance_id FROM public.serving_instances s
         WHERE s.last_seen < clock_timestamp() - interval '1 day'
         ORDER BY s.instance_id
         LIMIT 100
         FOR UPDATE SKIP LOCKED);
    SELECT e.epoch INTO STRICT v_epoch FROM public.platform_requirement_epoch e;
    SELECT coalesce(array_agg(t.kind ORDER BY t.kind), '{}'), coalesce(array_agg(t.n::integer ORDER BY t.kind), '{}')
      INTO v_kinds, v_counts
      FROM (SELECT s.kind, count(*) AS n FROM public.serving_instances s
             WHERE s.last_seen > clock_timestamp() - interval '30 seconds'
             GROUP BY s.kind) t;
    RETURN QUERY SELECT v_found, v_epoch, v_kinds, v_counts;
END;
$$;

-- A clean stop: deletes the instance's own row. Whether it was there.
CREATE FUNCTION instance_deregister(p_instance uuid, p_token bytea) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    DELETE FROM public.serving_instances
     WHERE instance_id = p_instance AND token_hash = sha256(coalesce(p_token, ''::bytea));
    RETURN FOUND;
END;
$$;

-- ---------------------------------------------------------------------------------------------------------------------
-- The functions of the operator (NOT executable by the runtime role: a compromised API process must not be able to
-- brick the fleet by recording a requirement). `control-plane` commands run them with the schema-owning role, as
-- `provision-tenant` does.
-- ---------------------------------------------------------------------------------------------------------------------

-- Takes the epoch FOR UPDATE and returns it. Held to the end of the transaction: every registration waits.
CREATE FUNCTION platform_requirement_lock() RETURNS bigint
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    v_epoch bigint;
BEGIN
    SELECT e.epoch INTO STRICT v_epoch FROM public.platform_requirement_epoch e FOR UPDATE;
    RETURN v_epoch;
END;
$$;

-- Records that every instance of `p_kinds` must list `p_capability`: lock the epoch FOR UPDATE, verify that every
-- live instance (seen within 30 seconds) of the kinds lists it, insert the requirement, move the epoch by one, write
-- the history. The caller runs the feature's own activation in the same transaction, after this returns.
--
-- outcome: RECORDED | ALREADY_RECORDED (same kinds: nothing done) | KINDS_DIFFER (recorded for other kinds: refused) |
--          INSTANCES_LACK_CAPABILITY (out_missing live instances lack it; out_examples names up to sixteen)
CREATE FUNCTION platform_requirement_activate(p_capability text, p_kinds text[], p_actor text)
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

-- The same command in reverse: removes the requirement and moves the epoch. The caller verifies, in the same
-- transaction and after taking the epoch lock, that nothing of the feature depends on it.
--
-- outcome: REMOVED | NOT_RECORDED
CREATE FUNCTION platform_requirement_remove(p_capability text, p_actor text)
RETURNS TABLE (out_outcome text, out_epoch bigint)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
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

-- ---------------------------------------------------------------------------------------------------------------------
-- Grants. The runtime role reads the three tables and calls the three functions of a process; it writes nothing and
-- locks nothing. The history is the operator's to read.
-- ---------------------------------------------------------------------------------------------------------------------

REVOKE ALL PRIVILEGES ON serving_instances, platform_requirements, platform_requirement_epoch,
    platform_requirement_history FROM PUBLIC;
REVOKE ALL ON FUNCTION instance_register(uuid, bytea, text, text, text, text[]) FROM PUBLIC;
REVOKE ALL ON FUNCTION instance_heartbeat(uuid, bytea) FROM PUBLIC;
REVOKE ALL ON FUNCTION instance_deregister(uuid, bytea) FROM PUBLIC;
REVOKE ALL ON FUNCTION platform_requirement_lock() FROM PUBLIC;
REVOKE ALL ON FUNCTION platform_requirement_activate(text, text[], text) FROM PUBLIC;
REVOKE ALL ON FUNCTION platform_requirement_remove(text, text) FROM PUBLIC;
REVOKE ALL ON FUNCTION platform_requirements_guard() FROM PUBLIC;
REVOKE ALL ON FUNCTION platform_requirement_epoch_guard() FROM PUBLIC;
REVOKE ALL ON FUNCTION platform_requirements_epoch_moved() FROM PUBLIC;
DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        REVOKE ALL PRIVILEGES ON serving_instances, platform_requirements, platform_requirement_epoch,
            platform_requirement_history FROM sagat_app;
        -- every column of the instances but the hash of the token
        GRANT SELECT (instance_id, kind, scheduler_owner, binary_version, capabilities, started_at, last_seen)
            ON serving_instances TO sagat_app;
        GRANT SELECT ON platform_requirements, platform_requirement_epoch TO sagat_app;
        GRANT EXECUTE ON FUNCTION instance_register(uuid, bytea, text, text, text, text[]) TO sagat_app;
        GRANT EXECUTE ON FUNCTION instance_heartbeat(uuid, bytea) TO sagat_app;
        GRANT EXECUTE ON FUNCTION instance_deregister(uuid, bytea) TO sagat_app;
    END IF;
END $grants$;
