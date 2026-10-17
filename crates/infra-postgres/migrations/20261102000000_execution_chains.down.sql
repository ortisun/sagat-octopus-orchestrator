DROP TABLE IF EXISTS child_chain_links;
ALTER TABLE IF EXISTS execution_chains DROP CONSTRAINT IF EXISTS execution_chains_head_segment_fk;
DROP TABLE IF EXISTS chain_segments;
DROP TABLE IF EXISTS execution_chains;
DROP FUNCTION IF EXISTS execution_chain_identity_guard();
ALTER TABLE runs DROP CONSTRAINT IF EXISTS runs_tenant_namespace_id_uniq;
