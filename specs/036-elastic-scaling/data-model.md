# Data Model: Elastic scaling driven by telemetry

Logical model first (the domain types, which have no storage in them), then the tables of the two migrations. Columns are listed to fix names and constraints, not as DDL. Ids and terms follow [`spec.md`](./spec.md); decisions are in [`research.md`](./research.md).

## 1. Domain types (`crates/domain/src/scaling/`)

### Signal and sample

| Type | Fields | Rules |
|---|---|---|
| `SignalName` | `[a-z][a-z0-9_.]{0,62}` | unique; built-ins are reserved; `source.<name>` is the signal of a source |
| `Unit` | `messages`, `seconds`, `count`, `ratio`, `percent`, `cores`, `bytes`, `replicas` | a policy's target value is in the signal's unit |
| `Scope` | `OPERATOR`, `NAMESPACE` | operator-only signals are invisible to tenants (FR-036-005) |
| `Origin` | `BUILTIN`, `METRIC`, `RESOURCE`, `SOURCE`, `CUSTOM` | |
| `SignalDef` | name, unit, scope, origin, freshness, valid range, exported | freshness 5 s to 1 h; range finite, `min ≤ max` |
| `SampleState` | `AVAILABLE`, `STALE`, `UNAVAILABLE` | a failed measurement is `UNAVAILABLE`, never value 0 (FR-036-006) |
| `Sample` | signal, value `Option<f64>`, state, reason class, coverage `Option<f64>` in (0, 1], sampled_at | value present only when `AVAILABLE` or `STALE`; a value outside the valid range makes the sample `UNAVAILABLE` |

State transitions of a signal: `AVAILABLE → STALE` when its age passes the freshness; any state `→ UNAVAILABLE` on a failed or invalid measurement; `UNAVAILABLE` or `STALE → AVAILABLE` on a valid sample. No other path.

### Policy

| Type | Fields | Rules |
|---|---|---|
| `Unit` (deployment) | `engine`, `control-plane`, `ingestion` (slice 2), `pool:<name>` | pool names come from configuration |
| `Role` | `INGEST`, `SCHEDULER`, `EVENT_DISPATCH`, `DELIVERY`, `WORKER_POOL`, `WORKER_API`, `INGRESS` | each carries its unit of division, ceiling rule and handover period (`RoleLimits`); slice 1 uses the first five |
| `Target` | unit, role, kind (`REPLICAS`, `TUNABLE`, `RESOURCE`), parameter | slice 1: `REPLICAS` only; **unique per (unit, role, kind, parameter)** — two policies on one tuple are refused |
| `Interpretation` | `DEMAND`, `UTILISATION` | formulas in `contracts/evaluation.md` |
| `SignalBinding` | signal, interpretation, target value `> 0` finite, tolerance (utilisation only, default 0.10) | a policy has 1 to 8 bindings |
| `Step` | `max_change ≥ 1`, `every_seconds ≥ 0` | one per direction |
| `Policy` | name, target, bindings, `min`, `max`, `scale_out: {step, stabilization_seconds}`, `scale_in: {step, stabilization_seconds}`, `cooldown_seconds`, `combine` (`MAX`), `cost_ceiling`, `mode` | `unit floor ≤ min ≤ max ≤ cost_ceiling`; `min ≤ budget_replicas`; windows 0 to 3600 s; a policy that wants lag from connectors that cannot drain binds `ingest.lag.backlog_all` (the default signal `ingest.lag.backlog` already leaves them out) |
| `Mode` | `OBSERVE`, `RECOMMEND`, `ENFORCE` | a new policy starts `OBSERVE` (FR-036-017) |
| `Owner` | `CONFIG`, `API` | fixed for the life of a policy name (FR-036-020) |
| `Override` | policy, kind (`HOLD`, `MIN`, `MAX`, `FREEZE`), value, expires_at, author, reason | expiry required, at most 24 h; at most one active per (policy, kind) |

Counts: at most 64 signals, 32 policies, 32 sources (they bound the metric label values, `D1723`).

### Decision

