# Approval confirmation of design 048: architecture (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: 8bcdf393ed0dd38c3b07b816f7538969a99343b5
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The last of the targeted
re-reads of the approval run under Owner decision 2026-11-25-2: one full round (`architecture-full.md`, at 1e283510),
then the fixes re-read in `reread-1-architecture.md` to `reread-5-architecture.md`. Not a full review. The reviewer read
files only, ran nothing, and read the whole of ADR-0130.

Re-read 5's two Nits are resolved: stores are built by `DbHandle::connect_lazy(config, health)`, `DbHandle::new` is test
support behind the `test-support` feature from Task 048-10 on, and Task 048-7 keeps the pools connecting at start-up
until Task 048-10 makes them lazy. `migrate`'s wait before each migration agrees with design 053 (Task 053-4 after Tasks
048-9 and 048-10). ADR-0130: nothing stops its acceptance; decision 1's closed networks hold with a metrics-only network,
and "Enforced by" names a task for each decision.

No Blocker, no Major.

## Nit

1. Requirement 048-002 still lists three networks; Task 048-13 adds a fourth, `db-metrics`, which the allowlist sentence
   names but nothing defines, and how Prometheus (declared in `compose.observability.yaml`) joins a network of
   `compose.ha.yaml` (one project, or an `external` network) is not stated.
2. Requirement 048-001's network policy admits only the platform's namespace, the instances and the operator; Task
   048-13 adds a monitoring peer, so Task 048-4's policy test written to the requirement would refuse it.
3. The Patroni allowlist assertion of Requirement 048-002 has no task: neither Task 048-3 nor Task 048-4 adds it to
   `scripts/check-ha-reference.py`.

Counts: Blocker 0, Major 0, Nit 3
