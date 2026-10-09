-- Feature 004, events in: endpoints that receive signed webhooks, the inbox every event is admitted
-- to, the deliveries that say what an event woke or started, and the triggers of the active
-- versions (FR-004-001..004, FR-004-008).

-- Where a provider posts signed events. The id is what the provider's URL carries, so it is random
-- (never time-ordered); the slug is what a workflow's trigger names.
CREATE TABLE event_endpoints (
    id                 uuid        PRIMARY KEY,
    tenant_id          uuid        NOT NULL REFERENCES tenants (id),
    namespace_id       uuid        NOT NULL,
    slug               text        NOT NULL,
    -- The authentication profile: HMAC today; the others are decisions recorded in the research.
    profile            text        NOT NULL DEFAULT 'HMAC',
    -- The secret of the namespace that signs the requests. Names only: a value never lives here.
    secret_name        text        NOT NULL,
    default_event_type text,
    dedupe_ttl_seconds integer     NOT NULL,
    skew_seconds       integer     NOT NULL,
    max_body_bytes     integer     NOT NULL,
    rate_per_second    integer     NOT NULL,
    status             text        NOT NULL DEFAULT 'ACTIVE',
    created_by         text        NOT NULL,
    created_at         timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at         timestamptz NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT event_endpoints_tenant_id_uniq UNIQUE (tenant_id, id),
    CONSTRAINT event_endpoints_slug_uniq UNIQUE (tenant_id, namespace_id, slug),
    CONSTRAINT event_endpoints_namespace_fk FOREIGN KEY (tenant_id, namespace_id)
        REFERENCES namespaces (tenant_id, id),
    CONSTRAINT event_endpoints_profile_valid CHECK (profile IN ('HMAC')),
    CONSTRAINT event_endpoints_status_valid CHECK (status IN ('ACTIVE', 'PAUSED')),
    CONSTRAINT event_endpoints_secret_format CHECK (secret_name ~ '^[a-z][a-z0-9._-]{0,63}$'),
    CONSTRAINT event_endpoints_ttl_range CHECK (dedupe_ttl_seconds BETWEEN 60 AND 2592000),
    CONSTRAINT event_endpoints_skew_range CHECK (skew_seconds BETWEEN 1 AND 3600),
    CONSTRAINT event_endpoints_body_range CHECK (max_body_bytes BETWEEN 1024 AND 245760),
    CONSTRAINT event_endpoints_rate_range CHECK (rate_per_second BETWEEN 1 AND 10000)
);

-- Every event ever admitted, until it expires. One row per tenant, namespace, source and event id:
-- the primary key is the deduplication (two namespaces may each have a source of the same name). The payload lives inline (and goes with the row) because the
-- content-addressed payloads of runs are never deleted.
CREATE TABLE inbox_events (
    tenant_id        uuid        NOT NULL REFERENCES tenants (id),
    source           text        NOT NULL,
    event_id         text        NOT NULL,
    namespace_id     uuid        NOT NULL,
    event_type       text        NOT NULL,
    correlation_key  text,
    subject          text,
    occurred_at      timestamptz,
    payload          bytea       NOT NULL,
    payload_digest   bytea       NOT NULL,
    received_at      timestamptz NOT NULL,
    -- The end of the deduplication window; after it the row is purged and the same id is a new event.
    expires_at       timestamptz NOT NULL,
    -- What the matcher has done with it.
    status           text        NOT NULL DEFAULT 'PENDING',
    attempts         integer     NOT NULL DEFAULT 0,
    next_attempt_at  timestamptz NOT NULL,
    last_error       text,
    dispatched_at    timestamptz,
    PRIMARY KEY (tenant_id, namespace_id, source, event_id),
    CONSTRAINT inbox_events_namespace_fk FOREIGN KEY (tenant_id, namespace_id)
        REFERENCES namespaces (tenant_id, id),
    CONSTRAINT inbox_events_status_valid CHECK (status IN ('PENDING', 'DISPATCHED', 'DEAD')),
    CONSTRAINT inbox_events_payload_limit CHECK (octet_length(payload) <= 245760),
    CONSTRAINT inbox_events_digest_length CHECK (octet_length(payload_digest) = 32),
    CONSTRAINT inbox_events_dispatched_consistent CHECK ((dispatched_at IS NULL) = (status <> 'DISPATCHED')),
    CONSTRAINT inbox_events_source_format CHECK (source ~ '^[a-z][a-z0-9-]{1,61}[a-z0-9]$')
);
-- What the matcher claims next.
CREATE INDEX inbox_events_pending_idx ON inbox_events (next_attempt_at) WHERE status = 'PENDING';
-- Backpressure counts a tenant's backlog.
CREATE INDEX inbox_events_backlog_idx ON inbox_events (tenant_id) WHERE status = 'PENDING';
-- A wait that looks back finds the events that arrived before it.
CREATE INDEX inbox_events_match_idx
    ON inbox_events (tenant_id, namespace_id, event_type, correlation_key, received_at)
    WHERE correlation_key IS NOT NULL;
