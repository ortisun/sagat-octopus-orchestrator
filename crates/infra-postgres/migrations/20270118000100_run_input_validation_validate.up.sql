-- Design 043, second of two: the CHECKs `20270118000000_run_input_validation` added `NOT VALID` are validated, each under
-- `SHARE UPDATE EXCLUSIVE`, so reads and writes go on while a table is scanned. Nothing else is in this transaction
-- (D043-145, D043-153).

ALTER TABLE schedule_actions VALIDATE CONSTRAINT schedule_actions_outcome_known;
ALTER TABLE serving_instances VALIDATE CONSTRAINT serving_instances_capabilities_known;
ALTER TABLE platform_requirements VALIDATE CONSTRAINT platform_requirements_capability_known;
ALTER TABLE platform_requirement_history VALIDATE CONSTRAINT platform_requirement_history_capability_known;
