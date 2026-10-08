-- MODEL DESIGN ONLY. Not a ready-to-run production migration.
-- PostgreSQL 16+ baseline example. Add enum/checks, partitions, RLS, auditing, FK, quotas and indexes in controlled migrations.
CREATE EXTENSION IF NOT EXISTS pgcrypto;
CREATE TABLE tenants (id uuid PRIMARY KEY DEFAULT gen_random_uuid(), slug text NOT NULL UNIQUE, created_at timestamptz NOT NULL DEFAULT now());
CREATE TABLE namespaces (id uuid PRIMARY KEY DEFAULT gen_random_uuid(), tenant_id uuid NOT NULL REFERENCES tenants(id), slug text NOT NULL, environment text NOT NULL, created_at timestamptz NOT NULL DEFAULT now(), UNIQUE(tenant_id,slug));
CREATE TABLE workflows (id uuid PRIMARY KEY DEFAULT gen_random_uuid(), tenant_id uuid NOT NULL REFERENCES tenants(id), namespace_id uuid NOT NULL REFERENCES namespaces(id), name text NOT NULL, active_version_id uuid, draft_revision bigint NOT NULL DEFAULT 1, created_at timestamptz NOT NULL DEFAULT now(), UNIQUE(tenant_id,namespace_id,name));
CREATE TABLE workflow_versions (id uuid PRIMARY KEY DEFAULT gen_random_uuid(), tenant_id uuid NOT NULL REFERENCES tenants(id), workflow_id uuid NOT NULL REFERENCES workflows(id), version_number integer NOT NULL, ir_json jsonb NOT NULL, digest text NOT NULL, published_at timestamptz NOT NULL DEFAULT now(), UNIQUE(workflow_id,version_number), UNIQUE(workflow_id,digest));
ALTER TABLE workflows ADD CONSTRAINT workflows_active_fk FOREIGN KEY (active_version_id) REFERENCES workflow_versions(id);
CREATE TABLE runs (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), tenant_id uuid NOT NULL REFERENCES tenants(id), namespace_id uuid NOT NULL REFERENCES namespaces(id),
 workflow_id uuid NOT NULL REFERENCES workflows(id), workflow_version_id uuid NOT NULL REFERENCES workflow_versions(id),
 status text NOT NULL CHECK (status IN ('CREATED','READY','RUNNING','WAITING','PAUSED','COMPENSATING','CANCELLING','SUCCEEDED','FAILED','COMPENSATED','COMPENSATION_FAILED','CANCELLED','TIMED_OUT')),
 run_version bigint NOT NULL DEFAULT 0, last_event_seq bigint NOT NULL DEFAULT 0,
 correlation_id text, started_by text, input_ref text, created_at timestamptz NOT NULL DEFAULT now(), updated_at timestamptz NOT NULL DEFAULT now(), deadline_at timestamptz
);
CREATE INDEX runs_tenant_status_idx ON runs(tenant_id,namespace_id,status,created_at DESC);
CREATE TABLE run_events (
 tenant_id uuid NOT NULL REFERENCES tenants(id), run_id uuid NOT NULL REFERENCES runs(id), seq bigint NOT NULL,
 event_type text NOT NULL, step_id text, attempt integer, payload_ref text, actor text,
 trace_id text, occurred_at timestamptz NOT NULL DEFAULT now(), PRIMARY KEY (tenant_id,run_id,seq)
); -- Partition by time in production after testing retention/foreign-key design.
CREATE TABLE steps (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), tenant_id uuid NOT NULL REFERENCES tenants(id), run_id uuid NOT NULL REFERENCES runs(id),
 logical_path text NOT NULL, node_key text NOT NULL, status text NOT NULL,
 attempt integer NOT NULL DEFAULT 0, generation bigint NOT NULL DEFAULT 0, lease_token_hash text,
 lease_expires_at timestamptz, next_attempt_at timestamptz, input_ref text, output_ref text,
 last_error_code text, updated_at timestamptz NOT NULL DEFAULT now(), UNIQUE(tenant_id,run_id,logical_path,node_key)
);
CREATE INDEX steps_ready_idx ON steps(tenant_id,status,next_attempt_at) WHERE status IN ('READY','RETRY_WAIT');
CREATE TABLE step_attempts (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), tenant_id uuid NOT NULL REFERENCES tenants(id), step_id uuid NOT NULL REFERENCES steps(id),
 attempt integer NOT NULL, generation bigint NOT NULL, worker_id text, started_at timestamptz, completed_at timestamptz, outcome text, output_hash text,
 UNIQUE(tenant_id,step_id,attempt,generation)
);
CREATE TABLE timers (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), tenant_id uuid NOT NULL REFERENCES tenants(id), run_id uuid NOT NULL REFERENCES runs(id),
 step_id uuid REFERENCES steps(id), due_at timestamptz NOT NULL, status text NOT NULL, fired_at timestamptz, dedupe_key text NOT NULL,
 UNIQUE(tenant_id,dedupe_key)
);
CREATE INDEX timers_due_idx ON timers(due_at) WHERE status='WAITING';
CREATE TABLE wait_subscriptions (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), tenant_id uuid NOT NULL REFERENCES tenants(id), run_id uuid NOT NULL REFERENCES runs(id),
 step_id uuid NOT NULL REFERENCES steps(id), event_type text NOT NULL, correlation_key text NOT NULL,
 status text NOT NULL, deadline_at timestamptz NOT NULL, received_event_id text,
 UNIQUE(tenant_id,run_id,step_id)
);
CREATE INDEX subscriptions_match_idx ON wait_subscriptions(tenant_id,event_type,correlation_key) WHERE status='WAITING';
CREATE TABLE event_inbox (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), tenant_id uuid NOT NULL REFERENCES tenants(id), namespace_id uuid NOT NULL REFERENCES namespaces(id),
 source text NOT NULL, external_event_id text NOT NULL, event_type text NOT NULL, correlation_key text,
 received_at timestamptz NOT NULL DEFAULT now(), payload_ref text, processed_at timestamptz,
 UNIQUE(tenant_id,source,external_event_id)
);
CREATE TABLE event_outbox (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), tenant_id uuid NOT NULL REFERENCES tenants(id), aggregate_id text NOT NULL,
 event_type text NOT NULL, payload_json jsonb, payload_ref text,
 created_at timestamptz NOT NULL DEFAULT now(), next_attempt_at timestamptz NOT NULL DEFAULT now(),
 attempt_count integer NOT NULL DEFAULT 0, delivered_at timestamptz, last_error_code text
);
CREATE INDEX outbox_dispatch_idx ON event_outbox(next_attempt_at) WHERE delivered_at IS NULL;
CREATE TABLE saga_entries (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), tenant_id uuid NOT NULL REFERENCES tenants(id), run_id uuid NOT NULL REFERENCES runs(id),
 step_id uuid NOT NULL REFERENCES steps(id), compensate_activity text, reconcile_activity text,
 status text NOT NULL, compensation_key text NOT NULL, dependency_json jsonb NOT NULL DEFAULT '[]'::jsonb,
 updated_at timestamptz NOT NULL DEFAULT now(), UNIQUE(tenant_id,run_id,compensation_key)
);
CREATE TABLE command_dedupe (
 tenant_id uuid NOT NULL REFERENCES tenants(id), operation_scope text NOT NULL, idempotency_key text NOT NULL,
 request_hash text NOT NULL, response_json jsonb, accepted_at timestamptz NOT NULL DEFAULT now(),
 PRIMARY KEY(tenant_id,operation_scope,idempotency_key)
);
CREATE TABLE script_versions (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), tenant_id uuid NOT NULL REFERENCES tenants(id), namespace_id uuid NOT NULL REFERENCES namespaces(id),
 name text NOT NULL, version_number integer NOT NULL, language text NOT NULL, source_hash text NOT NULL,
 artifact_digest text, policy_digest text, manifest_json jsonb NOT NULL, published_at timestamptz,
 UNIQUE(tenant_id,namespace_id,name,version_number)
);
CREATE TABLE audit_records (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), tenant_id uuid NOT NULL REFERENCES tenants(id), actor text NOT NULL,
 action text NOT NULL, resource_type text NOT NULL, resource_id text NOT NULL, reason text,
 trace_id text, created_at timestamptz NOT NULL DEFAULT now()
);
-- Security: real migrations MUST verify tenant-scoped FKs (composite tenant_id/id), policies and query enforcement.
-- Additional planned tables: worker_sessions, worker_pools, webhook_endpoints, webhook_deliveries,
-- connector_configs, quotas, compensation_attempts, graph_projection_checkpoint and build_jobs.
