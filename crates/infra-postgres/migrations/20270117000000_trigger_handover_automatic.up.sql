-- Feature 018, T018-082 (FR-018-053, ADR-0091, OD-2026-11-14-4 to -7 and -12): the automatic handover of the cron triggers by a
-- one-shot job of the deployment, with a role that can do only that.
--
-- What replaced the premise of `20270113000000_instance_registry.up.sql` (lines 20-21, "the registry ships in a release before any
-- feature activates through it"): the registry ships in the release that hands over, so a 0.2 binary is invisible to it. The job
-- therefore decides on more than the registry: a scheduler membership no registered engine names (a 0.2 engine), work of the
-- reconciliation within the horizon (`schedules.reconciled_at`: an old control plane, or the backlog of the upgrade), a live
-- control plane, an empty drift view, a fleet that held still for `--stable` rounds, and one horizon after its first round
-- (ADR-0091 decision 2). The database decides: every condition is computed here, by the round function and again by the wrapper
-- under the epoch lock, and the job only asks (D2259).
--
-- What is added:
--   * on `schedule_trigger_handover`: `changed_mode` (OPERATOR | AUTOMATIC, backfilled OPERATOR on a row already handed over) and
--     `changed_epoch` (the requirement epoch of the handover, which keys the census copy and the tenant set);
--   * owner-only, keyed by the epoch and written in the activation: the census copy and the tenant set (one random audit id per
--     tenant with a cron trigger, D2253, D2257);
--   * the attempt rows of the job (read by the engines for their gauges, by `sagat_app` and `sagat_handover`);
--   * the retirement singleton (owner-only, read by `migrate`, D2269);
--   * `schedules.reconciled_at`, which the reconciliation sets on make, revive, respec, resume and removal (D2243, D2258);
--   * the functions: the conditions (internal), the round and the wrapper `sagat_handover` calls, the record of the move (internal,
--     shared with the operator's `schedule_trigger_handover_complete`, replaced here to record the mode and the epoch), the audit
--     completion, the retirement, and the two functions `migrate` calls every run to ensure and grant the role (D2264, D2275).
--
-- Every function added here pins `search_path = pg_catalog, pg_temp`, is revoked from PUBLIC and granted by name; it is
-- `SECURITY DEFINER` only where its caller must not hold what it touches (the round, the wrapper, the completion, the retirement).
-- Nothing new is granted to `sagat_app` but SELECT on the attempt rows (D2267). The reason codes are one closed set
-- (`schedule_trigger_handover_reasons()`), held to the Rust enum and the runbook by a test.
--
-- Additive: an older binary never reads the new columns, tables or functions; the operator's `schedule_trigger_handover_complete`
-- keeps its signature (an older control plane that calls it records the mode OPERATOR). The new index on `schedules` is partial on
-- a column that is null everywhere when it is built, so its build reads the table once and writes nothing.

-- ---------------------------------------------------------------------------------------------------------------------
-- The handover row: the mode and the epoch.
-- ---------------------------------------------------------------------------------------------------------------------

ALTER TABLE schedule_trigger_handover ADD COLUMN changed_mode text;
ALTER TABLE schedule_trigger_handover ADD COLUMN changed_epoch bigint;
-- the guard refuses every change of a handed-over row; the backfill is the migration's own, as the owner
ALTER TABLE schedule_trigger_handover DISABLE TRIGGER schedule_trigger_handover_guard;
UPDATE schedule_trigger_handover h
   SET changed_mode = 'OPERATOR',
       changed_epoch = coalesce(
           (SELECT max(x.epoch) FROM platform_requirement_history x
             WHERE x.capability = 'schedules-v1' AND x.action = 'ACTIVATED'),
           (SELECT e.epoch FROM platform_requirement_epoch e))
 WHERE h.state = 'HANDED_OVER';
ALTER TABLE schedule_trigger_handover ENABLE ALWAYS TRIGGER schedule_trigger_handover_guard;
ALTER TABLE schedule_trigger_handover ADD CONSTRAINT schedule_trigger_handover_mode_valid
    CHECK (changed_mode IS NULL OR changed_mode IN ('OPERATOR', 'AUTOMATIC'));
ALTER TABLE schedule_trigger_handover ADD CONSTRAINT schedule_trigger_handover_mode_consistent
    CHECK ((state = 'LEGACY') = (changed_mode IS NULL));
ALTER TABLE schedule_trigger_handover ADD CONSTRAINT schedule_trigger_handover_epoch_consistent
    CHECK ((state = 'LEGACY') = (changed_epoch IS NULL));

-- ---------------------------------------------------------------------------------------------------------------------
-- Owner-only records of a handover, keyed by its epoch, written in the activation's transaction.
-- ---------------------------------------------------------------------------------------------------------------------

-- Every serving instance seen within the horizon when the handover was made: copied whole (the registry holds at most 1,000
-- rows), never truncated (D2257).
CREATE TABLE schedule_trigger_handover_census (
    epoch          bigint      NOT NULL,
    instance_id    uuid        NOT NULL,
    kind           text        NOT NULL,
    binary_version text        NOT NULL,
    started_at     timestamptz NOT NULL,
    last_seen      timestamptz NOT NULL,
    PRIMARY KEY (epoch, instance_id),
    CONSTRAINT schedule_trigger_handover_census_kind_valid CHECK (kind IN ('CONTROL_PLANE', 'EXECUTION_ENGINE')),
    CONSTRAINT schedule_trigger_handover_census_version_bounded CHECK (char_length(binary_version) BETWEEN 1 AND 64)
);
CREATE TRIGGER schedule_trigger_handover_census_append_only BEFORE UPDATE OR DELETE ON schedule_trigger_handover_census
    FOR EACH ROW EXECUTE FUNCTION execution_reject_mutation();
CREATE TRIGGER schedule_trigger_handover_census_no_truncate BEFORE TRUNCATE ON schedule_trigger_handover_census
    FOR EACH STATEMENT EXECUTE FUNCTION execution_reject_mutation();
ALTER TABLE schedule_trigger_handover_census ENABLE ALWAYS TRIGGER schedule_trigger_handover_census_append_only;
ALTER TABLE schedule_trigger_handover_census ENABLE ALWAYS TRIGGER schedule_trigger_handover_census_no_truncate;

-- One row per tenant that had a cron trigger when the handover was made, with a random audit id drawn there (the runtime role
-- cannot read it, so it cannot plant a record under it, D2253) and the time its audit record was written.
CREATE TABLE schedule_trigger_handover_tenants (
    epoch      bigint      NOT NULL,
    tenant_id  uuid        NOT NULL,
    audit_id   uuid        NOT NULL UNIQUE,
    audited_at timestamptz,
    PRIMARY KEY (epoch, tenant_id)
);
CREATE FUNCTION schedule_trigger_handover_tenants_guard() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    IF NEW.epoch IS DISTINCT FROM OLD.epoch OR NEW.tenant_id IS DISTINCT FROM OLD.tenant_id
       OR NEW.audit_id IS DISTINCT FROM OLD.audit_id OR OLD.audited_at IS NOT NULL OR NEW.audited_at IS NULL THEN
        RAISE EXCEPTION 'a row of the tenant set of a handover changes only once, when its audit record is written'
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER schedule_trigger_handover_tenants_guard BEFORE UPDATE ON schedule_trigger_handover_tenants
    FOR EACH ROW EXECUTE FUNCTION schedule_trigger_handover_tenants_guard();
CREATE TRIGGER schedule_trigger_handover_tenants_no_delete BEFORE DELETE ON schedule_trigger_handover_tenants
    FOR EACH ROW EXECUTE FUNCTION execution_reject_mutation();
CREATE TRIGGER schedule_trigger_handover_tenants_no_truncate BEFORE TRUNCATE ON schedule_trigger_handover_tenants
    FOR EACH STATEMENT EXECUTE FUNCTION execution_reject_mutation();
ALTER TABLE schedule_trigger_handover_tenants ENABLE ALWAYS TRIGGER schedule_trigger_handover_tenants_guard;
ALTER TABLE schedule_trigger_handover_tenants ENABLE ALWAYS TRIGGER schedule_trigger_handover_tenants_no_delete;
ALTER TABLE schedule_trigger_handover_tenants ENABLE ALWAYS TRIGGER schedule_trigger_handover_tenants_no_truncate;

-- The retirement of the role `sagat_handover` (who, when): `migrate` logs the role in only while this is empty (D2269).
CREATE TABLE schedule_trigger_handover_retirement (
    only_row   boolean     PRIMARY KEY DEFAULT true CHECK (only_row),
    retired_by text        NOT NULL,
    retired_at timestamptz NOT NULL,
    CONSTRAINT schedule_trigger_handover_retirement_by_bounded CHECK (char_length(retired_by) BETWEEN 1 AND 128)
);
CREATE TRIGGER schedule_trigger_handover_retirement_final BEFORE UPDATE OR DELETE ON schedule_trigger_handover_retirement
    FOR EACH ROW EXECUTE FUNCTION execution_reject_mutation();
CREATE TRIGGER schedule_trigger_handover_retirement_no_truncate BEFORE TRUNCATE ON schedule_trigger_handover_retirement
    FOR EACH STATEMENT EXECUTE FUNCTION execution_reject_mutation();
ALTER TABLE schedule_trigger_handover_retirement ENABLE ALWAYS TRIGGER schedule_trigger_handover_retirement_final;
ALTER TABLE schedule_trigger_handover_retirement ENABLE ALWAYS TRIGGER schedule_trigger_handover_retirement_no_truncate;

-- ---------------------------------------------------------------------------------------------------------------------
-- The job's attempts. One row per run id (the Job's UID in Helm, drawn at start in compose, D2263): its first round, horizon,
-- deadline and stricter values are kept from the first round (a retried pod resumes them); every round writes the last reason, how
-- many rounds in a row gave it, and when. The engines read the most recent row for their gauges.
-- ---------------------------------------------------------------------------------------------------------------------

-- The closed set of the reasons a handover is blocked (ADR-0091 decision 5). A test holds it to the Rust enum and the runbook.
CREATE FUNCTION schedule_trigger_handover_reasons() RETURNS text[]
LANGUAGE sql IMMUTABLE SET search_path = pg_catalog, pg_temp AS $$
    SELECT ARRAY['ENGINE_LACKS_CAPABILITY', 'ENGINE_UNREGISTERED', 'TOO_FEW_ENGINES', 'FLEET_CHANGING', 'RECONCILIATION_RECENT',
                 'NO_CONTROL_PLANE', 'UNCONVERGED', 'FIRST_HORIZON', 'BUSY', 'CHECK_TOO_SLOW', 'INTERRUPTED', 'DEADLINE']::text[]
$$;

CREATE TABLE schedule_trigger_handover_attempts (
    run_id         uuid        PRIMARY KEY,
    first_round_at timestamptz NOT NULL,
    deadline       timestamptz NOT NULL,
    horizon        interval    NOT NULL,
    stable_wanted  integer     NOT NULL,
    min_live       integer     NOT NULL,
    -- the streak: how many rounds in a row, at least a heartbeat apart, every condition held with the same live set
    stable_count   integer     NOT NULL DEFAULT 0,
    stable_set     uuid[],
    last_stable_at timestamptz,
    last_round_at  timestamptz NOT NULL,
    last_reason    text,
    reason_count   integer     NOT NULL DEFAULT 0,
    rounds         bigint      NOT NULL DEFAULT 0,
    CONSTRAINT schedule_trigger_handover_attempts_horizon_bounded
        CHECK (horizon BETWEEN interval '10 minutes' AND interval '1 hour'),
    CONSTRAINT schedule_trigger_handover_attempts_stable_bounded CHECK (stable_wanted BETWEEN 3 AND 1000),
    CONSTRAINT schedule_trigger_handover_attempts_live_bounded CHECK (min_live BETWEEN 1 AND 1000),
    CONSTRAINT schedule_trigger_handover_attempts_deadline_bounded
        CHECK (deadline >= first_round_at AND deadline <= first_round_at + interval '24 hours'),
    CONSTRAINT schedule_trigger_handover_attempts_set_bounded CHECK (stable_set IS NULL OR cardinality(stable_set) <= 1000),
    CONSTRAINT schedule_trigger_handover_attempts_reason_closed
        CHECK (last_reason IS NULL OR last_reason = ANY (public.schedule_trigger_handover_reasons()))
);
CREATE INDEX schedule_trigger_handover_attempts_recent_idx ON schedule_trigger_handover_attempts (last_round_at DESC);

-- The owner's floor of the live engines (security review m3): a caller of the round function may ask for a weaker `--min-live`
-- than the deployment runs; the round and the wrapper count with the greater of the run's and this one. `migrate` writes it from
-- SAGAT_HANDOVER_MIN_LIVE (D2291).
CREATE TABLE schedule_trigger_handover_floor (
    only_row boolean     PRIMARY KEY DEFAULT true CHECK (only_row),
    min_live integer     NOT NULL,
    set_by   text        NOT NULL,
    set_at   timestamptz NOT NULL,
    CONSTRAINT schedule_trigger_handover_floor_live_bounded CHECK (min_live BETWEEN 1 AND 1000),
    CONSTRAINT schedule_trigger_handover_floor_by_bounded CHECK (char_length(set_by) BETWEEN 1 AND 128)
);

-- Sets the floor (owner-only: `migrate` calls it). Returns the floor now in force.
CREATE FUNCTION schedule_trigger_handover_set_floor(p_min_live integer) RETURNS integer
LANGUAGE plpgsql SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    IF p_min_live IS NULL OR p_min_live NOT BETWEEN 1 AND 1000 THEN
        RAISE EXCEPTION 'the floor of the live engines is 1 to 1000' USING ERRCODE = 'invalid_parameter_value';
    END IF;
    INSERT INTO public.schedule_trigger_handover_floor AS f (only_row, min_live, set_by, set_at)
    VALUES (true, p_min_live, left(session_user::text, 128), clock_timestamp())
    ON CONFLICT (only_row) DO UPDATE SET min_live = EXCLUDED.min_live, set_by = EXCLUDED.set_by, set_at = EXCLUDED.set_at;
    RETURN p_min_live;
END;
$$;

-- The floor in force (internal): the greater of the run's and the owner's, at least one.
CREATE FUNCTION schedule_trigger_handover_min_live(p_run_min_live integer) RETURNS integer
LANGUAGE sql STABLE SET search_path = pg_catalog, pg_temp AS $$
    SELECT greatest(coalesce(p_run_min_live, 1), coalesce((SELECT f.min_live FROM public.schedule_trigger_handover_floor f), 1), 1)
$$;

-- Opens the attempt row of a run, or reads it (internal: the round and the note). A run's values are kept from its first call,
-- each floored; a new run is refused while 16 others are before their deadline and not stopped (security review m4, D2292).
CREATE FUNCTION schedule_trigger_handover_attempt_open(
    p_run uuid, p_stable integer, p_horizon interval, p_min_live integer, p_wait interval)
RETURNS public.schedule_trigger_handover_attempts
LANGUAGE plpgsql SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    v_now timestamptz := clock_timestamp();
    a     public.schedule_trigger_handover_attempts%ROWTYPE;
BEGIN
    IF p_run IS NULL THEN
        RAISE EXCEPTION 'a round names its run' USING ERRCODE = 'invalid_parameter_value';
    END IF;
    IF p_horizon IS NOT NULL AND p_horizon > interval '1 hour' THEN
        RAISE EXCEPTION 'the horizon is at most an hour: expired scheduler memberships are deleted after one'
            USING ERRCODE = 'invalid_parameter_value';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM public.schedule_trigger_handover_attempts t WHERE t.run_id = p_run) THEN
        PERFORM pg_advisory_xact_lock(hashtextextended('sagat:schedule_trigger_handover_attempts:open', 0));
        IF (SELECT count(*) FROM public.schedule_trigger_handover_attempts o
             WHERE o.run_id <> p_run AND o.deadline > v_now
               AND o.last_reason IS DISTINCT FROM 'INTERRUPTED' AND o.last_reason IS DISTINCT FROM 'DEADLINE') >= 16 THEN
            RAISE EXCEPTION 'TOO_MANY_RUNS: sixteen runs of the automatic handover are before their deadline'
                USING ERRCODE = 'program_limit_exceeded';
        END IF;
    END IF;
    INSERT INTO public.schedule_trigger_handover_attempts AS t
        (run_id, first_round_at, deadline, horizon, stable_wanted, min_live, last_round_at)
    VALUES (p_run, v_now,
            v_now + least(greatest(coalesce(p_wait, interval '2 hours'), interval '0'), interval '24 hours'),
            greatest(coalesce(p_horizon, interval '10 minutes'), interval '10 minutes'),
            least(greatest(coalesce(p_stable, 3), 3), 1000),
            least(greatest(coalesce(p_min_live, 1), 1), 1000),
            v_now)
    ON CONFLICT (run_id) DO NOTHING;
    SELECT * INTO STRICT a FROM public.schedule_trigger_handover_attempts t WHERE t.run_id = p_run FOR UPDATE;
    -- the rows of runs a week old give way, a few at a time
    DELETE FROM public.schedule_trigger_handover_attempts d WHERE d.run_id IN (
        SELECT o.run_id FROM public.schedule_trigger_handover_attempts o
         WHERE o.last_round_at < v_now - interval '7 days' AND o.run_id <> p_run
         ORDER BY o.last_round_at LIMIT 16 FOR UPDATE SKIP LOCKED);
    RETURN a;
