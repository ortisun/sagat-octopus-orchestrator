-- The runtime role stops deleting the pins of lake blobs: the retention job of the terminal run no longer releases them, and they stay until an
-- operator deletes them as the owner (as before 20270112000000). Nothing is lost: the pins are rows, and the grant is all this migration made.
DO $grants$ BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        REVOKE DELETE ON step_lease_blobs FROM sagat_app;
    END IF;
END $grants$;
