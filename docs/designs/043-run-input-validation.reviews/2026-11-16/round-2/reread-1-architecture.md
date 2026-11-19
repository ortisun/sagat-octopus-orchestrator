# Targeted re-read 1 of design 043: architecture (2026-11-16)

Doc: docs/designs/043-run-input-validation.md
Reviewed-commit: a763d9d01d36acea3bb3cad1e6e861d7c4f5ce74
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A targeted re-read, not a
full review. The reviewer read files only and ran nothing.

Round-2 Majors: 2 resolved (ADR-0092 decision 7), 3 resolved, 4 resolved as a mechanism (finding 1), 5 partly resolved
(findings 2, 3), 6 resolved. New edges and layering: no finding.

## Major

1. `IrError::ValidatorBusy` reaches callers that treat any parse error as final: the promotion (409), the publication
   replay (500), the dry run (200 with a problem).
2. The v1 pin does not hold `MAX_POINTER_BYTES`, nor the vocabulary and allowlist that decision 6 should name.
3. ADR-0092 decision 6 is recorded as accepted, but the owner never answered it.
4. The inventory's superseded test misses schedules that pin a version.
5. No test runs are recorded at the commit under review.

## Nit

6. `precheck_in` does not catch a panic; no `interrupted` pre-check outcome.
7. `acquire_blocking` lacks the timeout/grant race test of the async path.
8. The nil `TenantId` as the pre-check owner; the wiring rule checks the method name only.
9. `Counting::Ahead` and `AtStart` do not count `deadline` or `interrupted`.
10. An unreachable `VALIDATOR_BUSY` details arm; two swapped test doc comments.
11. ADR-0092 lists no alternatives for decisions 6 and 7.
12. `version_still_started` scans every run of the workflow per refused version.
13. The migration test undoes to a fixed stamp.

Counts: Blocker 0, Major 5, Nit 8
