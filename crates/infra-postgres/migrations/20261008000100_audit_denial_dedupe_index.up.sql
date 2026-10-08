-- Supports the duplicate check of cross-tenant denial events (see PgIdentityStore::record_denial):
-- "has this actor already been recorded for this action on this resource recently?".
CREATE INDEX audit_records_denial_dedupe_idx
    ON audit_records (tenant_id, actor, action, resource_id, created_at DESC);
