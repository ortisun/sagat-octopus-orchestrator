-- Feature 022, release 0.3.0 slice (decision D022-124; FR-022-052 as amended 2026-11-26; ADR-0089 decision 4 as amended
-- 2026-11-26): a waiter whose grant failed for a reason that does not clear on its own (a version this build cannot
-- read, a row a constraint refuses, an execution error) as the grant routine saw it. While a waiter has a row it is not
-- eligible (`eligible_gate_waiter!`): it reserves no room ahead of a younger waiter and holds back no new arrival, on a
-- key of limit 1 too; the pass's listing of a key's waiters still tries it first. One row per waiter: the waiter of a
-- run's start is node '' and attempt 0, as in the waiter tables, and a step waiting for both gates is one waiter.
--
-- Scheduler state, never the run's: no journal event writes or replays it and `verify_run` does not read it. It is
-- written in a statement of its own after the failed try's rollback, deleted by the first later try that decides the
-- waiter and by the grant that takes it, and goes with the run (`ON DELETE CASCADE`). `failure` is the closed class of
-- the failure, never its text.
--
-- Additive: an older engine neither reads nor writes it. Lock order: written outside every run and key lock (a leaf).

CREATE TABLE flow_control_grant_failures (
    tenant_id       uuid        NOT NULL,
    namespace_id    uuid        NOT NULL,
    run_id          uuid        NOT NULL,
    node            text        NOT NULL,
    attempt         integer     NOT NULL,
    failure         text        NOT NULL,
    first_failed_at timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, run_id, node, attempt),
    CONSTRAINT flow_control_grant_failures_run_fk FOREIGN KEY (tenant_id, namespace_id, run_id)
        REFERENCES runs (tenant_id, namespace_id, id) ON DELETE CASCADE,
    CONSTRAINT flow_control_grant_failures_node_shape CHECK (
        (node = '' AND attempt = 0)
        OR (node ~ '^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$' AND attempt >= 1)),
    CONSTRAINT flow_control_grant_failures_failure_valid CHECK (
        failure IN ('UNREADABLE_DEFINITION', 'REFUSED', 'EXECUTION', 'INTERNAL'))
);

REVOKE ALL PRIVILEGES ON flow_control_grant_failures FROM PUBLIC;
DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT, DELETE ON flow_control_grant_failures TO sagat_app;
    END IF;
END
$grants$;
