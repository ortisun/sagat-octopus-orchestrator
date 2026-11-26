# egm-review Stage 5 (Initial Zoom Out Review), round 1 (2026-11-24)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 44133eb732bb25f93904c0d144a7b5af1fa26d9f
Reviewer: egm-zoom-out, zoom-out standard, full file
Note: condensed by the author from the reviewer's output; every finding and severity is kept.

1. Major (lines 141-142, 270-271, 768-791): the statement list of the completion adds to 13 when four projections are
   touched, and eager leases are "folded" into two statements without proof; state the bound at the right height.
2. Major (156-160, 826-841, 1037): the hold, the `HELD` kind, `workflow_task.held/released` and the retry route
   duplicate the step review machinery (`mark_for_review`, `step.needs_review`, `ReviewAction::Hold/Retry`, batch
   `RETRY_STEP`); reuse it or justify the difference.
3. Major (693-695, 1012, 1040-1044): a second claim path, `worker_session_workflow_types` and a second ready index,
   where the claim already routes by `steps.activity = ANY(session.activities)` with `steps_ready_idx`, hints and the
   reconciler; encode (type, digest) in the activity name.
4. Major (744, 780, 1038-1039, 1107, 1290): `code_nodes` repeats `steps` and `step.released`; the graph route draws from
   the step snapshot; no named reader.
5. Major (366, 863-875, 1042, 1123): moves should be a batch action on feature 018's `batch_jobs` and `batch_items`
   (which already has `REPIN`), not a bespoke route and `code_move_items`.
6. Major (239-241, 687, 887-888, 1244): the promotion replay check predicts only whether a later move would match; a
   dry-run move and the CI replayer already answer it; drop or describe as a dry-run move over a sample.
7. Major (201-204, 920-925, 1013, 1016, 1042, 1348): worker queries are speculative scope (no story or journey needs
   them); ship declared queries only, reserve the task kind.
8. Major (217-221, 264-269, 790-791, 986-992, 1021-1022, 1246-1250): eager dispatch is an optimisation built before
   measurement; deliver the non-eager path, state 7 commits, add eager as a follow-up after Task 044-25.
9. Major (356-361, 663-680, 1085-1087): whole-binary digests make every compiled-language deploy a new version; say how
   it stays cheap: a separate workflow worker binary as the recommended topology, one explicit CLI step per deploy,
   journey 2 restated for a compiled language.
10. Major (236-238, 883-886, 994-1005): history export "payloads as stored" bypasses feature 016's reveal grants and
    ADR-0031; route it through the existing payload read path; add feature 016 to the interactions.
11. Major (338-340, 1094-1110, 1159-1164): the Constitution Check rests on 5.0.0, not on disk; add an entry gate that
    the amendment (with Gap 044-010's text) is merged before code.
12. Nit (556-562, 734, 1275-1276): behaviour flags have no use in 0.3.0; reserve the field, defer the mechanism.
13. Nit (631, 659, 810-811): `matching: strict` has no story; ship `standard`, register `strict` as a gap.
14. Nit (755, 1000): the search-attribute command can only be refused until feature 020; add it with feature 020.
15. Nit (407-410, 440-442, 449-450, 483-485, 512-516): spike numbers repeated from Decision 044-041; cite it.
16. Nit (1157-1332): no cut line in 28 tasks; mark a core slice and the deferrable tasks.

Counts: Blocker 0, Major 11, Nit 5
