-- Nothing a schedule did is lost by this rollback: it drops the function that deletes old actions and
-- the bookkeeping of which schedule it visited last. Actions that were already purged stay purged.
DROP FUNCTION schedule_purge_actions(integer, integer);
DROP INDEX schedules_purge_idx;
ALTER TABLE schedules DROP COLUMN actions_purged_at;
