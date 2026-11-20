-- Feature 022 (T022-022): validates the CHECK `20270203000000_flow_control_run_gates` added `NOT VALID` (D043-145): the
-- scan takes `SHARE UPDATE EXCLUSIVE`, so writes to `runs` go on while it runs.
ALTER TABLE runs VALIDATE CONSTRAINT runs_flow_control_gates_bounded;
