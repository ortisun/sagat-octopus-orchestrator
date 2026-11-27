# egm-review Stage 9 re-audit (implementation readiness), round 6 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: b5c23b83c9a63c8ed42f15b4cdf881ecb683dd4e
Reviewer: egm-implementation-readiness, implementation-readiness standard, DetailedRange from the Detailed Implementation Plan to the end, code claims checked
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Of round 5's findings,
2, 3, 5, 6, 7, 9, 10, 11 and 12 are closed and 1, 4 and 8 partly closed.

## Major

1. The call-site check counts every method name of `WorkflowDefinition`, `len`, `is_empty`, `name` and the builders
   included (`.len()` alone is 1,087 calls in 244 files): pin the counted set to the methods that resolve definition
   content, exclude the generic names, fail a count that shrinks too, and list Task 044-31's arms by file and function.
2. `envelope.timeoutLevel` has no type or values on this branch, and Task 044-3 is not gated on design 045: admit only
   `"none"` until design 045 lands, refuse design-045 fields in fragments, widen in Task 044-27; red tests.

## Nit

3. Task 044-31 names no home for `append_events` and not the files of the fail-closed arms.
4. The build-order example is stale; five core tasks wait behind Task 044-21.
5. Task 044-9's Files line misses the batched path, the gRPC handler and the reconciler; a test prefix is unclear.
6. Who creates `platform_requirement_hold`, and whether the down drops it.
7. The Rust SDK's copies have no sync script; name the hand step and its test.
8. CI jobs for the bundler package and the Python 3.11 and Java 21 workflow packages; Task 044-23's source files.
9. How the crash test knows the engine reached the point (`pg_locks`).

Counts: Blocker 0, Major 2, Nit 7
