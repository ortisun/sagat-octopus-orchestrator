DO $revoke$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        REVOKE ALL ON worker_bindings FROM sagat_app;
    END IF;
END
$revoke$;

DROP TABLE worker_bindings;
