-- A validated CHECK cannot be made `NOT VALID` again, and nothing else was changed: the rollback of each earlier migration
-- restores its own CHECKs.
SELECT 1;
