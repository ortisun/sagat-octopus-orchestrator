DO $revoke$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        REVOKE ALL ON payload_reveal_grants FROM sagat_app;
    END IF;
END
$revoke$;

DROP TABLE IF EXISTS payload_reveal_grants;
