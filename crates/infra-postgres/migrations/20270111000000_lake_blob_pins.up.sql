-- Feature 035, slice R (D035-144): the blobs a lake attempt may read under its lease.
--
-- The lease of an Iceberg write that takes its records from blobs names them in the document, and the lease transaction writes one row
-- here for each, keyed by the attempt it granted. `ReadBlob` of a lake attempt is allowed a blob only when a row names it for the
-- exact attempt and generation the request carries, so the document and the allow-list cannot disagree, and a lapsed, retried or
-- replaced attempt (another attempt, another generation) finds none. The lease writes the row from the catalog's own word on the
-- blob (committed output of this run) and not from the descriptor the workflow held.
--
-- Additive: an older engine never reads or writes the table, and leases no lake step. The rows are as immutable as the credential pins
-- they are modelled on, and as short-lived: they go with the run. The foreign key to the blob restricts its deletion for as long as a
-- pin exists, so no sweep or retention job removes bytes a live attempt was allowed to read (a committed blob is never swept; the
-- constraint keeps that true if it changes). The index serves that check.

CREATE TABLE step_lease_blobs (
    tenant_id uuid NOT NULL,
    namespace_id uuid NOT NULL,
    run_id uuid NOT NULL,
    node_id text NOT NULL CHECK (octet_length(node_id) BETWEEN 1 AND 64),
    attempt integer NOT NULL CHECK (attempt > 0),
    generation bigint NOT NULL CHECK (generation > 0),
    blob_id uuid NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (tenant_id, run_id, node_id, attempt, generation, blob_id),
    FOREIGN KEY (tenant_id, namespace_id, run_id) REFERENCES runs (tenant_id, namespace_id, id) ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, namespace_id, blob_id) REFERENCES blobs (tenant_id, namespace_id, id) ON DELETE RESTRICT
);
CREATE INDEX step_lease_blobs_blob_idx ON step_lease_blobs (tenant_id, namespace_id, blob_id);

CREATE FUNCTION sagat_lake_blob_pin_immutable() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, pg_temp AS $$ BEGIN
    RAISE EXCEPTION 'lake attempt blob pins are immutable' USING ERRCODE = 'check_violation';
END $$;
CREATE TRIGGER step_lease_blob_immutable BEFORE UPDATE ON step_lease_blobs
    FOR EACH ROW EXECUTE FUNCTION sagat_lake_blob_pin_immutable();
ALTER TABLE step_lease_blobs ENABLE ALWAYS TRIGGER step_lease_blob_immutable;

DO $grants$ BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT, DELETE ON step_lease_blobs TO sagat_app;
    END IF;
END $grants$;
