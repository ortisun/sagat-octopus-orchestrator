-- Rollback is an operator/quiesced-service operation. A requirement that is recorded, and the history of every
-- activation and removal, are evidence of what the fleet was asked to honour: rolling back never destroys them. A
-- process that is live (seen within 30 seconds) is using the registry: rolling the schema back under it would make
-- its heartbeat fail, so stop the processes first. Rows of processes that are gone are the registry's own bookkeeping
-- and go with it.
LOCK TABLE serving_instances, platform_requirements, platform_requirement_epoch, platform_requirement_history
    IN ACCESS EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM platform_requirements)
        OR EXISTS (SELECT 1 FROM platform_requirement_history) THEN
        RAISE EXCEPTION 'platform requirements, or the history of their activations, are retained; keep a compatible reader'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
    IF EXISTS (SELECT 1 FROM serving_instances WHERE last_seen > clock_timestamp() - interval '30 seconds') THEN
        RAISE EXCEPTION 'processes are registered and live; stop them before rolling the registry back'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

DROP FUNCTION platform_requirement_remove(text, text);
DROP FUNCTION platform_requirement_activate(text, text[], text);
DROP FUNCTION platform_requirement_lock();
DROP FUNCTION instance_deregister(uuid, bytea);
DROP FUNCTION instance_heartbeat(uuid, bytea);
DROP FUNCTION instance_register(uuid, bytea, text, text, text, text[]);

DROP TABLE platform_requirement_history;
DROP TABLE platform_requirements;
DROP TABLE platform_requirement_epoch;
DROP TABLE serving_instances;

DROP FUNCTION platform_requirements_epoch_moved();
DROP FUNCTION platform_requirement_epoch_guard();
DROP FUNCTION platform_requirements_guard();
