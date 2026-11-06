-- Start delay (feature 018, FR-018-023, T018-037): a run created with `startDelayMs` is `WAITING`, holds
-- its entry and releases it once, at this instant, through a fenced pass of the run shards. The column
-- mirrors the state of the run; it is set when the run is created and cleared by the release.
ALTER TABLE runs ADD COLUMN start_delayed_until timestamptz;

-- A delay is at most 366 days from the creation of the run. NOT VALID first, then validated: every row
-- that exists already satisfies it (the column is null).
ALTER TABLE runs ADD CONSTRAINT runs_start_delay_bounded CHECK (
    start_delayed_until IS NULL OR start_delayed_until <= created_at + interval '366 days 1 minute'
) NOT VALID;
ALTER TABLE runs VALIDATE CONSTRAINT runs_start_delay_bounded;

-- The runs whose entry is due for release, by shard. A run that is cancelled, times out or is paused
-- while it waits leaves the index (its status is no longer WAITING) and so is never released.
CREATE INDEX runs_start_delayed_idx ON runs (run_shard(id), start_delayed_until)
    WHERE start_delayed_until IS NOT NULL AND status = 'WAITING';

-- A delay is set once, in the transaction that creates the run (the row is written with the journal
-- entry `run.start_delayed`), and then only ever cleared, by the release. The trigger fires only for an
-- UPDATE that names the column, so ordinary updates of a run pay nothing.
CREATE FUNCTION runs_start_delay_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.start_delayed_until IS NOT NULL
       AND NEW.start_delayed_until IS NOT NULL
       AND NEW.start_delayed_until IS DISTINCT FROM OLD.start_delayed_until THEN
        RAISE EXCEPTION 'the start delay of a run is fixed once it is set and only cleared by its release'
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER runs_start_delay BEFORE UPDATE OF start_delayed_until ON runs
    FOR EACH ROW EXECUTE FUNCTION runs_start_delay_guard();
ALTER TABLE runs ENABLE ALWAYS TRIGGER runs_start_delay;
