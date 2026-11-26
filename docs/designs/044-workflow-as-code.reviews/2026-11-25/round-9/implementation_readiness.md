# egm-review Stage 9 re-audit (implementation readiness), round 9 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 842f013e1e19a2d4e09617fc0abf88296275b860
Reviewer: egm-implementation-readiness, implementation-readiness standard, DetailedRange from the Detailed Implementation Plan to the end, code claims checked
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Of round 8's findings,
1 to 4, 6 to 9 and 12 are closed; 5, 10 and 11 partly closed.

## Major

1. The SDKs' own tests break the precondition rules: the conformance harness and the Java pom run every worker under
   `TZ=Asia/Kolkata` on purpose, and pytest runs with a random hash seed; code-workflow workers start under UTC and a
   fixed seed, the workflows module overrides surefire, refusal tests use a subprocess or an injected reader.
2. `CommandDigester` is undefined and the domain cannot compute a golden hash: give its signature (bytes to SHA-256),
   test the fold with a recording fake, leave golden equality to Task 044-5.
3. No task packages the four new distributables (`package-sdks.sh`, the packaging workflow and runbook, release notes).
4. Red tests that wait on a later task have no way to land without breaking CI: `#[ignore = "green with Task 044-NN"]`,
   removed by the prerequisite task, and a release gate that none remain.

## Nit

5. The rename's callers.
6. `NodeMissing`'s files.
7. The Python 3.11 step's command; a contradictory Java sentence.
8. The exact body of `platform_requirement_hold`.
9. `golang.org/x/tools` v0.28.0 against the module's other requirements and toolchain.
10. The C# analyzer project's language version, release files and reference assemblies.
11. The `canonical` module must become public.
12. Node in the integration shard.

Counts: Blocker 0, Major 4, Nit 8
