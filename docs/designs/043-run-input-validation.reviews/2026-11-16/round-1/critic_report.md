# egm-review Stage 7 (Critic Review), round 1

Doc: docs/designs/043-run-input-validation.md
Reviewed commit: f562614874016262dd9812e241f734038d4d70de
Stage reviewers: egm-design and egm-structural (feature-spec-standard), egm-style, on FeatureRange 1-852
Note: condensed by the author; every finding and severity is kept, numbered across the report.

## egm-design

1. Major (110-116, 741-745, `contracts/errors.md:21`): the contract still promises graded disclosure for repeats.
2. Major (170-180, 340-342, 376-385, 474-479, 776): `report` still refuses the transient cases; CUJ 5 says starts are
   admitted again; a schema whose compile panics cannot start in either mode and has no operator escape.
3. Major (288-290, 453-456, 531-534, 543-571, 237-239): the charge schedule and the budget decide final outcomes and
   publication, so they are a compatibility contract under principle IV; replicas of two releases could disagree.
4. Major (72-73, 224-234, 246-251, 465-473, 573-583): scaling limits are not stated (four fixed permits, waits in engine
   passes, 256 entries across tenants, G043-006).
5. Major (49, 292-303, 793, 809-810): row II "Pass" rests on ADR-0092 (Proposed); no fallback if refused; "neither/both"
   with three items.
6. Major (237-239, 757-765, 786): 0.2.x reading the new values (an engine's schedule path on `INPUT_INVALID`,
   deliveries without a run, new stop codes) is not examined.
7. Major (298-300, 746-749, 774): the tuple-schema mitigation is overstated; such versions cannot start in `enforce`,
   and recording the requirement makes it permanent.
8. Nit (48, 383): register range.
9. Nit (443-449): unwired `ScheduleService`: skipped or refused?
10. Nit (396, 653-654): row 12 reads under the lock and checks after the commit; say so and name the fence.
11. Nit (604, 376): heading "a caller waits" includes the batch item and the pending reset.
12. Nit (688-690, 443-448, 776): `deadline` lumps three causes; no alert guidance.
13. Nit (116-118, 181-186, 245): SC-043-002 vs row 10; FR-043-003's acceptance suite.
14. Nit (717): the CLI help still says the owner role; the variable's help names only publication.
15. Nit (336-347, 756-761): rerun the inventory before going back to `enforce`; unmigrated database.
16. Nit (167-168, 368): one schema for manual and event starts needs an `anyOf` of both shapes; say so.

## egm-structural

17. Major (236, 277, 106, 500, 796): "no reference is resolved" contradicts the local `$ref` resolution.
18. Major (473, 527, 571, 580-581, Appendix A): load-bearing content sits in an "informative" appendix outside the
    FeatureRange.
19. Nit (25-49, 294): the Overview carries history; duplicated in "Owner decisions needed".
20. Nit (48, 383): register range.
21. Nit (376-385): the list of paths not found on `develop` is buried in the transient paragraph.
22. Nit (53-90, 355-356, 82-83, 612-613, 665): references that cannot be located (crate paths, missing directories).
23. Nit (405-432): the code fence has no info string.
24. Nit (486-600 vs 434-482, 604-658): sub-topic formatting differs between sections.
25. Nit (498): a 1,200-character table cell.
26. Nit (various): uneven line wrapping.
27. Nit (193-210): FR-043-016's seven rules should be a numbered list.
28. Nit (793, 810): row II should be "Pass with conditions".
29. Nit (846-849, 235-239, 255-262): NFR-043-SEC and NFR-043-COMPAT verification; user stories untraced.

## egm-style

30. Major (453-456, 814): "the counter is exact" overclaims; it is a proportional cost model.
31. Major (578-580): the heaviest-schema sentence cannot be parsed and mixes MiB and MB.
32. Major (598-600): "within a bounded time" has no number.
33. Major (193-210): FR-043-016 as a numbered list.
34. Major (112-115, 611): "whatever the caller may read" and the double negative.
35. Major (301, 781): "repeats" undefined.
36. Major (383): the paths not found on `develop` mid-paragraph.
37. Major (498): the long pattern cell.
38. Nit (25-49): the Overview history.
39. Nit (38, 40, 45): untranslated labels.
40. Nit (11-16): the thirteen paths in one sentence.
41. Nit (54): "almost nowhere".
42. Nit (63, 81, 87): rhetorical wording.
43. Nit (295-296): "neither/both" with three items.
44. Nit (136, 316, 365, 620): `pause_on_failure` vs `pauseOnFailure`.
45. Nit (609): "a with-start".
46. Nit (176, 346, 670-672): ADR-0050/ADR-0082 cited differently.
47. Nit (469): "returns soon".
48. Nit (289): the development machine not identified.
49. Nit (562): description or recommendation.
50. Nit (633-634): "but in".
51. Nit (643, 815): "miss memory" undefined.
52. Nit (377, 382): awkward word order.
53. Nit (784): "logged at every start".
54. Nit (809, 826, 837): terse judgments.
55. Nit (21, 486, 814, 824): "a validator of the platform".
56. Nit (846-849 and others): restated requirements.
57. Nit (various): uneven wrapping.

Counts: Blocker 0, Major 17, Nit 40
