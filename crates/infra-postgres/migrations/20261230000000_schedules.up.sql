-- Feature 018, first migration: schedules as a namespace resource, their immutable revisions, the
-- actions taken on their nominal times, backfills, the owed cancellations of CANCEL_OTHER and the
-- singleton that records whether the cron triggers were handed over to the schedule pass.
--
-- Additive: an older engine or control plane never reads these tables. No foreign key points at
-- `workflow_triggers`: an older control plane that republishes a workflow deletes and recreates its
-- triggers, and a foreign key would make that fail. A reconciliation pass keeps trigger-owned
-- schedules equal to the triggers instead (T018-060).
--
-- Business ids, the `runs` columns, batch jobs, resets, the quota resources and the instance
-- capability registry arrive in the next migrations of this feature.

CREATE TABLE schedules (
    tenant_id         uuid        NOT NULL REFERENCES tenants (id),
    namespace_id      uuid        NOT NULL,
    id                uuid        NOT NULL,
    name              text        NOT NULL,
    origin            text        NOT NULL,
    trigger_id        uuid,
    state             text        NOT NULL,
    head_revision     integer     NOT NULL,
    -- Compare-and-swap token: every mutation increments it in the statement that checks it.
    state_version     bigint      NOT NULL DEFAULT 1,
    note              text,
    note_by           text,
    note_at           timestamptz,
    authorized_by     text        NOT NULL,
    remaining_actions integer,
    next_action_at    timestamptz,
    next_actual_at    timestamptz,
    retry_at          timestamptz,
    recheck_at        timestamptz,
    next_wake_at      timestamptz GENERATED ALWAYS AS (least(next_actual_at, retry_at, recheck_at)) STORED,
    tracked_runs      uuid[]      NOT NULL DEFAULT '{}',
    buffered          integer     NOT NULL DEFAULT 0,
    counters          jsonb       NOT NULL DEFAULT '{}',
    shard             smallint    GENERATED ALWAYS AS (run_shard(id)) STORED,
    created_at        timestamptz NOT NULL,
    updated_at        timestamptz NOT NULL,
    deleted_at        timestamptz,
    PRIMARY KEY (tenant_id, id),
    CONSTRAINT schedules_namespace_same_tenant_fk
        FOREIGN KEY (tenant_id, namespace_id) REFERENCES namespaces (tenant_id, id),
    CONSTRAINT schedules_name_format
        CHECK (char_length(name) BETWEEN 1 AND 128 AND name ~ '^[a-z0-9][a-z0-9-]*$'),
    CONSTRAINT schedules_origin_valid CHECK (origin IN ('API', 'TRIGGER')),
    CONSTRAINT schedules_trigger_only_for_trigger_origin CHECK ((origin = 'TRIGGER') = (trigger_id IS NOT NULL)),
    CONSTRAINT schedules_state_valid CHECK (state IN ('ACTIVE', 'PAUSED', 'DELETED')),
    CONSTRAINT schedules_deleted_consistent CHECK ((state = 'DELETED') = (deleted_at IS NOT NULL)),
    CONSTRAINT schedules_head_positive CHECK (head_revision >= 1),
    CONSTRAINT schedules_version_positive CHECK (state_version >= 1),
    CONSTRAINT schedules_note_bounded CHECK (note IS NULL OR char_length(note) <= 512),
    CONSTRAINT schedules_note_has_actor CHECK ((note IS NULL) OR (note_by IS NOT NULL AND note_at IS NOT NULL)),
    CONSTRAINT schedules_note_by_bounded CHECK (note_by IS NULL OR char_length(note_by) BETWEEN 1 AND 255),
    CONSTRAINT schedules_authorized_by_bounded CHECK (char_length(authorized_by) BETWEEN 1 AND 255),
    CONSTRAINT schedules_remaining_bounded CHECK (remaining_actions IS NULL OR remaining_actions BETWEEN 0 AND 1000000),
    CONSTRAINT schedules_tracked_bounded CHECK (cardinality(tracked_runs) <= 1000),
    CONSTRAINT schedules_buffered_nonnegative CHECK (buffered >= 0),
    CONSTRAINT schedules_counters_is_object CHECK (jsonb_typeof(counters) = 'object'),
    CONSTRAINT schedules_counters_bounded CHECK (octet_length(counters::text) <= 1024)
);
-- A name is unique in its namespace while the schedule exists; a deleted one frees it.
CREATE UNIQUE INDEX schedules_name_uniq ON schedules (tenant_id, namespace_id, name) WHERE state <> 'DELETED';
CREATE UNIQUE INDEX schedules_trigger_uniq ON schedules (trigger_id) WHERE trigger_id IS NOT NULL;
-- The due scan of the schedule pass: owned shards, earliest wake first.
CREATE INDEX schedules_due_idx ON schedules (shard, next_wake_at) WHERE state = 'ACTIVE';

