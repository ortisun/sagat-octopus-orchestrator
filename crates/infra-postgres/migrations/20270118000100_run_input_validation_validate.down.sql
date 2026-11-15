-- A validated CHECK cannot be made `NOT VALID` again, and nothing else was changed: the rollback of the first migration
-- restores the old CHECKs.
SELECT 1;
