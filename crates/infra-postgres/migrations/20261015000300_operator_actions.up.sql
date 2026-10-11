-- Observability (feature 008; FR-008-004): what an operator may do to a run and to the workers that
-- serve a namespace.

-- A run started again from an ended one (RERUN_AS_NEW): the link never changes. The new run is an
-- ordinary run (its own journal, its own trace, its own quota); this says only where it came from.
ALTER TABLE runs ADD COLUMN rerun_of uuid;
ALTER TABLE runs ADD CONSTRAINT runs_rerun_of_same_tenant_fk
    FOREIGN KEY (tenant_id, rerun_of) REFERENCES runs (tenant_id, id);

-- A worker identity that is being drained: its sessions in the namespace are given no new work, and
-- the work they hold goes on until it ends. The mark is on the identity and not on a session so that
-- a worker that registers again (a restart) is still drained until someone says otherwise.
CREATE TABLE worker_drains (
    tenant_id    uuid        NOT NULL,
    namespace_id uuid        NOT NULL,
    subject      text        NOT NULL,
    reason       text        NOT NULL,
    drained_by   text        NOT NULL,
    drained_at   timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (tenant_id, namespace_id, subject),
    CONSTRAINT worker_drains_namespace_fk FOREIGN KEY (tenant_id, namespace_id)
        REFERENCES namespaces (tenant_id, id),
    CONSTRAINT worker_drains_reason_length CHECK (char_length(reason) BETWEEN 8 AND 2000),
    CONSTRAINT worker_drains_subject_length CHECK (char_length(subject) BETWEEN 1 AND 255)
);

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON worker_drains TO sagat_app;
    END IF;
END
$grants$;
