-- Rollback of `20270211000300_instance_heartbeat_reporting_change`: the function goes. Nothing is stored. Roll the binaries back
-- first: a process of this build beats with it, and its beats fail once it is gone (its gate closes after the liveness window,
-- ADR-0082 decision 5).
DROP FUNCTION instance_heartbeat_reporting_change(uuid, bytea);
