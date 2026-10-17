DO $revoke$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        REVOKE ALL ON secret_pools, worker_binding_pools FROM sagat_app;
    END IF;
END
$revoke$;

DROP TABLE worker_binding_pools;
DROP TABLE secret_pools;
