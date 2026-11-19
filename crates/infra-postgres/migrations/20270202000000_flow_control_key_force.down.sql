-- Undoes 20270202000000_flow_control_key_force.up.sql. Rollback is an operator/quiesced-service operation. A forced
-- retirement is evidence of what an operator did in an incident: rolling back never destroys it, so the rollback refuses
-- while one is on the record. The versions the force retired stay RETIRED, which the earlier code reads as it is.
LOCK TABLE flow_control_key_versions, flow_control_key_forced_retirements IN ACCESS EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM flow_control_key_forced_retirements) THEN
        RAISE EXCEPTION 'a forced retirement of a digest key version is on the record; keep a compatible reader'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

DROP FUNCTION flow_control_key_version_runs(smallint);
DROP FUNCTION flow_control_key_force_retire(smallint, text, text);
DROP TABLE flow_control_key_forced_retirements;
