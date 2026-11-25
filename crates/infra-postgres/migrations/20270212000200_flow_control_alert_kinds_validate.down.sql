-- A validated CHECK cannot be made `NOT VALID` again, and nothing else was changed: the rollback of 20270212000100
-- restores its own CHECK.
SELECT 1;
