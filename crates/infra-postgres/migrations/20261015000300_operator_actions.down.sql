DROP TABLE worker_drains;
ALTER TABLE runs DROP CONSTRAINT runs_rerun_of_same_tenant_fk;
ALTER TABLE runs DROP COLUMN rerun_of;