-- What identifies a schedule never changes.
CREATE FUNCTION schedules_identity_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.tenant_id IS DISTINCT FROM OLD.tenant_id
        OR NEW.namespace_id IS DISTINCT FROM OLD.namespace_id
        OR NEW.id IS DISTINCT FROM OLD.id
        OR NEW.origin IS DISTINCT FROM OLD.origin
        OR NEW.trigger_id IS DISTINCT FROM OLD.trigger_id
        OR NEW.created_at IS DISTINCT FROM OLD.created_at THEN
        RAISE EXCEPTION 'the identity of a schedule never changes'
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER schedules_identity BEFORE UPDATE ON schedules
    FOR EACH ROW EXECUTE FUNCTION schedules_identity_guard();
CREATE TRIGGER schedules_no_delete BEFORE DELETE ON schedules
    FOR EACH ROW EXECUTE FUNCTION execution_reject_mutation();
CREATE TRIGGER schedules_no_truncate BEFORE TRUNCATE ON schedules
    FOR EACH STATEMENT EXECUTE FUNCTION execution_reject_mutation();
ALTER TABLE schedules ENABLE ALWAYS TRIGGER schedules_identity;
ALTER TABLE schedules ENABLE ALWAYS TRIGGER schedules_no_delete;
ALTER TABLE schedules ENABLE ALWAYS TRIGGER schedules_no_truncate;

-- The name and the note are customer text kept for the namespace: they are charged to the tenant's
-- content bytes by the ALWAYS accounting triggers, as the manifest of storage classes says.
CREATE TRIGGER schedules_quota_delta AFTER INSERT OR UPDATE OR DELETE ON schedules
    FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'octet_length_utf8', 'name', 'content', 'octet_length_utf8', 'note');
ALTER TABLE schedules ENABLE ALWAYS TRIGGER schedules_quota_delta;
CREATE CONSTRAINT TRIGGER schedules_quota_flush AFTER INSERT OR UPDATE OR DELETE ON schedules
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE schedules ENABLE ALWAYS TRIGGER schedules_quota_flush;

-- One row per update of a schedule, never changed.
CREATE TABLE schedule_revisions (
    tenant_id    uuid        NOT NULL,
    schedule_id  uuid        NOT NULL,
    revision     integer     NOT NULL,
    spec         jsonb       NOT NULL,
    policies     jsonb       NOT NULL,
    action       jsonb       NOT NULL,
    input_digest bytea,
    actor        text        NOT NULL,
    request_id   text,
    created_at   timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (tenant_id, schedule_id, revision),
    CONSTRAINT schedule_revisions_schedule_fk
        FOREIGN KEY (tenant_id, schedule_id) REFERENCES schedules (tenant_id, id),
    CONSTRAINT schedule_revisions_input_fk
        FOREIGN KEY (tenant_id, input_digest) REFERENCES payloads (tenant_id, digest),
    CONSTRAINT schedule_revisions_revision_positive CHECK (revision >= 1),
    CONSTRAINT schedule_revisions_spec_is_object CHECK (jsonb_typeof(spec) = 'object'),
    CONSTRAINT schedule_revisions_policies_is_object CHECK (jsonb_typeof(policies) = 'object'),
    CONSTRAINT schedule_revisions_action_is_object CHECK (jsonb_typeof(action) = 'object'),
    CONSTRAINT schedule_revisions_spec_bounded CHECK (octet_length(spec::text) <= 131072),
    CONSTRAINT schedule_revisions_policies_bounded CHECK (octet_length(policies::text) <= 1024),
    CONSTRAINT schedule_revisions_action_bounded CHECK (octet_length(action::text) <= 8192),
    CONSTRAINT schedule_revisions_actor_bounded CHECK (char_length(actor) BETWEEN 1 AND 255),
    CONSTRAINT schedule_revisions_request_bounded CHECK (request_id IS NULL OR char_length(request_id) <= 255)
);
CREATE TRIGGER schedule_revisions_append_only BEFORE UPDATE OR DELETE ON schedule_revisions
    FOR EACH ROW EXECUTE FUNCTION execution_reject_mutation();
