-- Durable sequential-loop projections. Writers update these rows only with the matching journal,
-- run version, usage counters, audit record and outbox entry in one transaction.

CREATE TABLE loop_scopes (
    tenant_id             uuid        NOT NULL,
    namespace_id          uuid        NOT NULL,
    run_id                uuid        NOT NULL,
    owner_instance_id     uuid        NOT NULL,
    owner_path_nodes      text[]      NOT NULL,
    owner_path_indices    integer[]   NOT NULL,
    owner_node            text        NOT NULL,
    kind                  text        NOT NULL,
    lifecycle             text        NOT NULL,
    max_iterations        integer     NOT NULL,
    deadline_at           timestamptz NOT NULL,
    initial_carry_digest  bytea       NOT NULL,
    current_carry_digest  bytea       NOT NULL,
    next_iteration        integer     NOT NULL DEFAULT 0,
    active_iteration      integer,
    delay_due_at          timestamptz,
    entered_sequence      bigint      NOT NULL,
    last_checkpoint_sequence bigint,
    created_at            timestamptz NOT NULL,
    updated_at            timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, run_id, owner_instance_id),
    CONSTRAINT loop_scopes_owner_uniq UNIQUE
        (tenant_id, run_id, owner_path_nodes, owner_path_indices, owner_node),
    CONSTRAINT loop_scopes_run_fk FOREIGN KEY (tenant_id, run_id)
        REFERENCES runs (tenant_id, id) ON DELETE CASCADE,
    CONSTRAINT loop_scopes_namespace_fk FOREIGN KEY (tenant_id, namespace_id)
        REFERENCES namespaces (tenant_id, id),
    CONSTRAINT loop_scopes_initial_carry_fk FOREIGN KEY (tenant_id, initial_carry_digest)
        REFERENCES payloads (tenant_id, digest),
    CONSTRAINT loop_scopes_current_carry_fk FOREIGN KEY (tenant_id, current_carry_digest)
        REFERENCES payloads (tenant_id, digest),
    CONSTRAINT loop_scopes_owner_path CHECK (
        execution_instance_path_valid(owner_path_nodes, owner_path_indices)),
    CONSTRAINT loop_scopes_owner_node CHECK (
        owner_node ~ '^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$'),
    CONSTRAINT loop_scopes_kind CHECK (kind IN ('WHILE', 'REPEAT_UNTIL')),
    CONSTRAINT loop_scopes_lifecycle CHECK (
        lifecycle IN ('READY', 'RUNNING', 'DELAYING', 'SUCCEEDED', 'FAILED', 'DRAINING', 'CANCELLED')),
    CONSTRAINT loop_scopes_limits CHECK (
        max_iterations BETWEEN 1 AND 10000 AND next_iteration BETWEEN 0 AND max_iterations),
    CONSTRAINT loop_scopes_active CHECK (
        (lifecycle = 'RUNNING' AND active_iteration IS NOT NULL
            AND active_iteration >= 0 AND active_iteration < max_iterations
            AND delay_due_at IS NULL)
        OR (lifecycle = 'DELAYING' AND active_iteration IS NULL AND delay_due_at IS NOT NULL)
        OR (lifecycle NOT IN ('RUNNING', 'DELAYING')
            AND active_iteration IS NULL AND delay_due_at IS NULL)),
    CONSTRAINT loop_scopes_sequences CHECK (
        entered_sequence >= 1
        AND (last_checkpoint_sequence IS NULL OR last_checkpoint_sequence > entered_sequence)),
    CONSTRAINT loop_scopes_carry_digest CHECK (
        octet_length(initial_carry_digest) = 32 AND octet_length(current_carry_digest) = 32)
);

