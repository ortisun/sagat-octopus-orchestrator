-- Undoes 20270215000000_quota_resources_of_flow_control_and_step_logs.up.sql by keeping what it wrote. The lists in
-- force before it were those of 20270206000000, which on the merged tree lack the step-log names of 20270201000000, a
-- migration still applied after this rollback: restoring them would refuse the step-log limits and ceilings that
-- migration allows (decision D022-089). The union stays; the downs of 20270206000000 and 20270201000000 each take their
-- own names out, and each refuses while a limit or a ceiling of its names is stored.
SELECT 1;
