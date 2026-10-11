-- Observability (feature 008; FR-008-005): the alerts the engine raises for conditions that need a
-- person, one row per (namespace, kind, subject) that toggles between FIRING and RESOLVED.
--
-- A row carries counts and ages and nothing else: never a payload, a run's data or a secret. A
-- state change is written in the same transaction as the obligation to tell the namespace's
-- webhook subscribers (the transactional outbox of feature 004), so an alert is announced exactly
-- when it is recorded.

CREATE TABLE operational_alerts (
    id             uuid             PRIMARY KEY,
    tenant_id      uuid             NOT NULL REFERENCES tenants (id),
    namespace_id   uuid             NOT NULL,
    kind           text             NOT NULL,
    -- What the alert is about within the kind: an activity name, the kind of dead item. Bounded.
    subject        text             NOT NULL DEFAULT '',
    state          text             NOT NULL,
    severity       text             NOT NULL,
    first_seen_at  timestamptz      NOT NULL,
    last_seen_at   timestamptz      NOT NULL,
    resolved_at    timestamptz,
    -- How many times it has begun to fire.
    fired_count    integer          NOT NULL DEFAULT 1,
    -- What was measured the last time it was seen.
    item_count     bigint           NOT NULL,
    oldest_seconds double precision NOT NULL,
    CONSTRAINT operational_alerts_key UNIQUE (tenant_id, namespace_id, kind, subject),
    CONSTRAINT operational_alerts_namespace_fk FOREIGN KEY (tenant_id, namespace_id)
        REFERENCES namespaces (tenant_id, id),
    CONSTRAINT operational_alerts_kind_valid CHECK (kind IN
        ('OUTBOX_STUCK', 'LEASE_NOT_REAPED', 'STEP_UNCLAIMED', 'REVIEW_PENDING', 'DEAD_ITEMS')),
    CONSTRAINT operational_alerts_state_valid CHECK (state IN ('FIRING', 'RESOLVED')),
    CONSTRAINT operational_alerts_severity_valid CHECK (severity IN ('warning', 'critical')),
    CONSTRAINT operational_alerts_resolved_consistent CHECK ((resolved_at IS NOT NULL) = (state = 'RESOLVED')),
    CONSTRAINT operational_alerts_subject_length CHECK (char_length(subject) <= 120)
);
CREATE INDEX operational_alerts_listing_idx ON operational_alerts (tenant_id, namespace_id, id DESC);
CREATE INDEX operational_alerts_firing_idx ON operational_alerts (kind) WHERE state = 'FIRING';
CREATE INDEX operational_alerts_retention_idx ON operational_alerts (resolved_at) WHERE state = 'RESOLVED';

-- What the monitor asks about, so that asking is a range scan over a small index and never a scan of
-- a table that grows with history.
CREATE INDEX inbox_events_dead_idx ON inbox_events (tenant_id, namespace_id) WHERE status = 'DEAD';
CREATE INDEX runs_live_by_status_idx ON runs (status) WHERE finished_at IS NULL;

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON operational_alerts TO sagat_app;
    END IF;
END
$grants$;

-- A subscription may ask for the alerts of its namespace as well as for finished runs.
ALTER TABLE webhook_subscriptions DROP CONSTRAINT webhook_subscriptions_kinds_valid;
ALTER TABLE webhook_subscriptions ADD CONSTRAINT webhook_subscriptions_kinds_valid CHECK (
    cardinality(kinds) BETWEEN 1 AND 8
    AND kinds <@ ARRAY['run.finished', 'ops.alert.fired', 'ops.alert.resolved']);