CREATE TABLE loop_iterations (
    tenant_id              uuid        NOT NULL,
    run_id                 uuid        NOT NULL,
    owner_instance_id      uuid        NOT NULL,
    iteration_index        integer     NOT NULL,
    entry_instance_id      uuid        NOT NULL,
    entry_path_nodes       text[]      NOT NULL,
    entry_path_indices     integer[]   NOT NULL,
    entry_node             text        NOT NULL,
    activity               text        NOT NULL,
    lifecycle              text        NOT NULL,
    slot_held              boolean     NOT NULL,
    reserved_instances     integer     NOT NULL,
    remaining_reservations integer     NOT NULL,
    admitted_sequence      bigint      NOT NULL,
    return_instance_id     uuid,
    return_node            text,
    result_digest          bytea,
    carry_digest           bytea,
    condition              boolean,
    checked_at             timestamptz,
    checkpoint_sequence    bigint,
    created_at             timestamptz NOT NULL,
    updated_at             timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, run_id, owner_instance_id, iteration_index),
    CONSTRAINT loop_iterations_scope_fk FOREIGN KEY (tenant_id, run_id, owner_instance_id)
        REFERENCES loop_scopes (tenant_id, run_id, owner_instance_id) ON DELETE CASCADE,
    CONSTRAINT loop_iterations_result_fk FOREIGN KEY (tenant_id, result_digest)
        REFERENCES payloads (tenant_id, digest),
    CONSTRAINT loop_iterations_carry_fk FOREIGN KEY (tenant_id, carry_digest)
        REFERENCES payloads (tenant_id, digest),
    CONSTRAINT loop_iterations_entry_uniq UNIQUE (tenant_id, entry_instance_id),
    CONSTRAINT loop_iterations_index CHECK (iteration_index BETWEEN 0 AND 9999),
    CONSTRAINT loop_iterations_entry_path CHECK (
        execution_instance_path_valid(entry_path_nodes, entry_path_indices)),
    CONSTRAINT loop_iterations_entry_node CHECK (
        entry_node ~ '^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$'),
    CONSTRAINT loop_iterations_activity CHECK (
        char_length(activity) BETWEEN 1 AND 128 AND activity !~ '[[:cntrl:]]'),
    CONSTRAINT loop_iterations_lifecycle CHECK (lifecycle IN ('ADMITTED', 'CHECKPOINTED', 'DRAINED')),
    CONSTRAINT loop_iterations_reservations CHECK (
        reserved_instances >= 1
        AND remaining_reservations BETWEEN 0 AND reserved_instances),
    CONSTRAINT loop_iterations_sequences CHECK (
        admitted_sequence >= 1
        AND (checkpoint_sequence IS NULL OR checkpoint_sequence > admitted_sequence)),
    CONSTRAINT loop_iterations_checkpoint CHECK (
        (lifecycle = 'ADMITTED' AND slot_held AND return_instance_id IS NULL
            AND return_node IS NULL AND result_digest IS NULL AND carry_digest IS NULL
            AND condition IS NULL AND checked_at IS NULL AND checkpoint_sequence IS NULL)
        OR (lifecycle IN ('CHECKPOINTED', 'DRAINED') AND NOT slot_held
            AND return_instance_id IS NOT NULL
            AND return_node ~ '^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$'
            AND octet_length(result_digest) = 32 AND octet_length(carry_digest) = 32
            AND condition IS NOT NULL AND checked_at IS NOT NULL
            AND checkpoint_sequence IS NOT NULL))
);

CREATE INDEX loop_iterations_active_idx
    ON loop_iterations (tenant_id, run_id, owner_instance_id, iteration_index)
    WHERE slot_held;

CREATE TABLE loop_checkpoints (
    tenant_id          uuid        NOT NULL,
    run_id             uuid        NOT NULL,
    owner_instance_id  uuid        NOT NULL,
    iteration_index    integer     NOT NULL,
    return_instance_id uuid        NOT NULL,
    result_digest      bytea       NOT NULL,
    carry_digest       bytea       NOT NULL,
    condition          boolean     NOT NULL,
    checked_at         timestamptz NOT NULL,
    continuation       text        NOT NULL,
    delay_due_at       timestamptz,
    timer_id           uuid,
    journal_sequence   bigint      NOT NULL,
    outbox_id          uuid        NOT NULL,
    committed_run_version bigint   NOT NULL,
    created_at         timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, run_id, owner_instance_id, iteration_index),
    CONSTRAINT loop_checkpoints_iteration_fk
        FOREIGN KEY (tenant_id, run_id, owner_instance_id, iteration_index)
        REFERENCES loop_iterations (tenant_id, run_id, owner_instance_id, iteration_index)
        ON DELETE CASCADE,
    CONSTRAINT loop_checkpoints_result_fk FOREIGN KEY (tenant_id, result_digest)
        REFERENCES payloads (tenant_id, digest),
    CONSTRAINT loop_checkpoints_carry_fk FOREIGN KEY (tenant_id, carry_digest)
        REFERENCES payloads (tenant_id, digest),
    CONSTRAINT loop_checkpoints_outbox_uniq UNIQUE (outbox_id),
    CONSTRAINT loop_checkpoints_timer_uniq UNIQUE (timer_id),
    CONSTRAINT loop_checkpoints_continuation CHECK (
        continuation IN ('CONTINUE', 'DELAY', 'COMPLETE', 'ITERATION_LIMIT')),
    CONSTRAINT loop_checkpoints_delay CHECK (
        (continuation = 'DELAY' AND delay_due_at IS NOT NULL AND timer_id IS NOT NULL
            AND delay_due_at > checked_at)
        OR (continuation <> 'DELAY' AND delay_due_at IS NULL AND timer_id IS NULL)),
    CONSTRAINT loop_checkpoints_sequences CHECK (
        journal_sequence >= 2 AND committed_run_version >= journal_sequence),
    CONSTRAINT loop_checkpoints_digests CHECK (
        octet_length(result_digest) = 32 AND octet_length(carry_digest) = 32)
);

