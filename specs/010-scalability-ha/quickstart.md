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

## Reviews of the implementation (T010-035)

Two rounds, each by two independent reviewers with a fresh context and read access only (security, and correctness), told to
substantiate every finding with the concrete input or interleaving and to drop what they could not. The first round came
before the first benchmark and its findings are D246, D248, D250-D252 and G259-G264 of `research.md`. The second came after
the fairness search was reworked and the last fixes of the first were in; what each of its findings became:

| Finding (severity) | Outcome |
|---|---|
| Security: the counters' `SECURITY DEFINER` function can be hijacked through a temporary table, and attached to a table of the caller's own (high) | Fixed: `pg_temp` last in the path, qualified names, `EXECUTE` revoked from PUBLIC (D255). `a_temporary_table_cannot_stand_in_for_the_counters_or_borrow_the_function` fails against the old function |
| Security: a pool's gauges can be inflated by a tenant with no workers (medium) | Recorded, not built (G265): the ceiling `maxReplicaCount` bounds the cost; the runbook says so |
| Security: the autoscaler follows a signal a tenant can raise; no check of connections against the server (low) | Partly fixed: `database.serverMaxConnections` makes the chart refuse a ceiling the server does not hold (G267) |
| Security: the HA example sets plaintext secrets for the worker API (low) | Fixed: the example has the engine terminate TLS |
| Security: the NetworkPolicy is ingress only (low) | Resolved after the original review: chart 0.4.0 adds required role-specific dependency egress and worker-pool isolation, rejects unsafe peers/ports, and is covered by render tests plus the kind/Cilium verifier (G266) |
| Security: a lease can be set to `infinity` (low) | Fixed: a `CHECK`, and the keeper no longer panics on a time it cannot take (`a_lease_cannot_be_made_to_last_for_ever`) |
| Security: `SAGAT_SCHEDULER_BATCH` and `SAGAT_OUTBOX_BATCH` unbounded (low) | Fixed: at most 5000 (`a_batch_a_database_should_not_be_asked_for_is_refused_by_name`) |
| Security: the chart's control-plane probes ask for the engine's paths, so the pods would not start (found as low) | Fixed (D254): `every_probe_of_the_chart_asks_for_a_path_its_process_serves` |
| Correctness: a failed second page of the fairness search throws the first away and sets no rest; a search that finds somebody is never rested (medium) | Fixed (D257): `a_search_that_fails_shares_what_was_read_and_asks_for_a_rest`; the cost of the search while a burst and another tenant coexist is now stated (ADR-0020, decision 10) |
| Correctness: the keeper's interval of a third of the lease does not survive two failed rounds (medium) | Fixed: a fifth (D256): `two_failed_rounds_in_a_row_do_not_lose_the_leases` fails with a third |
| Correctness: the renewal waits for a row an orphan holds, which fails the renewal of every shard (medium-low) | Fixed: `SKIP LOCKED`, and a skipped shard adopted in the same round is not counted as lost (`a_renewal_does_not_wait_for_a_shard_an_orphan_holds_and_renews_the_others`, `a_shard_skipped_by_the_renewal_for_a_moment_is_adopted_in_the_same_round_and_not_lost`) |
| Correctness: the remainder of a weighted share starves a light tenant among many heavy ones (low-medium) | Fixed (D258): `a_light_tenant_among_many_heavy_ones_is_given_its_fraction_before_they_are_given_theirs` |
| Correctness: a tick longer than the snapshot's validity makes every later action `fenced` (low) | Fixed: a view per pass. No test of its own: a tick that long is not made on purpose; the e2e and chaos suites run through it |
| Correctness: an infinite expiry panics the keeper, which nothing notices (low) | Fixed: the clamp and `CHECK` prevent the known infinite-expiry panic, and `supervision.rs` now treats any critical task's return, error, panic or cancellation as a process failure, makes probes unhealthy, stops sibling tasks and returns exit 1. Intentional shutdown has a bounded 30-second drain (G269 resolved). Focused agent-run evidence: all 32 `sagat-execution-engine` tests and package Clippy passed; the historical CI run below predates this change. |
| Correctness: a failed sample leaves the autoscalers' gauges at their last value (low) | Fixed: zeroed (`a_sample_that_fails_zeroes_the_gauges_the_autoscalers_read_and_the_backlog_is_the_monitors` fails without it) |
| Correctness: every replica counts the whole outbox at every round (low) | Fixed: the monitor sets `outbox_backlog`, once (the same test reads it from the page) |
| Correctness: the chart accepts what the binaries refuse at start (low) | Fixed for secrets without a protected hop, pool names and limits, batch and concurrency, whole numbers; 11 new refusals in `refused.yaml` |
| Correctness: `data-model.md` says `-infinity` (low) | Fixed |
| Correctness: `lost_leases` merges two branches and truncates (low) | Recorded (G268) |

Mutation checks of the new tests: with the old counters' function, with a renewal that waits, with the remainder to the
heaviest weight, with the interval at a third and with a monitor that does not zero its gauges, the tests above fail; with
the fixes they pass.

The benchmark found one more defect of the fairness search after these reviews (D260): its floor of two seconds between
searches was longer than the whole drain of the noisy-neighbour scenario, so a tenant that the first page already showed
was served once and then starved until the rest was over. The floor is a tenth of a second now, the rest stays
proportional to what the search costs, and the scenario's timers are given an explicit order so that the three policies
are given the same work.

What the original reviews could not look at remains historical evidence. The subsequent feature-009 completion ran the
NetworkPolicy and restricted-admission subset in kind/Cilium. A full product/KEDA installation, node-drain behavior, real
dependency protocols, the database under a real network partition and the original local PostgreSQL-16 comparison remain
outside that evidence (G247).

Current integrated CI evidence: [run 38192086606](https://github.com/ortisun/sagat-octopus-orchestrator/actions/runs/38192086606)
on `65841f33cd5b1af462ba4b6a41e92b918939c63d` passed the full Rust workspace, formatting, Clippy,
architecture, contracts, observability, dependency audit, Helm lint, the small load harness and the small DR drill. CI
syntax-checks the Cilium verifier; the live cluster execution recorded above was separate and must not be inferred from that
run.
