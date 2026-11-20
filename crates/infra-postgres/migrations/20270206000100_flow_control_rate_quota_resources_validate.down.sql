-- A validated CHECK cannot be made `NOT VALID` again, and nothing else was changed: the rollback of 20270206000000
-- restores its own CHECKs.
SELECT 1;
