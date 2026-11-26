-- Rolls back 20270216000000_flow_control_grant_failures (decision D022-124). Rollback is an operator/quiesced-service
-- operation for the earlier build, which neither reads nor writes the table (this build's gate statements read it). A row
-- only keeps a waiter whose grant keeps failing out of the waiters counted ahead; nothing an older build reads is in it,
-- and that build counts the waiter ahead as it always did. Dropping it loses no run data.
DROP TABLE flow_control_grant_failures;
