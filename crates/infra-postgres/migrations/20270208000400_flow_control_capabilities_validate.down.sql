-- A validated CHECK cannot be made `NOT VALID` again, and nothing else was changed: the rollback of
-- `20270208000300_flow_control_capabilities` restores the old CHECKs.
SELECT 1;
