# egm-review Stage 7 design lens, targeted re-read, round 9 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 842f013e1e19a2d4e09617fc0abf88296275b860
Reviewer: egm-design, feature-spec standard, FeatureRange from the start to the Detailed Implementation Plan
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Of round 8's findings,
A, B, C, E and Nits 2 to 5 and 7 are closed; D and Nits 1 and 6 partly closed.

## Major

1. Values carried as blobs have no canonical body: the body names a server-assigned blob id, so a cold replay, a move
   check and a re-emitted deferral never reproduce it; hash a descriptor (SHA-256 and size of the value's RFC 8785
   bytes) whatever the carrier; a deferral holds its blob; say whether blob values are schema-validated; conformance.
2. More than 256 commands or 64 KiB of metadata in one activation has no outcome: a deterministic typed error thrown at
   the crossing call, live and in replay; a scenario; state the limit.
3. The closing window's two events per outstanding item are too few, since each settling outcome under a deferral
   costs three: size it at 3n or do not wake on obligation outcomes until the last one settles.

## Nit

4. The window multiplier and "open scopes" are defined two ways.
5. `version: active` said to be resolved at the handoff in section 9.
6. A leased step's move outcome: `RUN_BUSY` against `RUN_NOT_MUTABLE`.
7. The lease-kind table's `WriteBlob` row is narrower than the values that may be blobs.
8. The can-act index and derived steps; members whose compensation finished.
9. `steps.outcome_seq` has no enforcing mechanism; the pair in the plan event; the code arm of `compensate`; an O(n) release.
10. When the overlay cache is filled.
11. Saga writes missing from the statement list.
12. Exported histories cannot replay blob-carried values.
13. The trigger that writes initiator rows in graph paths.
14. Section 3.1 omits `maxBytes`.

Counts: Blocker 0, Major 3, Nit 11
