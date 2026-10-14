-- ADR-0027: all retained-content writers now have transactional accounting and quota-safe
-- terminal recovery. Deploy with the coordinated writer upgrade in the governance runbook.
DO $$ BEGIN
    UPDATE quota_accounting_control
       SET content_enforcement_enabled = true, log_enforcement_enabled = true
     WHERE singleton;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'retained usage control row is missing';
    END IF;
END $$;
