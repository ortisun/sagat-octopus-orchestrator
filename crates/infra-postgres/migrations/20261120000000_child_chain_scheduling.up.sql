-- Child continuation chains (014, D301). The scheduler now looks for chain links whose final outcome
-- has not reached the parent, for chain heads whose parent asked them to stop, and, before it hands a
-- boundary off, for children of the run that has not been settled. Each is a small partial index, so a
-- quiet chain costs a row and nothing on the hot path.
CREATE INDEX IF NOT EXISTS runs_parent_idx
    ON runs (tenant_id, parent_run_id) WHERE parent_run_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS child_chain_links_ended_shard_idx
    ON child_chain_links (run_shard(parent_run_id), updated_at)
    WHERE final_outcome IS NOT NULL AND settled = false;
CREATE INDEX IF NOT EXISTS child_chain_links_cancel_idx
    ON child_chain_links (tenant_id, child_chain_id) WHERE cancel_requested;
