# Security review of design 044, round 6 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: b5c23b83c9a63c8ed42f15b4cdf881ecb683dd4e
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Round 5's Majors 1
(approval initiators) and 2 (opaque secret reference) are closed as asked; every round-5 Nit is applied. No finding on
the tenant boundary, secrets on the workflow task, the runtime role, the requirement hold, injection, metric labels or
the re-checked races.

## Major

1. The four-eyes rule does not carry across continue-as-new, children or resets, leaves out review resolvers and the
   publisher, and the approval tables hold one initiator: keep a run's initiators in a table copied by the handoff, the
   child start and the reset, bound by the issuing activation; tests for each.
2. Blob payloads get around the `secret` placeholder and feature 016's redaction: a lease reads a blob output with a
   `secret` field and can route its reference to another activity; the export's treatment of blobs is unsaid.
3. `ReadHistory` streams and update waits have no per-tenant cap; the completion and answer RPCs are shed first as
   `Discovery`: lifetime caps and a byte budget per lease, class `Lease`, a tenant share of the 256 waits.

## Nit

4. `sagat_move_code_run` lacks the columns to check its replay verdict, and the last-row check fails a move off review.
5. Which governs declared query and update answers: the `pii` classification or Architecture decision 0031's disclosure.
6. The example approval policy names a role that does not exist.
7. Blobs written under a workflow lease that no command names are never released.

Counts: Blocker 0, Major 3, Nit 4