END;
$$;

-- ---------------------------------------------------------------------------------------------------------------------
-- The mark of the reconciliation: set when it makes, revives, respecifies, resumes or removes a trigger-owned schedule, never by a
-- publication (D2243, D2258).
-- ---------------------------------------------------------------------------------------------------------------------

ALTER TABLE schedules ADD COLUMN reconciled_at timestamptz;
CREATE INDEX schedules_reconciled_at_idx ON schedules (reconciled_at) WHERE reconciled_at IS NOT NULL;

-- ---------------------------------------------------------------------------------------------------------------------
-- The conditions (internal: called by the round and the wrapper, which run as the owner).
-- ---------------------------------------------------------------------------------------------------------------------

-- Every condition of ADR-0091 decision 2 but the stable rounds and the first horizon, in its order: the first that fails is the
-- reason (null when all hold). The live set is the execution engines seen within 30 seconds, sorted.
CREATE FUNCTION schedule_trigger_handover_conditions(p_horizon interval, p_min_live integer)
RETURNS TABLE (out_reason text, out_live integer, out_set uuid[], out_examples uuid[])
LANGUAGE plpgsql SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    v_now      timestamptz := clock_timestamp();
    v_set      uuid[];
    v_examples uuid[];
BEGIN
    SELECT coalesce(array_agg(s.instance_id ORDER BY s.instance_id), '{}'::uuid[]) INTO v_set
      FROM public.serving_instances s
     WHERE s.kind = 'EXECUTION_ENGINE' AND s.last_seen > v_now - interval '30 seconds';
    -- every engine seen within the horizon lists schedules-v1 (one that crashed without it counts)
    SELECT (array_agg(s.instance_id ORDER BY s.instance_id))[1:16] INTO v_examples
      FROM public.serving_instances s
     WHERE s.kind = 'EXECUTION_ENGINE' AND s.last_seen > v_now - p_horizon
       AND NOT ('schedules-v1' = ANY (s.capabilities));
    IF v_examples IS NOT NULL THEN
        RETURN QUERY SELECT 'ENGINE_LACKS_CAPABILITY'::text, cardinality(v_set), v_set, v_examples;
        RETURN;
    END IF;
    -- a membership within the horizon (expired ones count) that no engine seen within the horizon names: a 0.2 engine (D2242)
    IF EXISTS (SELECT 1 FROM public.scheduler_members m
                WHERE m.expires_at > v_now - p_horizon
                  AND NOT EXISTS (SELECT 1 FROM public.serving_instances s
                                   WHERE s.kind = 'EXECUTION_ENGINE' AND s.last_seen > v_now - p_horizon
                                     AND s.scheduler_owner = m.owner)) THEN
        RETURN QUERY SELECT 'ENGINE_UNREGISTERED'::text, cardinality(v_set), v_set, '{}'::uuid[];
        RETURN;
    END IF;
    IF cardinality(v_set) < greatest(coalesce(p_min_live, 1), 1) THEN
        RETURN QUERY SELECT 'TOO_FEW_ENGINES'::text, cardinality(v_set), v_set, '{}'::uuid[];
        RETURN;
    END IF;
    -- work of the reconciliation within the horizon: an old control plane, or the backlog of the upgrade (D2243)
    IF EXISTS (SELECT 1 FROM public.schedules x WHERE x.reconciled_at > v_now - p_horizon) THEN
        RETURN QUERY SELECT 'RECONCILIATION_RECENT'::text, cardinality(v_set), v_set, '{}'::uuid[];
        RETURN;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM public.serving_instances s
                    WHERE s.kind = 'CONTROL_PLANE' AND s.last_seen > v_now - interval '30 seconds') THEN
        RETURN QUERY SELECT 'NO_CONTROL_PLANE'::text, cardinality(v_set), v_set, '{}'::uuid[];
        RETURN;
    END IF;
    IF public.schedule_trigger_unconverged() THEN
        RETURN QUERY SELECT 'UNCONVERGED'::text, cardinality(v_set), v_set, '{}'::uuid[];
        RETURN;
    END IF;
    RETURN QUERY SELECT NULL::text, cardinality(v_set), v_set, '{}'::uuid[];
