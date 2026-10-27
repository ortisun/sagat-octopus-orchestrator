-- O701: an explicit zero stops fresh admission; accepted work remains admitted.
ALTER TABLE tenant_collection_policies
    DROP CONSTRAINT tenant_collection_policies_limit,
    ADD CONSTRAINT tenant_collection_policies_limit CHECK (
        max_active_iterations BETWEEN 0 AND 1000000000);
