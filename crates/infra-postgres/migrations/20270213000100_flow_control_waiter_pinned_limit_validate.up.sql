-- Feature 022 (decision D022-078 item 1): the CHECKs 20270213000000 added `NOT VALID` are validated under `SHARE UPDATE
-- EXCLUSIVE`, so the queues are read and written while they are scanned (the rule of design 043, D043-145, D043-153).
ALTER TABLE concurrency_waiters VALIDATE CONSTRAINT concurrency_waiters_pinned_limit_valid;
ALTER TABLE rate_limit_waiters VALIDATE CONSTRAINT rate_limit_waiters_pinned_limit_valid;
