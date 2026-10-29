SELECT jsonb_build_object(
  'stepId',step_id,'namespace',namespace_id,'node',node,
  'pathNodes',instance_path_nodes,'pathIndices',instance_path_indices,
  'status',status,'routed',routed,'kind',kind,'nodeKind',node_kind,
  'attempt',attempt,'generation',generation,'cancelRequested',cancel_requested,
  'waitingKind',waiting_kind,'dueAt',due_at,'eventType',event_type,
  'correlationKey',correlation_key,'since',event_since,'scannedAt',event_scanned_at,
  'output',encode(output_digest,'hex'),'activity',activity)
FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='payment'
  AND instance_path_nodes='{}'::text[] AND instance_path_indices='{}'::integer[]
