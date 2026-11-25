# egm-review Stage 9 re-audit (implementation readiness), round 8 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 3884f9f3db9c7ec9c7405f2970a02985c4249711
Reviewer: egm-implementation-readiness, implementation-readiness standard, DetailedRange from the Detailed Implementation Plan to the end, code claims checked
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Of round 7's findings,
2 to 9 are closed and 1 partly closed. Every path the plan names exists except one (Nit 7).

## Major

1. Task 044-30's red test either fails to compile (breaking the whole contract target and the clippy gate until Task
   044-5) or is green at once: make it a guard with a test-local recomputation and the generator's check, and leave the
   red side to Task 044-5.
2. Tasks 044-16 and 044-17 need `golang.org/x/tools` and Roslyn, which nobody chose: pin them, place the analyzer
   project, review them as esbuild is, and update the Constitution Check.
3. What `NodeMissing` returns to a caller is undefined: state its mapping, the run's fate and the test's assertion.

## Nit

4. `plan_entries` is in `execution/sagas.rs`.
5. `get_version` is already used 17 times; pick an unused name; list the cache's files.
6. Two Task 044-4 source-text tests can pass only after Task 044-31; composing callers belong to no task.
7. The public RFC 8785 function and the `CommandDigester` implementation are unnamed.
8. The guard test of Task 044-8 does not exist; point to the existing budget test.
9. Java 21: the below-21 test cannot be observed; the conformance module needs release 21.
10. CI steps for the Python 3.11 package; Java needs none.
11. The exact body of `platform_requirement_hold` is not in this tree.
12. The version-list route has two owning tasks.

Counts: Blocker 0, Major 3, Nit 9
