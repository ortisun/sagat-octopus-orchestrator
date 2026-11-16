-- Rollback is an operator/quiesced-service operation. A permit, a waiter, a recorded key version or a journal that
-- holds a concurrency event says what the engine of this version understood: rolling back never destroys any of them,
-- so it refuses while one exists (drain the keys, then retire nothing: a deployment that activated a key version keeps
-- this schema).
LOCK TABLE concurrency_semaphores, concurrency_holders, concurrency_waiters, flow_control_key_versions, run_events
    IN SHARE ROW EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM concurrency_semaphores)
       OR EXISTS (SELECT 1 FROM concurrency_holders)
       OR EXISTS (SELECT 1 FROM concurrency_waiters)
       OR EXISTS (SELECT 1 FROM flow_control_key_versions)
       OR EXISTS (SELECT 1 FROM run_events WHERE event_type LIKE 'concurrency.%') THEN
        RAISE EXCEPTION 'concurrency permits, waiters, key versions or their journal events are retained; keep a compatible reader'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

DROP FUNCTION flow_control_key_activate(smallint, text);
DROP TABLE concurrency_waiters;
DROP TABLE concurrency_holders;
DROP TABLE concurrency_semaphores;
DROP FUNCTION concurrency_rows_immutable();
DROP FUNCTION concurrency_semaphore_count();
DROP TABLE flow_control_key_versions;
