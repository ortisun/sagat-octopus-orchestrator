# Data Model: Performance, scalability and high availability

## Context

Scale scheduler, workers and millions of waits without state loss, and quantify the real limits of the architecture.

Data ownership follows the bounded contexts of `docs/architecture/overview.md`, with the reference physical schema in `specs/001-platform-foundation/data-model.md` and illustrative SQL in `contracts/schema.sql`. Feature 010 adds **five tables, two functions, three triggers and eleven indexes** in three migrations (`20261017000000_scheduler_shards`, `20261017000100_active_run_counts`, `20261019000000_activity_quotas`), each with a `.down.sql` that is tested back and forth. The activity-quota migration also extends the closed resource constraints of the feature 009 quota tables.

## What the feature adds

### `run_shard(id uuid) -> smallint`

`(get_byte(uuid_send(id), 15) & 63)`: the last byte of the UUID modulo 64. `IMMUTABLE PARALLEL SAFE STRICT`, so it can lead an index. The domain has the same function (`sagat_domain::sharding::Shard::of`) and a test holds the two to the same answer for 2 000 identifiers. The number of shards is **fixed for the life of a database** (it is in the indexes).

| What it is applied to | Why |
|---|---|
| `steps.run_id`, `runs.id` | the work of a run is its shard's |
| `runs.parent_run_id` | a child that ended is reported to its parent: the action is on the parent, so the shard is the parent's |
| `event_outbox.aggregate_id` | the messages of a run are dispatched by the owner of its shard, in the order they were written |

### `scheduler_shards` — the leases

| Column | Meaning |
|---|---|
| `shard smallint PK` (0..63) | one row per shard, present from the migration and never removed (the runtime role cannot insert or delete) |
| `owner text` | the replica's name with a random suffix chosen at start; `NULL` when never held |
| `generation bigint` | goes up by one on every **takeover**; a renewal does not change it |
| `expires_at timestamptz` | `epoch` when never held (not `-infinity`: before PostgreSQL 17 a timestamp cannot be subtracted from an infinite one) and never `infinity` (a `CHECK`); the lease is good while `expires_at > clock_timestamp()` |
| `acquired_at timestamptz` | when the current owner took it |

States of a lease: **free** (never held, given back, or `expires_at` past), **held** (`expires_at` in the future). Transitions, each one statement that the database serialises on the row:

| Transition | Statement | Result |
|---|---|---|
| take | `UPDATE ... SET owner = $1, generation = generation + 1, expires_at = now + ttl WHERE shard = ANY($2) AND expires_at <= clock_timestamp()` | the replicas racing for a shard cannot both win it: the second finds it held |
| renew | `UPDATE ... SET expires_at = now + ttl WHERE owner = $1 AND generation = $2 AND expires_at > clock_timestamp()`, over the rows it can lock without waiting (`FOR UPDATE SKIP LOCKED`) | only by the owner, at the generation it holds, and only while it is good; a row held `FOR SHARE` by an action (or by an orphan of a cut-off connection) is renewed at the next round, and the others are not held up by it |
| give back | `UPDATE ... SET expires_at = now WHERE owner = $1 AND generation = $2` | the next round of another replica takes it at once |
| fence | `SELECT shard ... WHERE shard = $1 AND generation = $2 AND owner = $3 AND expires_at > clock_timestamp() FOR SHARE` in the transaction of the action | a takeover (an `UPDATE ... FOR UPDATE SKIP LOCKED`) skips a shard whose row an action holds, takes the free ones, and takes the skipped one at the next round; pools end a transaction idle for more than 60 s |

All time is the database's. A lease that ran out is never renewed, only taken again with a new generation, so anything its owner began before it ran out cannot commit.

### `scheduler_members` — the replicas that are alive

`owner text PK`, `started_at`, `expires_at`. A replica announces itself every round (`INSERT ... ON CONFLICT DO UPDATE`); the count of rows with a future `expires_at` is the number of replicas the share is divided by. It is apart from the leases so that a replica that holds nothing yet is counted and the others give it its share.

### `tenant_scheduling` — the weights

`tenant_id uuid PK -> tenants`, `weight smallint CHECK (1..16)`, `updated_at`. Written only by the operator's command (`control-plane scheduling-weight`, one transaction with its audit record, action `quota.changed`, resource `scheduling-weight`); read by the engine every keeper round. A tenant with no row weighs 1. The runtime role can read it and cannot write it.

### `run_active_counts` — the quota of active runs, kept

`tenant_id`, `namespace_id`, `slot smallint (0..15)`, `active bigint`; one row per (tenant, namespace, slot). The slot is the run's identifier modulo 16, so runs that finish together do not all wait for one row. Three triggers on `runs` (`AFTER INSERT` of an unfinished run, `AFTER UPDATE OF finished_at` that changes whether it is finished, `AFTER DELETE` of an unfinished run) call one `SECURITY DEFINER` function that adds or takes away one, never below zero. The quota check (`ensure_room_for_run`, `usage`) sums at most sixteen rows per namespace and at most sixteen per namespace of the tenant, under the same advisory lock as before, whatever the number of runs. The runtime role reads the table and cannot write it. The migration counts the unfinished runs under a `SHARE ROW EXCLUSIVE` lock on `runs` (reads continue, writes wait for the count: run it in a quiet moment on a large database). A counter that drifted (a bug, an operator's `UPDATE`) is re-made by `DELETE FROM run_active_counts` and the same `INSERT ... SELECT` as the migration.

### `activity_quota_limits` — exact queue admission

