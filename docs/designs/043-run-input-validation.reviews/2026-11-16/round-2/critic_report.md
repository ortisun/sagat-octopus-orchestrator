# egm-review Stage 7 (Critic Review), round 2

Doc: docs/designs/043-run-input-validation.md
Reviewed commit: b9b0f1b56d121ba3f977e2cea888305402f2eb47
Stage reviewers: egm-design and egm-structural (feature-spec-standard), egm-style, on FeatureRange 1-852
Note: condensed by the author; every finding and severity is kept, numbered across the report.

## egm-design

1. Major: the inventory's exit 0 is unreachable once any immutable stored version is outside the subset.
2. Major: the tenant share of the cache is not work-conserving; a lone tenant with more than 64 hot schemas thrashes.
3. Major: the pre-check outside a start waits unbounded and unfairly, and is described as the start pool.
4. Major: inputs inside the subset that fail their schema are not visible before the upgrade, yet the Risks table cites
   the inventory.
5. Major: an engine pass can stall on permits, head of line; unbounded.
6. to 13. Nits: FR-043-011's rows; `INVALID_DEFINITION`; `NOMINAL_TIME_FIELD_INVALID` mapping; `ScheduleService`;
   "never inside a transaction" untested; a wrong digest can admit; a deterministic panic retried forever; `workLimit`'s
   remediation.

## egm-structural

14. Major: "Appendix A" cited at line 517 does not exist.
15. to 23. Nits: row 12's fragment; the table's file-and-function convention; FR-043-016's acceptance placement; two
    wrap defects; long lines; FR-043-011's rows; row II's Result cell; OD-2026-11-14-3 not listed; user-story mapping
    in the wrong section.

## egm-style

24. Major: FR-043-006, -007, -011, -013 pack many obligations into one paragraph.
25. Major: NFR-043-BOUND is one sentence without units.
26. Major: row 12's dangling "input at".
27. Major: the `deadline` outcome covers a panic.
28. Major: the validator has three names.
29. Major: "answer" and "room" used loosely.
30. to 46. Nits: wrapping; repeated review counts; repeated 7.5 ms; vague capacity; "nowhere"; "a creator nobody
    waits for" undefined; titles and antecedents; awkward phrases; a long paragraph; vague "warrants"; fragments;
    headings; unclear references.

Counts: Blocker 0, Major 12, Nit 34
