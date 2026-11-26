-- Feature 022 (task T022-116; decisions D022-136 and D022-137): the flow-control capabilities are activated by the
-- generic `control-plane capability-activate`, whose census (`platform_capability_census`, feature 006) judges that
-- every required kind has a live process listing the capability. The feature step of the removed command,
-- `flow_control_kinds_without_capable_process` of `20270211000400_flow_control_capabilities`, has no caller left and goes.
-- An operator-only function (revoked from PUBLIC); nothing else depends on it. The down creates it again.
DROP FUNCTION flow_control_kinds_without_capable_process(text, text[]);