`tenant_id`, `namespace_id`, `activity` form the primary key; `max_inflight` is 0 to 1,000,000,000 and `updated_by`/`updated_at` identify the last change. The composite namespace foreign key and every query carry the tenant. The activity uses the domain's canonical grammar and is at most 128 bytes. A row is a namespace override for one activity; without one, the namespace-wide `activity_concurrency` setting applies, then the tenant default, then the platform-granted ceiling. With none set the default is effectively unlimited, preserving existing deployments.

The worker claim transaction takes a transaction-scoped advisory lock on `(tenant, namespace, activity)`, counts steps of that activity whose status is `LEASED` or `RUNNING` and whose lease is still in the future on the database clock, and changes a ready step to leased before it commits. Concurrent pollers therefore cannot pass the last place together. A refused candidate remains ready, and the bounded claim scan continues so another activity in that scan can be leased. An expired lease returns its place without a repair job. Each forward, compensation or reconciliation task consumes the quota of the activity name it advertises to workers; two task kinds share capacity only when they use the same activity name.

The setting and its `quota.changed` audit record commit in one transaction. Reads require `quota:read`; changes require `quota:manage`, validate the namespace inside the caller's tenant and cannot exceed the operator-granted `activity_concurrency` ceiling. The runtime refuses a claim without exposing the limit or activity as a metric label.

### The indexes

Eleven partial indexes, shard first, next to the ones of feature 002 (which stay): `steps_{timer,wait_deadline,retry,lease_expiry,attempt_deadline,rehint,child_pending}_shard_idx`, `runs_{deadline,unsettled_children,children_to_cancel}_shard_idx`, `event_outbox_pending_shard_idx`. Each has the shard as its leading column and the time the pass orders by as its second, with the predicate of the pass, so a replica reads the first entries of its own shards and a shard with nothing due costs one probe. The bound of the time is `now()` (research D233).

## Observable state added

| Where | What |
|---|---|
| gauges | `scheduler_shards_owned`, `scheduler_replicas` (keeper), `scheduler_lag_seconds{pass}`, `pool_ready_steps{pool}`, `pool_queue_lag_seconds{pool}` (monitor) |
| counters | `scheduler_shards_{acquired,released,lost}_total`, `scheduler_keeper_failures_total`, `scheduler_fenced_total{kind}`, `scheduler_fairness_{limited,exhausted}_total{pass}`, the `fenced` outcome of `scheduler_actions_total`, and `quota_exceeded_total{limit="activity_concurrency"}` |
| `Report` of a tick | `fenced`: candidates of a shard the replica no longer holds |
| API | tenant/namespace `activity_concurrency` defaults through the existing quota resource; namespace activity overrides through `/namespaces/{namespaceId}/activity-quotas` |
| configuration | `SAGAT_SHARD_LEASE_SECS`, `SAGAT_REPLICA_ID`, `SAGAT_SCHEDULER_CONCURRENCY`, `SAGAT_SCHEDULER_FAIRNESS`, `SAGAT_WORKER_POOLS` |

## In-memory types (no persistence)

- `ShardSet`: a `u64` bitset of shards. `Held { owner, generation, remaining }`: what a replica sees of a lease. `plan(me, replicas, leases) -> Plan { acquire, release }`: pure; the share is `ceil(64 / replicas)`.
- `Placement`: a replica's name and its last snapshot (`leases`, `weights`, `valid_until`), published by the keeper and read by the passes and the dispatcher. `valid_until` is on the replica's monotonic clock and is the **start of the round that renewed the leases plus the lease**, so it never outlives what the database granted.
- `Window { shards, exclude, limit }`: what a candidate query is asked. `Weights`: tenant to weight, default 1.

## Invariants

- Every tenant-scoped record validates the security context. The shard of a run is derived from its identifier and carries no tenant information; no candidate query returns a payload.
- A shard has at most one owner at a time (the lease row), and an action on a run commits only if its owner holds the shard at the generation it believes **in the same transaction** (the fence).
- A replica that cannot renew its leases selects no work after they would have run out, with no call to the database.
- The passes are idempotent under the run's lock whether or not a replica is fenced: the fence bounds redundant work, it does not make the result right.
- A tenant found in a pass gets at least `floor(batch * w / W)` of it, or all it has if that is less (`W` the weight of the tenants found); a lone tenant gets all of it.
- A claim never commits more unexpired held leases for an activity than its effective limit; quota settings and counts never cross a tenant or namespace boundary.
- The messages of one run are dispatched by one owner, in the order they were written; a message claimed and never confirmed is visible again after its visibility timeout.
- Gauges carry no tenant, namespace, run, activity or user text; a pool's name is the operator's, bounded to 32 pools.
- Large payloads still go to the blob store with a digest; nothing here puts one in a metric, a log or a lease.

## Migrations

The first two migrations add the function, four scheduler/count tables, eleven indexes and grants (to `sagat_app` if the role exists: `SELECT, UPDATE` on `scheduler_shards`, full DML on `scheduler_members`, `SELECT` on `tenant_scheduling`). The third adds `activity_quota_limits`, grants its DML to `sagat_app`, and extends the closed resource constraints on `quota_limits` and `quota_ceilings`. The indexes are created in the migration's transaction, which is right for a database that has few rows and wrong for one that has millions: **on a large database create them with `CREATE INDEX CONCURRENTLY` first** (same names, same definitions; the migration uses `IF NOT EXISTS`, so it then finds them and goes on) and check that none is left invalid by a build that failed (`SELECT indexrelid::regclass FROM pg_index WHERE NOT indisvalid`, drop it and build it again) before running the migration (G240). The down migrations drop everything they add and restore the earlier constraints; rollback and forward restoration are tested (`integration platform-foundation: migrations.rs`).
