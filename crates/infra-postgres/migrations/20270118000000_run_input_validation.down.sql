-- Rollback of the first half: the new CHECKs are dropped; the old ones were never removed by this migration.
ALTER TABLE schedule_actions DROP CONSTRAINT schedule_actions_outcome_known;
ALTER TABLE serving_instances DROP CONSTRAINT serving_instances_capabilities_known;
ALTER TABLE platform_requirements DROP CONSTRAINT platform_requirements_capability_known;
ALTER TABLE platform_requirement_history DROP CONSTRAINT platform_requirement_history_capability_known;
