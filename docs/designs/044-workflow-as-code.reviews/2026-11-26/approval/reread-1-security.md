# Targeted re-read of design 044 for approval: security, first pass (2026-11-26)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: ed293ea93a1f7658e503d8e4b4ffd78676014ab7
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Scope: the text changed by
round 11's fixes only, on the approval path of OD-2026-11-25-2. Round 11's Majors 2 and 3 are closed, 1 closed with
residues and 4 partly closed. Every finding below was fixed in f4f55b14 (security findings are fixed, never settled).

## Major

1. The new chain receipt route has no declared permission or `404` rule.
2. A move still declassifies an output of a step in flight when it arrives: classify an outcome by the issuing version.

## Nit

3. Requirement 044-015 and section 8.2 still scope a key by the run.
4. The semaphore's `COMMAND_INVALID` wording; a per-tenant floor; check the lease token first.
5. One principal can fill a run's 64 open updates.
6. Where the verifier's binding is re-read.
7. A crashed lease's uncommitted blobs against the next attempt's 16.
8. No test for a carried signal's classification across a handoff.
9. A children entry may drop a child's `secret` annotation.

Counts: Blocker 0, Major 2, Nit 7
