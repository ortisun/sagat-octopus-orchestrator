# Targeted re-read 1 of design 054: security (2026-11-26)

Doc: docs/designs/054-buffered-signals.md
Reviewed-commit: ca46330f11d64ee1962ac6fec6e5e26e0fe7a397
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and its severity is kept. A targeted re-read of the
fixes of `../security.md` (the approval rule of Owner decision 2026-11-25-2, step 4), not a full review. The reviewer read
files only and ran nothing.

Findings 1 to 9 of the security review: resolved. The reviewer traced the hold against `platform_requirement_remove` in both
orders and under `REPEATABLE READ` (it fails closed), and confirmed the journal read returns the digest column and never the
detail.

Counts: Blocker 0, Major 0, Nit 3

## Nit

1. Requirement 054-019 still forbids the tenant and run in a log line, which Technical Plan 14 now puts in authorized log fields.
   *Answer: fixed:* identifiers never as metric labels, only in authorized span and log fields, never the name or the payload in a
   log line.
2. The move limit is per signal, not per chain (a refilled inbox keeps moving rows, bounded by the rate of signal requests); a
   handoff may need three events before its handoff event; `moves` has no check. *Answer: fixed:* Risks states that the rate
   quota bounds the total; Technical Plan 7 gives three events and their sequences; `moves` gets a check added `NOT VALID`.
3. The namespace ceiling does not bound the tenant's total. *Answer: fixed:* a namespace's setting is capped at the tenant's
   content quota and the tenant's total of pending bytes is bounded by a tenant default of the same resource; the residual is stated
   in Risks.