CREATE TRIGGER schedule_revisions_no_truncate BEFORE TRUNCATE ON schedule_revisions
    FOR EACH STATEMENT EXECUTE FUNCTION execution_reject_mutation();
ALTER TABLE schedule_revisions ENABLE ALWAYS TRIGGER schedule_revisions_append_only;
ALTER TABLE schedule_revisions ENABLE ALWAYS TRIGGER schedule_revisions_no_truncate;

-- The spec and the action (which carries the business-id prefix) are customer content too.
CREATE TRIGGER schedule_revisions_quota_delta AFTER INSERT OR UPDATE OR DELETE ON schedule_revisions
    FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'canonical_json_utf8', 'spec', 'content', 'canonical_json_utf8', 'action');
ALTER TABLE schedule_revisions ENABLE ALWAYS TRIGGER schedule_revisions_quota_delta;
CREATE CONSTRAINT TRIGGER schedule_revisions_quota_flush AFTER INSERT OR UPDATE OR DELETE ON schedule_revisions
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE schedule_revisions ENABLE ALWAYS TRIGGER schedule_revisions_quota_flush;

-- The head of a schedule is a revision that exists by the time the transaction commits.
ALTER TABLE schedules ADD CONSTRAINT schedules_head_revision_fk
    FOREIGN KEY (tenant_id, id, head_revision) REFERENCES schedule_revisions (tenant_id, schedule_id, revision)
    DEFERRABLE INITIALLY DEFERRED;

-- One row per (schedule, origin, nominal time): the identity of an action. A scheduled and a
-- backfilled action share the spec's nominal times (origin SPEC); trigger-now has its own (MANUAL).
CREATE TABLE schedule_actions (
    tenant_id       uuid        NOT NULL,
    schedule_id     uuid        NOT NULL,
    origin          text        NOT NULL,
    nominal_at      timestamptz NOT NULL,
    kind            text        NOT NULL,
    revision        integer     NOT NULL,
    outcome         text        NOT NULL,
    reason          text,
    actual_at       timestamptz,
    decided_at      timestamptz NOT NULL,
    run_id          uuid,
    backfill_id     uuid,
    summary_count   integer,
    summary_last_at timestamptz,
    PRIMARY KEY (tenant_id, schedule_id, origin, nominal_at),
    CONSTRAINT schedule_actions_schedule_fk
        FOREIGN KEY (tenant_id, schedule_id) REFERENCES schedules (tenant_id, id),
    CONSTRAINT schedule_actions_run_fk
        FOREIGN KEY (tenant_id, run_id) REFERENCES runs (tenant_id, id),
    CONSTRAINT schedule_actions_origin_valid CHECK (origin IN ('SPEC', 'MANUAL')),
    CONSTRAINT schedule_actions_kind_valid CHECK (kind IN ('SCHEDULED', 'TRIGGERED', 'BACKFILL')),
    CONSTRAINT schedule_actions_kind_matches_origin CHECK ((kind = 'TRIGGERED') = (origin = 'MANUAL')),
    CONSTRAINT schedule_actions_revision_positive CHECK (revision >= 1),
    CONSTRAINT schedule_actions_outcome_valid CHECK (outcome IN
        ('STARTED', 'BUFFERED', 'SKIPPED_OVERLAP', 'DROPPED_BUFFER_FULL', 'DROPPED_DELETED', 'COALESCED',
         'MISSED_CATCHUP_WINDOW', 'ALREADY_ACTED', 'AUTHORITY_REVOKED', 'BUSINESS_ID_CONFLICT')),
    CONSTRAINT schedule_actions_started_has_run CHECK (outcome <> 'STARTED' OR run_id IS NOT NULL),
    CONSTRAINT schedule_actions_reason_bounded CHECK (reason IS NULL OR reason ~ '^[A-Z][A-Z0-9_]{0,63}$'),
    CONSTRAINT schedule_actions_summary_consistent CHECK (
        (outcome IN ('COALESCED', 'MISSED_CATCHUP_WINDOW')) = (summary_count IS NOT NULL)
        AND (summary_count IS NULL OR summary_count >= 1)
        AND (summary_count IS NULL) = (summary_last_at IS NULL))
);
-- The recent actions of a schedule, newest first.
CREATE INDEX schedule_actions_recent_idx ON schedule_actions (tenant_id, schedule_id, decided_at DESC);
-- The waiting actions, oldest first: the next one to start when a tracked run ends.
CREATE INDEX schedule_actions_buffered_idx ON schedule_actions (tenant_id, schedule_id, nominal_at)
    WHERE outcome = 'BUFFERED';

