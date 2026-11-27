-- Feature 022: the CHECKs the flow-control migrations added `NOT VALID` are validated, each under `SHARE UPDATE EXCLUSIVE`,
-- so reads and writes go on while a table is scanned (the rule of design 043, D043-145, D043-153). Nothing else is in this
-- transaction. `steps_waiting_kind_valid` is left as every earlier redefinition of it left it. The registry's capability checks
-- are validated by `20270211000500_flow_control_capabilities_validate`, restamped with their migration (D006-033).
ALTER TABLE quota_ceilings VALIDATE CONSTRAINT quota_ceilings_resource_valid;
ALTER TABLE quota_limits VALIDATE CONSTRAINT quota_limits_resource_valid;
ALTER TABLE quota_limits VALIDATE CONSTRAINT quota_limits_tenant_scoped;
ALTER TABLE flow_control_key_versions VALIDATE CONSTRAINT flow_control_key_versions_role_bounded;
ALTER TABLE steps VALIDATE CONSTRAINT steps_flow_control_gates_bounded;
ALTER TABLE steps VALIDATE CONSTRAINT steps_gate_wait_projected;
