-- Dormant fixed OrdinaryLeafV1 scoped-path capacity binding metadata.
-- This migration adds no scoped writer, release INSERT privilege or production caller.
-- Acquire every affected relation before DDL, with the referenced run relation first.
-- This local migration order is not a global deadlock guarantee; focused PostgreSQL
-- lock and reverse-FK behavior remains executable evidence for the delivery.
LOCK TABLE runs IN ACCESS EXCLUSIVE MODE;
LOCK TABLE capacity_groups IN ACCESS EXCLUSIVE MODE;
LOCK TABLE capacity_group_members IN ACCESS EXCLUSIVE MODE;
LOCK TABLE capacity_group_finish_receipts IN ACCESS EXCLUSIVE MODE;

ALTER TABLE capacity_group_members
    ADD COLUMN original_run_id uuid,
    ADD COLUMN root_owner_node text,
    ADD COLUMN iteration_index integer,
    ALTER COLUMN chain_id DROP NOT NULL;

ALTER TABLE capacity_group_members
    DROP CONSTRAINT capacity_group_members_kind,
    ADD CONSTRAINT capacity_group_members_kind CHECK (
        (member_kind='CHILD_ROOT'
            AND chain_id IS NOT NULL
            AND original_run_id IS NULL
            AND root_owner_node IS NULL
            AND iteration_index IS NULL)
        OR
        (member_kind='SCOPED_PATH'
            AND chain_id IS NULL
            AND original_run_id IS NOT NULL
            AND root_owner_node IS NOT NULL
            AND root_owner_node ~ '^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$'
            AND iteration_index IS NOT NULL
            AND iteration_index BETWEEN 0 AND 9999)
    ),
    ADD CONSTRAINT capacity_group_members_scoped_path_unique UNIQUE
        (tenant_id,group_id,namespace_id,original_run_id,root_owner_node,iteration_index),
    ADD CONSTRAINT capacity_group_members_scoped_binding_unique UNIQUE
        (tenant_id,group_id,member_id,namespace_id,original_run_id,root_owner_node,iteration_index),
    ADD CONSTRAINT capacity_group_members_original_run_fk
        FOREIGN KEY (tenant_id,namespace_id,original_run_id)
        REFERENCES runs(tenant_id,namespace_id,id) MATCH SIMPLE;

-- Extend the immutable member identity with the complete discriminated binding.
CREATE OR REPLACE FUNCTION capacity_group_members_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'capacity member tombstones cannot be deleted' USING ERRCODE='23001';
    END IF;
    IF (OLD.tenant_id,OLD.group_id,OLD.member_id,OLD.namespace_id,OLD.member_kind,OLD.chain_id,
            OLD.original_run_id,OLD.root_owner_node,OLD.iteration_index)
        IS DISTINCT FROM
        (NEW.tenant_id,NEW.group_id,NEW.member_id,NEW.namespace_id,NEW.member_kind,NEW.chain_id,
            NEW.original_run_id,NEW.root_owner_node,NEW.iteration_index) THEN
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
    ADD COLUMN member_kind text NOT NULL DEFAULT 'CHILD_ROOT',
    ADD COLUMN original_run_id uuid,
    ADD COLUMN root_owner_node text,
    ADD COLUMN iteration_index integer,
    ALTER COLUMN chain_id DROP NOT NULL,
    ALTER COLUMN head_run_id DROP NOT NULL,
    ALTER COLUMN accepted_chain_revision DROP NOT NULL;

-- The discriminator is total: exactly one complete ChildRoot or fixed scoped-path
-- identity is present. Existing common counter and possession checks remain in force.
ALTER TABLE capacity_group_finish_receipts
    ADD CONSTRAINT capacity_group_finish_receipts_kind CHECK (
        (member_kind='CHILD_ROOT'
            AND chain_id IS NOT NULL
            AND head_run_id IS NOT NULL
            AND accepted_chain_revision IS NOT NULL
            AND original_run_id IS NULL
            AND root_owner_node IS NULL
            AND iteration_index IS NULL)
        OR
        (member_kind='SCOPED_PATH'
            AND chain_id IS NULL
            AND head_run_id IS NULL
            AND accepted_chain_revision IS NULL
            AND original_run_id IS NOT NULL
            AND root_owner_node IS NOT NULL
            AND root_owner_node ~ '^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$'
            AND iteration_index IS NOT NULL
            AND iteration_index BETWEEN 0 AND 9999
            AND accepted_run_revision >= 1)
    ),
    ADD CONSTRAINT capacity_group_finish_receipts_scoped_binding_fk
        FOREIGN KEY
            (tenant_id,group_id,member_id,namespace_id,original_run_id,root_owner_node,iteration_index)
        REFERENCES capacity_group_members
            (tenant_id,group_id,member_id,namespace_id,original_run_id,root_owner_node,iteration_index)
        MATCH SIMPLE;

-- Preserve the native ChildRoot writer's exact 20-column INSERT and keep every
-- discriminator/scoped column unavailable to sagat_app.
DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname='sagat_app') THEN
        REVOKE INSERT ON capacity_group_finish_receipts FROM sagat_app;
        GRANT INSERT (
            tenant_id,group_id,operation_key,fingerprint,namespace_id,member_id,chain_id,
            head_run_id,expected_group_revision,expected_group_epoch,accepted_chain_revision,
            accepted_run_revision,resulting_revision,resulting_epoch,resulting_active_members,
            resulting_possession,resulting_holder_member_id,finished_at,audit_id,outbox_id
        ) ON capacity_group_finish_receipts TO sagat_app;
    END IF;
END $grants$;
