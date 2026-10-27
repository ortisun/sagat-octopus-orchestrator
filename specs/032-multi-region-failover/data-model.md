# Data model — feature 032 (proposed, not implemented)

One additive migration, `crates/infra-postgres/migrations/<ts>_region_authority.{up,down}.sql` (the
timestamp is taken at implementation, after the last migration present then; the last one at
`70f09435` is `20261220000000_reveal_grants_role_change`). Everything replicates physically with the rest
of the cluster; nothing here is region-local state.

## Domain types (`crates/domain/src/region.rs`, pure)

| Type | Rule |
|---|---|
| `RegionName` | `^[a-z][a-z0-9-]{1,30}[a-z0-9]$`, no `--`; operator-chosen; at most 3 per cell |
| `CellId` | same grammar as a region name; fixed for the life of a database |
| `Epoch` | `u64`, starts at 1 on `region-init`, `next()` is +1 and the only constructor besides parsing |
| `Role` | `Active`, `Draining`, `Passive`, `Superseded`, `Unconfigured`; `role(config, authority, in_recovery, peer_ahead)` is the plan's truth table |
| `AuthorityState` | `Active`, `Draining` |
| `ReplicationMode` | `SyncQuorum`, `Async` |
| `Protection` | `Protected`, `Unprotected`, `Unknown`; `protection(mode, observed standbys)` is pure |
| `ActivationKind` | `Planned { final_lsn }`, `Unplanned { attestation }` |
| `Residency` | `None` (any region of the cell) or a non-empty set of `RegionName`; `satisfied(cell_regions)` = cell regions ⊆ allowed |

Transitions (each a pure function returning the new authority or a closed refusal):

```text
(none)                 --init(regions, active, mode)-->  epoch 1, ACTIVE
epoch N, ACTIVE        --drain(N)-->                     epoch N, DRAINING
epoch N, DRAINING      --undrain(N)-->                   epoch N, ACTIVE
epoch N, DRAINING      --activate(N, Planned{lsn}) [replayed >= lsn, not in recovery]-->  epoch N+1, ACTIVE, active := target
epoch N, ACTIVE|DRAINING --activate(N, Unplanned{attestation}) [not in recovery, preflight ok]-->  epoch N+1, ACTIVE, active := target
epoch N, any           --mode(N, mode)-->                epoch N, same state, mode := mode
epoch N, any           --topology(N, regions) [residency ok, active ∈ regions]-->  epoch N, regions := regions
any                    --activate(M ≠ current)-->        refused EPOCH_CONFLICT
```

Refusal reasons of the transitions (closed; the full command list is in `contracts/errors.md`):
`EPOCH_CONFLICT`, `IN_RECOVERY`, `NOT_ACTIVE`, `NOT_DRAINING`, `CATCH_UP_UNVERIFIED`,
`ATTESTATION_REQUIRED`, `PREFLIGHT_BLOCKED`, `TARGET_NOT_IN_CELL`, `TARGET_IS_ACTIVE`, `ACTIVE_REMOVED`,
`RESIDENCY_CONFLICT`, `TOO_SOON` (minimum interval between activations), `NOT_INITIALIZED`,
`ALREADY_INITIALIZED`.

## Tables

### `region_authority` — the one record of who may act

| Column | Type / constraint |
|---|---|
| `singleton` | `boolean PRIMARY KEY DEFAULT true CHECK (singleton)` — at most one row |
| `cell_id` | `text NOT NULL` (grammar CHECK) |
| `regions` | `text[] NOT NULL CHECK (cardinality(regions) BETWEEN 2 AND 3)` + element grammar CHECK |
| `active_region` | `text NOT NULL CHECK (active_region = ANY (regions))` |
| `epoch` | `bigint NOT NULL CHECK (epoch >= 1)` |
| `state` | `text NOT NULL CHECK (state IN ('ACTIVE','DRAINING'))` |
| `replication_mode` | `text NOT NULL CHECK (replication_mode IN ('SYNC_QUORUM','ASYNC'))` |
| `activated_at` | `timestamptz NOT NULL` — database clock of the activation (start of the blob grace) |
| `updated_at`, `updated_by` | `timestamptz NOT NULL`, `text NOT NULL` (1–255, no control characters) |

