-- Feature 009 (security review): revoking a worker binding releases the attempts leased by the
-- sessions it closes (`UPDATE steps ... WHERE leased_by = ANY(..)`) while it holds the binding lock
-- and the session rows, so every `register` and every report of that identity waits for it. With no
-- index on `leased_by` it read every lease in the system. Only live leases are indexed, so the index
-- is as small as the work in flight.

CREATE INDEX steps_leased_by_idx ON steps (tenant_id, leased_by) WHERE status IN ('LEASED', 'RUNNING');
