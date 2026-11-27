# Approval confirmation of design 048: correctness (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: 8bcdf393ed0dd38c3b07b816f7538969a99343b5
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The last of the targeted
re-reads of the approval run under Owner decision 2026-11-25-2: one full round (`correctness-full.md`, at 1e283510),
then the fixes re-read in `reread-1-correctness.md` to `reread-8-correctness.md`. Not a full review. The reviewer read
files only and ran nothing.

Re-read 8's three Nits are resolved: the passive observer's handle offers `ReadTx` and no `WriteTx`; the bootstrap
across the migration that creates the wait's function and table is written out; a failed scrape fires neither standby
alert, and the `up == 0` or `absent(up{job=...})` alert fires within 5 minutes, each case with a rule test.

Checked with no finding: Patroni's allowlist governs only the unsafe methods, so the `GET /metrics` scrape from
`db-metrics` works and the members reach each other's unsafe endpoints over `db-control`; `migrate`'s wait before each
migration, the first included, with the fresh-database rule in the run that creates the wait, and each wait re-checking
the identity. Noted, not counted against design 048: on a database that holds data, `migrate` answers "retry later"
while every process is down; stopped upgrades after 0.3.0 belong to design 053.

No Blocker, no Major.

## Nit

1. The requirements' lists of networks and peers miss the scrape that Task 048-13 adds: Requirement 048-001's admitted
   peers omit the monitoring namespace's Prometheus pods on the metrics port, so Task 048-4's policy test would refuse
   Task 048-13's peer; Requirement 048-002 still says "Three networks" while its allowlist sentence names `db-metrics`.
   Fix: add the monitoring peer to Requirement 048-001; "Four networks" with `db-metrics` in Requirement 048-002.

Counts: Blocker 0, Major 0, Nit 1
