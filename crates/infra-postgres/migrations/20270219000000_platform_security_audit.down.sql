-- Rollback of `20270219000000_platform_security_audit`. The table is evidence of probes across the tenant boundary, kept by
-- no other table: it is refused while it holds a row. Export it first (`COPY security_audit_records TO ...`) and remove its
-- rows as the owner, with the triggers disabled, if the rollback must go on; the build this rolls back to writes these
-- probes into the caller's tenant's `audit_records` again.
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM security_audit_records) THEN
        RAISE EXCEPTION 'security_audit_records holds probes across the tenant boundary that no other table keeps; export and remove them before rolling back'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END
$guard$;
DROP TABLE security_audit_records;
DROP FUNCTION security_audit_records_reject_mutation();
