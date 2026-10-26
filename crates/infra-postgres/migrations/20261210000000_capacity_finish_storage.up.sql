-- Dormant ChildRoot capacity metadata, seeded only by isolated owner fixtures.
-- Trusted server boundary: explicit tenant predicates/composite FKs, no RLS/GUC claim.
-- No production creator, execution permit or tenant usage projection is introduced.
CREATE TABLE capacity_groups (
    tenant_id uuid NOT NULL,
    group_id uuid NOT NULL,
    revision bigint NOT NULL,
    epoch bigint NOT NULL,
    active_members bigint NOT NULL,
    possession text NOT NULL,
    holder_member_id uuid,
    holder_state text GENERATED ALWAYS AS
        (CASE WHEN holder_member_id IS NULL THEN NULL::text ELSE 'ACTIVE'::text END) STORED,
    PRIMARY KEY (tenant_id, group_id),
    CONSTRAINT capacity_groups_tenant_fk FOREIGN KEY (tenant_id) REFERENCES tenants(id),
    CONSTRAINT capacity_groups_counters CHECK (
        revision >= 1 AND epoch >= 1 AND epoch <= revision
        AND active_members BETWEEN 0 AND 4294967295 AND active_members <= revision),
    CONSTRAINT capacity_groups_possession_value CHECK (possession IN ('HELD','VACANT','RETIRED')),
    CONSTRAINT capacity_groups_possession_consistent CHECK (
        (possession='HELD' AND active_members > 0 AND holder_member_id IS NOT NULL) OR
        (possession='VACANT' AND active_members > 0 AND holder_member_id IS NULL) OR
        (possession='RETIRED' AND active_members=0 AND holder_member_id IS NULL))
);

CREATE TABLE capacity_group_members (
    tenant_id uuid NOT NULL,
    group_id uuid NOT NULL,
    member_id uuid NOT NULL,
    namespace_id uuid NOT NULL,
    member_kind text NOT NULL,
    chain_id uuid NOT NULL,
    state text NOT NULL,
    finished_at timestamptz,
    PRIMARY KEY (tenant_id, group_id, member_id),
    CONSTRAINT capacity_group_members_group_fk FOREIGN KEY (tenant_id,group_id)
        REFERENCES capacity_groups(tenant_id,group_id),
    -- Permanent in this dormant slice: chain/run history retention compatibility is a C0 gate.
    CONSTRAINT capacity_group_members_chain_fk FOREIGN KEY (tenant_id,namespace_id,chain_id)
        REFERENCES execution_chains(tenant_id,namespace_id,chain_id),
    CONSTRAINT capacity_group_members_chain_unique UNIQUE (tenant_id,group_id,namespace_id,chain_id),
    CONSTRAINT capacity_group_members_state_unique UNIQUE (tenant_id,group_id,member_id,state),
    CONSTRAINT capacity_group_members_binding_unique UNIQUE
        (tenant_id,group_id,member_id,namespace_id,chain_id),
    CONSTRAINT capacity_group_members_kind CHECK (member_kind='CHILD_ROOT'),
    CONSTRAINT capacity_group_members_state CHECK (
        (state='ACTIVE' AND finished_at IS NULL) OR
        (state='FINISHED' AND finished_at IS NOT NULL))
);

-- member.state is part of the referenced eligible UNIQUE key. ACTIVE→FINISHED has UPDATE
-- strength and performs the reverse FK check; every such writer must already hold G.
-- Deferral permits the matching header/member transition to settle at actual COMMIT.
ALTER TABLE capacity_groups ADD CONSTRAINT capacity_groups_holder_active_fk
    FOREIGN KEY (tenant_id,group_id,holder_member_id,holder_state)
    REFERENCES capacity_group_members(tenant_id,group_id,member_id,state)
    DEFERRABLE INITIALLY DEFERRED;

