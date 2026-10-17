-- Logical retained bytes are charged per step-output reference. Content-addressed payload
-- deduplication is a storage optimization and must not create extra room in the run limit.

ALTER TABLE run_instance_usage
    ADD COLUMN logical_output_bytes bigint NOT NULL DEFAULT 0;

UPDATE run_instance_usage usage
SET logical_output_bytes = retained.bytes
FROM (
    SELECT step.tenant_id, step.run_id,
           coalesce(sum(octet_length(payload.content)), 0)::bigint AS bytes
    FROM steps step
    JOIN payloads payload
      ON payload.tenant_id = step.tenant_id AND payload.digest = step.output_digest
    GROUP BY step.tenant_id, step.run_id
) retained
WHERE usage.tenant_id = retained.tenant_id AND usage.run_id = retained.run_id;

ALTER TABLE run_instance_usage ADD CONSTRAINT run_instance_usage_output_bytes CHECK (
    logical_output_bytes >= 0 AND logical_output_bytes <= 4194304);
