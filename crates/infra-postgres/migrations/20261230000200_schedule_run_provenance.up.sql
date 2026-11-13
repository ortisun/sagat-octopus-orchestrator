-- Feature 018: a run started by a schedule says which schedule, at which nominal time, by which kind
-- of action and under which revision. Additive: four nullable columns, null for every run that exists
-- and for every run that no schedule started. Older code neither reads nor writes them.
ALTER TABLE runs
    ADD COLUMN schedule_id          uuid,
    ADD COLUMN schedule_nominal_at  timestamptz,
    ADD COLUMN schedule_action_kind text,
    ADD COLUMN schedule_revision    integer;

-- NOT VALID first, then validated: the check never takes the long lock of a full scan under an
-- exclusive lock, and every row that exists already satisfies it (all four are null).
ALTER TABLE runs ADD CONSTRAINT runs_schedule_provenance_whole CHECK (
    (schedule_id IS NULL) = (schedule_nominal_at IS NULL)
    AND (schedule_id IS NULL) = (schedule_action_kind IS NULL)
    AND (schedule_id IS NULL) = (schedule_revision IS NULL)) NOT VALID;
ALTER TABLE runs VALIDATE CONSTRAINT runs_schedule_provenance_whole;
ALTER TABLE runs ADD CONSTRAINT runs_schedule_action_kind_valid CHECK (
    schedule_action_kind IS NULL OR schedule_action_kind IN ('SCHEDULED', 'TRIGGERED', 'BACKFILL')) NOT VALID;
ALTER TABLE runs VALIDATE CONSTRAINT runs_schedule_action_kind_valid;
ALTER TABLE runs ADD CONSTRAINT runs_schedule_revision_positive CHECK (
    schedule_revision IS NULL OR schedule_revision >= 1) NOT VALID;
ALTER TABLE runs VALIDATE CONSTRAINT runs_schedule_revision_positive;

-- The runs of a schedule, newest first.
CREATE INDEX runs_schedule_idx ON runs (tenant_id, schedule_id, created_at) WHERE schedule_id IS NOT NULL;

-- What a run says about the schedule that started it is fixed when it is created. The trigger fires
-- only for an UPDATE that names one of these columns, so ordinary updates of a run pay nothing.
CREATE FUNCTION runs_schedule_provenance_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.schedule_id IS DISTINCT FROM OLD.schedule_id
        OR NEW.schedule_nominal_at IS DISTINCT FROM OLD.schedule_nominal_at
        OR NEW.schedule_action_kind IS DISTINCT FROM OLD.schedule_action_kind
        OR NEW.schedule_revision IS DISTINCT FROM OLD.schedule_revision THEN
        RAISE EXCEPTION 'the schedule provenance of a run never changes'
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER runs_schedule_provenance
    BEFORE UPDATE OF schedule_id, schedule_nominal_at, schedule_action_kind, schedule_revision ON runs
    FOR EACH ROW EXECUTE FUNCTION runs_schedule_provenance_guard();
ALTER TABLE runs ENABLE ALWAYS TRIGGER runs_schedule_provenance;
