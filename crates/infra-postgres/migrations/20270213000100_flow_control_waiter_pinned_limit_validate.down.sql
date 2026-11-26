-- A validated CHECK cannot be made `NOT VALID` again, and nothing else was changed: the rollback of 20270213000000
-- drops the CHECKs with their column.
SELECT 1;
