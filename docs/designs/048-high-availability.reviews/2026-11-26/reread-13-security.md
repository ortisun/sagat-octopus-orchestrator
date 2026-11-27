# Targeted re-read 13 of design 048: security (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: 05c8428062558a42503d7bf7a593eb45ff5d29de
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation re-read of
the changes since fa671a75, not a full review. The reviewer read files only and ran nothing.

Checked with no finding: the alert expressions, the drop rule during a commit, `ConnectionAdmission`,
`DbHandle::connect_lazy`, the `test-support` gating, eager pools with `InertHealth` until Task 048-10, the passive
observer's read-only handle, ADR-0130's decision 5 and "Enforced by".

No Blocker, no Major.

## Nit

1. The alerts have no scrape path (Patroni's REST interface is on an internal network; the Kubernetes policy admits no
   monitoring), and the `up` guard then silences them; a later fix might expose `db-control`. Fix: a scrape network of
   its own, a policy peer for the monitoring namespace on the metrics port, a scrape-down alert.
2. "Before a migration that is not its first" lets `migrate`'s first migration skip the wait. Fix: each migration, the
   first included, once the wait's own migration is applied.

Counts: Blocker 0, Major 0, Nit 2

Both were fixed at the next commit.
