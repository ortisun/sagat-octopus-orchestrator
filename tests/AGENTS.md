# tests

Layout `tests/<kind>/<feature>/` with kind one of `contract`, `integration`, `fault`, `e2e`, `performance`; Cargo targets are `<kind>_<feature>` (`docs/test-strategy.md`). No feature numbers in names.

- Use real PostgreSQL (Testcontainers) and the real API. Do not mock the database. Docker must be reachable; the project's own Colima daemon is `DOCKER_HOST=unix://$HOME/.colima/sagat-remaining/docker.sock` when you run several agents.
- Write the test, run it, see it fail for the right reason, then fix. Races use held locks, barriers or `pg_stat_activity`, never sleeps. A flaky test has a cause (an unexpired membership row, a transient state, load): find it.
- Goldens (`tests/contract/sdk-conformance/expected/`, `docs/reference/`) are blessed only after reading the diff, then rerun without the bless switch.
- The cross-tenant battery (`tests/contract/security-multitenancy/rest/battery.rs`) needs a valid request body for every operation that takes one (`body_of`) and lists the operations not built yet; a planned operation carries `x-planned`, a built one `x-implemented`.
- Slow or toolchain-bound suites are `#[ignore]` and run in CI with `-- --ignored` (SDK conformance, e2e, fault of the SDKs). Run them locally before opening a PR; CI stops at its first failing binary and hides the rest.
- A benchmark report states the environment, the load of the machine and the failures, not only the throughput (`tests/performance/reports/`).
