# egm-review Stage 9 re-audit (implementation readiness), round 10 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 4a943e7c15d9ca73e9c1afa6274ad5da8cfb5cbb
Reviewer: egm-implementation-readiness, implementation-readiness standard, DetailedRange from the Detailed Implementation Plan to the end, code claims checked
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Of round 9's findings,
2, 3, 4, 6, 7, 8, 10, 11 and 12 are closed and 1, 5 and 9 partly closed.

## Major

A. The conformance "workflow entry" is undefined, and today's harness runs every worker inside one runner process
   under `TZ=Asia/Kolkata`: a `run.sh workflow-worker` subcommand spawned by the runner with its own environment, a
   control and log protocol over standard input and output, documented in the conformance README.
B. The bytes of `d(k)` are not fully specified (each kind's string, each typed command's target, the digest's text
   form): a table in section 6.2 and the hex form.

## Nit

1. The registry's closed checks were renamed by design 043 (`..._known`); follow its swap.
2. Validation in the same migration holds the lock; a separate validate migration.
3. Gap 044-026 is not on Task 044-6 or the build order.
4. More callers of the cache's rename.
5. The `golang.org/x/tools` rule against the Go directive.
6. The existing Python step and the local time zone of TypeScript and C# tests.
7. The release gate should use `git grep`.
8. Tasks 044-11, 044-12, 044-13, 044-21 and 044-24 have no Files line.

Counts: Blocker 0, Major 2, Nit 8
