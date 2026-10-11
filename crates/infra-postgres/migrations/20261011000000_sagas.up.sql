-- Feature 003: sagas, compensations and reconciliation (FR-003-001..007).
--
-- The compensation and the reconciliation of an activity are steps like any other (derived nodes
-- `<id>~c` and `<id>~r` of the definition), so they live in `steps` and use its leases, retries and
-- fencing. What is new is what a run knows about its sagas, the plan of each saga as published, and
-- the states a run and a step can now be in.

-- The constraints below only widen what already holds, or are new on a column that has a default
-- (every row satisfies them), so they are added NOT VALID: they bind every row written from now on,
-- and adding them does not scan the table while holding its lock. `ALTER TABLE … VALIDATE CONSTRAINT`
-- (it does not block writes) can mark them validated at leisure.
--
-- A step is forward work, a compensation, or a reconciliation. The kind follows from the node id,
-- which the database checks: `~` appears only in the ids the engine derives.
ALTER TABLE steps ADD COLUMN kind text NOT NULL DEFAULT 'FORWARD';
ALTER TABLE steps ADD CONSTRAINT steps_kind_valid
    CHECK (kind IN ('FORWARD', 'COMPENSATION', 'RECONCILIATION')) NOT VALID;
ALTER TABLE steps ADD CONSTRAINT steps_kind_matches_node CHECK (
    CASE kind
        WHEN 'FORWARD' THEN position('~' IN node) = 0
        WHEN 'COMPENSATION' THEN node LIKE '%~c'
        ELSE node LIKE '%~r'
    END) NOT VALID;

ALTER TABLE steps DROP CONSTRAINT steps_status_valid;
ALTER TABLE steps ADD CONSTRAINT steps_status_valid CHECK (status IN
    ('PENDING', 'READY', 'LEASED', 'RUNNING', 'WAITING', 'RETRY_WAIT', 'SUCCEEDED', 'FAILED',
     'CANCELLED', 'SKIPPED')) NOT VALID;
ALTER TABLE steps DROP CONSTRAINT steps_waiting_kind_valid;
ALTER TABLE steps ADD CONSTRAINT steps_waiting_kind_valid CHECK (waiting_kind IS NULL OR waiting_kind IN
    ('TIMER', 'EVENT', 'BRANCHES', 'CHILD', 'SAGA', 'RECONCILIATION', 'OPERATOR')) NOT VALID;
-- A step that waits for an operator says why (the code is kept in `failure_code`).
ALTER TABLE steps ADD CONSTRAINT steps_operator_has_a_reason CHECK (
    waiting_kind IS DISTINCT FROM 'OPERATOR' OR failure_code IS NOT NULL) NOT VALID;

-- Runs: COMPENSATING and MANUAL_REVIEW are live; COMPENSATED, PARTIAL and COMPENSATION_FAILED end it.
ALTER TABLE runs DROP CONSTRAINT runs_status_valid;
ALTER TABLE runs ADD CONSTRAINT runs_status_valid CHECK (status IN
    ('RUNNING', 'WAITING', 'PAUSED', 'CANCELLING', 'COMPENSATING', 'MANUAL_REVIEW', 'SUCCEEDED',
     'FAILED', 'CANCELLED', 'TIMED_OUT', 'COMPENSATED', 'PARTIAL', 'COMPENSATION_FAILED')) NOT VALID;
ALTER TABLE runs DROP CONSTRAINT runs_stop_reason_valid;
ALTER TABLE runs ADD CONSTRAINT runs_stop_reason_valid CHECK (stop_reason IS NULL OR stop_reason IN
    ('CANCELLED', 'TIMED_OUT', 'FAILED', 'COMPENSATED', 'PARTIAL', 'COMPENSATION_FAILED')) NOT VALID;
ALTER TABLE runs DROP CONSTRAINT runs_finished_consistent;
ALTER TABLE runs ADD CONSTRAINT runs_finished_consistent CHECK ((finished_at IS NULL) = (status NOT IN
    ('SUCCEEDED', 'FAILED', 'CANCELLED', 'TIMED_OUT', 'COMPENSATED', 'PARTIAL', 'COMPENSATION_FAILED'))) NOT VALID;

