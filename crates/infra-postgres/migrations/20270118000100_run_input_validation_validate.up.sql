-- Design 043, second of two: the CHECKs `20270118000000_run_input_validation` added `NOT VALID` are validated, each in this
-- transaction only (`VALIDATE CONSTRAINT` takes `SHARE UPDATE EXCLUSIVE`, so reads and writes go on while a table is
-- scanned), and the old CHECKs that did not know the new values are dropped (D043-031, D043-145).

ALTER TABLE schedule_actions VALIDATE CONSTRAINT schedule_actions_outcome_known;
ALTER TABLE serving_instances VALIDATE CONSTRAINT serving_instances_capabilities_known;
ALTER TABLE platform_requirements VALIDATE CONSTRAINT platform_requirements_capability_known;
ALTER TABLE platform_requirement_history VALIDATE CONSTRAINT platform_requirement_history_capability_known;

ALTER TABLE schedule_actions DROP CONSTRAINT schedule_actions_outcome_valid;
ALTER TABLE serving_instances DROP CONSTRAINT serving_instances_capabilities_closed;
ALTER TABLE platform_requirements DROP CONSTRAINT platform_requirements_capability_valid;
ALTER TABLE platform_requirement_history DROP CONSTRAINT platform_requirement_history_capability_valid;