No row = single-region mode. The runtime role has `SELECT` only; the owner role (operator commands)
writes it. A trigger refuses an `UPDATE` that lowers `epoch` or raises it by more than one, and a
`DELETE` (the row is never removed once created; going back to single-region mode is out of scope).

### `region_history` — append-only audit of every change

| Column | Type / constraint |
|---|---|
| `id` | `uuid PRIMARY KEY` (UUIDv7) |
| `kind` | `text NOT NULL CHECK (kind IN ('INIT','DRAIN','UNDRAIN','ACTIVATE_PLANNED','ACTIVATE_UNPLANNED','MODE_CHANGE','TOPOLOGY_CHANGE','DIVERGENCE_REPORT','REINGEST','TOMBSTONES'))` |
| `epoch` | `bigint NOT NULL` — epoch after the change |
| `cell_id`, `regions`, `active_region`, `previous_region`, `state`, `replication_mode` | snapshot after the change (`previous_region` NULL except for activations) |
| `actor` | `text NOT NULL` (1–255) — operator identity given to the command |
| `reason` | `text NOT NULL CHECK (char_length BETWEEN 8 AND 2000)` |
| `attestation` | `text NULL CHECK (char_length BETWEEN 8 AND 2000)`; NOT NULL when `kind = 'ACTIVATE_UNPLANNED'` |
| `cut_lsn` | `pg_lsn NULL` — `pg_last_wal_replay_lsn()` at activation |
| `cut_replay_at` | `timestamptz NULL` — `pg_last_xact_replay_timestamp()` (the former primary's clock) |
| `cut_timeline` | `integer NULL` — timeline id after promotion |
| `final_lsn` | `pg_lsn NULL` — operator-given final LSN (planned) |
| `preflight` | `jsonb NULL` — closed object `{check: outcome}` of fixed check names and `ok|warn|blocked`; bounded 4 KiB |
| `counts` | `jsonb NULL` — divergence/re-ingest counts by table, no identifiers; bounded 16 KiB |
| `platform_version` | `text NOT NULL` (build version of the command) |
| `created_at` | `timestamptz NOT NULL DEFAULT clock_timestamp()` |

`BEFORE UPDATE OR DELETE` and `BEFORE TRUNCATE` triggers reject mutation (pattern of
`execution_reject_mutation()`). Runtime role: `SELECT`. Retained for the life of the cell (one row per
change; activations are rate-limited, so growth is bounded by operator actions). Planned activation
refuses when `pg_last_wal_replay_lsn()` is NULL (a promoted server restarted before activation) — the
operator then uses the unplanned kind with an attestation.

### Column additions (ADR-0020 extension)

| Table | Column | Use |
|---|---|---|
| `scheduler_shards` | `region_epoch bigint NOT NULL DEFAULT 0` | set on take; the fence predicate adds `AND region_epoch = $current`; activation sets every row to `generation + 1`, `expires_at = 'epoch'`, `owner = NULL`, `region_epoch = N+1` |
| `engine_leases` | `region_epoch bigint NOT NULL DEFAULT 0` | singleton take/renew require `region_epoch = $current`; activation deletes rows with `region_epoch < N+1` |
| `run_events` | `region_epoch bigint NOT NULL`, added `DEFAULT 0` (metadata-only for existing rows), then `SET DEFAULT sagat_region_epoch()` | the epoch the event was committed under; key component for followers, span ids, 020 records and 026 cursors (D1594) |

`scheduler_members` and `connector_leases` get no column: activation deletes all their rows (members
re-announce within a keeper round; connectors re-acquire their streams). In single-region mode `$current`
is 0 and the predicates are always true.

### Functions and triggers (database enforcement, D1593)

```sql
-- The epoch of this database's authority; 0 for a single-region cell. Non-volatile, so it is allowed as
-- a column default; one indexed read of a one-row table per inserted event. Runtime role: EXECUTE.
CREATE FUNCTION sagat_region_epoch() RETURNS bigint LANGUAGE sql STABLE
  SET search_path = pg_catalog, pg_temp
  AS $$ SELECT COALESCE((SELECT epoch FROM public.region_authority), 0) $$;
```

`region_lease_guard()` (`BEFORE INSERT OR UPDATE` on `scheduler_shards`, `engine_leases`,
`scheduler_members`, `connector_leases`): when `region_authority` has a row and `NEW.expires_at >
clock_timestamp()` (a take, a renewal or an announcement), it raises `insufficient_privilege` unless
`current_setting('sagat.region', true)` equals `active_region`, `state = 'ACTIVE'` and, for the two tables
with the column, `NEW.region_epoch = epoch`. A release or an expiry passes. Without an authority row it
returns `NEW` at once. Fixed `search_path`, qualified names, `EXECUTE` revoked from `PUBLIC` (the D255
lesson of feature 010).

`region-init` also runs `ALTER ROLE sagat_app SET default_transaction_read_only = on` (owner role; kept
in `pg_db_role_setting`, physically replicated). Region-aware processes connect with
`options=-c sagat.region=<r> -c default_transaction_read_only=off` while their role is `ACTIVE` or
`DRAINING` (in `DRAINING` the triggers refuse new takes because the authority is not `ACTIVE`; reports
and releases still write) and rebuild the pool on every role or epoch change. The
down migration and a future "leave regions" procedure reset the role setting.

### `region_beacons` — durability lease of each process (`sync_quorum` cells, D1590)

| Column | Type / constraint |
|---|---|
| `process` | `text PRIMARY KEY` (replica name with its random start suffix, as ADR-0020's owner) |
| `region` | `text NOT NULL` (grammar CHECK) |
| `at` | `timestamptz NOT NULL` — database clock of the last beacon |

Written only in `sync_quorum` cells, by `UPDATE … SET at = now()` (insert on first use) in its own
transaction with `synchronous_commit` at least `on`; the lease is valid on the process's monotonic clock
from the start of that transaction for `SAGAT_DURABILITY_LEASE_MS`. Rows of processes gone for a day are
deleted by the monitor. At most 1,024 rows (a CHECK through a count trigger is not needed: the insert path
refuses above the ceiling). Runtime role: `SELECT, INSERT, UPDATE, DELETE`.

### `cell_key_fingerprints` — key inventory (C11, D1595)

| Column | Type / constraint |
|---|---|
| `key_name` | `text NOT NULL CHECK (key_name IN ('secrets_key_ring','script_signing','intent_binding','flow_control_digest','push_signing','fairness_digest','decision_link','codec_assertion','template_index_pins'))` |
| `key_id` | `text NOT NULL` (1–64, `^[A-Za-z0-9._:-]+$`) |
| `fingerprint` | `bytea NOT NULL CHECK (octet_length(fingerprint) IN (16, 32))` |
| `recorded_region`, `recorded_at` | `text NOT NULL`, `timestamptz NOT NULL` |

PRIMARY KEY `(key_name, key_id)`. Upserted by the active region's processes at start; read by passive
processes and by the activation preflight. Holds no key material. Runtime role: `SELECT, INSERT, UPDATE`.

### `namespace_placements` — declared residency

| Column | Type / constraint |
|---|---|
| `tenant_id`, `namespace_id` | `uuid`, PRIMARY KEY `(tenant_id, namespace_id)`, FK `(tenant_id, namespace_id) → namespaces (tenant_id, id)` |
| `allowed_regions` | `text[] NULL CHECK (allowed_regions IS NULL OR cardinality(allowed_regions) BETWEEN 1 AND 3)` + grammar |
| `revision` | `bigint NOT NULL DEFAULT 1` — compare-and-set for `PUT` |
| `updated_by`, `updated_at` | as above |

A namespace with no row has no residency declaration. The placement view is derived (authority + this
row); it is never stored twice. Runtime role: `SELECT, INSERT, UPDATE`. Every query carries the tenant.

### `region_fanout` and `region_epoch_notices` — tenant audit and notices after an activation

| Table | Columns |
|---|---|
| `region_fanout` | `epoch bigint PRIMARY KEY`, `cursor_tenant_id uuid NULL`, `pending boolean NOT NULL`, `completed_at timestamptz NULL` |
| `region_epoch_notices` | `(tenant_id, epoch) PRIMARY KEY`, FK `tenant_id → tenants`, `audited_at timestamptz NOT NULL`, `notices integer NOT NULL CHECK (notices >= 0)` |

The fan-out loop (one holder via `engine_leases`, name `region-fanout`) takes ≤ batch tenants after the
cursor in id order and, per tenant, in one transaction: inserts the `region_epoch_notices` row (the
primary key makes a repeat a no-op), the tenant `audit_records` row (action `region.activated`, actor
`system:region`, resource type `cell`, resource id `<cell>:<epoch>`), and one outbound delivery per
opted-in `ops.region.activated` subscription of each namespace into `webhook_deliveries`
(deterministic event id UUIDv5 over `cell, epoch, namespace`; the existing
`webhook_deliveries_event_uniq (subscription_id, event_id)` dedupes); then advances the cursor.
Runtime role: `SELECT, INSERT, UPDATE` on both. Notices rows are retained with the audit trail.

The migration extends the closed kind list of `webhook_subscriptions` (the CHECK added by
`20261015000200_operational_alerts`, `kinds <@ ARRAY['run.finished', 'ops.alert.fired',
'ops.alert.resolved']`) with `ops.region.activated`; the down migration restores it and refuses while a
subscription uses the new kind.

## State that is not new but matters after a failover

| Record | What happens at activation |
|---|---|
| `worker_sessions`, `steps` leases | unchanged (D1569); a lease or session the authority lacks is refused by existing checks |
| `inbox_events`, `connector_offsets` | unchanged; dedupe and positions continue from the cut |
| `event_outbox`, `webhook_deliveries`, outbound deliveries | undelivered rows are republished by the new region with their ids |
| `telemetry_pending` | the projector resumes from its cursor; span ids are computed (ADR-0017) |
| `blobs` | catalog rows present; objects may lag in the bucket (grace from `region_authority.activated_at`) |
| `command_dedupe` (API idempotency keys) and the other receipt tables | replicated up to the cut; a retried request after the cut is new |

## Time classification (028's `crates/infra-postgres/time-classification.json`, C13)

Every time column of this feature is class `infrastructure`: `region_authority.activated_at`,
`.updated_at`; `region_history.created_at`, `.cut_replay_at` (the former primary's clock, recorded as
such); `namespace_placements.updated_at`; `region_fanout.completed_at`; `region_epoch_notices.audited_at`;
`region_beacons.at`; `cell_key_fingerprints.recorded_at`. A test deployment's virtual time never advances
them.

## Storage classification (`crates/infra-postgres/storage-classification.json`)

| Column | Class | Bound |
|---|---|---|
| `region_history.reason`, `.attestation`, `.actor` | `protected_system` | CHECK lengths above |
| `region_history.preflight`, `.counts` | `protected_system` | closed keys, byte-length CHECKs |
| `region_history.regions`, `region_authority.regions`, `namespace_placements.allowed_regions` | `protected_system` | cardinality + grammar CHECKs |
| `region_authority.cell_id`, `.active_region`, `.updated_by` | `protected_system` | grammar / length CHECKs |
| `region_beacons.process`, `.region`; `cell_key_fingerprints.key_name`, `.key_id`, `.fingerprint`, `.recorded_region` | `protected_system` | closed set, grammar and length CHECKs |

None of these columns holds payload, secret or tenant-authored free text except `allowed_regions`, which
is a closed grammar; ADR-0033 classification of payload fields is unaffected.

## Migration rules

- Up: create tables and triggers, add the two columns with defaults (metadata-only on PostgreSQL 11+),
  grants to `sagat_app` when the role exists, partial indexes none (all tables are tiny).
- Down: refuses (raises) when `region_history` has any row or `region_authority` exists, so a rollback
  cannot erase the audit of a region change; on an unconfigured cell it drops everything it added.
- Up/down/up tested in `tests/integration/platform-foundation/migrations.rs` style; the runtime-role
  privilege test proves `sagat_app` cannot write `region_authority` or `region_history`.
- Contract test (D1587): no migration declares `UNLOGGED`, `CREATE SEQUENCE`, `serial`/`bigserial` or an
  identity column.
- `run_events` is large: the column is added with a constant default first (no rewrite), then the
  default is changed to the function (metadata only); the append-only triggers are unaffected.
