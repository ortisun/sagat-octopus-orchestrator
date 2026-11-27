# egm-review Stage 7 structural lens, round 1 (2026-11-24)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 44133eb732bb25f93904c0d144a7b5af1fa26d9f
Reviewer: egm-structural, feature-spec standard, FeatureRange 1-1156
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The reviewer found the
header, the section order, the heading levels, the fences, the tables, the links and the Constitution Check prefixes in
order.

1. Blocker (1094-1110): the Constitution Check is written against 5.0.0, which is not committed; against 4.7.0 the
   Delivery Constraints row would not pass, and nothing gates the work on the amendment; evaluate against 4.7.0 with
   the amendment as the remedy, or gate Task 044-1 on 5.0.0.
2. Major (18-20, 95, 293, 315, 336-340, 1096, 1140): owner decisions 2026-11-24-1 to -4 have no row in
   `docs/owner-decisions.md` on this branch; "Owner decisions needed: None" is unsupported; no task has an `Owner-gate:`.
3. Major (169-173): Requirement 044-011 cites Task 044-15 to Task 044-20; it should be Task 044-10 and Task 044-15 to
   Task 044-19.
4. Major (768-795): the numbered list mixes in-memory checks with statements and exceeds 12.
5. Major (174-180, 742-766, 1000): the command set disagrees across sections (search attributes); the event
   `search_attributes.upserted` is neither existing nor listed as new.
6. Major (258-297, 1148-1153): no task's `Refs:` names a non-functional requirement or a success criterion.
7. Major (403-406, 497-538, 572-581): the per-language subsections do not keep the promised facets (C#, Java, Rust lack
   isolation or detection bullets; preconditions in the table have no source text).
8. Nit (826-835, 156-160, 242-245): no catalogue of causes.
9. Nit (1148-1153): the story-to-journey mapping belongs under User stories; heading capitalisation.
10. Nit (994-1005): the interactions table is unordered; Requirement 044-029 covers five of eight.
11. Nit (31-48, 1066-1078): the Overview list duplicates the comparison table.
12. Nit (583-612): the diagram comes before the prose that defines its terms.
13. Nit: "Technical Plan 4.3" and "section 4.3" mixed.
14. Nit (629): the example digest is malformed.
15. Nit (63-79): envelope, decider, visible event and eager dispatch not in Terms.
16. Nit (140): "AGENTS rule 3" bare.
17. Nit: line 333 quotes the old owner-decision form; the template's comment is stale (separate fix).

Counts: Blocker 1, Major 6, Nit 10
