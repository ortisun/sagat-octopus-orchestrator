-- Bounded global keyset traversal must not sort all pending waits or scan unrelated steps.
-- Keep the legacy updated_at index for readers from the previous release.
CREATE INDEX steps_event_unscanned_path_idx
    ON steps (tenant_id, run_id, instance_path_nodes, instance_path_indices, node)
    WHERE status = 'WAITING' AND waiting_kind = 'EVENT'
      AND event_scanned_at IS NULL AND event_since IS NOT NULL;
