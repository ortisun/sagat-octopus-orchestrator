-- Feature 010: the scheduler's work is split into 64 shards by the run it concerns, and each shard
-- is owned by one replica of the engine at a time (a lease with a generation that grows each time
-- the owner changes). The replicas that look at disjoint shards stop doing each other's work, and
-- an owner that was slow or cut off finds, when it comes back, that the generation moved (the fence).

-- The shard of the work that concerns an identifier: the last byte of the UUID, which is random in a
-- UUIDv7, modulo 64. The same function is `Shard::of` in the domain (`sagat_domain::sharding`).
-- IMMUTABLE, so it can be the leading column of an index.
CREATE FUNCTION run_shard(id uuid) RETURNS smallint
    LANGUAGE sql IMMUTABLE PARALLEL SAFE STRICT
    AS $$ SELECT (get_byte(uuid_send(id), 15) & 63)::smallint $$;

-- One row per shard, always present. A shard is free when `expires_at` is past (a shard never held has
-- the epoch, not -infinity: before PostgreSQL 17 a timestamp cannot be subtracted from an infinite one); `generation` goes up
-- every time a different owner (or the same one after the lease ran out) takes it, never on a renewal.
CREATE TABLE scheduler_shards (
    shard       smallint    PRIMARY KEY CHECK (shard BETWEEN 0 AND 63),
    owner       text,
    generation  bigint      NOT NULL DEFAULT 0 CHECK (generation >= 0),
    expires_at  timestamptz NOT NULL DEFAULT 'epoch' CHECK (expires_at < 'infinity'),
    acquired_at timestamptz
);
INSERT INTO scheduler_shards (shard) SELECT generate_series(0, 63);

-- The replicas that are alive, apart from the leases: a replica that holds nothing yet must be
-- counted, or the others would never give it its share.
CREATE TABLE scheduler_members (
    owner       text        PRIMARY KEY,
    started_at  timestamptz NOT NULL,
    expires_at  timestamptz NOT NULL CHECK (expires_at < 'infinity')
);

-- How much of the scheduler's attention a tenant is given when others have work too: its share of a
-- pass is its weight over the weights of the tenants that have work (1 when it has no row). Set by
-- an operator (`control-plane scheduling-weight`), read by the engine.
CREATE TABLE tenant_scheduling (
    tenant_id   uuid        PRIMARY KEY REFERENCES tenants (id),
    weight      smallint    NOT NULL CHECK (weight BETWEEN 1 AND 16),
    updated_at  timestamptz NOT NULL
);

-- What each pass of the scheduler asks, shard first: a replica looks at its own shards only, and
-- within a shard the oldest due item is the first entry of the index. They are partial, as the
-- indexes they sit next to are, so a suspended run costs a row and nothing in the hot path.
CREATE INDEX IF NOT EXISTS steps_timer_shard_idx ON steps (run_shard(run_id), due_at)
    WHERE status = 'WAITING' AND waiting_kind = 'TIMER';
CREATE INDEX IF NOT EXISTS steps_wait_deadline_shard_idx ON steps (run_shard(run_id), due_at)
    WHERE status = 'WAITING' AND waiting_kind = 'EVENT';
CREATE INDEX IF NOT EXISTS steps_retry_shard_idx ON steps (run_shard(run_id), next_attempt_at)
    WHERE status = 'RETRY_WAIT';
CREATE INDEX IF NOT EXISTS steps_lease_expiry_shard_idx ON steps (run_shard(run_id), lease_expires_at)
    WHERE status IN ('LEASED', 'RUNNING');
CREATE INDEX IF NOT EXISTS steps_attempt_deadline_shard_idx ON steps (run_shard(run_id), attempt_deadline_at)
    WHERE status IN ('LEASED', 'RUNNING') AND attempt_deadline_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS steps_rehint_shard_idx ON steps (run_shard(run_id), last_hinted_at)
    WHERE status = 'READY' AND hint_count < 5;
CREATE INDEX IF NOT EXISTS steps_child_pending_shard_idx ON steps (run_shard(run_id), updated_at)
    WHERE status = 'WAITING' AND waiting_kind = 'CHILD' AND child_started = false;
CREATE INDEX IF NOT EXISTS runs_deadline_shard_idx ON runs (run_shard(id), deadline_at)
    WHERE finished_at IS NULL AND stop_reason IS NULL AND deadline_at IS NOT NULL;
-- A child that ended is acted on in its parent: the shard is the parent's.
CREATE INDEX IF NOT EXISTS runs_unsettled_children_shard_idx ON runs (run_shard(parent_run_id), finished_at)
    WHERE parent_run_id IS NOT NULL AND parent_settled = false AND finished_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS runs_children_to_cancel_shard_idx ON runs (run_shard(id), id)
    WHERE parent_cancel AND finished_at IS NULL AND stop_reason IS NULL;
CREATE INDEX IF NOT EXISTS event_outbox_pending_shard_idx ON event_outbox (run_shard(aggregate_id), next_attempt_at)
    WHERE delivered_at IS NULL;

-- `IF NOT EXISTS` lets an operator build these CONCURRENTLY first on a large database; a build that
-- failed leaves an index that exists and is INVALID, which the planner never uses, so it is refused here.
DO $valid$
DECLARE
    broken text;
BEGIN
    SELECT string_agg(c.relname, ', ') INTO broken
      FROM pg_index i JOIN pg_class c ON c.oid = i.indexrelid
     WHERE NOT i.indisvalid AND c.relname IN (
         'steps_timer_shard_idx', 'steps_wait_deadline_shard_idx', 'steps_retry_shard_idx',
         'steps_lease_expiry_shard_idx', 'steps_attempt_deadline_shard_idx', 'steps_rehint_shard_idx',
         'steps_child_pending_shard_idx', 'runs_deadline_shard_idx', 'runs_unsettled_children_shard_idx',
         'runs_children_to_cancel_shard_idx', 'event_outbox_pending_shard_idx');
    IF broken IS NOT NULL THEN
        RAISE EXCEPTION 'invalid index left by a failed concurrent build, drop it and run again: %', broken;
    END IF;
END
$valid$;

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, UPDATE ON scheduler_shards TO sagat_app;
        GRANT SELECT, INSERT, UPDATE, DELETE ON scheduler_members TO sagat_app;
        GRANT SELECT ON tenant_scheduling TO sagat_app;
    END IF;
END
$grants$;