-- What an operator looks for: the runs that wait for a decision.
CREATE INDEX runs_manual_review_idx ON runs (tenant_id, namespace_id, updated_at)
    WHERE status = 'MANUAL_REVIEW';

-- What a run knows about each of its sagas (a projection of the journal, written with it).
CREATE TABLE run_sagas (
    tenant_id    uuid        NOT NULL,
    run_id       uuid        NOT NULL,
    node         text        NOT NULL,
    phase        text        NOT NULL,
    -- Why the saga is compensating, or was.
    cause_code   text,
    outcome      text,
    -- It ended, or was aborted, while the run was being stopped.
    during_stop  boolean     NOT NULL DEFAULT false,
    -- Completed steps whose effect could not be undone.
    irreversible text[]      NOT NULL DEFAULT '{}',
    updated_at   timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, run_id, node),
    CONSTRAINT run_sagas_run_same_tenant_fk FOREIGN KEY (tenant_id, run_id) REFERENCES runs (tenant_id, id),
    CONSTRAINT run_sagas_phase_valid CHECK (phase IN ('FORWARD', 'ABORTING', 'COMPENSATING', 'DONE')),
    CONSTRAINT run_sagas_outcome_valid CHECK (outcome IS NULL OR outcome IN
        ('COMPENSATED', 'PARTIAL', 'COMPENSATION_FAILED')),
    CONSTRAINT run_sagas_outcome_only_when_done CHECK (outcome IS NULL OR phase = 'DONE')
);

-- The compensations a saga planned for a run, with the ones each waits for. Written once, when the
-- saga's forward work is over.
CREATE TABLE run_saga_entries (
    tenant_id       uuid  NOT NULL,
    run_id          uuid  NOT NULL,
    saga_node       text  NOT NULL,
    node            text  NOT NULL,
    -- Stable per (workflow version, node): the handle a provider can recognise across runs.
    compensation_id uuid  NOT NULL,
    after           text[] NOT NULL DEFAULT '{}',
    PRIMARY KEY (tenant_id, run_id, saga_node, node),
    CONSTRAINT run_saga_entries_saga_fk FOREIGN KEY (tenant_id, run_id, saga_node)
        REFERENCES run_sagas (tenant_id, run_id, node)
);

-- The compensation plan of each saga of a published version: a pure function of the definition,
-- stored once so that the plan a run followed can be shown and audited without re-deriving it.
-- Immutable, like the version it belongs to.
CREATE TABLE saga_plans (
    tenant_id               uuid        NOT NULL,
    workflow_version_id     uuid        NOT NULL,
    saga_node               text        NOT NULL,
    on_compensation_failure text        NOT NULL,
    cancel_policy           text        NOT NULL,
    irreversible            text[]      NOT NULL DEFAULT '{}',
    created_at              timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (tenant_id, workflow_version_id, saga_node),
    CONSTRAINT saga_plans_version_same_tenant_fk FOREIGN KEY (tenant_id, workflow_version_id)
        REFERENCES workflow_versions (tenant_id, id),
    CONSTRAINT saga_plans_failure_valid CHECK (on_compensation_failure IN ('manual_review', 'fail')),
    CONSTRAINT saga_plans_cancel_valid CHECK (cancel_policy IN
        ('compensate_completed', 'abort_no_compensation', 'wait_in_flight_then_compensate'))
);

CREATE TABLE saga_plan_entries (
    tenant_id             uuid  NOT NULL,
    workflow_version_id   uuid  NOT NULL,
    saga_node             text  NOT NULL,
    -- The order a worker could follow one by one: never before an entry it waits for.
    position              integer NOT NULL,
    node                  text  NOT NULL,
    compensation_id       uuid  NOT NULL,
    compensation_activity text  NOT NULL,
    after                 text[] NOT NULL DEFAULT '{}',
    PRIMARY KEY (tenant_id, workflow_version_id, saga_node, node),
    CONSTRAINT saga_plan_entries_plan_fk FOREIGN KEY (tenant_id, workflow_version_id, saga_node)
        REFERENCES saga_plans (tenant_id, workflow_version_id, saga_node),
    CONSTRAINT saga_plan_entries_position_valid CHECK (position >= 0)
);
CREATE UNIQUE INDEX saga_plan_entries_order_uniq
    ON saga_plan_entries (tenant_id, workflow_version_id, saga_node, position);

