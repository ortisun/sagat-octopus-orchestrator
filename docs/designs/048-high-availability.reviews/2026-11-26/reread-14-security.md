# Targeted re-read 14 of design 048: security (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: 82e1645a2e3464876aa0ec78ab2832b0d4a68496
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation of the
fixes to re-read 13 (both confirmed resolved). The reviewer read files only and ran nothing.

No Blocker, no Major.

## Nit

1. The scrape-down alert `up == 0` does not fire when no target is scraped at all (a `PodMonitor` that selects nothing),
   and the guarded alerts then stay silent. Fix: also `absent(up{job=...})` for 5 minutes, with a rule test.
2. Patroni's `/metrics` shares its REST listener, so `db-metrics` reaches the whole REST interface and the allowlist's
   contents are not stated. Fix: the allowlist limited to the members on `db-control`, asserted by
   `scripts/check-ha-reference.py`.

Counts: Blocker 0, Major 0, Nit 2

Both were fixed at the next commit.
