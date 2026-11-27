# egm-review Stage 8 (Intermediate Zoom Out Review), round 3 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 2249e90100039f5c012b6217e4e5bb454ace1296
Reviewer: egm-zoom-out, zoom-out standard, full file
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The reviewer found that
round 1's answers to zoom-out findings 2, 3, 5, 6, 8 to 12, 14 and 16 hold, and did not raise the settled finding 7 again.

1. Major (728-738, 350-354, 1439, 1575-1586): about 30 production call sites read `WorkflowDefinition::node` and
   `entrypoint` from the graph directly; the audit of `node_for_instance` alone misses them. Make the run's node source
   one type all lookups go through, so a missed path is a type error.
2. Major (1253-1266, 1385, 1480-1488): `code_saga_plan_entries` reinvents `run_sagas` and `run_saga_entries` (a run-scoped
   plan with `after`) and feature 014's `scope_sagas`; its `status` duplicates the compensation steps. Reuse them.
3. Major (1295-1304, 373-376, 1083-1084, 1096-1098, 1346, 1389, 1608-1611): sticky exclusivity is an unmeasured
   optimisation in the core slice with five writers of one routing string; "another session first" has no mechanism.
   Keep the worker-local cache in the core; move exclusivity and its pieces to the deferrable task.
4. Major (1534-1539 and the requirements they serve): the deferrable list conflicts with the release criteria (worker
   queries and patches in Success criterion 044-001; the abandoned cap and the pause in deferrable tasks; Task 044-25
   deferrable but a release gate; the 5 commits assume eager dispatch). Name what slips with each deferrable task.
5. Major (128, 248-255, 1179-1191, 1450, 1702-1712, 1780-1787): the core slice depends on the inbox of Architecture
   decision 0111 (Proposed, another branch) without saying who builds it or where it sits in the build order.
6. Nit (1217-1223, 1386, 1399, 1553-1573): core tasks build parts of the deferrable worker queries.
7. Nit (1136-1158, 1387, 1391): `MOVE_CODE_VERSION` beside `REPIN` is not distinguished; retries and verdicts duplicate
   `batch_items`; model the single-run move as a one-item job.
8. Nit (728-738, 978, 1384, 1389): `code_step_nodes` returns after round 1 said a per-node table was dropped; the fragment
   digest is stored twice; why derived companions are frozen.
9. Nit (358-361, 950-951, 1204-1207): the wait schedule, the type-name pattern and the budget arithmetic are below design
   altitude.

Counts: Blocker 0, Major 5, Nit 4
