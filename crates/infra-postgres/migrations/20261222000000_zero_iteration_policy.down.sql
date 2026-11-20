-- Serialize retained-value inspection with quota INSERT/UPDATE/DELETE.
LOCK TABLE tenant_collection_policies IN ACCESS EXCLUSIVE MODE;
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM tenant_collection_policies WHERE max_active_iterations = 0) THEN
        RAISE EXCEPTION USING ERRCODE = '55000',
            MESSAGE = 'an explicit zero iteration policy is retained';
    END IF;
END $$;
ALTER TABLE tenant_collection_policies
    DROP CONSTRAINT tenant_collection_policies_limit,
    ADD CONSTRAINT tenant_collection_policies_limit CHECK (
        max_active_iterations BETWEEN 1 AND 1000000000);
