-- Nothing a batch job or an item holds is lost by this rollback: it drops the function that deletes them and the
-- index that function reads. Jobs and items that were already purged stay purged; the ones that are left are read as
-- before, and an engine without the function never purges them (the tables only grow again until it is applied).
DROP FUNCTION batch_jobs_purge(integer, integer);
DROP INDEX IF EXISTS batch_jobs_purge_idx;
