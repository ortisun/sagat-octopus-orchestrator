-- Read-only candidate discovery consumes a bounded keyset before applying routing filters.
-- No new authority, metadata writer or production group creation is enabled.
CREATE INDEX capacity_members_active_discovery_idx
    ON capacity_group_members(tenant_id,group_id,member_id)
    INCLUDE(namespace_id,chain_id)
    WHERE state='ACTIVE' AND member_kind='CHILD_ROOT';
