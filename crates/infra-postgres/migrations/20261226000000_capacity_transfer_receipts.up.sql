-- Dormant ChildRoot transfer receipts. No production caller or group creator is enabled.
CREATE TABLE capacity_group_transfer_receipts (
    tenant_id uuid NOT NULL,
    group_id uuid NOT NULL,
    operation_key uuid NOT NULL,
    kind text NOT NULL CHECK (kind IN ('VACATE','ACQUIRE')),
    fingerprint bytea NOT NULL CHECK (octet_length(fingerprint)=32),
    namespace_id uuid NOT NULL,
    member_id uuid NOT NULL,
    chain_id uuid NOT NULL,
    head_run_id uuid NOT NULL,
    expected_group_revision bigint NOT NULL,
    expected_group_epoch bigint NOT NULL,
    accepted_chain_revision bigint NOT NULL,
    accepted_run_revision bigint NOT NULL,
    resulting_revision bigint NOT NULL,
    resulting_epoch bigint NOT NULL,
    resulting_active_members bigint NOT NULL,
    resulting_possession text NOT NULL,
    resulting_holder_member_id uuid,
    transitioned_at timestamptz NOT NULL,
    audit_id uuid NOT NULL,
    outbox_id uuid NOT NULL,
    PRIMARY KEY (tenant_id,group_id,operation_key),
    UNIQUE (tenant_id,group_id,resulting_revision),
    FOREIGN KEY (tenant_id,group_id) REFERENCES capacity_groups(tenant_id,group_id),
    FOREIGN KEY (tenant_id,group_id,member_id,namespace_id,chain_id)
        REFERENCES capacity_group_members(tenant_id,group_id,member_id,namespace_id,chain_id),
    FOREIGN KEY (tenant_id,namespace_id,chain_id)
        REFERENCES execution_chains(tenant_id,namespace_id,chain_id),
    FOREIGN KEY (tenant_id,group_id,resulting_holder_member_id)
        REFERENCES capacity_group_members(tenant_id,group_id,member_id),
    CHECK (expected_group_revision >= 1 AND expected_group_epoch >= 1
        AND expected_group_epoch <= expected_group_revision
        AND accepted_chain_revision >= 0 AND accepted_run_revision >= 0
        AND resulting_revision - expected_group_revision=1
        AND resulting_epoch - expected_group_epoch=1 AND resulting_epoch <= resulting_revision
        AND resulting_active_members BETWEEN 1 AND 4294967295
        AND resulting_active_members <= resulting_revision),
    CHECK ((kind='VACATE' AND resulting_possession='VACANT' AND resulting_holder_member_id IS NULL)
        OR (kind='ACQUIRE' AND resulting_possession='HELD' AND resulting_holder_member_id IS NOT NULL AND resulting_holder_member_id=member_id))
);
-- ACQUIRE is a reserved receipt discriminator; this slice implements only native VACATE.
CREATE TRIGGER capacity_group_transfer_receipts_append_only
    BEFORE UPDATE OR DELETE ON capacity_group_transfer_receipts
    FOR EACH ROW EXECUTE FUNCTION execution_reject_mutation();
CREATE TRIGGER capacity_group_transfer_receipts_no_truncate
    BEFORE TRUNCATE ON capacity_group_transfer_receipts
    FOR EACH STATEMENT EXECUTE FUNCTION execution_reject_mutation();
ALTER TABLE capacity_group_transfer_receipts ENABLE ALWAYS TRIGGER capacity_group_transfer_receipts_append_only;
ALTER TABLE capacity_group_transfer_receipts ENABLE ALWAYS TRIGGER capacity_group_transfer_receipts_no_truncate;
CREATE INDEX steps_run_capacity_lease_idx ON steps(tenant_id,run_id)
    WHERE status IN ('LEASED','RUNNING');
REVOKE ALL PRIVILEGES ON capacity_group_transfer_receipts FROM PUBLIC;
DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname='sagat_app') THEN
        REVOKE ALL PRIVILEGES ON capacity_group_transfer_receipts FROM sagat_app;
        GRANT SELECT ON capacity_group_transfer_receipts TO sagat_app;
        GRANT INSERT (tenant_id,group_id,operation_key,kind,fingerprint,namespace_id,member_id,
            chain_id,head_run_id,expected_group_revision,expected_group_epoch,accepted_chain_revision,
            accepted_run_revision,resulting_revision,resulting_epoch,resulting_active_members,
            resulting_possession,resulting_holder_member_id,transitioned_at,audit_id,outbox_id)
            ON capacity_group_transfer_receipts TO sagat_app;
    END IF;
END $grants$;
