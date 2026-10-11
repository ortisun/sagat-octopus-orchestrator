# Validation quickstart: Performance, scalability and high availability

Everything here runs. The numbers it produces are in [`tests/performance/reports/scalability-ha.md`](../../tests/performance/reports/scalability-ha.md) and [`tests/performance/reports/disaster-recovery.md`](../../tests/performance/reports/disaster-recovery.md); how to operate what it proves is in [`docs/runbooks/scaling.md`](../../docs/runbooks/scaling.md) and [`docs/runbooks/disaster-recovery.md`](../../docs/runbooks/disaster-recovery.md).

## Prerequisites

- The Rust toolchain pinned by the repository.
- PostgreSQL for the tests: Docker (Testcontainers starts one), or `SAGAT_TEST_DATABASE_URL=postgres://postgres:postgres@127.0.0.1:55432/postgres` for a server you run (test databases are left behind: drop the `t_*` ones now and then).
- NATS for the tests that need a broker: Docker, or `SAGAT_TEST_NATS_BIN=$(command -v nats-server)` for a `nats-server` binary (a process per test, killed with it).
- `helm` (the chart's contract test renders it; a machine without it fails that suite on purpose).
- For the benchmark and the drill: PostgreSQL's **server** binaries (`initdb`, `pg_ctl`, `pg_basebackup`; `PGBIN` names their directory) and `psql`.

## Validation flow

1. **The stories (SC-010-002).** US01..US04 against the real `execution-engine`, started the way the binary starts, one of them killed like a process:
   `cargo test -p sagat-tests --test e2e_scalability_ha`
2. **Leases, the fence, failover and fairness on a real PostgreSQL**, the outbox, the lag gauges, the operator's weights:
   `cargo test -p sagat-tests --test integration_scalability_ha` and the unit tests of `sagat-domain`, `sagat-scheduler` (`fairness`, `shards`) and `sagat-execution-engine`.
3. **Chaos (FR-010-005).** The database's connections killed over and over under load, a replica cut off from the database, a replica killed in the middle of a pass, a replica held up past its lease, the broker gone while a dispatcher changes hands, three workers in ten lost:
   `cargo test -p sagat-tests --test fault_failover` (about 25 s) and `--test fault_scalability_ha` (mixed versions, a deadline racing a cancel, the runtime role's privileges, what the keeper logs).
4. **The chart.** `helm lint infra/helm -f infra/helm/examples/ha.yaml`, and the contract test that renders it and reads it back:
   `cargo test -p sagat-tests --test contract_scalability_ha`
5. **Confirm outcomes by hand**, with two engines against one database (the compose file runs one):

   ```sh
   export DATABASE_URL=postgres://sagat_app:...@127.0.0.1:5432/sagat   # the runtime role
   SAGAT_GRPC_ADDR=127.0.0.1:9001 SAGAT_ADMIN_ADDR=127.0.0.1:9091 SAGAT_REPLICA_ID=a execution-engine serve &
   SAGAT_GRPC_ADDR=127.0.0.1:9002 SAGAT_ADMIN_ADDR=127.0.0.1:9092 SAGAT_REPLICA_ID=b execution-engine serve &
   psql "$DATABASE_URL" -c "SELECT split_part(owner,'-',1) AS replica, count(*) FROM scheduler_shards WHERE expires_at > now() GROUP BY 1"
   #  a | 32     b | 32
   kill -9 %1     # a dies: after the lease (15 s) b holds 64 and the timers of a's shards fire
   curl -s 127.0.0.1:9092/metrics | grep -E 'scheduler_(shards_owned|lag_seconds|fenced)'
   ```
6. **The benchmark (FR-010-006).** Starts a PostgreSQL of its own with the configuration it reports, makes up to a million suspended runs through the platform's services, and writes the report. A full run takes about an hour on the machine of the report; `SAGAT_PERF_MAX_WAITS=100000` is a quarter of that:
   `tests/performance/scalability-ha/run.sh`
7. **The disaster-recovery drill (FR-010-007).** A point-in-time restore with the journal digest compared before and after, and a failover with `kill -9` and a count of the acknowledged runs that survive:
   `infra/dr/drill.sh`
8. Take down the engine, the broker or the database at a controlled moment and watch the platform recover: steps 3 and 5.

## Gates

- Performance reports with hardware/configuration, zero confirmed Run loss in fault injection and validated DR restore: the two reports above, and the chaos and drill steps that fail when a run is lost.
- Reproducible, annotated errors; no confirmed transition lost.
- `[x]` only when a real test has passed: `tasks.md` names the test of each.
