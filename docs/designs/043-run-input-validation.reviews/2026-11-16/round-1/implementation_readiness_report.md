# egm-review Stage 9 (Implementation Readiness), round 1

Doc: docs/designs/043-run-input-validation.md
Reviewed commit: f562614874016262dd9812e241f734038d4d70de
Stage reviewers: egm-implementation-readiness and egm-codebase-details (implementation-readiness-standard), egm-style,
on DetailedRange 853-1147
Note: condensed by the author; every finding and severity is kept, numbered across the report.

## egm-implementation-readiness

1. Major: T043-3 "tokio stays a dev-dependency" contradicts the plan.
2. Major: T043-4 phasing and files (creators outside `runs.rs`, the four builders, `RunInputChecker`'s removal window,
   T043-20's order).
3. Major: T043-5 asks for graded disclosure.
4. Major: T043-7 names the wrong file and no cron store method.
5. Major: T043-10 names `target_check`, not `target_version`, nor `with_run_inputs`.
6. Major: T043-11 names one migration; files, the static test and the down refusals missing.
7. Major: T043-12 before T043-11 would fail to register.
8. Major: T043-13 lists none of the files that carry the flag and the mode.
9. Major: T043-24 owner role against the runtime role; the read joins `workflows`.
10. Major: the `report` audit writes have no task.
11. Major: the REST `Run.failureCode` has no server or Studio task.
12. Major: T043-21's permit tests belong to the adapter's real queue, not a double.
13. Major: FR-043-011's per-row acceptance is unassigned.
14. Major: T043-2 leaves the regex dependency undecided and omits module files.
15. to 22. Nits: phase list; T043-12 file names; T043-19 one file; T043-24 test locations; T043-6 Refs; T043-15/16
    paths; the verification plan's rows and fixture command; cache-share tests.

## egm-codebase-details

23. Major: the plan does not say what is built.
24. Blocker: T043-3 "tokio stays a dev-dependency" is false.
25. Major: T043-2/T043-3 boundary and file lists, dependencies.
26. Major: T043-4 and T043-9 anchors and test paths.
27. Major: T043-12 files and tests.
28. Major: T043-13 files.
29. Major: T043-16 files.
30. Major: T043-17 and the labels test.
31. Major: T043-21 has no barrier double; `permits.rs` unlisted.
32. Major: T043-24 test paths.
33. Major: verification plan misses `integration_run_input_cache`, `contract_payload_classification`,
    `contract_iceberg_writes`.
34. Major: the server's `failureCode` is in no task.
35. Major: the ordering paragraph against the phases.
36. Blocker: T043-14 says `workflow.schema.json` is still `{"type": "object"}`.
37. Major: T043-11 one migration pair.
38. to 42. Nits: `sdks/SPEC.md` line; T043-19 file; Appendix A's assertion wording; the application README cites
    `RunInputChecker`; the cache is generic over failure and has a share.

## egm-style

43. Major: the ordering paragraph and T043-24's place.
44. Major: T043-2 is one 28-line sentence.
45. Major: `pattern.rs` missing from T043-2.
46. Major: T043-3 and T043-12 both build "the mode".
47. Major: T043-21's grammar.
48. Major: "among others" in T043-4.
49. to 67. Nits: path choices, informal wording, duplicated words, heading level, wrapping, Refs order, slashes,
    "strict", the fixture description.

Counts: Blocker 2, Major 33, Nit 32