-- An action is final the moment it is decided, except a BUFFERED one, which later starts, is dropped
-- or missed. Nothing else about it changes, and a waiting one is never deleted (retention purges
-- final rows only).
CREATE FUNCTION schedule_actions_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        IF OLD.outcome = 'BUFFERED' THEN
            RAISE EXCEPTION 'a waiting schedule action is never purged'
                USING ERRCODE = 'restrict_violation';
        END IF;
        RETURN OLD;
    END IF;
    IF OLD.outcome <> 'BUFFERED' OR NEW.outcome = 'BUFFERED'
        OR NEW.tenant_id IS DISTINCT FROM OLD.tenant_id
        OR NEW.schedule_id IS DISTINCT FROM OLD.schedule_id
        OR NEW.origin IS DISTINCT FROM OLD.origin
        OR NEW.nominal_at IS DISTINCT FROM OLD.nominal_at
        OR NEW.kind IS DISTINCT FROM OLD.kind
        OR NEW.revision IS DISTINCT FROM OLD.revision THEN
        RAISE EXCEPTION 'a schedule action changes only from BUFFERED to a final outcome'
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER schedule_actions_guard BEFORE UPDATE OR DELETE ON schedule_actions
    FOR EACH ROW EXECUTE FUNCTION schedule_actions_guard();
CREATE TRIGGER schedule_actions_no_truncate BEFORE TRUNCATE ON schedule_actions
    FOR EACH STATEMENT EXECUTE FUNCTION execution_reject_mutation();
ALTER TABLE schedule_actions ENABLE ALWAYS TRIGGER schedule_actions_guard;
ALTER TABLE schedule_actions ENABLE ALWAYS TRIGGER schedule_actions_no_truncate;

CREATE TABLE schedule_backfills (
    tenant_id    uuid        NOT NULL,
    schedule_id  uuid        NOT NULL,
    id           uuid        NOT NULL,
    range_start  timestamptz NOT NULL,
    range_end    timestamptz NOT NULL,
    cursor_at    timestamptz,
    overlap      text        NOT NULL,
    state        text        NOT NULL,
    requested_by text        NOT NULL,
    request_id   text,
    counters     jsonb       NOT NULL DEFAULT '{}',
    created_at   timestamptz NOT NULL,
    finished_at  timestamptz,
    PRIMARY KEY (tenant_id, schedule_id, id),
    CONSTRAINT schedule_backfills_schedule_fk
        FOREIGN KEY (tenant_id, schedule_id) REFERENCES schedules (tenant_id, id),
    CONSTRAINT schedule_backfills_range_valid CHECK (range_start < range_end),
    CONSTRAINT schedule_backfills_overlap_valid CHECK (overlap IN
        ('SKIP', 'BUFFER_ONE', 'BUFFER_ALL', 'CANCEL_OTHER', 'ALLOW_ALL')),
    CONSTRAINT schedule_backfills_state_valid CHECK (state IN ('PENDING', 'RUNNING', 'COMPLETED', 'CANCELLED')),
    CONSTRAINT schedule_backfills_finished_consistent CHECK ((finished_at IS NOT NULL) = (state IN ('COMPLETED', 'CANCELLED'))),
    CONSTRAINT schedule_backfills_requested_by_bounded CHECK (char_length(requested_by) BETWEEN 1 AND 255),
    CONSTRAINT schedule_backfills_request_bounded CHECK (request_id IS NULL OR char_length(request_id) <= 255),
    CONSTRAINT schedule_backfills_counters_is_object CHECK (jsonb_typeof(counters) = 'object'),
    CONSTRAINT schedule_backfills_counters_bounded CHECK (octet_length(counters::text) <= 1024)
);
CREATE INDEX schedule_backfills_open_idx ON schedule_backfills (tenant_id, schedule_id)
    WHERE state IN ('PENDING', 'RUNNING');