CREATE TABLE capacity_group_finish_receipts (
    tenant_id uuid NOT NULL,
    group_id uuid NOT NULL,
    operation_key uuid NOT NULL,
    fingerprint bytea NOT NULL,
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
    finished_at timestamptz NOT NULL,
    audit_id uuid NOT NULL,
    outbox_id uuid NOT NULL,
    PRIMARY KEY (tenant_id,group_id,operation_key),
    CONSTRAINT capacity_group_finish_receipts_result_revision_unique
        UNIQUE (tenant_id,group_id,resulting_revision),
    CONSTRAINT capacity_group_finish_receipts_member_unique UNIQUE (tenant_id,group_id,member_id),
    CONSTRAINT capacity_group_finish_receipts_group_fk FOREIGN KEY (tenant_id,group_id)
        REFERENCES capacity_groups(tenant_id,group_id),
    CONSTRAINT capacity_group_finish_receipts_member_binding_fk
        FOREIGN KEY (tenant_id,group_id,member_id,namespace_id,chain_id)
        REFERENCES capacity_group_members(tenant_id,group_id,member_id,namespace_id,chain_id),
    CONSTRAINT capacity_group_finish_receipts_chain_fk
        FOREIGN KEY (tenant_id,namespace_id,chain_id)
        REFERENCES execution_chains(tenant_id,namespace_id,chain_id),
    -- Historical holder identity survives its later ACTIVE→FINISHED transition.
    CONSTRAINT capacity_group_finish_receipts_result_holder_fk
        FOREIGN KEY (tenant_id,group_id,resulting_holder_member_id)
        REFERENCES capacity_group_members(tenant_id,group_id,member_id),
    -- No run/audit/outbox FK: receipt evidence cannot pin delivered outbox rows.
    CONSTRAINT capacity_group_finish_receipts_fingerprint CHECK (octet_length(fingerprint)=32),
    CONSTRAINT capacity_group_finish_receipts_counters CHECK (
        expected_group_revision >= 1 AND expected_group_epoch >= 1
        AND expected_group_epoch <= expected_group_revision
        AND accepted_chain_revision >= 0 AND accepted_run_revision >= 0
        AND resulting_revision >= 1 AND resulting_epoch >= 1
        AND resulting_revision - expected_group_revision=1
        AND resulting_epoch >= expected_group_epoch
        AND resulting_epoch - expected_group_epoch <= 1
        AND resulting_epoch <= resulting_revision
        AND resulting_active_members BETWEEN 0 AND 4294967295
        AND resulting_active_members <= resulting_revision),
    CONSTRAINT capacity_group_finish_receipts_result_possession CHECK (
        (resulting_possession='HELD' AND resulting_active_members > 0
            AND resulting_holder_member_id IS NOT NULL AND resulting_holder_member_id <> member_id
            AND resulting_epoch=expected_group_epoch) OR
        (resulting_possession='VACANT' AND resulting_active_members > 0
            AND resulting_holder_member_id IS NULL) OR
        (resulting_possession='RETIRED' AND resulting_active_members=0
            AND resulting_holder_member_id IS NULL AND resulting_epoch - expected_group_epoch=1))
);

CREATE FUNCTION capacity_groups_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'capacity group tombstones cannot be deleted' USING ERRCODE='23001';
    END IF;
    IF (OLD.tenant_id,OLD.group_id) IS DISTINCT FROM (NEW.tenant_id,NEW.group_id) THEN
        RAISE EXCEPTION 'capacity group identity is immutable' USING ERRCODE='23514';
    END IF;
    IF OLD.possession='RETIRED' AND OLD IS DISTINCT FROM NEW THEN
        RAISE EXCEPTION 'retired capacity groups are immutable' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END $$;
CREATE FUNCTION capacity_group_members_guard() RETURNS trigger LANGUAGE plpgsql AS $$
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
CREATE TRIGGER capacity_groups_guard BEFORE UPDATE OR DELETE ON capacity_groups
    FOR EACH ROW EXECUTE FUNCTION capacity_groups_guard();
CREATE TRIGGER capacity_groups_no_truncate BEFORE TRUNCATE ON capacity_groups
    FOR EACH STATEMENT EXECUTE FUNCTION execution_reject_mutation();
CREATE TRIGGER capacity_group_members_guard BEFORE UPDATE OR DELETE ON capacity_group_members
    FOR EACH ROW EXECUTE FUNCTION capacity_group_members_guard();
CREATE TRIGGER capacity_group_members_no_truncate BEFORE TRUNCATE ON capacity_group_members
    FOR EACH STATEMENT EXECUTE FUNCTION execution_reject_mutation();
CREATE TRIGGER capacity_group_finish_receipts_append_only BEFORE UPDATE OR DELETE ON capacity_group_finish_receipts
    FOR EACH ROW EXECUTE FUNCTION execution_reject_mutation();
CREATE TRIGGER capacity_group_finish_receipts_no_truncate BEFORE TRUNCATE ON capacity_group_finish_receipts
    FOR EACH STATEMENT EXECUTE FUNCTION execution_reject_mutation();
ALTER TABLE capacity_groups ENABLE ALWAYS TRIGGER capacity_groups_guard;
ALTER TABLE capacity_groups ENABLE ALWAYS TRIGGER capacity_groups_no_truncate;
ALTER TABLE capacity_group_members ENABLE ALWAYS TRIGGER capacity_group_members_guard;
ALTER TABLE capacity_group_members ENABLE ALWAYS TRIGGER capacity_group_members_no_truncate;
ALTER TABLE capacity_group_finish_receipts ENABLE ALWAYS TRIGGER capacity_group_finish_receipts_append_only;
ALTER TABLE capacity_group_finish_receipts ENABLE ALWAYS TRIGGER capacity_group_finish_receipts_no_truncate;

REVOKE ALL PRIVILEGES ON capacity_groups,capacity_group_members,capacity_group_finish_receipts FROM PUBLIC;
DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname='sagat_app') THEN
        -- Clear any direct default table grants before giving the dormant operation its columns.
        REVOKE ALL PRIVILEGES ON capacity_groups,capacity_group_members,capacity_group_finish_receipts FROM sagat_app;
        GRANT SELECT ON capacity_groups,capacity_group_members,capacity_group_finish_receipts TO sagat_app;
        GRANT UPDATE (revision,epoch,active_members,possession,holder_member_id) ON capacity_groups TO sagat_app;
        GRANT UPDATE (state,finished_at) ON capacity_group_members TO sagat_app;
        GRANT INSERT ON capacity_group_finish_receipts TO sagat_app;
    END IF;
END $grants$;
