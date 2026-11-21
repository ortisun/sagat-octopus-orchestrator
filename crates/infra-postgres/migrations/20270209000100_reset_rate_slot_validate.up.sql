-- Feature 022 (T022-086): the CHECK 20270209000000 restated `NOT VALID` is validated under `SHARE UPDATE EXCLUSIVE`, so
-- reads and writes of the reset register go on while it is scanned (the rule of design 043, D043-145, D043-153).
ALTER TABLE run_resets VALIDATE CONSTRAINT run_resets_failure_reason_valid;
