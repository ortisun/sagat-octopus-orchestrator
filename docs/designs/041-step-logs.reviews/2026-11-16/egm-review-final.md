# egm-review of design 041 on the owner's approval path (summary)

Doc: docs/designs/041-step-logs.md
Reviewed-commit: f9d4a65fa278e1c782a59e3d15adadd6e764371a
Reviewer: egm-review
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Claude-Code: 2.1.325
Note: the author's summary for the approval record (docs/designs/constitution-4.0.0-egm.md, section 3). The last complete run of the five egm-review stages is the third, on `6dceb5482d1a5a27b22be4018ed2df83e106137b`, run in order with fresh agents of the registered types (stage reports `../2026-11-15/egm-stages-5-7-approval-3.md`, `egm-stage-8-approval-3.md`, `egm-stage-9-approval-3.md`; summary `egm-review-approval-3.md`). The owner then chose the approval path "Corrigir e aceitar o resto" (OD-2026-11-16-7): fix the Blocker and the defects of resolution.md section 11, apply the answers OD-2026-11-16-8 to -10, have the three repository reviewers re-read the fixes, and run no further egm-review round. This summary carries that run's findings and counts unchanged into the text it approves, `f9d4a65f`, which differs from `6dceb548` by those fixes (resolution.md sections 12 and after). No EGM stage read `f9d4a65f`; the repository reviewers' re-reads did (`architecture-reread-3.md`, `security-reread-3.md`, `correctness-reread-3.md`). Each Blocker and Major below is settled by one owner-decision row, OD-2026-11-16-200 to OD-2026-11-16-262, citing `#<n>`; the disposition says whether it was fixed on the owner's path or accepted as is.

| Stage | Blocker | Major | Nit |
| --- | ---: | ---: | ---: |
| 5 zoom-out | 0 | 4 | 5 |
| 6 comprehension | 0 | 2 | 12 |
| 7 design, structural, style | 0 | 16 | 42 |
| 8 intermediate zoom-out | 0 | 14 | 21 |
| 9 readiness, codebase details, style | 1 | 26 | 38 |

## The Blocker and the Majors, numbered

