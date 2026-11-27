# Approval confirmation of design 048: security (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: 8bcdf393ed0dd38c3b07b816f7538969a99343b5
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The last of the targeted
re-reads of the approval run under Owner decision 2026-11-25-2: one full round (`security-full.md`, at 1e283510), then
the fixes re-read in `reread-1-security.md` to `reread-14-security.md`, every finding of each fixed before the next. Not
a full review. The reviewer read files only and ran nothing.

Re-read 14's Nits are resolved. Patroni's REST `allowlist` is limited to the members' addresses on `db-control`
(`allowlist_include_members`, no `db-metrics` address, asserted by `scripts/check-ha-reference.py`); the allowlist and
the REST authentication govern the unsafe methods, so the scrape network reads the GET endpoints it needs and cannot
write to the cluster. The scrape-down alert fires on `up == 0` or `absent(up{job=...})` for 5 minutes, with rule tests
of a target down and of no target, so the `up` guard can no longer make the standby alerts silent. The edit between
82e1645a and this commit touches only those two passages; re-read 14 found no Blocker or Major at 82e1645a.

Every finding of every security report of this approval run is fixed; none is accepted.

No Blocker, no Major, no Nit.

Counts: Blocker 0, Major 0, Nit 0
