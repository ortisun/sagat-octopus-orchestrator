-- SQLx retains this lock/check/drop sequence in one migration transaction.
LOCK TABLE capacity_groups, capacity_group_members, capacity_group_finish_receipts IN ACCESS EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM capacity_groups)
        OR EXISTS (SELECT 1 FROM capacity_group_members)
        OR EXISTS (SELECT 1 FROM capacity_group_finish_receipts) THEN
        RAISE EXCEPTION 'capacity finish storage is retained; keep a compatible reader until capacity tombstones and receipts have a compatible retention path'
            USING ERRCODE='55000';
    END IF;
END $guard$;
DO $revoke$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname='sagat_app') THEN
        REVOKE UPDATE (revision,epoch,active_members,possession,holder_member_id) ON capacity_groups FROM sagat_app;
        REVOKE UPDATE (state,finished_at) ON capacity_group_members FROM sagat_app;
        REVOKE ALL PRIVILEGES ON capacity_group_finish_receipts,capacity_group_members,capacity_groups FROM sagat_app;
    END IF;
END $revoke$;
DROP TABLE capacity_group_finish_receipts;
ALTER TABLE capacity_groups DROP CONSTRAINT capacity_groups_holder_active_fk;
DROP TABLE capacity_group_members;
DROP TABLE capacity_groups;
DROP FUNCTION capacity_group_members_guard();
DROP FUNCTION capacity_groups_guard();
