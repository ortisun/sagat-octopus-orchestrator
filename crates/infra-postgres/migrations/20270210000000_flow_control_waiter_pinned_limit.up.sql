-- Feature 022, task T022-080 (decisions D022-076 item 7 and D022-078 item 1): a waiter is bounded at every decision by
-- the lower of its declaration and the namespace's current ceiling, so a raised ceiling applies to a queued waiter as a
-- lowered one does. The waiter rows of both gates gain `pinned_limit`, the declaration the run's version pins, before any
-- ceiling; `declared_limit` keeps its meaning (the filed limit, the declaration clamped at admission, as the row's
-- `*.queued` event records it).
--
-- Additive: the column is nullable and has no default, so the statement rewrites nothing. A row written before it has
-- `NULL` and is bounded by its filed limit; an older binary of the rollout neither writes nor reads it, and keeps its
-- behaviour (a raised ceiling unapplied to the waiters it files). The CHECK is added `NOT VALID`, so no scan runs under
-- the `ACCESS EXCLUSIVE` lock; `20270210000100` validates it under `SHARE UPDATE EXCLUSIVE` (the rule of design 043,
-- D043-145, D043-153). The table grants of `sagat_app` (`SELECT, INSERT, DELETE`) cover the column; a fixed-size column
-- has no storage class.
--
-- ROLLOUT ORDER: apply the migration before the engine that writes the column: against a schema without it the insert of
-- a waiter fails, and the deciding transaction rolls back (a retryable failure, never a half-written queue).

ALTER TABLE concurrency_waiters ADD COLUMN pinned_limit integer;
ALTER TABLE rate_limit_waiters ADD COLUMN pinned_limit integer;
ALTER TABLE concurrency_waiters ADD CONSTRAINT concurrency_waiters_pinned_limit_valid
    CHECK (pinned_limit IS NULL OR pinned_limit BETWEEN declared_limit AND 10000) NOT VALID;
ALTER TABLE rate_limit_waiters ADD CONSTRAINT rate_limit_waiters_pinned_limit_valid
    CHECK (pinned_limit IS NULL OR pinned_limit BETWEEN declared_limit AND 10000) NOT VALID;
