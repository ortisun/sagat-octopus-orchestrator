-- A digest of what a restore must bring back, to compare before a disaster and after the restore
-- (feature 010, FR-010-007). The journal and the audit trail are append-only, so a database that was
-- restored to a point in time has exactly the rows it had at that point: the same count and the same
-- digest. Run it with: psql -X -At -f infra/dr/journal_digest.sql <database>
--
-- The digest covers every field of the journal that a replay reads (type, node, attempt, detail,
-- payload digest, actor, time) in the order of the journal, and the audit trail the same way. It does
-- not cover the projections (`steps`, `runs`...): those are checked by replaying the journal
-- (`PgRunStore::verify_run`), which is what `dr_verify` does for every run.
SELECT 'runs=' || count(*) FROM runs;
SELECT 'journal_events=' || count(*) FROM run_events;
SELECT 'audit_records=' || count(*) FROM audit_records;
SELECT 'journal_sha256=' || coalesce(encode(sha256(convert_to(string_agg(
           tenant_id::text || ':' || run_id::text || ':' || seq::text || ':' || event_type || ':' ||
           coalesce(node, '') || ':' || coalesce(attempt::text, '') || ':' || detail::text || ':' ||
           coalesce(encode(payload_digest, 'hex'), '') || ':' || actor || ':' || occurred_at::text,
           E'\n' ORDER BY tenant_id, run_id, seq), 'UTF8')), 'hex'), 'empty')
  FROM run_events;
SELECT 'audit_sha256=' || coalesce(encode(sha256(convert_to(string_agg(
           id::text || ':' || tenant_id::text || ':' || action || ':' || resource_type || ':' ||
           resource_id || ':' || actor || ':' || created_at::text,
           E'\n' ORDER BY id), 'UTF8')), 'hex'), 'empty')
  FROM audit_records;