CREATE INDEX inbox_events_expiry_idx ON inbox_events (expires_at);

-- What an event woke or started, once per consumer: the evidence of a delivery, and the reason a
-- retried dispatch cannot deliver twice.
CREATE TABLE event_deliveries (
    tenant_id    uuid        NOT NULL,
    namespace_id uuid        NOT NULL,
    source       text        NOT NULL,
    event_id     text        NOT NULL,
    -- `wait:{run}/{node}` for a waiting step, `trigger:{id}` for a trigger.
    consumer     text        NOT NULL,
    run_id       uuid,
    node         text,
    trigger_id   uuid,
    delivered_at timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, namespace_id, source, event_id, consumer),
    CONSTRAINT event_deliveries_event_fk FOREIGN KEY (tenant_id, namespace_id, source, event_id)
        REFERENCES inbox_events (tenant_id, namespace_id, source, event_id) ON DELETE CASCADE
);
CREATE INDEX event_deliveries_run_idx ON event_deliveries (tenant_id, run_id) WHERE run_id IS NOT NULL;

-- The triggers of the active version of each workflow. Their ids are stable across versions (a hash
-- of the workflow and what the trigger is about), so republishing a version that keeps a trigger
-- does not make an event that was already processed start a second run.
CREATE TABLE workflow_triggers (
    id            uuid        PRIMARY KEY,
    tenant_id     uuid        NOT NULL REFERENCES tenants (id),
    namespace_id  uuid        NOT NULL,
    workflow_id   uuid        NOT NULL,
    version_id    uuid        NOT NULL,
    kind          text        NOT NULL,
    match_source  text,
    match_type    text,
    filter        text,
    cron          text,
    timezone      text,
    next_fire_at  timestamptz,
    -- When a tick that could not start (no room, a store that was down) is tried again. The tick
    -- stays `next_fire_at`, so every try is made under the same key and cannot start a run twice.
    retry_at      timestamptz,
    last_fire_at  timestamptz,
    CONSTRAINT workflow_triggers_workflow_fk FOREIGN KEY (tenant_id, workflow_id)
        REFERENCES workflows (tenant_id, id),
    CONSTRAINT workflow_triggers_version_fk FOREIGN KEY (tenant_id, version_id)
        REFERENCES workflow_versions (tenant_id, id),
    CONSTRAINT workflow_triggers_kind_valid CHECK (kind IN ('event', 'webhook', 'message', 'cron')),
    CONSTRAINT workflow_triggers_cron_has_schedule CHECK ((kind = 'cron') = (cron IS NOT NULL))
);
CREATE INDEX workflow_triggers_event_idx ON workflow_triggers (tenant_id, namespace_id)
    WHERE kind <> 'cron';
CREATE INDEX workflow_triggers_cron_idx ON workflow_triggers (next_fire_at) WHERE kind = 'cron';

-- An event wait accepts the events received since `event_since`: the moment it began waiting, less
-- its `lookbackMs`. So an event that arrived before the wait is delivered only when the author asked
-- for it, and one that arrived while the matcher was still looking is never lost.
ALTER TABLE steps ADD COLUMN event_since timestamptz;
-- Set once the inbox has been searched for events the wait should have: the matcher finds a wait
-- from an event, and this finds an event from a wait, so neither order loses one.
ALTER TABLE steps ADD COLUMN event_scanned_at timestamptz;
CREATE INDEX steps_event_unscanned_idx ON steps (updated_at)
    WHERE status = 'WAITING' AND waiting_kind = 'EVENT' AND event_scanned_at IS NULL;
