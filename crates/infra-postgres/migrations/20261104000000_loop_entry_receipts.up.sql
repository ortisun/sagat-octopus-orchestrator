ALTER TABLE loop_scopes
    ADD COLUMN initial_condition boolean,
    ADD COLUMN entry_outbox_id uuid;

UPDATE loop_scopes AS scope
SET initial_condition = CASE
        WHEN scope.kind = 'WHILE' THEN (event.detail ->> 'initial_condition')::boolean
        ELSE NULL
    END
FROM run_events AS event
WHERE event.tenant_id = scope.tenant_id
  AND event.run_id = scope.run_id
  AND event.seq = scope.entered_sequence
  AND event.event_type = 'loop.entered';

ALTER TABLE loop_scopes
    ADD CONSTRAINT loop_scopes_initial_condition CHECK (
        entry_outbox_id IS NULL
        OR (kind = 'WHILE' AND initial_condition IS NOT NULL)
        OR (kind = 'REPEAT_UNTIL' AND initial_condition IS NULL));

CREATE UNIQUE INDEX loop_scopes_entry_outbox_uniq
    ON loop_scopes (tenant_id, entry_outbox_id)
    WHERE entry_outbox_id IS NOT NULL;
