-- Reverse only the dormant scoped-path binding extension. Retained scoped members
-- or receipts, including finished tombstones, make the downgrade unsafe.
-- Acquire every affected relation before inspection or DDL, with the referenced
-- run relation first. This local order is not a global deadlock guarantee.
LOCK TABLE runs IN ACCESS EXCLUSIVE MODE;
LOCK TABLE capacity_groups IN ACCESS EXCLUSIVE MODE;
LOCK TABLE capacity_group_members IN ACCESS EXCLUSIVE MODE;
LOCK TABLE capacity_group_finish_receipts IN ACCESS EXCLUSIVE MODE;

DO $guard$
BEGIN
    IF EXISTS (
        SELECT 1 FROM capacity_group_members WHERE member_kind='SCOPED_PATH'
    ) OR EXISTS (
        SELECT 1 FROM capacity_group_finish_receipts WHERE member_kind='SCOPED_PATH'
    ) THEN
        RAISE EXCEPTION 'scoped capacity finish bindings are retained; downgrade requires ChildRoot-only members and receipts'
            USING ERRCODE='55000';
    END IF;
END $guard$;

-- Restore the exact legacy member guard before removing the scoped identity columns.
CREATE OR REPLACE FUNCTION capacity_group_members_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'capacity member tombstones cannot be deleted' USING ERRCODE='23001';
    END IF;
    IF (OLD.tenant_id,OLD.group_id,OLD.member_id,OLD.namespace_id,OLD.member_kind,OLD.chain_id)
        IS DISTINCT FROM
        (NEW.tenant_id,NEW.group_id,NEW.member_id,NEW.namespace_id,NEW.member_kind,NEW.chain_id) THEN
        RAISE EXCEPTION 'capacity member identity is immutable' USING ERRCODE='23514';
    END IF;
    IF OLD.state='FINISHED' AND OLD IS DISTINCT FROM NEW THEN
        RAISE EXCEPTION 'finished capacity members are immutable' USING ERRCODE='23514';
    END IF;
    IF OLD.state IS DISTINCT FROM NEW.state AND NOT (OLD.state='ACTIVE' AND NEW.state='FINISHED') THEN
        RAISE EXCEPTION 'capacity member state is irreversible' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END $$;

ALTER TABLE capacity_group_finish_receipts
    DROP CONSTRAINT capacity_group_finish_receipts_scoped_binding_fk,
    DROP CONSTRAINT capacity_group_finish_receipts_kind,
    ALTER COLUMN chain_id SET NOT NULL,
    ALTER COLUMN head_run_id SET NOT NULL,
    ALTER COLUMN accepted_chain_revision SET NOT NULL,
    DROP COLUMN member_kind,
    DROP COLUMN original_run_id,
    DROP COLUMN root_owner_node,
    DROP COLUMN iteration_index;

ALTER TABLE capacity_group_members
    DROP CONSTRAINT capacity_group_members_original_run_fk,
    DROP CONSTRAINT capacity_group_members_scoped_binding_unique,
    DROP CONSTRAINT capacity_group_members_scoped_path_unique,
    DROP CONSTRAINT capacity_group_members_kind,
    ALTER COLUMN chain_id SET NOT NULL,
    DROP COLUMN original_run_id,
    DROP COLUMN root_owner_node,
    DROP COLUMN iteration_index,
    ADD CONSTRAINT capacity_group_members_kind CHECK (member_kind='CHILD_ROOT');

-- Remove the narrow column grant and restore the legacy table-level receipt INSERT.
DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname='sagat_app') THEN
        REVOKE INSERT (
            tenant_id,group_id,operation_key,fingerprint,namespace_id,member_id,chain_id,
            head_run_id,expected_group_revision,expected_group_epoch,accepted_chain_revision,
            accepted_run_revision,resulting_revision,resulting_epoch,resulting_active_members,
            resulting_possession,resulting_holder_member_id,finished_at,audit_id,outbox_id
        ) ON capacity_group_finish_receipts FROM sagat_app;
        GRANT INSERT ON capacity_group_finish_receipts TO sagat_app;
    END IF;
END $grants$;
