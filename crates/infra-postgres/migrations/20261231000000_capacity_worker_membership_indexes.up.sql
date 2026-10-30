-- Bounded cross-group worker lookup; do not assume global chain membership uniqueness.
-- Includes retained FINISHED facts so no-active cannot accidentally become legacy/no-history.
CREATE INDEX capacity_members_child_chain_worker_idx
    ON capacity_group_members (tenant_id,namespace_id,chain_id,state,group_id,member_id)
    WHERE member_kind='CHILD_ROOT';
-- Unsupported scoped registrations fail closed until their own coordinator is accepted.
CREATE INDEX capacity_members_scoped_run_worker_idx
    ON capacity_group_members (tenant_id,namespace_id,original_run_id)
    WHERE member_kind='SCOPED_PATH';
