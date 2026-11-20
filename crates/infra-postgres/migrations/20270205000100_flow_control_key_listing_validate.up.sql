-- Feature 022 (T022-026, D022-058): validates the CHECK `20270205000000_flow_control_key_listing` added `NOT VALID`; the
-- scan takes `SHARE UPDATE EXCLUSIVE`, so the gates go on writing while it runs.
ALTER TABLE concurrency_semaphores VALIDATE CONSTRAINT concurrency_semaphores_oldest_wait_iff_waiters;