-- A cancellation that CANCEL_OTHER owes an open run, kept until it is done.
CREATE TABLE schedule_obligations (
    tenant_id       uuid        NOT NULL,
    schedule_id     uuid        NOT NULL,
    nominal_at      timestamptz NOT NULL,
    run_id          uuid        NOT NULL,
    kind            text        NOT NULL,
    state           text        NOT NULL,
    idempotency_key text        NOT NULL,
    created_at      timestamptz NOT NULL,
    done_at         timestamptz,
    PRIMARY KEY (tenant_id, schedule_id, nominal_at, run_id),
    CONSTRAINT schedule_obligations_schedule_fk
        FOREIGN KEY (tenant_id, schedule_id) REFERENCES schedules (tenant_id, id),
    CONSTRAINT schedule_obligations_kind_valid CHECK (kind IN ('CANCEL_RUN')),
    CONSTRAINT schedule_obligations_state_valid CHECK (state IN ('PENDING', 'DONE')),
    CONSTRAINT schedule_obligations_done_consistent CHECK ((done_at IS NOT NULL) = (state = 'DONE')),
    CONSTRAINT schedule_obligations_key_bounded CHECK (char_length(idempotency_key) BETWEEN 1 AND 255)
);
CREATE INDEX schedule_obligations_pending_idx ON schedule_obligations (tenant_id) WHERE state = 'PENDING';

-- Whether the cron triggers of workflows still fire from the legacy pass or from the schedule pass.
-- One row, changed only by the audited operator command (a later migration adds its function).
CREATE TABLE schedule_trigger_handover (
    only_row   boolean     PRIMARY KEY DEFAULT true CHECK (only_row),
    state      text        NOT NULL DEFAULT 'LEGACY',
    changed_by text,
    changed_at timestamptz,
    CONSTRAINT schedule_trigger_handover_state_valid CHECK (state IN ('LEGACY', 'HANDED_OVER')),
    CONSTRAINT schedule_trigger_handover_changed_consistent CHECK ((state = 'LEGACY') = (changed_by IS NULL)),
    CONSTRAINT schedule_trigger_handover_changed_by_bounded CHECK (changed_by IS NULL OR char_length(changed_by) <= 255)
);
INSERT INTO schedule_trigger_handover (only_row, state) VALUES (true, 'LEGACY');
CREATE TRIGGER schedule_trigger_handover_single_row BEFORE DELETE ON schedule_trigger_handover
    FOR EACH ROW EXECUTE FUNCTION execution_reject_mutation();
CREATE TRIGGER schedule_trigger_handover_no_truncate BEFORE TRUNCATE ON schedule_trigger_handover
    FOR EACH STATEMENT EXECUTE FUNCTION execution_reject_mutation();
ALTER TABLE schedule_trigger_handover ENABLE ALWAYS TRIGGER schedule_trigger_handover_single_row;
ALTER TABLE schedule_trigger_handover ENABLE ALWAYS TRIGGER schedule_trigger_handover_no_truncate;

-- The runtime role reads and writes what the stores need. UPDATE is what lets it take the row lock of
-- a schedule, a backfill or an obligation. Nothing is deleted by the role: retention runs through
-- bounded purge functions added with the maintenance work (T018-028).
REVOKE ALL PRIVILEGES ON schedules, schedule_revisions, schedule_actions, schedule_backfills,
    schedule_obligations, schedule_trigger_handover FROM PUBLIC;
DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        REVOKE ALL PRIVILEGES ON schedules, schedule_revisions, schedule_actions, schedule_backfills,
            schedule_obligations, schedule_trigger_handover FROM sagat_app;
        GRANT SELECT, INSERT, UPDATE ON schedules, schedule_actions, schedule_backfills, schedule_obligations
            TO sagat_app;
        GRANT SELECT, INSERT ON schedule_revisions TO sagat_app;
        GRANT SELECT ON schedule_trigger_handover TO sagat_app;
    END IF;
END $grants$;