END;
$$;

-- ---------------------------------------------------------------------------------------------------------------------
-- The round (executable by `sagat_handover`). It takes the run id and the job's own stricter values, each floored here: the
-- horizon between 10 minutes and an hour, at least 3 stable rounds, at least 1 live engine, a wait of at most 24 hours. They are
-- kept from the run's first round. What only the job can see is written by the note function below (OD-2026-11-15-1).
--
-- outcome: HANDED_OVER | UNKNOWN_STATE | BLOCKED (out_reason says why) | COUNTING | STABLE (the wrapper may be called now)
--          | DEADLINE (out_reason is the reason the round found)
-- ---------------------------------------------------------------------------------------------------------------------

CREATE FUNCTION schedule_trigger_handover_round(
    p_run uuid, p_stable integer, p_horizon interval, p_min_live integer, p_wait interval)
RETURNS TABLE (out_outcome text, out_reason text, out_stable integer, out_wanted integer, out_live integer,
               out_first_round_at timestamptz, out_deadline timestamptz, out_now timestamptz, out_examples uuid[])
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    v_now     timestamptz := clock_timestamp();
    v_state   text;
    a         public.schedule_trigger_handover_attempts%ROWTYPE;
    c         record;
    v_reason  text;
    v_outcome text;
    v_count   integer;
    v_set     uuid[];
    v_at      timestamptz;
