-- Feature 022: the three registry checks `20270211000400_flow_control_capabilities` added `NOT VALID` are validated, each
-- under `SHARE UPDATE EXCLUSIVE`, so reads and writes go on while a table is scanned (D043-145). Nothing else is in this
-- transaction. Restamped with its migration from `20270130000700_flow_control_validate`, which validates the other checks
-- of the feature (decision D006-033 of `specs/006-sdk-multilanguage/research.md`).
ALTER TABLE serving_instances VALIDATE CONSTRAINT serving_instances_capabilities_known;
ALTER TABLE platform_requirements VALIDATE CONSTRAINT platform_requirements_capability_known;
ALTER TABLE platform_requirement_history VALIDATE CONSTRAINT platform_requirement_history_capability_known;