| Type | Fields |
|---|---|
| `Decision` | id, policy, policy_version, evaluated_at, evaluator, `inputs`, `raw`, `limits`, `final`, `outcome`, `authority`, `reason`, `repeat_count`, `last_seen_at` |
| `Inputs` | current replicas and where it came from, per-signal `{signal, value, age_seconds, state, coverage, used}`, recent raw values used by stabilization, ceilings `{role, budget, cost}`, overrides, global freeze, mode, `now` |
| `Limit` | ordered, each `{name, before, after}`; names: `STABILIZATION_OUT`, `STABILIZATION_IN`, `STEP_UP`, `STEP_DOWN`, `COOLDOWN`, `BOUNDS`, `OVERRIDE_MIN`, `OVERRIDE_MAX`, `OVERRIDE_HOLD`, `FROZEN`, `COVERAGE`, `ROLE_CEILING`, `PARTITION_BOUND`, `DB_BUDGET`, `COST_CEILING` |
| `Outcome` | `OBSERVED`, `RECOMMENDED`, `APPLIED`, `HELD` |
| `Authority` | `ADVISORY` (recommend), `AUTHORITATIVE` (enforce), none (observe) |
| `Reason` | `OK`, `NO_SIGNAL`, `STABILIZING`, `COOLDOWN`, `FROZEN`, `OVERRIDDEN`, `AT_CEILING`, `DRAIN_EXCLUDED`, `PARTIAL_COVERAGE` |

`evaluate(policy_version, samples, state, context, now) → Decision` is total and deterministic (FR-036-026).

### Connector lag and assignment

| Type | Fields | Rules |
|---|---|---|
| `DrainState` | `CAUGHT_UP`, `BEHIND`, `BLOCKED`, `RATE_LIMITED`, `PAUSED`, `FAILING` | precedence for display: `FAILING > BLOCKED > RATE_LIMITED > PAUSED > BEHIND > CAUGHT_UP` |
| `Lag` | backlog `Option<u64>`, oldest_age `Option<Duration>`, coverage, state, retry_after, sampled_at | unknown is `None`, not zero |
| `Targets` | `partition_targets(partitions, members) → per-rank count` | the first `P mod M` ranks get `⌊P/M⌋+1`, the rest `⌊P/M⌋`; `M > P` leaves the high ranks at 0 |
| `AssignmentPlan` | `plan(held, free, rank, targets) → {give_back, take}` | gives back the highest held beyond the target, takes the lowest free up to it |

State transitions of a partition: `FREE → HELD(holder)` by a lease; `HELD → FREE` by a release or an expiry; never `HELD(a) → HELD(b)` directly. A holder renews at a third of the lease.

Drain state of a connector: `BEHIND ↔ CAUGHT_UP` by the lag; `BLOCKED` while the tenant's undispatched events are at the limit; `RATE_LIMITED` while the connector or namespace bucket is empty (with `retry_after`); `PAUSED` while `event_connectors.status = PAUSED`; `FAILING` after the reader's failure threshold. The state in force is the one most recently flushed (`D1731`).

## 2. Tables

Names are prefixed `scaling_` or already belong to connectors. All times are `timestamptz` on the database clock. `sagat_app` receives only the privileges the stores use.

### Migration `20261226000000_scaling_core`

