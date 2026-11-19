# egm-review Stage 9 (Implementation Readiness), round 2

Doc: docs/designs/043-run-input-validation.md
Reviewed commit: b9b0f1b56d121ba3f977e2cea888305402f2eb47
Stage reviewers: egm-implementation-readiness and egm-codebase-details (implementation-readiness-standard), egm-style,
on DetailedRange 853-1195
Note: condensed by the author; every finding and severity is kept, numbered across the report.

## egm-implementation-readiness

1. Major: the phase order cannot build green (T043-13 in phase 1 needs T043-12's mode type; fixtures swept in phase 2).
2. Major: `integration_run_input_cache` belongs to no task.
3. Major: T043-17's sentinel coverage of rows 2 and 13 does not exist; the first sentinel test omits the metrics text.
4. Major: T043-15 omits the SDK error-type and transport files and names no test.
5. Major: the harness wiring (`tests/support/api.rs`, `engine.rs`, `schedules.rs`, `run_input.rs`) is in no task.
6. Major: `publishing.rs` (stored versions re-parsed in `report`, D043-081), the control plane's wiring and
   `sagat workflow validate` are in no task.
7. Major: the gate's API signatures are elided (`check_schema`, `recheck`, `new`, `RunInputValidator::gate`, `Mode`).
8. to 12. Nits: follow-ups not actionable; a commit hash as a file; unnamed tests; the infra-schema commands and its
   `test-support` feature; SC-043-001 missing from the Refs of T043-5 to T043-11.

## egm-codebase-details

13. Major: T043-17, rows 2 and 13 have no sentinel.
14. Major: `integration_durable_execution` does not cover row 2.
15. Major: `publishing.rs` and the CLI's `workflow validate` missing from Files.
16. Major: T043-15 misses the SDK error-type files and names no test.
17. Major: the test support modules and the cache target belong to no task.
18. to 23. Nits: the second down migration is `SELECT 1`; `SchemaKey` is not re-exported; "one gate per process" is
    one validator; `crates/infra-schema/src/lib.rs` missing from T043-5; test locators; the Verification Plan's command
    forms.

## egm-style

24. Major: the tasks are imperative though the introduction says they record what was built.
25. Major: T043-4's sentence does not parse.
26. Major: T043-11's two migrations and four files in one nested sentence.
27. Major: "no room", "no permit", `Busy`, "transient" for one state.
28. Major: "as its rows need" is no selection rule.
29. Major: T043-2 and others describe tests without naming them.
30. to 43. Nits: punctuation, "twins", anthropomorphisms, synonym drift, wrapping, "two spans", "register it",
    "budget" as a verb, "the warning", the commit hash, Refs order, heading levels, T043-26's wording, the failure-case
    list.

Counts: Blocker 0, Major 18, Nit 25
