-- The platform's security audit (feature 009, decision D60; gap G041-028): a lookup that named another tenant's resource,
-- recorded with the caller's tenant, the actor, the identifier and the tenant that owns it. It used to be an `audit_records`
-- row in the caller's tenant, written only when the identifier existed elsewhere, which made the tenant's trail tell whether
-- an identifier exists in another tenant. This table is platform scope: no API reads it and no tenant can; the operator reads
-- it with SQL (and design 040's operator sink, its follow-up task T040-31). The caller's tenant now gets
-- `access.denied.not_found` in `audit_records`, the same for a missing and a foreign identifier.
--
-- Bounded by its one writer (`crates/infra-postgres/src/security_audit.rs`): the same caller, kind and identifier once in five
-- minutes; in five minutes at most 100 rows per caller's tenant, actor and owning tenant, and 1000 per caller's tenant and
-- owning tenant, so that a caller who fills its share with identifiers of a tenant of its own cannot hide a probe of
-- another tenant (the security review of 2026-11-27). Append-only, as `audit_records` is; rows are kept until the operator
-- exports and removes them (`docs/runbooks/security-governance.md`). Additive: an older build neither reads nor writes it.
CREATE TABLE security_audit_records (
    id               uuid        PRIMARY KEY,
    caller_tenant_id uuid        NOT NULL REFERENCES tenants (id),
    actor            text        NOT NULL,
    action           text        NOT NULL,
    resource_type    text        NOT NULL,
    resource_id      text        NOT NULL,
    -- the tenant that held the resource when the record was written; no foreign key, so that the evidence outlives it
    owner_tenant_id  uuid,
    trace_id         text,
    created_at       timestamptz NOT NULL,
    CONSTRAINT security_audit_records_actor_bounded CHECK (char_length(actor) BETWEEN 1 AND 255),
    CONSTRAINT security_audit_records_action_valid CHECK (action IN ('access.denied.cross_tenant')),
    CONSTRAINT security_audit_records_resource_type_bounded CHECK (char_length(resource_type) BETWEEN 1 AND 64),
    CONSTRAINT security_audit_records_resource_id_bounded CHECK (char_length(resource_id) BETWEEN 1 AND 255),
    CONSTRAINT security_audit_records_trace_id_format CHECK (trace_id IS NULL OR trace_id ~ '^[0-9a-f]{32}$')
);

-- The writer's bounds, per caller's tenant and owning tenant (a probe whose resource vanished before the write counts as
-- the nil tenant's), with and without the actor; and the operator's question "who probed, lately".
CREATE INDEX security_audit_records_caller_owner_actor_idx ON security_audit_records (
    caller_tenant_id, (coalesce(owner_tenant_id, '00000000-0000-0000-0000-000000000000'::uuid)), actor, created_at DESC);
CREATE INDEX security_audit_records_caller_owner_idx ON security_audit_records (
    caller_tenant_id, (coalesce(owner_tenant_id, '00000000-0000-0000-0000-000000000000'::uuid)), created_at DESC);
CREATE INDEX security_audit_records_created_idx ON security_audit_records (created_at DESC, id DESC);

CREATE FUNCTION security_audit_records_reject_mutation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'security_audit_records is append-only (% attempted)', TG_OP
        USING ERRCODE = 'restrict_violation';
END;
$$;

CREATE TRIGGER security_audit_records_append_only
    BEFORE UPDATE OR DELETE ON security_audit_records
    FOR EACH ROW EXECUTE FUNCTION security_audit_records_reject_mutation();
CREATE TRIGGER security_audit_records_no_truncate
    BEFORE TRUNCATE ON security_audit_records
    FOR EACH STATEMENT EXECUTE FUNCTION security_audit_records_reject_mutation();
-- Even in a session that sets `session_replication_role = replica`.
ALTER TABLE security_audit_records ENABLE ALWAYS TRIGGER security_audit_records_append_only;
ALTER TABLE security_audit_records ENABLE ALWAYS TRIGGER security_audit_records_no_truncate;

-- The runtime role appends and reads (its writer's bound reads the caller's recent rows); it neither changes nor removes.
DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT ON security_audit_records TO sagat_app;
    END IF;
END
$grants$;
