-- Old writers retain ACTIVITY as their default. Controls have no worker notification.
ALTER TABLE collection_iterations
    ADD COLUMN entry_dispatch text NOT NULL DEFAULT 'ACTIVITY',
    ALTER COLUMN activity DROP NOT NULL,
    ALTER COLUMN outbox_id DROP NOT NULL,
    ADD CONSTRAINT collection_iterations_entry_dispatch CHECK (
        (entry_dispatch = 'ACTIVITY' AND activity IS NOT NULL AND outbox_id IS NOT NULL)
        OR (entry_dispatch = 'SCHEDULER' AND activity IS NULL AND outbox_id IS NULL)
    );