-- Waits that began before this migration (feature 002 had event waits, signalled by hand) have no
-- instant to receive from, and there is no inbox from before the migration to search for them:
-- they wait from when they began, and are marked as searched.
UPDATE steps SET event_since = updated_at, event_scanned_at = clock_timestamp()
    WHERE status = 'WAITING' AND waiting_kind = 'EVENT';

-- Events out (FR-004-006): who asked to be told, and what is owed to them.
CREATE TABLE webhook_subscriptions (
    id               uuid        PRIMARY KEY,
    tenant_id        uuid        NOT NULL REFERENCES tenants (id),
    namespace_id     uuid        NOT NULL,
    name             text        NOT NULL,
    url              text        NOT NULL,
    -- The secret of the namespace that signs what is sent. Names only: a value never lives here.
    secret_name      text        NOT NULL,
    kinds            text[]      NOT NULL,
    rate_per_second  integer     NOT NULL,
    timeout_ms       integer     NOT NULL,
    status           text        NOT NULL DEFAULT 'ACTIVE',
    -- Dead deliveries in a row; at the limit the platform switches the subscription off.
    consecutive_dead integer     NOT NULL DEFAULT 0,
    last_success_at  timestamptz,
    last_failure_at  timestamptz,
    disabled_reason  text,
    -- The earliest the subscription is next offered to a worker: the rate limit, across replicas.
    next_slot_at     timestamptz NOT NULL DEFAULT '-infinity',
    created_by       text        NOT NULL,
    created_at       timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at       timestamptz NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT webhook_subscriptions_tenant_id_uniq UNIQUE (tenant_id, id),
    CONSTRAINT webhook_subscriptions_name_uniq UNIQUE (tenant_id, namespace_id, name),
    CONSTRAINT webhook_subscriptions_namespace_fk FOREIGN KEY (tenant_id, namespace_id)
        REFERENCES namespaces (tenant_id, id),
    CONSTRAINT webhook_subscriptions_status_valid CHECK (status IN ('ACTIVE', 'PAUSED', 'DISABLED')),
    CONSTRAINT webhook_subscriptions_name_format CHECK (name ~ '^[a-z][a-z0-9-]{1,61}[a-z0-9]$'),
    CONSTRAINT webhook_subscriptions_secret_format CHECK (secret_name ~ '^[a-z][a-z0-9._-]{0,63}$'),
    CONSTRAINT webhook_subscriptions_url_length CHECK (octet_length(url) <= 2048),
    CONSTRAINT webhook_subscriptions_kinds_valid CHECK (
        cardinality(kinds) BETWEEN 1 AND 8 AND kinds <@ ARRAY['run.finished']),
    CONSTRAINT webhook_subscriptions_rate_range CHECK (rate_per_second BETWEEN 1 AND 100),
    CONSTRAINT webhook_subscriptions_timeout_range CHECK (timeout_ms BETWEEN 1000 AND 30000)
);
-- Finding who wants an event when it is committed.
CREATE INDEX webhook_subscriptions_fanout_idx ON webhook_subscriptions (tenant_id, namespace_id)
    WHERE status = 'ACTIVE';
-- Finding who has something to send.
CREATE INDEX webhook_subscriptions_slot_idx ON webhook_subscriptions (next_slot_at)
    WHERE status = 'ACTIVE';

