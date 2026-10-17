-- G141 (owner decision 2026-10-16): the hash of the lease whose holder reported the attempt
-- (CompleteStep or FailStep). A heartbeat that was already on its way when that report committed
-- proves with its token that it belongs to the lease that reported, and is answered without an
-- error. Cleared whenever the step is leased again, so a lost lease can never match it.
ALTER TABLE steps ADD COLUMN reported_lease_token_hash bytea;
