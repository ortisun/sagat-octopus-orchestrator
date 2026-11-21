-- Feature 035, D035-147 (G035-080): the pins of the blobs a lake attempt may read are released with the retention of the terminal run, by
-- the job that releases the credential pins (`secret_cleanup`, a day after the run ended). That job deletes them, so the runtime role may.
-- The grant of the table was SELECT and INSERT while nothing deleted a pin (20270111000000); it is DELETE as well now, and nothing else: a
-- pin is still never updated (the trigger refuses it), and only the retention of a terminal run deletes one.
--
-- Additive and cheap: a grant. The rollback revokes it.
DO $grants$ BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT DELETE ON step_lease_blobs TO sagat_app;
    END IF;
END $grants$;