CREATE TABLE loop_timers (
    tenant_id          uuid        NOT NULL,
    run_id             uuid        NOT NULL,
    owner_instance_id  uuid        NOT NULL,
    timer_id           uuid        NOT NULL,
    checkpoint_iteration integer   NOT NULL,
    next_iteration     integer     NOT NULL,
    due_at             timestamptz NOT NULL,
    lifecycle          text        NOT NULL,
    scheduled_sequence bigint      NOT NULL,
    scheduled_outbox_id uuid       NOT NULL,
    fired_sequence     bigint,
    fired_outbox_id    uuid,
    created_at         timestamptz NOT NULL,
    updated_at         timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, timer_id),
    CONSTRAINT loop_timers_scope_fk FOREIGN KEY (tenant_id, run_id, owner_instance_id)
        REFERENCES loop_scopes (tenant_id, run_id, owner_instance_id) ON DELETE CASCADE,
    CONSTRAINT loop_timers_checkpoint_fk FOREIGN KEY
        (tenant_id, run_id, owner_instance_id, checkpoint_iteration)
        REFERENCES loop_checkpoints (tenant_id, run_id, owner_instance_id, iteration_index)
        DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT loop_timers_scheduled_outbox_uniq UNIQUE (scheduled_outbox_id),
    CONSTRAINT loop_timers_fired_outbox_uniq UNIQUE (fired_outbox_id),
    CONSTRAINT loop_timers_lifecycle CHECK (lifecycle IN ('WAITING', 'FIRED', 'CANCELLED')),
    CONSTRAINT loop_timers_index CHECK (
        checkpoint_iteration BETWEEN 0 AND 9998
        AND next_iteration = checkpoint_iteration + 1),
    CONSTRAINT loop_timers_sequences CHECK (
        scheduled_sequence >= 2
        AND ((lifecycle = 'WAITING' AND fired_sequence IS NULL AND fired_outbox_id IS NULL)
            OR (lifecycle = 'FIRED' AND fired_sequence > scheduled_sequence
                AND fired_outbox_id IS NOT NULL)
            OR (lifecycle = 'CANCELLED' AND fired_sequence IS NULL AND fired_outbox_id IS NULL)))
);

CREATE INDEX loop_timers_due_idx
    ON loop_timers (due_at, tenant_id, run_id) WHERE lifecycle = 'WAITING';

CREATE TRIGGER loop_checkpoints_append_only
    BEFORE UPDATE OR DELETE ON loop_checkpoints
    FOR EACH ROW EXECUTE FUNCTION execution_reject_mutation();

ALTER TABLE steps DROP CONSTRAINT steps_waiting_kind_valid;
ALTER TABLE steps ADD CONSTRAINT steps_waiting_kind_valid CHECK (
    waiting_kind IS NULL OR waiting_kind IN
    ('TIMER', 'EVENT', 'BRANCHES', 'CHILD', 'SAGA', 'RECONCILIATION', 'OPERATOR', 'COLLECTION', 'LOOP')) NOT VALID;

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT, UPDATE ON loop_scopes, loop_iterations, loop_timers TO sagat_app;
        GRANT SELECT, INSERT ON loop_checkpoints TO sagat_app;
    END IF;
END
$grants$;
