-- Observability (feature 008; FR-008-001, FR-008-002): the telemetry projection and the leases of
-- the engine's singleton loops.
--
-- The spans of a run and its attempts, and the measurements of how long they took, are derived from
-- the journal after the fact (ADR-0017). The write path does one thing for that, in the
-- transaction that changes the run: it notes how far the journal has got. The projector reads these
-- notes, derives, exports, and notes how far it has got. A run that is behind has a row with
-- `applied_seq < head_seq`; the age of the oldest such row is the lag, exactly.

CREATE TABLE telemetry_pending (
    tenant_id    uuid        NOT NULL,
    run_id       uuid        NOT NULL,
    -- The last journal sequence of the run, as the write path left it.
    head_seq     bigint      NOT NULL,
    -- The last sequence whose spans the projector has taken.
    applied_seq  bigint      NOT NULL DEFAULT 0,
    -- Since when the run has had changes that are not projected; null while it is caught up.
    behind_since timestamptz,
    PRIMARY KEY (tenant_id, run_id),
    CONSTRAINT telemetry_pending_seq_valid CHECK (applied_seq >= 0 AND head_seq >= applied_seq),
    CONSTRAINT telemetry_pending_behind_consistent CHECK ((behind_since IS NULL) = (applied_seq >= head_seq)),
    CONSTRAINT telemetry_pending_run_fk FOREIGN KEY (tenant_id, run_id)
        REFERENCES runs (tenant_id, id) ON DELETE CASCADE
);

-- What the projector asks for: the runs that are behind, the oldest first. Only those are indexed.
CREATE INDEX telemetry_pending_behind_idx ON telemetry_pending (behind_since)
    WHERE applied_seq < head_seq;

-- A singleton loop of the engine (the projector, the monitor) runs in one process at a time. The
-- lease is a row with an expiry on the database's clock: a process that dies is replaced when its
-- lease runs out, with no session-level lock to leak through a connection pool.
CREATE TABLE engine_leases (
    name        text        PRIMARY KEY,
    holder      text        NOT NULL,
    expires_at  timestamptz NOT NULL
);

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON telemetry_pending, engine_leases TO sagat_app;
    END IF;
END
$grants$;
