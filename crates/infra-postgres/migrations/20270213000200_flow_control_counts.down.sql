-- The counters are derived from the waiter rows, which stay: rolling back loses nothing. The ceiling's check of an older
-- build counts the waiter rows again.
DROP TRIGGER IF EXISTS concurrency_waiters_queued_runs ON concurrency_waiters;
DROP TRIGGER IF EXISTS concurrency_waiters_queued_runs_out ON concurrency_waiters;
DROP TRIGGER IF EXISTS rate_limit_waiters_queued_runs ON rate_limit_waiters;
DROP TRIGGER IF EXISTS rate_limit_waiters_queued_runs_out ON rate_limit_waiters;
DROP FUNCTION IF EXISTS flow_control_counts_track();
DROP TABLE IF EXISTS flow_control_counts;
