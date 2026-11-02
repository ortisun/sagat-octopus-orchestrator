-- Disable/fence the opt-in caller before downgrade. Indexes contain no independent business
-- data; retained native receipts continue to enforce their existing data-preserving guards.
DROP INDEX capacity_members_scoped_run_worker_idx;
DROP INDEX capacity_members_child_chain_worker_idx;