| Table | Columns | Constraints and notes |
|---|---|---|
| `scaling_definitions` | `kind` (`POLICY`/`SOURCE`/`SIGNAL`), `name`, `version int`, `owner`, `spec jsonb`, `author`, `reason`, `created_at`, `removed bool` | PK `(kind, name, version)`; append-only (trigger refuses update and delete); `spec` size bounded by a check; classified |
| `scaling_definition_head` | `kind`, `name`, `version`, `owner`, `removed` | PK `(kind, name)`; FK to the version; the only mutable pointer; restoring is a new version copied from an old one |
| `scaling_policy_state` | `policy_name PK`, `recent jsonb` (raw values with times, bounded), `last_up_at`, `last_down_at`, `updated_at` | written by the evaluator in the decision's transaction |
| `scaling_custom_samples` | `name PK`, `value double precision`, `sampled_at`, `expires_at`, `publisher` | value finite and inside the signal's range (checked on write) |
| `scaling_signal_state` | `name PK`, `value double precision null`, `state`, `reason`, `coverage null`, `sampled_at`, `last_ok_at null` | what the exporter and the API read; ≤ 64 + built-ins rows; `reason` carries the failure class (`timeout`, `refused`, `invalid`, `unavailable`, `credential_unavailable`) and `last_ok_at` is the health of a source |
| `scaling_overrides` | `id uuid PK`, `policy_name`, `kind`, `value int null`, `expires_at`, `author`, `reason`, `created_at`, `revoked_at null` | partial unique index on active `(policy_name, kind)`; `expires_at − created_at ≤ 24 h` |
| `scaling_controls` | `key PK` (`global`), `frozen bool`, `reason`, `author`, `updated_at` | the global switch (FR-036-047) |
| `scaling_desired` | `unit`, `kind`, `parameter`, `value`, `authority`, `decision_id`, `updated_at` | PK `(unit, kind, parameter, authority)`; two rows per target at most (advisory, authoritative) |
| `scaling_decisions` | `id uuid PK` (v7), `policy_name`, `policy_version`, `evaluated_at`, `last_seen_at`, `repeat_count`, `evaluator`, `inputs jsonb`, `raw`, `limits jsonb`, `final`, `outcome`, `authority null`, `reason` | append-only except `last_seen_at`/`repeat_count` on coalescing and the retention purge; index `(policy_name, evaluated_at DESC)` and `(evaluated_at)` |
| `platform_audit` | `id uuid PK`, `actor` (the token subject; `configuration:<digest>`; `scaling-evaluator`), `token_id null`, `action`, `resource_type`, `resource_id`, `reason`, `change_digest`, `trace_id null`, `created_at` | append-only; written in the operator listener's transaction (`D1714`) |

The evaluator's lease is a row of the existing `engine_leases` named `scaling-evaluator`; no new table.

### Migration `20261226000100_connector_lag_members`

| Table or column | Notes |
|---|---|
| `connector_members(holder PK, started_at, expires_at)` | live members of the connector readers; rows expired for over an hour are deleted, as for `scheduler_members` |
| `connector_lag(connector_id, holder, partitions_covered int, partitions_total int, backlog bigint null, oldest_at null, sampled_at)` | PK `(connector_id, holder)`; one row per holder per connector; fresh rows are those inside the freshness limit; FK to `event_connectors` with cascade |
| `event_connectors.drain_state` | text, default `CAUGHT_UP`, check on the six values |
| `event_connectors.drain_state_at`, `rate_retry_at` | set by the existing progress flush |
| `event_connectors.parallelism` | int, default 1, check `1..256`; bounded at write time by the namespace's `connector_parallelism` quota (default and ceiling: `SAGAT_CONNECTOR_MAX_PARALLELISM`) |

### Migration `20261226000200_connector_parallelism_quota`

The quota kind `connector_parallelism` (scope: namespace, unit: messages in flight per replica): the rewritten CHECK on `quota_limits` and `quota_ceilings` that names the resources, the `sagat_app` grants if any are new, and the storage classification, following `20261105000000_iteration_capacity_quota`. The domain list of quota kinds grows from 32 to 33.
| `connector_leases` | unchanged; the stream named `~lag` is the sampler lease of a NATS or SQS connector |

### Settings that are not in the database

The ceilings, the destination allowlist, the database budget (server maximum, reserve, per-replica connections, weights), the pool names and the retention are in the configuration document and are read by the evaluator. They are not rows, so the management interface cannot change them (`D1706`).

## 3. Aggregates the evaluator reads

| Signal (slice 1) | Computed from |
|---|---|
| `ingest.lag.backlog`, `ingest.lag.age_max`, `ingest.lag.excluded_connectors`, `ingest.readers_ceiling` | fresh `connector_lag` rows joined to `event_connectors.drain_state` and `status` |
| `events.pending`, `events.oldest_age` | `inbox_events` pending, capped at 100,000 as today |
| `outbox.backlog`, `outbox.oldest_age` | the monitor's sample |
| `deliveries.pending`, `deliveries.oldest_age` | pending `webhook_deliveries`; the oldest age is a new query |
| `scheduler.lag.max`, `pool.<name>.ready_steps`, `pool.<name>.queue_lag_seconds` | the monitor's sample |
| `source.<name>` | a declared source |
| `<custom>` | `scaling_custom_samples` |

Full list with units, scope and freshness: [`contracts/signals-catalogue.md`](./contracts/signals-catalogue.md).
