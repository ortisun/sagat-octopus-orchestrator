# Review: feature 022, 0.3.0 slice, second build — security

- **Change:** `feature/022-throughput-control`, commits `fe8b2d57..162ec3aa` (T022-002, T022-013, T022-069, T022-095)
- **Reviewer:** `security-reviewer` (independent read-only agent, given the changed files and the spec, not the author's
  conclusions), 2026-11-16
- **Recorded by:** the author, from the reviewer's report. No Blocker; 2 Major, 3 minor, 2 nits. The reviewer ran nothing;
  each answer below names the test that now shows it.

| Finding | Severity | Answer |
|---|---|---|
| S1 — a rotation can be held open indefinitely by any tenant's long-lived permit, so an emergency rotation of a leaked key (a third version) is refused and the retiring key must stay configured | Major | Not decided by the author: a product and security trade-off. ADR-0093 (proposed) states it as a consequence, and O1170 asks the owner what ends such a rotation (options and a recommendation in `research.md`); T022-095 is not ticked until answered |
| S2 — nothing enforces a single RETIRING version; the earlier command left every replaced version RETIRING, and `SELECT … INTO` would take any of them, which could split a key | Major (no such database exists outside tests: no build ran the earlier command in an environment) | Fixed: the migration retires what the earlier command left, and a unique index keeps at most one RETIRING. Tests `migrations::the_rotation_migration_retires_what_the_old_command_left_and_its_down_refuses_during_a_rotation` (two RETIRING rows made by the earlier command, retired by the migration) and the `23505` of a second RETIRING row in `a_rotation_runs_with_live_rows_and_retires_the_old_version_with_its_last_row` |
| S3 — the declarations table does not tie namespace, workflow and version together, and its rows are immutable | minor | Fixed: foreign keys `(tenant, namespace, workflow)` → `workflows` and `(tenant, workflow, version)` → `workflow_versions`; the read joins on the namespace too |
| S4 — the boundary test cannot catch a missing namespace filter (only another tenant was tried) | minor | Fixed: `publication::the_warning_reads_this_namespace_and_the_active_versions_only` (another namespace of the same tenant; an inactive version); red with the namespace filter removed and with the active-version join removed |
| S5 — re-activating an older version bypasses a lowered node-declarations ceiling | minor | Decided (D022-043): a publication-time policy, like the `workflows` quota; lowering a limit never revokes what is published (FR-022-021). Recorded, not changed |
| N1 — `FiledKey` prints the full digest | nit | Fixed: its `Debug` shows the label only |
| N2 — rolling back the rotation migration loses `activated_role` and restores a command that could re-activate an unused RETIRING version | nit | Partly fixed: the down migration retires every unused RETIRING version before restoring the earlier command. The role column goes with the rollback, as any column a down migration removes |
| Suspicion — lock order when a retiring row vanishes | — | Confirmed by the correctness review (F1); fixed, see there |
| Suspicion — collation of `group_name` between Rust and SQL | — | Recorded (D022-043): SQL paths that lock the same rows sort `COLLATE "C"` |
| Spec drift — FR-022-020 and G1163 still describe the drain-first rotation | — | G1195: approved texts change with the owner's answer to O1170 / ADR-0093 |
