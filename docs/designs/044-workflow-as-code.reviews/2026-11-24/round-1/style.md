# egm-review style lens, round 1 (2026-11-24)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 44133eb732bb25f93904c0d144a7b5af1fa26d9f
Reviewer: egm-style (stages 7 and 9, one run over the whole file)
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The reviewer read the
working tree, which was clean at that commit, and had no git access.

## FeatureRange (lines 1 to 1156)

1. Major: the old form `ADR-NNNN` in prose and link text; new prose uses "Architecture decision NNNN".
2. Major: "(FR-022-017)" in a table cell; write "Requirement 022-017".
3. Major: "CUJ" is a coined abbreviation used about 20 times; write "journey".
4. Major: "where Temporal's users suffer most, and the problems are well known" is unsourced.
5. Major: "continue them as graph workflows (none exists automatically)" is unclear.
6. Nit: subjective words ("cannot express well", "write exactly this", "surprise people late", "a better answer").
7. Nit: "first-class model" is jargon.
8. Nit: the owner's Portuguese words have no translation.
9. Nit: IR, CEL, CI, I/O not spelled out at first use; "WASM" beside "WebAssembly".
10. Nit: the term "Hold" is defined as a run, used as a state.
11. Nit: "AGENTS rule 3" is clipped.
12. Nit: "Task 044-15 to Task 044-20" is the wrong range for the runtimes.
13. Nit: bare feature numbers ("(022)", "019 adds").
14. Nit: the quoted "delegate (OD-2026-11-20-6)" brings back the old form.
15. Nit: "simply" is filler.
16. Nit: "strongest" and "weakest of the six" without a criterion.
17. Nit: "release 17" does not say Java.
18. Nit: British and American spelling mixed.
19. Nit: "id" and "ids" in prose.
20. Nit: "What it adds ... is small" is a judgment.
21. Nit: "the new minor" is clipped.
22. Nit: the hashed object mixes prose words and member names.
23. Nit: "held for activation k + 1" reuses the term Hold.
24. Nit: "what the engine does better than code" is subjective.
25. Nit: "ADR-0109" placed after the feature 006 clause reads as feature 006's decision.
26. Nit: bare section reference "(5.3)".
27. Nit: vague rejection reasons in Complexity Tracking and Alternatives.
28. Nit: "strict and honest" for a digest.
29. Nit: numbers without their kind in Non-Functional Requirements.

## DetailedRange (lines 1157 to 1369)

30. Nit: bare "5.3".
31. Nit: "the four lock files moved together" unnamed.
32. Nit: bare feature numbers in Task 044-27 and the build order.
33. Nit: the build order drops "Task".
34. Nit: "this machine" and "the rule of two heavy builds" without a source.
35. Nit: "an exception holds it after five tries" differs from Requirement 044-008.
36. Nit: "the completion held by a barrier" reuses Hold.
37. Nit: "during 100 open runs" differs from Success criterion 044-002.

Counts: Blocker 0, Major 5, Nit 32
