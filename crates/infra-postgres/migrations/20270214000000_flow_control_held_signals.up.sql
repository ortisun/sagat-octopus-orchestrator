-- Feature 022, release 0.3.0 slice (T022-109; FR-022-018, FR-018-024; decision D022-087): the signal of a
-- Signal-with-Start whose new run queued for its start's permit or slot is held with the run, and the grant of the run's
-- start delivers it in its transaction as `signalRun` would. One row a run, the run's own: it is written by the run's
-- creation transaction and deleted by the grant that delivers it, or by the commit that ends the queued run (both
-- audited), and it goes with the run (`ON DELETE CASCADE`). The payload is a `payloads` row, named by its digest, as the
-- payload of every delivered signal is (charged to `tenant_content_bytes` by the trigger that charges every payload).
--
-- Additive: an older engine never writes or reads it (it refuses a Signal-with-Start whose run would queue, G022-010).
-- Lock order: the row is written and deleted under the run's own lock (slot 3R), as the rest of the run's rows.

CREATE TABLE flow_control_held_signals (
    tenant_id      uuid        NOT NULL,
    namespace_id   uuid        NOT NULL,
    run_id         uuid        NOT NULL,
    name           text        NOT NULL,
    payload_digest bytea       NOT NULL,
    actor          text        NOT NULL,
    trace_id       text,
    held_at        timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, run_id),
    CONSTRAINT flow_control_held_signals_run_fk FOREIGN KEY (tenant_id, namespace_id, run_id)
        REFERENCES runs (tenant_id, namespace_id, id) ON DELETE CASCADE,
    CONSTRAINT flow_control_held_signals_payload_fk FOREIGN KEY (tenant_id, payload_digest)
        REFERENCES payloads (tenant_id, digest),
    CONSTRAINT flow_control_held_signals_name_valid CHECK (char_length(name) BETWEEN 1 AND 128),
    CONSTRAINT flow_control_held_signals_digest_sha256 CHECK (octet_length(payload_digest) = 32),
    CONSTRAINT flow_control_held_signals_actor_valid CHECK (char_length(actor) BETWEEN 1 AND 255),
    CONSTRAINT flow_control_held_signals_trace_id_format CHECK (trace_id IS NULL OR trace_id ~ '^[0-9a-f]{32}$')
);

-- The signal's name is customer content (classified `content` in storage-classification.json): charged to the tenant
-- by the ALWAYS accounting triggers, as every accounted table is (`durable_quotas::manifest_covers_every_variable_size_
-- column_and_every_accounted_table_has_always_triggers`, integration_security_multitenancy). The payload is charged in
-- `payloads`, the actor and the trace are bounded system values.
CREATE TRIGGER flow_control_held_signals_quota_delta AFTER INSERT OR UPDATE OR DELETE ON flow_control_held_signals
    FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'octet_length_utf8', 'name');
ALTER TABLE flow_control_held_signals ENABLE ALWAYS TRIGGER flow_control_held_signals_quota_delta;
CREATE CONSTRAINT TRIGGER flow_control_held_signals_quota_flush AFTER INSERT OR UPDATE OR DELETE
    ON flow_control_held_signals DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE flow_control_held_signals ENABLE ALWAYS TRIGGER flow_control_held_signals_quota_flush;

REVOKE ALL PRIVILEGES ON flow_control_held_signals FROM PUBLIC;
DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT, DELETE ON flow_control_held_signals TO sagat_app;
    END IF;
END
$grants$;
