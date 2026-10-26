DROP INDEX loop_scopes_actionable_idx;
DROP INDEX collection_iterations_actionable_idx;
DROP INDEX iteration_scopes_open_owner_idx;
DROP INDEX steps_path_actionable_idx;
ALTER TABLE steps DROP CONSTRAINT steps_node_kind_valid;
ALTER TABLE steps DROP COLUMN node_kind;
