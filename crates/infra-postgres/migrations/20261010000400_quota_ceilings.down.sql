DO $revoke$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        REVOKE ALL ON quota_ceilings FROM sagat_app;
    END IF;
END
$revoke$;

DROP TABLE quota_ceilings;