CREATE TRIGGER saga_plans_immutable BEFORE UPDATE OR DELETE ON saga_plans
    FOR EACH ROW EXECUTE FUNCTION execution_reject_mutation();
CREATE TRIGGER saga_plans_no_truncate BEFORE TRUNCATE ON saga_plans
    FOR EACH STATEMENT EXECUTE FUNCTION execution_reject_mutation();
CREATE TRIGGER saga_plan_entries_immutable BEFORE UPDATE OR DELETE ON saga_plan_entries
    FOR EACH ROW EXECUTE FUNCTION execution_reject_mutation();
CREATE TRIGGER saga_plan_entries_no_truncate BEFORE TRUNCATE ON saga_plan_entries
    FOR EACH STATEMENT EXECUTE FUNCTION execution_reject_mutation();
ALTER TABLE saga_plans ENABLE ALWAYS TRIGGER saga_plans_immutable;
ALTER TABLE saga_plans ENABLE ALWAYS TRIGGER saga_plans_no_truncate;
ALTER TABLE saga_plan_entries ENABLE ALWAYS TRIGGER saga_plan_entries_immutable;
ALTER TABLE saga_plan_entries ENABLE ALWAYS TRIGGER saga_plan_entries_no_truncate;

-- The circuit breaker of the compensations of one provider operation (an activity name) of one
-- namespace (FR-003-003). A provider that is down is not asked again by every run that has something
-- to undo: after enough consecutive failures no compensation of that activity is leased until the
-- window is over, and then exactly one probe is. It is per namespace because a namespace is an
-- environment: a worker that fails in `dev` must not hold back the same activity in `prod`. A row
-- exists only while the breaker has seen failures; a success deletes it.
CREATE TABLE compensation_breakers (
    tenant_id    uuid        NOT NULL,
    namespace_id uuid        NOT NULL,
    activity     text        NOT NULL,
    -- Attempts of compensations of this activity that ended without success, in a row.
    failures     integer     NOT NULL,
    -- NULL: closed. In the future: open (or the probe is in flight). In the past: half open, the
    -- next lease is the probe.
    opened_until timestamptz,
    -- The lease that was granted when the window was over is the probe: its failure reopens the
    -- breaker for twice as long, while a failure of an attempt that was already in flight when the
    -- breaker opened changes nothing.
    probing      boolean     NOT NULL DEFAULT false,
    -- How many times in a row the probe failed; the window is the base doubled this many times.
    level        integer     NOT NULL DEFAULT 0,
    updated_at   timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, namespace_id, activity),
    CONSTRAINT compensation_breakers_namespace_same_tenant_fk
        FOREIGN KEY (tenant_id, namespace_id) REFERENCES namespaces (tenant_id, id),
    CONSTRAINT compensation_breakers_failures_valid CHECK (failures >= 0),
    CONSTRAINT compensation_breakers_level_valid CHECK (level >= 0)
);

-- A compensation is claimed on its own index, so that the compensations an open breaker holds back
-- (an outage can leave a great many of them READY) are never walked by the claim of anything else,
-- and the forward index no longer carries them.
DROP INDEX steps_ready_idx;
CREATE INDEX steps_ready_idx ON steps (tenant_id, namespace_id, activity, ready_at)
    WHERE status = 'READY' AND kind <> 'COMPENSATION';
CREATE INDEX steps_ready_compensation_idx ON steps (tenant_id, namespace_id, activity, ready_at)
    WHERE status = 'READY' AND kind = 'COMPENSATION';

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT, UPDATE ON run_sagas TO sagat_app;
        GRANT SELECT, INSERT ON run_saga_entries, saga_plans, saga_plan_entries TO sagat_app;
        GRANT SELECT, INSERT, UPDATE, DELETE ON compensation_breakers TO sagat_app;
    END IF;
END
$grants$;
