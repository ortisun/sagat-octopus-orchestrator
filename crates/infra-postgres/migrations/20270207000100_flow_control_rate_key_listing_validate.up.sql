-- Feature 022 (T022-078, D022-067): validates the CHECK `20270207000000_flow_control_rate_key_listing` added `NOT VALID`;
-- the scan takes `SHARE UPDATE EXCLUSIVE`, so the gates go on writing while it runs.
ALTER TABLE rate_limit_windows VALIDATE CONSTRAINT rate_limit_windows_oldest_wait_iff_waiters;
