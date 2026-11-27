-- A validated CHECK cannot be made `NOT VALID` again, and nothing else was changed: the rollback of 20270213000300
-- restores its own CHECK.
SELECT 1;
