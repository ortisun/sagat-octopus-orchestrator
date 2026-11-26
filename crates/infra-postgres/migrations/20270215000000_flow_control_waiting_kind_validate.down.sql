-- A validated CHECK cannot be made `NOT VALID` again, and nothing else was changed: the rollback of 20270130000600
-- restores its own CHECK.
SELECT 1;
