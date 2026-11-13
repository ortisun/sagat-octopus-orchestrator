LOCK TABLE runs IN ACCESS EXCLUSIVE MODE;
LOCK TABLE collection_iterations IN ACCESS EXCLUSIVE MODE;
LOCK TABLE run_events IN SHARE MODE;
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM collection_iterations WHERE entry_dispatch = 'SCHEDULER')
       OR EXISTS (SELECT 1 FROM run_events WHERE event_type='collection.iteration_admitted'
                  AND detail->>'entry_dispatch'='SCHEDULER') THEN
        RAISE EXCEPTION USING ERRCODE = '55000',
            MESSAGE = 'scheduler-owned collection admission evidence is retained';
    END IF;
END $$;
ALTER TABLE collection_iterations
    ALTER COLUMN activity SET NOT NULL,
    ALTER COLUMN outbox_id SET NOT NULL,
    DROP CONSTRAINT collection_iterations_entry_dispatch,
    DROP COLUMN entry_dispatch;
