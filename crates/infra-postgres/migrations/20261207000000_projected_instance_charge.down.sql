-- Derived metadata only. The compatible previous reader computes the same predicate from steps.
DROP TRIGGER steps_instance_charge_projection ON steps;
DROP FUNCTION project_step_instance_charge();
ALTER TABLE run_instance_usage DROP CONSTRAINT run_instance_usage_charged_nonnegative;
ALTER TABLE run_instance_usage DROP COLUMN charged_instances;
