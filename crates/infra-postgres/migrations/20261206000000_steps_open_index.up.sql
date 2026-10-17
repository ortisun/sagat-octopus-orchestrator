-- G461: a collection or loop transition ends by deciding whether the run is still working
-- (`finish_version`: "is any step of this run PENDING, READY, LEASED, RUNNING or RETRY_WAIT, and if
-- not, is any WAITING"). Both are EXISTS probes over the run's steps. The steps that answer them are
-- the few that are open, but without an index on status the probe walks the run's steps in primary
-- key order until it meets one, and the open ones are the newest: a run with thousands of finished
-- iterations read all of them on every transition. Partial indexes hold only the open steps, so the
-- probe is one index entry whatever the progress of the run. Rows enter and leave an index only when
-- their status crosses the predicate.
CREATE INDEX steps_run_open_idx ON steps (tenant_id, run_id)
    WHERE status IN ('PENDING', 'READY', 'LEASED', 'RUNNING', 'RETRY_WAIT');
CREATE INDEX steps_run_waiting_idx ON steps (tenant_id, run_id)
    WHERE status = 'WAITING';
