-- Feature 006, second of two: the checks `20270208000000_lease_withdrawal` added `NOT VALID` are validated, each under
-- `SHARE UPDATE EXCLUSIVE`, so reads and writes go on while a table is scanned. Nothing else is in this transaction.

ALTER TABLE serving_instances VALIDATE CONSTRAINT serving_instances_capabilities_known;
ALTER TABLE platform_requirements VALIDATE CONSTRAINT platform_requirements_capability_known;
ALTER TABLE platform_requirement_history VALIDATE CONSTRAINT platform_requirement_history_capability_known;
