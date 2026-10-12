-- Roll back activation without discarding counters, reservations or diagnostic evidence.
-- Stop all writers before downgrading binaries; this switch does not make mixed writers safe.
DO $$ BEGIN
    UPDATE quota_accounting_control
       SET content_enforcement_enabled = false, log_enforcement_enabled = false
     WHERE singleton;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'retained usage control row is missing';
    END IF;
END $$;
