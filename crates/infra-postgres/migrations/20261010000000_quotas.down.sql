DO $revoke$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        REVOKE ALL ON quota_limits FROM sagat_app;
    END IF;
END
$revoke$;

DROP INDEX runs_active_idx;
DROP TABLE quota_limits;
