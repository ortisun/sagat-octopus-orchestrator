-- Feature 022 (T022-064; D022-052, D022-054; reviews of 2026-11-20, creators B1/M1): the release of a start delay of a run
-- whose definition gates its start decides the gate at that instant (FR-022-018). When a ceiling of the key refuses it (a
-- full waiter queue, `queued_runs`, the live keys) the release rolls back and the run stays delayed; this row then holds
-- the run back from the release pass until `retry_at`, with a backoff (5 s doubling to 5 min, 018's plan row 4), so that
-- deferred runs neither take the pass's whole batch on every tick nor hide the delayed starts behind them. One row per
-- run, written in a transaction of its own after the refused release; it is never read once the run is released (its
-- `start_delayed_until` is then null), and goes with the run (ON DELETE CASCADE).
--
-- Additive: a new table; no existing row or reader changes. `sagat_app` reads and writes it.
CREATE TABLE start_release_deferrals (
    tenant_id         uuid        NOT NULL,
    namespace_id      uuid        NOT NULL,
    run_id            uuid        NOT NULL,
    deferrals         integer     NOT NULL,
    first_deferred_at timestamptz NOT NULL,
    retry_at          timestamptz NOT NULL,
    reason            text        NOT NULL,
    PRIMARY KEY (tenant_id, run_id),
    CONSTRAINT start_release_deferrals_run_fk FOREIGN KEY (tenant_id, namespace_id, run_id)
        REFERENCES runs (tenant_id, namespace_id, id) ON DELETE CASCADE,
    CONSTRAINT start_release_deferrals_count CHECK (deferrals >= 1),
    CONSTRAINT start_release_deferrals_order CHECK (retry_at >= first_deferred_at),
    CONSTRAINT start_release_deferrals_reason CHECK (
        reason IN ('CONCURRENCY_QUEUE_FULL', 'RATE_LIMIT_QUEUE_FULL', 'FLOW_CONTROL_LIMIT'))
);

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT, UPDATE ON start_release_deferrals TO sagat_app;
    END IF;
END
$grants$;