1. stage 5 Z3-1: four namespace-tunable ceilings and the capture switch as a ceiling of 0. Disposition: kept as the owner decided: "Configuráveis", OD-2026-11-16-9; accepted as is; owner row OD-2026-11-16-200.
2. stage 5 Z3-2: the exact run ceiling (a table, a lock, two owner questions). Disposition: fixed: no run ceiling, "Retirar", OD-2026-11-16-10; owner row OD-2026-11-16-201.
3. stage 5 Z3-3: the script marker inside record_execution's transaction. Disposition: fixed: the marker in its own transaction after the report commits (Q4); owner row OD-2026-11-16-202.
4. stage 5 Z3-4: the Technical Plan written at statement level. Disposition: accepted as is (F15; stage 8 rejected it as a Major); owner row OD-2026-11-16-203.
5. stage 6 C3-1: ADR-0085 describes mechanisms the design dropped. Disposition: fixed: a Note in ADR-0085 (Q8); owner row OD-2026-11-16-204.
6. stage 6 C3-2: the state of a script item with no marker stream. Disposition: fixed: the state comes from the execution (F5); owner row OD-2026-11-16-205.
7. stage 7 K3-1: the ingest's secrets lock delays leases, heartbeats and database admission. Disposition: fixed: binding and membership read without a lock (Q5); owner row OD-2026-11-16-206.
8. stage 7 K3-2: one tenant can fill the Logs class. Disposition: fixed: no call waits for a ticket (F1); owner row OD-2026-11-16-207.
9. stage 7 K3-3: redaction cost on the activity's thread. Disposition: fixed: a doomed record is dropped before formatting (F6); owner row OD-2026-11-16-208.
10. stage 7 K3-4: ingest overload slows execution. Disposition: fixed: a risk row and the spike's saturated workload (F7); owner row OD-2026-11-16-209.
11. stage 7 K3-5: tenant quota sizing. Disposition: fixed: a risk row, runbook sizing, explicit quota in the benchmarks (F7); owner row OD-2026-11-16-210.
12. stage 7 K3-6: O041-025 (a) wider than the report's stop rule. Disposition: fixed: O041-025 restated to the code's rule (Q6); owner row OD-2026-11-16-211.
13. stage 7 K3-7: ADR-0085 text against the design. Disposition: fixed: a Note in ADR-0085 (Q8); owner row OD-2026-11-16-212.
14. stage 7 K3-8: a step's view scans the whole run. Disposition: fixed: a stream index and the Studio's paging (F4); owner row OD-2026-11-16-213.
15. stage 7 S3-1: Constitution Check V names the rejected trigger. Disposition: fixed; owner row OD-2026-11-16-214.
16. stage 7 S3-2: G041-017 missing from the Constitution Check. Disposition: fixed: G041-017 accepted and listed (Q7); owner row OD-2026-11-16-215.
17. stage 7 T3-1: section 10's switch sentence never says "when false". Disposition: fixed (F15); owner row OD-2026-11-16-216.
18. stage 7 T3-2: Constitution V "trigger". Disposition: fixed; owner row OD-2026-11-16-217.
19. stage 7 T3-3: Constitution VIII gap range. Disposition: fixed; owner row OD-2026-11-16-218.
20. stage 7 T3-4: O041-025 does not say what the ADR says now. Disposition: fixed: O041-025 restated (Q6); owner row OD-2026-11-16-219.
21. stage 7 T3-5: FR-041-006 in one paragraph. Disposition: fixed: split into a list (F15); owner row OD-2026-11-16-220.
22. stage 7 T3-6: the Overview's last sentence. Disposition: fixed (F15); owner row OD-2026-11-16-221.
23. stage 8 merge K3-1: the secrets lock held by the ingest. Disposition: fixed (Q5); owner row OD-2026-11-16-222.
24. stage 8 merge K3-2: the final batch's wait for a ticket. Disposition: fixed (F1); owner row OD-2026-11-16-223.
25. stage 8 merge K3-6 and T3-4: O041-025's predicate. Disposition: fixed (Q6); owner row OD-2026-11-16-224.
26. stage 8 merge K3-7, C3-1, S3-1, T3-2: ADR-0085 and Constitution V. Disposition: fixed (Q8); owner row OD-2026-11-16-225.
27. stage 8 merge S3-2 and T3-3: G041-017 for written acceptance. Disposition: fixed: accepted by the owner (Q7); owner row OD-2026-11-16-226.
28. stage 8 merge C3-2: a script item's state from the execution. Disposition: fixed (F5); owner row OD-2026-11-16-227.
29. stage 8 merge K3-8: a stream index and empty-page paging. Disposition: fixed (F4); owner row OD-2026-11-16-228.
30. stage 8 merge Z3-2: the exact run ceiling. Disposition: fixed: removed, OD-2026-11-16-10; owner row OD-2026-11-16-229.
31. stage 8 merge Z3-3: the marker in its own transaction. Disposition: fixed (Q4); owner row OD-2026-11-16-230.
32. stage 8 merge-partly Z3-1: constant attempt ceilings. Disposition: kept as the owner decided (OD-2026-11-16-9); accepted as is; owner row OD-2026-11-16-231.
33. stage 8 merge-partly K3-3: drop before formatting. Disposition: fixed (F6); owner row OD-2026-11-16-232.
34. stage 8 merge-partly K3-4: an overload risk row and workload. Disposition: fixed (F7); owner row OD-2026-11-16-233.
35. stage 8 merge-partly K3-5: quota sizing. Disposition: fixed (F7); owner row OD-2026-11-16-234.
36. stage 8 missed: the per-batch statement count. Disposition: fixed: 14 statements stated (F8); owner row OD-2026-11-16-235.
37. stage 9 IR3-1: raising the minor breaks three tests. Disposition: fixed: T041-7 lists them (F10); owner row OD-2026-11-16-236.
38. stage 9 IR3-2: CI runs one promtool file. Disposition: fixed (the Blocker); owner row OD-2026-11-16-237.
39. stage 9 IR3-3: metric ownership and definitions. Disposition: fixed (F11); owner row OD-2026-11-16-238.
40. stage 9 IR3-4: micro-benchmark harness and matcher. Disposition: fixed (F11); owner row OD-2026-11-16-239.
41. stage 9 IR3-5: the spike's decision rules. Disposition: fixed (F11); owner row OD-2026-11-16-240.
42. stage 9 IR3-6: a timing assertion in a CI suite. Disposition: fixed: moved to T041-9 (F11); owner row OD-2026-11-16-241.
43. stage 9 IR3-7: the forced database error. Disposition: fixed: a test-installed trigger (F11); owner row OD-2026-11-16-242.
44. stage 9 IR3-8: T041-9 and the private tickets. Disposition: fixed: the spike's model (F11); owner row OD-2026-11-16-243.
45. stage 9 IR3-9: T041-32's SDK paths. Disposition: fixed (F12); owner row OD-2026-11-16-244.
46. stage 9 IR3-10: no runner has an unknown-behaviour refusal. Disposition: fixed: the six unknown-op refusals (F11); owner row OD-2026-11-16-245.
47. stage 9 IR3-11: the Studio's script-execution view. Disposition: fixed: ScriptLogPanel (F11); owner row OD-2026-11-16-246.
48. stage 9 CD3-B1 (Blocker): T041-25's false claim about CI's rule tests. Disposition: fixed: the CI step runs every rule test (the Blocker); owner row OD-2026-11-16-247.
49. stage 9 CD3-1: no integration job in ci.yml. Disposition: fixed: the rust job and its condition (F13); owner row OD-2026-11-16-248.
50. stage 9 CD3-2: the digest predicate. Disposition: fixed: both keys (F13); owner row OD-2026-11-16-249.
51. stage 9 CD3-3: the migration tests' files. Disposition: fixed (F13); owner row OD-2026-11-16-250.
52. stage 9 CD3-4: T041-32's SDK path set. Disposition: fixed (F12); owner row OD-2026-11-16-251.
53. stage 9 CD3-5: eventually's 5 s bound. Disposition: fixed: eventually_within and bounded holds (F13); owner row OD-2026-11-16-252.
54. stage 9 ST3-1: owner tasks with no pull request. Disposition: fixed: documentation pull requests (F14); owner row OD-2026-11-16-253.
55. stage 9 ST3-2: no hold after T3. Disposition: fixed: six hooks (F14); owner row OD-2026-11-16-254.
56. stage 9 ST3-3: T041-6 polls reads that do not exist. Disposition: fixed: stored rows asserted by SQL (F14); owner row OD-2026-11-16-255.
57. stage 9 ST3-4: T041-10 tests reader behaviour. Disposition: fixed: moved to T041-11 (F14); owner row OD-2026-11-16-256.
58. stage 9 ST3-5: T041-28 before T041-11. Disposition: fixed: T041-28 withdrawn (F14); owner row OD-2026-11-16-257.
59. stage 9 ST3-6: T041-32's path set. Disposition: fixed (F12); owner row OD-2026-11-16-258.
60. stage 9 ST3-7: T041-3's done condition. Disposition: fixed (F14); owner row OD-2026-11-16-259.
61. stage 9 ST3-8: T041-13's Options field. Disposition: fixed: one field (F14); owner row OD-2026-11-16-260.
62. stage 9 ST3-9: the release-note line. Disposition: fixed: written by T041-29 (F14); owner row OD-2026-11-16-261.
63. stage 9 ST3-10: the canary in route bodies. Disposition: fixed: T041-11 and T041-29 (F14); owner row OD-2026-11-16-262.

The Nits are in the stage reports and do not block.

Counts: Blocker 1, Major 62, Nit 118
