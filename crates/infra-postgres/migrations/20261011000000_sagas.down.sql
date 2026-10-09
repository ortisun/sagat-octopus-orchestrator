DO $revoke$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        REVOKE ALL ON run_sagas, run_saga_entries, saga_plans, saga_plan_entries, compensation_breakers
            FROM sagat_app;
    END IF;
END
$revoke$;

DROP TABLE compensation_breakers;

DROP INDEX steps_ready_compensation_idx;
DROP INDEX steps_ready_idx;
CREATE INDEX steps_ready_idx ON steps (tenant_id, namespace_id, activity, ready_at) WHERE status = 'READY';

DROP TRIGGER saga_plan_entries_no_truncate ON saga_plan_entries;
DROP TRIGGER saga_plan_entries_immutable ON saga_plan_entries;
DROP TRIGGER saga_plans_no_truncate ON saga_plans;
DROP TRIGGER saga_plans_immutable ON saga_plans;
DROP TABLE saga_plan_entries;
DROP TABLE saga_plans;
DROP TABLE run_saga_entries;
DROP TABLE run_sagas;

DROP INDEX runs_manual_review_idx;
ALTER TABLE runs DROP CONSTRAINT runs_finished_consistent;
ALTER TABLE runs ADD CONSTRAINT runs_finished_consistent CHECK ((finished_at IS NULL) = (status NOT IN
    ('SUCCEEDED', 'FAILED', 'CANCELLED', 'TIMED_OUT')));
ALTER TABLE runs DROP CONSTRAINT runs_stop_reason_valid;
ALTER TABLE runs ADD CONSTRAINT runs_stop_reason_valid CHECK (stop_reason IS NULL OR stop_reason IN
    ('CANCELLED', 'TIMED_OUT', 'FAILED'));
ALTER TABLE runs DROP CONSTRAINT runs_status_valid;
ALTER TABLE runs ADD CONSTRAINT runs_status_valid CHECK (status IN
    ('RUNNING', 'WAITING', 'PAUSED', 'CANCELLING', 'SUCCEEDED', 'FAILED', 'CANCELLED', 'TIMED_OUT'));

ALTER TABLE steps DROP CONSTRAINT steps_operator_has_a_reason;
ALTER TABLE steps DROP CONSTRAINT steps_waiting_kind_valid;
ALTER TABLE steps ADD CONSTRAINT steps_waiting_kind_valid CHECK (waiting_kind IS NULL OR waiting_kind IN
    ('TIMER', 'EVENT', 'BRANCHES', 'CHILD'));
ALTER TABLE steps DROP CONSTRAINT steps_status_valid;
ALTER TABLE steps ADD CONSTRAINT steps_status_valid CHECK (status IN
    ('PENDING', 'READY', 'LEASED', 'RUNNING', 'WAITING', 'RETRY_WAIT', 'SUCCEEDED', 'FAILED', 'CANCELLED'));
ALTER TABLE steps DROP CONSTRAINT steps_kind_matches_node;
ALTER TABLE steps DROP CONSTRAINT steps_kind_valid;
ALTER TABLE steps DROP COLUMN kind;
