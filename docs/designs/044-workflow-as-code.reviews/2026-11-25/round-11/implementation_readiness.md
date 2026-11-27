# egm-review Stage 9 re-audit (implementation readiness), round 11 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 9bdb707d84f3fef3c190979cde128256f2b8159a
Reviewer: egm-implementation-readiness, implementation-readiness standard, DetailedRange from the Detailed Implementation Plan to the end, code claims checked
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Of round 10's findings,
A, B and Nits 2 and 4 to 8 are closed; Nits 1 and 3 partly closed. Every task can be started.

## Major

1. The unserved-run alert's mechanism is undecided (the in-app `AlertKind` set or a Prometheus rule), and the
   integration test can only observe the first: name the kind, subject, threshold, severity and runbook anchor with its
   contract and migration files, or make it a rule with a rule-file test.

## Nit

2. The workflow-task claim's budget test: its target and figures.
3. `code_history.rs` is never created; `code_workflows.rs` is modified before the task that creates it.
4. The failure injected at statement 7 has no mechanism.
5. The conformance worker: Java's locale, a readiness line, the README in Files, child cleanup.
6. One stale constraint name remains.
7. The build order omits Gap 044-026.
8. Tasks 044-7, 044-22 and 044-33 have no Files line.

Counts: Blocker 0, Major 1, Nit 7
