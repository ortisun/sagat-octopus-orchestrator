-- Feature 041, task T041-36: the carry's functions are dropped. No data is lost: they hold none, and the lines and
-- streams a carry wrote stay, read by the same routes as a batch's.
DROP FUNCTION sagat_step_log_carry(uuid, uuid, uuid, uuid, text, text[], integer[], integer, bigint, bigint[], text[],
                                   timestamptz[], text[], text[], text[], boolean[], integer[], bigint, integer, text);
DROP FUNCTION sagat_step_log_carry_write(uuid, uuid, uuid, uuid, text, text[], integer[], integer, bigint, bigint[],
                                         text[], timestamptz[], text[], text[], text[], boolean[], integer[], bigint,
                                         text, bigint, bigint, integer, bigint, boolean);
DROP FUNCTION sagat_step_log_carry_plan(bigint, bigint, bigint, bigint, text, text, bigint, bigint, bigint, bigint,
                                        bigint, bigint, bigint, bigint[], integer[], boolean[], bigint, boolean, text);
DROP FUNCTION sagat_step_log_policies(uuid, uuid);
DROP FUNCTION sagat_step_log_stream_lock_key(uuid, uuid, uuid, integer, bigint);
DROP FUNCTION sagat_step_log_policy_fence_keys(uuid, uuid);
