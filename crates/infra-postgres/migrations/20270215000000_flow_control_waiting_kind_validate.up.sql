-- Feature 022, after the merge with `develop` (decision D022-089): `steps_waiting_kind_valid`, which
-- 20270130000600_flow_control_step_gates restated `NOT VALID` with `CONCURRENCY` and `RATE_LIMIT`, is validated under
-- `SHARE UPDATE EXCLUSIVE`, so reads and writes of `steps` go on while it is scanned. 20270130000700 left it as every
-- earlier restatement had left it; design 043's rule, as `develop` now checks it
-- (`migrations::a_constraint_added_not_valid_is_validated_by_a_later_migration`, integration_platform_foundation), wants
-- every CHECK added `NOT VALID` from 20270118000000 on validated by a later migration. Every restatement of the list,
-- from the table's creation on, only added kinds, so every stored row satisfies it. Nothing else is in this transaction
-- (design 043, D043-145, D043-153).
ALTER TABLE steps VALIDATE CONSTRAINT steps_waiting_kind_valid;
