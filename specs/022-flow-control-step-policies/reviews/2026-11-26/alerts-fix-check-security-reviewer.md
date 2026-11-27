# Security check of the alerts fix (2026-11-26, commit 5ea3af9a0)

Recorded by the implementer from the coordinator's relay of the security reviewer's check.

Verdict on the items of the quota-fix check: fixed. Item 1 (the flow-control alerts' `items` withheld from a caller
without the reveal right), item 2 (the revealed quota usage read in one statement) and item 3 (the canonical contract
carrying the branch's descriptions) are fixed.

New findings:

1. (low) `getRun`'s `rateLimit.position` (`crates/application/src/runs.rs:2449-2467`), the run's rank among its key's
   waiters, is shown to a caller who may not see the key's label: how many runs wait on the same key ahead of it is a
   count of the runs that share it. Withhold it as `estimatedGrantAt` is withheld (gap G022-014).
2. (low) A flow-control alert's very state leaks per-key facts at a waiter cap of 2 or more: `RATE_LIMIT_QUEUE_SATURATED`
   fires for a key when `waiters * 5 > cap * 4`, so whether it fires tells whether two waiting runs share a key;
   `CONCURRENCY_QUEUE_STUCK` likewise; `RATE_LIMIT_GRANT_LAGGING`'s `oldestSeconds` gives a key's `next_eligible_at`. The
   rows and the `ops.alert.*` events of these kinds reach callers and endpoints without the reveal right.
3. (defence in depth) The withheld `items` is zeroed in the DTO only, not in `OperationsService::alerts`, and
   `AlertReader::list_alerts`, which reads every alert without the caller's right, is part of the production trait.

Conclusion: the family "which runs share a key, visible without the reveal right" is not closed.
