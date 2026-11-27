# Targeted re-read of design 046, fourth pass: security (2026-11-26)

Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: 4b6a3e6d00f2cf217fa7a708f24c532283945767
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output, condensed by the author; nothing of substance left out. The fourth and last targeted
security re-read after review round 3, on the approval rule of Owner decision 2026-11-25-2 (fix Blockers and every
security finding, re-read the fixes). Its history: `reread-1-security.md` (Major 1, Nit 5), `reread-2-security.md`
(Nit 6), `reread-3-security.md` (Nit 3), every finding fixed before the next pass. The reviewer read the diff of the last
fixes, the design and the four contracts, and ran nothing.

## The previous findings

1. Answered. The job row stores `charged_day` and `charged_bytes`; settlement happens on that day's row only, never
   below zero (the "Fetch budget" section and the data model; `a_claim_before_midnight_settles_on_its_own_day`). A
   second completion cannot settle twice ("a finished job answers `204` and changes nothing"); a retry's new claim
   overwrites the charge and the old attempt's lease token is dead.
2. Answered. Parses are capped at 4 per tenant, keyed on the signed grant's `tenant` claim, which the runner cannot
   choose; the `503` maps to `PACKAGE_GATEWAY_UNAVAILABLE`, retryable with feature 005's backoff and a final `FAILED`
   (`one_tenant_cannot_hold_every_parse_slot`). Two busy tenants filling 8 slots is ordinary capacity, bounded by the
   cache and retries, not an attack by one tenant.
3. Answered. The private file route uses the namespace of the grant's job, test run or materialization. The reviewer
   tried a runner presenting a grant of namespace A (hold 0) to fetch a file still held in namespace B: it gains nothing,
   since B's resolve never sees the file, B's author lock refuses it (`PACKAGE_QUARANTINED`), and an install fetches only
   files named in a lock validated against B's hold.

## New findings

None. The new `charged_day` and `charged_bytes` columns need no new grant (the table already has UPDATE), and settlement
cannot deadlock with claims (claims skip locked job rows).

Counts: Blocker 0, Major 0, Nit 0