BEGIN
    a := public.schedule_trigger_handover_attempt_open(p_run, p_stable, p_horizon, p_min_live, p_wait);

    SELECT h.state INTO STRICT v_state FROM public.schedule_trigger_handover h;
    IF v_state = 'HANDED_OVER' OR v_state <> 'LEGACY' THEN
        UPDATE public.schedule_trigger_handover_attempts
           SET last_round_at = v_now, last_reason = NULL, reason_count = 0, rounds = rounds + 1
         WHERE run_id = p_run;
        RETURN QUERY SELECT CASE WHEN v_state = 'HANDED_OVER' THEN 'HANDED_OVER' ELSE 'UNKNOWN_STATE' END::text, NULL::text,
                            a.stable_count, a.stable_wanted, 0, a.first_round_at, a.deadline, v_now, '{}'::uuid[];
        RETURN;
    END IF;

    SELECT * INTO STRICT c
      FROM public.schedule_trigger_handover_conditions(a.horizon, public.schedule_trigger_handover_min_live(a.min_live));
    v_count := a.stable_count;
    v_set := a.stable_set;
    v_at := a.last_stable_at;
    -- a streak with a gap of more than two heartbeats is not consecutive any more (correctness review): it starts over
    IF v_at IS NOT NULL AND v_at < v_now - interval '30 seconds' THEN
        v_count := 0;
        v_set := NULL;
        v_at := NULL;
    END IF;
    IF c.out_reason IS NOT NULL THEN
        -- the streak ends
        v_reason := c.out_reason;
        v_count := 0;
        v_set := NULL;
        v_at := NULL;
    ELSIF v_set IS NULL OR v_set <> c.out_set THEN
        -- a new streak with this live set; a changed set is the fleet changing
        v_reason := CASE WHEN v_set IS NOT NULL THEN 'FLEET_CHANGING' END;
        v_count := 1;
        v_set := c.out_set;
        v_at := v_now;
    ELSIF v_at <= v_now - interval '15 seconds' THEN
        -- one more round at least a heartbeat after the last one counted
        v_count := v_count + 1;
        v_at := v_now;
    END IF;
    IF v_reason IS NULL AND v_now < a.first_round_at + a.horizon THEN
        v_reason := 'FIRST_HORIZON';
    END IF;
    v_outcome := CASE WHEN v_now >= a.deadline THEN 'DEADLINE'
                      WHEN v_reason IS NOT NULL THEN 'BLOCKED'
                      WHEN v_count >= a.stable_wanted THEN 'STABLE'
                      ELSE 'COUNTING' END;
    UPDATE public.schedule_trigger_handover_attempts
       SET stable_count = v_count, stable_set = v_set, last_stable_at = v_at, last_round_at = v_now,
           last_reason = CASE WHEN v_outcome = 'DEADLINE' THEN 'DEADLINE' ELSE v_reason END,
           reason_count = CASE WHEN v_outcome = 'DEADLINE' THEN 1
                               WHEN v_reason IS NULL THEN 0
                               WHEN last_reason = v_reason THEN reason_count + 1 ELSE 1 END,
           rounds = rounds + 1
     WHERE run_id = p_run;
    RETURN QUERY SELECT v_outcome, v_reason, v_count, a.stable_wanted, c.out_live, a.first_round_at, a.deadline, v_now,
                        c.out_examples;
