-- A validated CHECK cannot be made `NOT VALID` again, and nothing else was changed: the rollback of
-- 20270210000000_step_logs drops the column and its CHECK.
SELECT 1;
