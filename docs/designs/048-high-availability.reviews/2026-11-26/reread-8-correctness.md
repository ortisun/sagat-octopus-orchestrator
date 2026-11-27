# Targeted re-read 8 of design 048: correctness (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: 617ed7d55e8267bdee28d6fd7d94ec847edd3a93
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation re-read of
the changes since 6cdd2c00, not a full review. The reviewer read files only and ran nothing.

Checked with no finding: the drop rule during a commit, `ConnectionAdmission`, `DbHandle::connect_lazy`, eager pools
until Task 048-10, the `test-support` gating, a failed region lookup, the alert expressions (a healthy quorum counts 2),
the grants of `command_await_durability`, ADR-0130's decision 5.

No Blocker, no Major.

## Nit

1. The rule limits where the passive observer is used, not what it can do: it could open a `WriteTx`.
2. `migrate`'s wait has no bootstrap across the migration that creates the wait's function and table.
3. A failed scrape looks like a lost standby to the alerts.

Counts: Blocker 0, Major 0, Nit 3

All three were fixed at the next commit.