-- One row per event per subscription. The payload is the exact bytes that are signed and sent.
CREATE TABLE webhook_deliveries (
    id               uuid        PRIMARY KEY,
    tenant_id        uuid        NOT NULL,
    namespace_id     uuid        NOT NULL,
    subscription_id  uuid        NOT NULL,
    event_id         text        NOT NULL,
    kind             text        NOT NULL,
    payload          bytea       NOT NULL,
    status           text        NOT NULL DEFAULT 'PENDING',
    -- Attempts made in all, which number them. A redrive gives the delivery a full set of attempts
    -- again without forgetting the ones already made: the new set starts after this many.
    attempts         integer     NOT NULL DEFAULT 0,
    attempts_before_redrive integer NOT NULL DEFAULT 0,
    next_attempt_at  timestamptz NOT NULL,
    last_outcome     text,
    last_http_status integer,
    last_error       text,
    created_at       timestamptz NOT NULL DEFAULT clock_timestamp(),
    delivered_at     timestamptz,
    CONSTRAINT webhook_deliveries_subscription_fk FOREIGN KEY (tenant_id, subscription_id)
        REFERENCES webhook_subscriptions (tenant_id, id) ON DELETE CASCADE,
    CONSTRAINT webhook_deliveries_event_uniq UNIQUE (subscription_id, event_id),
    CONSTRAINT webhook_deliveries_status_valid CHECK (status IN ('PENDING', 'DELIVERED', 'DEAD')),
    CONSTRAINT webhook_deliveries_payload_limit CHECK (octet_length(payload) <= 65536),
    CONSTRAINT webhook_deliveries_delivered_consistent CHECK ((delivered_at IS NOT NULL) = (status = 'DELIVERED'))
);
CREATE INDEX webhook_deliveries_due_idx ON webhook_deliveries (subscription_id, next_attempt_at)
    WHERE status = 'PENDING';
CREATE INDEX webhook_deliveries_listing_idx ON webhook_deliveries (subscription_id, id DESC);
CREATE INDEX webhook_deliveries_dead_idx ON webhook_deliveries (subscription_id, created_at)
    WHERE status = 'DEAD';
CREATE INDEX webhook_deliveries_retention_idx ON webhook_deliveries (created_at)
    WHERE status <> 'PENDING';

-- Every attempt, so an operator can see what the receiver said and when.
CREATE TABLE webhook_attempts (
    delivery_id  uuid        NOT NULL REFERENCES webhook_deliveries (id) ON DELETE CASCADE,
    attempt      integer     NOT NULL,
    tenant_id    uuid        NOT NULL,
    started_at   timestamptz NOT NULL,
    duration_ms  integer     NOT NULL,
    outcome      text        NOT NULL,
    http_status  integer,
    -- A class of failure written by the platform; never the receiver's words.
    error        text,
    PRIMARY KEY (delivery_id, attempt),
    CONSTRAINT webhook_attempts_outcome_valid CHECK (outcome IN
        ('DELIVERED', 'REJECTED', 'SERVER_ERROR', 'THROTTLED', 'TIMEOUT', 'NETWORK', 'DENIED', 'UNSIGNED'))
);

-- Events in from the brokers of a tenant (FR-004-005): what is consumed, where it stopped and what
-- could not be turned into an event.
CREATE TABLE event_connectors (
    id                  uuid        PRIMARY KEY,
    tenant_id           uuid        NOT NULL REFERENCES tenants (id),
    namespace_id        uuid        NOT NULL,
    -- The source of the events it admits; shared with the slugs of the endpoints of the namespace.
    slug                text        NOT NULL,
    kind                text        NOT NULL,
    -- Brokers, topic, queue, stream: nothing secret. Read back through the constructors of the domain.
    settings            jsonb       NOT NULL,
    secret_name         text,
    default_event_type  text,
    rate_per_second     integer     NOT NULL,
    status              text        NOT NULL DEFAULT 'ACTIVE',
    last_error          text,
    last_message_at     timestamptz,
    consumed_total      bigint      NOT NULL DEFAULT 0,
    dead_lettered_total bigint      NOT NULL DEFAULT 0,
    created_by          text        NOT NULL,
    created_at          timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at          timestamptz NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT event_connectors_tenant_id_uniq UNIQUE (tenant_id, id),
    CONSTRAINT event_connectors_slug_uniq UNIQUE (tenant_id, namespace_id, slug),
    CONSTRAINT event_connectors_namespace_fk FOREIGN KEY (tenant_id, namespace_id)
        REFERENCES namespaces (tenant_id, id),
    CONSTRAINT event_connectors_kind_valid CHECK (kind IN ('NATS', 'KAFKA', 'SQS')),
    CONSTRAINT event_connectors_status_valid CHECK (status IN ('ACTIVE', 'PAUSED')),
    CONSTRAINT event_connectors_slug_format CHECK (slug ~ '^[a-z][a-z0-9-]{1,61}[a-z0-9]$'),
    CONSTRAINT event_connectors_secret_format CHECK (secret_name IS NULL OR secret_name ~ '^[a-z][a-z0-9._-]{0,63}$'),
    CONSTRAINT event_connectors_rate_range CHECK (rate_per_second BETWEEN 1 AND 10000)
);
CREATE INDEX event_connectors_active_idx ON event_connectors (id) WHERE status = 'ACTIVE';

