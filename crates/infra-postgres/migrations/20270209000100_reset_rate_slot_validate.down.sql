-- A validated CHECK cannot be made `NOT VALID` again, and nothing else was changed: the rollback of 20270209000000
-- restores its own CHECK.
SELECT 1;