END;
$$;

-- ---------------------------------------------------------------------------------------------------------------------
-- The note (executable by `sagat_handover`, OD-2026-11-15-1): what only the job sees, written as its run's last reason and counting
-- nothing. A closed set: BUSY, CHECK_TOO_SLOW, INTERRUPTED, DEADLINE. It takes the run's values as the round does, so that a run whose
-- every round failed still has its row (the job's first round may be the one that timed out).
--
-- outcome: NOTED | HANDED_OVER
-- ---------------------------------------------------------------------------------------------------------------------

CREATE FUNCTION schedule_trigger_handover_note(
    p_run uuid, p_reason text, p_stable integer, p_horizon interval, p_min_live integer, p_wait interval)
RETURNS TABLE (out_outcome text, out_reason text, out_deadline timestamptz, out_now timestamptz)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    v_now timestamptz := clock_timestamp();
    a     public.schedule_trigger_handover_attempts%ROWTYPE;
BEGIN
    IF p_reason IS NULL OR p_reason NOT IN ('BUSY', 'CHECK_TOO_SLOW', 'INTERRUPTED', 'DEADLINE') THEN
        RAISE EXCEPTION 'a note is BUSY, CHECK_TOO_SLOW, INTERRUPTED or DEADLINE: what the job alone can see'
            USING ERRCODE = 'invalid_parameter_value';
    END IF;
    a := public.schedule_trigger_handover_attempt_open(p_run, p_stable, p_horizon, p_min_live, p_wait);
    IF (SELECT h.state FROM public.schedule_trigger_handover h) = 'HANDED_OVER' THEN
        RETURN QUERY SELECT 'HANDED_OVER'::text, NULL::text, a.deadline, v_now;
        RETURN;
    END IF;
    UPDATE public.schedule_trigger_handover_attempts
       SET last_round_at = v_now, last_reason = p_reason,
           reason_count = CASE WHEN last_reason = p_reason THEN reason_count + 1 ELSE 1 END, rounds = rounds + 1
     WHERE run_id = p_run;
    RETURN QUERY SELECT 'NOTED'::text, p_reason, a.deadline, v_now;
END;
$$;

-- ---------------------------------------------------------------------------------------------------------------------
-- The move (internal). Run by the operator's `schedule_trigger_handover_complete` and by the wrapper, both as the owner, with
-- the requirement epoch held FOR UPDATE by their transaction. Takes the triggers' table (bounded, HANDOVER_BUSY) and the handover
-- row, in the order of D2070; refuses unless the requirement is recorded and nothing is unconverged; writes the census copy and the
-- tenant set, keyed by the epoch, and moves the state with the mode and the epoch. Null when it was handed over before.
-- ---------------------------------------------------------------------------------------------------------------------

CREATE FUNCTION schedule_trigger_handover_record(p_actor text, p_mode text, p_horizon interval) RETURNS integer
LANGUAGE plpgsql SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    v_state   text;
    v_epoch   bigint;
    v_census  integer;
    v_tenants integer;
BEGIN
    IF p_actor IS NULL OR char_length(p_actor) NOT BETWEEN 1 AND 255 THEN
        RAISE EXCEPTION 'the handover names the operator that did it, 1 to 255 characters'
            USING ERRCODE = 'invalid_parameter_value';
    END IF;
    IF p_mode IS NULL OR p_mode NOT IN ('OPERATOR', 'AUTOMATIC') THEN
        RAISE EXCEPTION 'the mode of a handover is OPERATOR or AUTOMATIC' USING ERRCODE = 'invalid_parameter_value';
    END IF;
    PERFORM public.schedule_trigger_handover_lock();
    SELECT h.state INTO STRICT v_state FROM public.schedule_trigger_handover h FOR UPDATE;
    IF v_state = 'HANDED_OVER' THEN
        RETURN NULL;
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
    IF public.schedule_trigger_unconverged() THEN
        RAISE EXCEPTION 'HANDOVER_UNCONVERGED: cron trigger(s) have no schedule equal to them'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
    SELECT e.epoch INTO STRICT v_epoch FROM public.platform_requirement_epoch e;
    INSERT INTO public.schedule_trigger_handover_census (epoch, instance_id, kind, binary_version, started_at, last_seen)
    SELECT v_epoch, s.instance_id, s.kind, s.binary_version, s.started_at, s.last_seen
      FROM public.serving_instances s
     WHERE s.last_seen > clock_timestamp() - greatest(coalesce(p_horizon, interval '10 minutes'), interval '10 minutes');
    GET DIAGNOSTICS v_census = ROW_COUNT;
    IF v_census > 1000 THEN
        RAISE EXCEPTION 'the census of the handover holds more than 1,000 instances, which the registry never holds'
            USING ERRCODE = 'program_limit_exceeded';
    END IF;
    INSERT INTO public.schedule_trigger_handover_tenants (epoch, tenant_id, audit_id)
    SELECT v_epoch, d.tenant_id, gen_random_uuid()
      FROM (SELECT DISTINCT t.tenant_id FROM public.workflow_triggers t WHERE t.kind = 'cron') d;
    GET DIAGNOSTICS v_tenants = ROW_COUNT;
    PERFORM set_config('sagat.handover_change', 'handover', true);
    UPDATE public.schedule_trigger_handover
       SET state = 'HANDED_OVER', changed_by = p_actor, changed_at = clock_timestamp(),
           changed_db_role = session_user::text, changed_mode = p_mode, changed_epoch = v_epoch;
    PERFORM set_config('sagat.handover_change', '', true);
    RETURN v_tenants;
END;
$$;

-- The operator's function, replaced: the same checks and the same answer, now through the move, with the mode OPERATOR.
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
    IF public.schedule_trigger_handover_record(p_actor, 'OPERATOR', interval '10 minutes') IS NULL THEN
        RETURN 'ALREADY_HANDED_OVER';
    END IF;
    RETURN 'HANDED_OVER';
END;
$$;

-- ---------------------------------------------------------------------------------------------------------------------
-- The wrapper (executable by `sagat_handover`): the automatic handover, with the run id only. It refuses unless `session_user` is
-- exactly `sagat_handover` (a member of the role that logs in as itself has another session_user, D2271); takes the epoch FOR UPDATE,
-- then the triggers' table and the handover row (D2070); computes every condition again under them, with the run's first horizon,
-- its stable count reached with the same live set and its last stable round younger than one heartbeat (12 s) on the database's
-- clock; only then records `schedules-v1` for the engines and moves the state, with the mode AUTOMATIC and the actor
-- `system:handover-job:<run id>`. A refusal records nothing.
--
-- outcome: HANDED_OVER | ALREADY_HANDED_OVER | REFUSED (out_reason) | KINDS_DIFFER | UNKNOWN_STATE
-- ---------------------------------------------------------------------------------------------------------------------

CREATE FUNCTION schedule_trigger_handover_auto(p_run uuid)
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

-- ---------------------------------------------------------------------------------------------------------------------
-- The audit completion (executable by `sagat_handover`; the owner calls it after the operator's command too). After the commit of
-- a handover it writes, for every tenant of its recorded set not yet audited, the audit record under the id drawn in the
-- activation, with the actor, the mode and the time of the handover row, and marks the row. An id already present is compared; a
-- record that differs is refused (AUDIT_CONFLICT, SQLSTATE 23505) and nothing of the call is kept. Concurrent completions
-- serialise on the rows of the set.
-- ---------------------------------------------------------------------------------------------------------------------

CREATE FUNCTION schedule_trigger_handover_audit()
RETURNS TABLE (out_written integer, out_tenants integer, out_epoch bigint, out_mode text)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    h         public.schedule_trigger_handover%ROWTYPE;
    t         record;
    e         public.audit_records%ROWTYPE;
    v_reason  text;
    v_written integer := 0;
    v_total   integer;
BEGIN
    SELECT * INTO STRICT h FROM public.schedule_trigger_handover;
    IF h.state <> 'HANDED_OVER' OR h.changed_epoch IS NULL THEN
        RETURN QUERY SELECT 0, 0, NULL::bigint, NULL::text;
        RETURN;
    END IF;
    v_reason := format('the cron triggers were handed over to the schedule pass (mode %s, requirement epoch %s)',
                       h.changed_mode, h.changed_epoch);
    FOR t IN SELECT s.tenant_id, s.audit_id FROM public.schedule_trigger_handover_tenants s
              WHERE s.epoch = h.changed_epoch AND s.audited_at IS NULL
              ORDER BY s.tenant_id FOR UPDATE LOOP
        SELECT * INTO e FROM public.audit_records x WHERE x.id = t.audit_id;
        IF FOUND THEN
            IF e.tenant_id <> t.tenant_id OR e.actor <> h.changed_by OR e.action <> 'schedule.trigger_handover'
               OR e.resource_type <> 'platform' OR e.resource_id <> 'schedule_trigger_handover'
               OR e.reason IS DISTINCT FROM v_reason OR e.created_at <> h.changed_at
               OR e.trace_id IS DISTINCT FROM NULL::text THEN
                RAISE EXCEPTION 'AUDIT_CONFLICT: an audit record with the id drawn for a tenant of the handover exists and differs'
                    USING ERRCODE = 'unique_violation';
            END IF;
        ELSE
            INSERT INTO public.audit_records (id, tenant_id, actor, action, resource_type, resource_id, reason, trace_id, created_at)
            VALUES (t.audit_id, t.tenant_id, h.changed_by, 'schedule.trigger_handover', 'platform', 'schedule_trigger_handover',
                    v_reason, NULL, h.changed_at);
            v_written := v_written + 1;
        END IF;
        UPDATE public.schedule_trigger_handover_tenants
           SET audited_at = clock_timestamp()
         WHERE epoch = h.changed_epoch AND tenant_id = t.tenant_id;
    END LOOP;
    SELECT count(*)::integer INTO v_total FROM public.schedule_trigger_handover_tenants s WHERE s.epoch = h.changed_epoch;
    RETURN QUERY SELECT v_written, v_total, h.changed_epoch, h.changed_mode;
END;
$$;

-- ---------------------------------------------------------------------------------------------------------------------
-- The retirement (executable by `sagat_handover`): after the handover and its audit, records who retired the role and when, and
-- takes its login away. Where the owner may not alter the role (PostgreSQL 16 without ADMIN OPTION, D2274) the retirement is still
-- recorded, so that `migrate` does not log the role in again, and the operator runs the statement the runbook gives.
--
-- outcome: RETIRED | RECORDED_NOT_ALTERED | NOT_HANDED_OVER | AUDIT_PENDING
-- ---------------------------------------------------------------------------------------------------------------------

CREATE FUNCTION schedule_trigger_handover_retire() RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    h public.schedule_trigger_handover%ROWTYPE;
BEGIN
    SELECT * INTO STRICT h FROM public.schedule_trigger_handover;
    IF h.state <> 'HANDED_OVER' THEN
        RETURN 'NOT_HANDED_OVER';
    END IF;
    IF EXISTS (SELECT 1 FROM public.schedule_trigger_handover_tenants s
                WHERE s.epoch = h.changed_epoch AND s.audited_at IS NULL) THEN
        RETURN 'AUDIT_PENDING';
    END IF;
    INSERT INTO public.schedule_trigger_handover_retirement (only_row, retired_by, retired_at)
    VALUES (true, left(session_user::text, 128), clock_timestamp())
    ON CONFLICT (only_row) DO NOTHING;
    BEGIN
        -- no login, and no password left to try (security review m5)
        ALTER ROLE sagat_handover NOLOGIN PASSWORD NULL;
    EXCEPTION WHEN insufficient_privilege OR undefined_object THEN
        RETURN 'RECORDED_NOT_ALTERED';
    END;
    -- the sessions the role still holds end, best effort (the owner may lack the privilege to signal them)
    BEGIN
        PERFORM pg_catalog.pg_terminate_backend(a.pid) FROM pg_catalog.pg_stat_activity a
         WHERE a.usename = 'sagat_handover' AND a.pid <> pg_catalog.pg_backend_pid();
    EXCEPTION WHEN insufficient_privilege THEN
        NULL;
    END;
    RETURN 'RETIRED';
END;
$$;

-- ---------------------------------------------------------------------------------------------------------------------
-- The role, ensured by every `migrate` run (owner-only, not definers: they run as the migrating role, D2264).
-- ---------------------------------------------------------------------------------------------------------------------

-- Creates `sagat_handover` NOLOGIN when it is missing and the migrating role may create roles.
-- outcome: EXISTS | CREATED | NOT_PERMITTED
CREATE FUNCTION schedule_trigger_handover_role_ensure() RETURNS text
LANGUAGE plpgsql SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = 'sagat_handover') THEN
        RETURN 'EXISTS';
    END IF;
    BEGIN
        CREATE ROLE sagat_handover NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
    EXCEPTION
        WHEN duplicate_object OR unique_violation THEN
            RETURN 'EXISTS';
        WHEN insufficient_privilege THEN
            RETURN 'NOT_PERMITTED';
    END;
    RETURN 'CREATED';
END;
$$;

-- Whether `sagat_handover` is as narrow as decided (D2275, security review M1 and m6): no attribute beyond LOGIN, a member of no role,
-- no member that can act as it (one that may SET it or INHERIT its privileges; a member with ADMIN OPTION alone, as PostgreSQL 16
-- records the role's creator, can only grant it), owning nothing. Owner-only: the grant function and `migrate`'s login step ask it.
-- outcome: NARROW | WIDER | MISSING
CREATE FUNCTION schedule_trigger_handover_role_narrow() RETURNS text
LANGUAGE plpgsql STABLE SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    r         pg_catalog.pg_roles%ROWTYPE;
    v_members boolean;
BEGIN
    SELECT * INTO r FROM pg_catalog.pg_roles WHERE rolname = 'sagat_handover';
    IF NOT FOUND THEN
        RETURN 'MISSING';
    END IF;
    IF current_setting('server_version_num')::integer >= 160000 THEN
        EXECUTE 'SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_auth_members m WHERE m.roleid = $1 AND (m.set_option OR m.inherit_option))'
           INTO v_members USING r.oid;
    ELSE
        SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_auth_members m WHERE m.roleid = r.oid) INTO v_members;
    END IF;
    IF r.rolsuper OR r.rolcreatedb OR r.rolcreaterole OR r.rolreplication OR r.rolbypassrls OR v_members
       OR EXISTS (SELECT 1 FROM pg_catalog.pg_auth_members m WHERE m.member = r.oid)
       OR EXISTS (SELECT 1 FROM pg_catalog.pg_class c WHERE c.relowner = r.oid)
       OR EXISTS (SELECT 1 FROM pg_catalog.pg_proc p WHERE p.proowner = r.oid)
       OR EXISTS (SELECT 1 FROM pg_catalog.pg_namespace n WHERE n.nspowner = r.oid)
       OR EXISTS (SELECT 1 FROM pg_catalog.pg_type y WHERE y.typowner = r.oid) THEN
        RETURN 'WIDER';
    END IF;
    RETURN 'NARROW';
END;
$$;

-- Holds every grant of `sagat_handover`, and grants them again (so a role created after this migration gets them). Refuses a role
-- wider than the design: such a role is left with none of these grants (D2275). The grants are the five functions of ADR-0091
-- decision 9, SELECT on the handover and attempt rows, and SELECT on the one column of the requirement epoch that ADR-0082's
-- deferred check reads at the commit of the job's activation (OD-2026-11-15-1, D2289).
-- outcome: GRANTED | WIDER | MISSING
CREATE FUNCTION schedule_trigger_handover_role_grants() RETURNS text
LANGUAGE plpgsql SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    v_narrow text := public.schedule_trigger_handover_role_narrow();
BEGIN
    IF v_narrow = 'MISSING' THEN
        RETURN 'MISSING';
    END IF;
    IF v_narrow = 'WIDER' THEN
        REVOKE ALL ON FUNCTION public.schedule_trigger_handover_round(uuid, integer, interval, integer, interval)
            FROM sagat_handover;
        REVOKE ALL ON FUNCTION public.schedule_trigger_handover_note(uuid, text, integer, interval, integer, interval)
            FROM sagat_handover;
        REVOKE ALL ON FUNCTION public.schedule_trigger_handover_auto(uuid) FROM sagat_handover;
        REVOKE ALL ON FUNCTION public.schedule_trigger_handover_audit() FROM sagat_handover;
        REVOKE ALL ON FUNCTION public.schedule_trigger_handover_retire() FROM sagat_handover;
        REVOKE ALL PRIVILEGES ON public.schedule_trigger_handover, public.schedule_trigger_handover_attempts FROM sagat_handover;
        REVOKE ALL PRIVILEGES (changed_xact) ON public.platform_requirement_epoch FROM sagat_handover;
        RETURN 'WIDER';
    END IF;
    GRANT EXECUTE ON FUNCTION public.schedule_trigger_handover_round(uuid, integer, interval, integer, interval)
        TO sagat_handover;
    GRANT EXECUTE ON FUNCTION public.schedule_trigger_handover_note(uuid, text, integer, interval, integer, interval)
        TO sagat_handover;
    GRANT EXECUTE ON FUNCTION public.schedule_trigger_handover_auto(uuid) TO sagat_handover;
    GRANT EXECUTE ON FUNCTION public.schedule_trigger_handover_audit() TO sagat_handover;
    GRANT EXECUTE ON FUNCTION public.schedule_trigger_handover_retire() TO sagat_handover;
    GRANT SELECT ON public.schedule_trigger_handover, public.schedule_trigger_handover_attempts TO sagat_handover;
    GRANT SELECT (changed_xact) ON public.platform_requirement_epoch TO sagat_handover;
    RETURN 'GRANTED';
END;
$$;

-- How many tenants of the handover's recorded set still lack their audit record (null before the handover): what the engines
-- export as trigger_schedule_handover_audit_pending, so that a committed handover whose audit could not be completed is seen
-- (correctness review). A view with its owner's rights: its reader sees a count, never the tenant set or its audit ids.
CREATE VIEW schedule_trigger_handover_audit_pending AS
SELECT CASE WHEN h.changed_epoch IS NULL THEN NULL
            ELSE (SELECT count(*)::integer FROM public.schedule_trigger_handover_tenants s
                   WHERE s.epoch = h.changed_epoch AND s.audited_at IS NULL) END AS tenants
  FROM public.schedule_trigger_handover h;

-- ---------------------------------------------------------------------------------------------------------------------
-- Grants. PUBLIC gets nothing; the runtime role reads the attempt rows (the engines' gauges) and nothing else new; the handover
-- role's grants are the grant function's, which `migrate` runs after the migrations.
-- ---------------------------------------------------------------------------------------------------------------------

REVOKE ALL PRIVILEGES ON schedule_trigger_handover_census, schedule_trigger_handover_tenants,
    schedule_trigger_handover_retirement, schedule_trigger_handover_attempts, schedule_trigger_handover_floor,
    schedule_trigger_handover_audit_pending FROM PUBLIC;
REVOKE ALL ON FUNCTION schedule_trigger_handover_reasons() FROM PUBLIC;
REVOKE ALL ON FUNCTION schedule_trigger_handover_tenants_guard() FROM PUBLIC;
REVOKE ALL ON FUNCTION schedule_trigger_handover_conditions(interval, integer) FROM PUBLIC;
REVOKE ALL ON FUNCTION schedule_trigger_handover_round(uuid, integer, interval, integer, interval) FROM PUBLIC;
REVOKE ALL ON FUNCTION schedule_trigger_handover_note(uuid, text, integer, interval, integer, interval) FROM PUBLIC;
REVOKE ALL ON FUNCTION schedule_trigger_handover_attempt_open(uuid, integer, interval, integer, interval) FROM PUBLIC;
REVOKE ALL ON FUNCTION schedule_trigger_handover_set_floor(integer) FROM PUBLIC;
REVOKE ALL ON FUNCTION schedule_trigger_handover_min_live(integer) FROM PUBLIC;
REVOKE ALL ON FUNCTION schedule_trigger_handover_role_narrow() FROM PUBLIC;
REVOKE ALL ON FUNCTION schedule_trigger_handover_record(text, text, interval) FROM PUBLIC;
REVOKE ALL ON FUNCTION schedule_trigger_handover_complete(text) FROM PUBLIC;
REVOKE ALL ON FUNCTION schedule_trigger_handover_auto(uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION schedule_trigger_handover_audit() FROM PUBLIC;
REVOKE ALL ON FUNCTION schedule_trigger_handover_retire() FROM PUBLIC;
REVOKE ALL ON FUNCTION schedule_trigger_handover_role_ensure() FROM PUBLIC;
REVOKE ALL ON FUNCTION schedule_trigger_handover_role_grants() FROM PUBLIC;
DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        REVOKE ALL PRIVILEGES ON schedule_trigger_handover_census, schedule_trigger_handover_tenants,
            schedule_trigger_handover_retirement, schedule_trigger_handover_attempts, schedule_trigger_handover_floor FROM sagat_app;
        GRANT SELECT ON schedule_trigger_handover_attempts TO sagat_app;
        -- whether the tenant audit of the handover is complete, for the engines' gauge: a count, never an audit id (D2294)
        GRANT SELECT ON schedule_trigger_handover_audit_pending TO sagat_app;
    END IF;
END $grants$;
