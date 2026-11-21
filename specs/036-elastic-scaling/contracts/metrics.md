# Contract: new metric series

All series follow ADR-0017: no tenant, namespace, connector, run, step, activity, workflow or worker label. The rule is the test `no_metric_is_labelled_with_something_that_grows_with_the_data` (`tests/contract/observability-operations/main.rs:352`), which this feature does not loosen. Names are snake_case, counters end in `_total`, durations in `_seconds`. The chart's contract test requires every autoscaler query to use a metric that the code emits, so each name below must exist in `crates` or `apps` as a macro call.

Label values are bounded by validation: at most 64 signals, 32 policies, the fixed unit names, operator-named pools (at most 32), and the fixed limit, outcome and state names of `data-model.md`.

## Published from stored state by every replica of the engine and the control plane

| Series | Type | Labels | Meaning |
|---|---|---|---|
| `scaling_desired_replicas` | gauge | `unit`, `authority` (`advisory`, `authoritative`) | desired replicas of a unit, from `scaling_desired`. The chart follows `authority="authoritative"` only. Persists while the evaluator is down (hold). Absent for a unit with no policy in that mode |
| `scaling_desired_age_seconds` | gauge | `unit`, `authority` | age of that value; an alert fires when it grows while an evaluator should be running |
| `scaling_signal_value` | gauge | `signal` | the latest value of a signal that an operator marked `exported`; `NaN` when the signal is unavailable or stale (`G1704`), never 0 |
| `scaling_signal_state` | gauge | `signal`, `state` | 1 for the state in force, 0 for the others |

Every replica sets the same values from the same rows, so the provider's query takes `max(…)` over instances, as the engine query does today.

## Set by the evaluator (under its lease)

| Series | Type | Labels | Meaning |
|---|---|---|---|
| `scaling_decisions_total` | counter | `unit`, `outcome`, `reason` | evaluations by outcome and reason |
| `scaling_limit_hits_total` | counter | `unit`, `limit` | how often a limit changed the value (`PARTITION_BOUND`, `DB_BUDGET`, `COST_CEILING`, …) |
| `scaling_evaluation_seconds` | histogram | none | time of a whole round |
| `scaling_evaluator_holder` | gauge | none | 1 on the replica that holds the lease, 0 elsewhere |
| `scaling_source_queries_total` | counter | `kind`, `outcome` | pulled-source queries by kind (`metrics_query`, `broker_queue`) and outcome (`ok`, `unavailable`, `refused`, `timeout`, `invalid`) |
| `scaling_config_reload_total` | counter | `outcome` | `applied`, `invalid`, `unreadable` |
| `scaling_freeze` | gauge | `scope` (`global`, `policy`) | 1 while a freeze is in force |
| `scaling_refused_total` | counter | `interface` (`operator`, `tenant`, `source`), `reason` (`audience`, `lifetime`, `permission`, `rate`, `not_found`, `destination`, `credential`, `limits`) | refused attempts, counted without tenant identity (FR-036-065); emitted by the control plane and the evaluator |
| `scaling_definition_refused_total` | counter | `kind` (`policy`, `source`, `signal`) | a stored version that the evaluator refused to use because it violates a configuration-only limit (`D1732`) |
| `scaling_credential_uses_total` | counter | `outcome` (`ok`, `failed`) | every use of a source credential; carries neither the source nor the credential name. The first use after a start or a rotation, and every failure, are also audited in the platform trail (FR-036-067, D1736) |

## Connector lag, aggregated by kind only

| Series | Type | Labels | Meaning |
|---|---|---|---|
| `connector_lag_messages` | gauge | `kind` | total backlog by connector kind (`KAFKA`, `NATS`, `SQS`) over fresh rows |
| `connector_lag_age_seconds` | gauge | `kind` | largest oldest-message age by kind where known |
| `connector_drain_state` | gauge | `kind`, `state` | number of connectors of the kind in the state |
| `connector_partition_bound` | gauge | `kind` | number of connectors whose useful readers are limited by partitions |
| `connector_partitions_held` | gauge | none | partitions this replica currently holds (per instance by the scrape, never per connector) |
| `connector_rebalances_total` | counter | `reason` (`join`, `leave`, `expiry`) | assignment changes made by this replica |
| `connector_lag_probe_failures_total` | counter | `kind` | a probe that could not measure |

The detail per connector is in the tenant API, the logs and the traces, not in a metric.

## Not changed

Existing series keep their names and meaning, including the monitor's zero-on-failure behaviour (`scheduler_lag_seconds`, `outbox_*`, `pool_*`, `inbox_pending_events`, `connector_messages_total`). New alert rules (`NFR-036-OPS`) are added to the observability rules and the runbook: signals stale, a policy held at its ceiling, partition-bound, database-budget-bound, evaluator failing (`scaling_evaluator_holder` summing to 0 for the interval), and a freeze left on.