-- Where a consumer that keeps its own position (Kafka) reads next, per topic and partition. Moved in
-- the transaction that admits the message, so a crash repeats nothing and loses nothing.
CREATE TABLE connector_offsets (
    connector_id uuid        NOT NULL REFERENCES event_connectors (id) ON DELETE CASCADE,
    tenant_id    uuid        NOT NULL,
    stream       text        NOT NULL,
    next_offset  bigint      NOT NULL,
    updated_at   timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (connector_id, stream)
);

-- Who reads a stream now. Only the holder consumes it, so replicas do not read the same partition.
CREATE TABLE connector_leases (
    connector_id uuid        NOT NULL REFERENCES event_connectors (id) ON DELETE CASCADE,
    stream       text        NOT NULL,
    holder       text        NOT NULL,
    expires_at   timestamptz NOT NULL,
    PRIMARY KEY (connector_id, stream)
);

-- Messages that could not become events: with why, and what they carried, for a person to look at.
CREATE TABLE connector_dead_letters (
    id           uuid        PRIMARY KEY,
    tenant_id    uuid        NOT NULL,
    namespace_id uuid        NOT NULL,
    connector_id uuid        NOT NULL REFERENCES event_connectors (id) ON DELETE CASCADE,
    position     text        NOT NULL,
    reason       text        NOT NULL,
    headers      jsonb       NOT NULL DEFAULT '{}',
    body         bytea       NOT NULL,
    truncated    boolean     NOT NULL DEFAULT false,
    received_at  timestamptz NOT NULL DEFAULT clock_timestamp(),
    -- A message set aside twice (redelivered) is set aside once.
    CONSTRAINT connector_dead_letters_position_uniq UNIQUE (connector_id, position),
    CONSTRAINT connector_dead_letters_reason_valid CHECK (reason IN
        ('NO_EVENT_TYPE', 'BAD_ENVELOPE', 'NOT_A_JSON_OBJECT', 'TOO_LARGE', 'EVENT_ID_REUSED')),
    CONSTRAINT connector_dead_letters_body_limit CHECK (octet_length(body) <= 65536)
);
CREATE INDEX connector_dead_letters_listing_idx ON connector_dead_letters (connector_id, id DESC);

-- Endpoints are a quota resource, counted per namespace.
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_resource_valid;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_resource_valid CHECK (resource IN
    ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets',
     'event_endpoints', 'webhook_subscriptions', 'event_connectors'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_tenant_scoped;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_tenant_scoped CHECK (
    namespace_id IS NULL OR resource IN ('active_runs', 'workflows', 'secrets', 'event_endpoints',
                                         'webhook_subscriptions', 'event_connectors'));
ALTER TABLE quota_ceilings DROP CONSTRAINT quota_ceilings_resource_valid;
ALTER TABLE quota_ceilings ADD CONSTRAINT quota_ceilings_resource_valid CHECK (resource IN
    ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets',
     'event_endpoints', 'webhook_subscriptions', 'event_connectors'));

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON event_endpoints, workflow_triggers TO sagat_app;
        GRANT SELECT, INSERT, UPDATE, DELETE ON webhook_subscriptions, webhook_deliveries TO sagat_app;
        GRANT SELECT, INSERT, DELETE ON webhook_attempts TO sagat_app;
        GRANT SELECT, INSERT, UPDATE, DELETE ON event_connectors, connector_offsets, connector_leases TO sagat_app;
        GRANT SELECT, INSERT, DELETE ON connector_dead_letters TO sagat_app;
        GRANT SELECT, INSERT, UPDATE, DELETE ON inbox_events TO sagat_app;
        GRANT SELECT, INSERT, DELETE ON event_deliveries TO sagat_app;
    END IF;
END
$grants$;
