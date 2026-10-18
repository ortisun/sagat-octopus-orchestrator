# Lost native leaf COMMIT response, not process restart

Root sole writer/test executor. Fresh Astra/xhigh reviews are read-only, no tests performed by
reviewers. The current-review choice persists; no external CLI review ran. Production native
logic/privilege migrations are unchanged. This is bounded B1b.2a.1d.3c.1 only.

## Cycle 1

One valid actionable fixture defect: abort constraint marker disagreed with the reused one-shot
relay's validated ErrorResponse marker. Actual first.txt failed at server-resolution observation.
The injected synthetic constraint now uses capacity_finish_proxy_forced_abort, deliberately the
shared relay's exact test contract. The wire helper's protocol logic is unchanged. Corrected
leaf test passes both actual server outcomes in green.txt; no production defect RED is claimed.

Strict Clippy then rejected loading the same proxy source twice (duplicate_mod). It now has one
module declaration under capacity_finish and explicit crate-visible test-only type/module access;
leaf tests import it. No lint suppression or duplicate protocol implementation was introduced.
Both consumers must pass after this integration change, not just the new leaf test.

## Cycle 2

Two valid actionable oracle/claim defects:

1. Scheduler FOR UPDATE NOWAIT probe also conflicts with an insufficient KEY SHARE guard. The
   final negative probe uses NO KEY UPDATE; a positive SHARE probe also checks the fence remains
   shareable. Root independently strengthened the Run probe to KEY SHARE NOWAIT, which detects
   the required exclusive Run guard rather than accepting weaker row modes.
2. Final reconstruction reused the cancelled waiter's pool. After actual server resolution the
   waiter pool now closes under deadline, and a new runtime pool is created for final recovery.
   Cache/store and current revision/authority discovery are fresh too.

## Cycle 3

No concrete issue found in bounded lifecycle/oracle/shared-module scope. Stop condition met after
3cycles. Final shared-consumer test passes2/2, each with actual committed/aborted server outcomes;
strict targeted Clippy/fmt/architecture pass. Exact commands, times and sources are retained.

This increment establishes object/pool/cache reconstruction only; a native reconciliation-process crash/restart is
separately pending, as are other producers, observations, release activation and full nesting.
