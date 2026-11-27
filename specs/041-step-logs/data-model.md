# Data model of feature 041 (step logs)

The tables, columns, closed sets, classification entries, grants, quota resources and metric names that the design
[docs/designs/041-step-logs.md](../../docs/designs/041-step-logs.md) builds. The design keeps the invariants (what is
locked, in which order, what is charged, what a reader may see); this file holds the detail an implementation task may
correct without a new review of the design, as long as every invariant still holds. A correction is made in the pull
request of the task that needs it and named in its description.

## 1. Closed sets

| Set | Values (stored as text, lower case) | Wire (proto) |
|---|---|---|
| `level` | `trace`, `debug`, `info`, `warn`, `error`, in that order (`minLevel` compares by it) | `StepLogLevel` |
| `kind` | `line`, `ceiling_marker`, `script_marker` | none: markers are the server's |
| `marker_reason` | `attempt_records`, `attempt_bytes` (for `ceiling_marker`) | none |
| `source` (streams) | `worker`, `script` | none |
| `ceiling_reason` | `attempt_records`, `attempt_bytes`, `tenant_quota` (a ceiling met; the busy loss is not a ceiling, decision D041-129) | none |
| `not_stored_reason` (streams) | `service_busy`: a carry's records were not stored because the tenant's quota could not be checked within its budget or with no wait slot free (decision D041-129, gap G041-026) | none |
| `DropReason` (one domain enum, `crates/domain/src/step_logs.rs`, decision D041-128) | `worker_gap`, `attempt_records`, `attempt_bytes`, `tenant_quota`, `service_busy`, `out_of_order`, `invalid`, `secret`, `content_fallback`, `failed`: the label of `step_log_lines_dropped_total{reason}` (section 7), the keys of the carry's `dropped_by_server` (section 8a), and, the subset `invalid`, `secret`, `content_fallback`, `failed`, the values the carry's `p_mark_reason` may take | none |

The `DROPPED` record kind of the 2026-11-13 revision is gone (D041-028). The `script` source is back for the script
marker of OD-2026-11-15-9 (D041-057): one stream per inserted script execution, keyed on it (D041-060): its
`public_id` is the execution's id, it has the execution's node and attempt and no step, generation or instance path,
and only `StepLogStore::record_script_marker` writes it, after `record_execution` commits. A stream's state on the routes: `deleted` when `deleted_at` is set, `expired`
when `removed_at` is set otherwise, `ended` when `final_at` is set, `ended_without_final` when none of those is set and
the `steps` row of its `step_id` no longer holds this attempt's lease (design section 5), `open` otherwise; a script
stream is `deleted`, `expired` or `ended`.

## 2. Tables (one migration, T041-5)

Timestamp: later than every file in `crates/infra-postgres/migrations/` and on the other open branches when T041-5 is
written (today the last is `20270116000000_batch_selection_failures`).

```sql
CREATE TABLE step_log_streams (
    tenant_id             uuid        NOT NULL,
    stream_id             bigint      GENERATED ALWAYS AS IDENTITY,
    public_id             uuid        NOT NULL,            -- what the routes show; the identity is never exposed;
                                                           -- for source 'script', the script execution's id
    namespace_id          uuid        NOT NULL,
    run_id                uuid        NOT NULL,
    step_id               uuid,                            -- steps.step_id: (version, run, path, node); null for 'script'
    node                  text        NOT NULL,
    instance_path_nodes   text[],                          -- copied from the steps row the lease read saw ('{}' for a
    instance_path_indices integer[],                       -- root step); null for 'script' (no iteration, G041-007)
    attempt               integer     NOT NULL,
    generation            bigint,                          -- null for 'script'
    source                text        NOT NULL DEFAULT 'worker',  -- 'script': the marker stream of a script execution
    records               integer     NOT NULL DEFAULT 0,  -- worker lines stored; the marker is not counted here
    bytes                 bigint      NOT NULL DEFAULT 0,  -- text bytes stored: the UTF-8 octets of message, exception
                                                           -- and fields (the ceilings' measure; the tenant quota
                                                           -- charges 128 more per row)
    last_sequence         bigint      NOT NULL DEFAULT 0,  -- highest worker sequence seen; only rises (GREATEST)
    ceiling_sequence      bigint,                          -- last_sequence when a ceiling was first met
    next_marker_sequence  bigint      NOT NULL DEFAULT 4611686018427387904,  -- 2^62
    dropped_by_worker     bigint      NOT NULL DEFAULT 0,  -- saturating at 2^53
    dropped_by_server     bigint      NOT NULL DEFAULT 0,
    ceiling_reason        text,
    not_stored_reason     text,                            -- 'service_busy' (decision D041-129); never a ceiling
    truncated_records     integer     NOT NULL DEFAULT 0,
    retention_days        smallint    NOT NULL,            -- step_log_retention_days of the namespace at creation (ADR-0027)
    last_stored_xid       xid8,                            -- highest id of a transaction that stored a line of this stream
                                                           -- (design section 2a, the order guard); never lowered; null
                                                           -- before the first
    final_at              timestamptz,                     -- set only by the attempt's carry (its final mark)
    first_at              timestamptz,                     -- server time of the first recorded batch or carry
    last_at               timestamptz NOT NULL,            -- server time (clock_timestamp()) of the last recorded batch or carry
    expires_at            timestamptz NOT NULL,            -- last_at + retention_days, written with every batch and carry
    expires_day           integer     GENERATED ALWAYS AS  -- whole days since the Unix epoch (UTC) of expires_at: the
        ((floor(extract(epoch FROM (expires_at - '1970-01-01 00:00:00+00'::timestamptz)) / 86400))::integer) STORED,
                                                           -- expiry index's key, so that an update keeping the day is
                                                           -- HOT (D041-111); an immutable expression (no time zone)
    deleted_at            timestamptz,                     -- the delete route
    removed_at            timestamptz,                     -- removal began (purge mark or delete): no line is served
    purged_at             timestamptz,                     -- no line is left
    PRIMARY KEY (tenant_id, stream_id),
    CONSTRAINT step_log_streams_public_uniq UNIQUE (tenant_id, public_id),
    CONSTRAINT step_log_streams_attempt_uniq UNIQUE (tenant_id, run_id, step_id, attempt, generation, source),
    CONSTRAINT step_log_streams_source_valid CHECK (source IN ('worker', 'script')),
    CONSTRAINT step_log_streams_source_shape CHECK (
        (source = 'worker' AND step_id IS NOT NULL AND generation IS NOT NULL
            AND instance_path_nodes IS NOT NULL AND instance_path_indices IS NOT NULL)
        OR (source = 'script' AND step_id IS NULL AND generation IS NULL
            AND instance_path_nodes IS NULL AND instance_path_indices IS NULL)),
    CONSTRAINT step_log_streams_attempt_valid CHECK (attempt >= 1 AND (generation IS NULL OR generation >= 0)),
    CONSTRAINT step_log_streams_counters_valid CHECK (records >= 0 AND bytes >= 0 AND last_sequence >= 0
        AND dropped_by_worker BETWEEN 0 AND 9007199254740992 AND dropped_by_server >= 0),
    CONSTRAINT step_log_streams_ceiling_valid CHECK (ceiling_reason IS NULL OR ceiling_reason IN
        ('attempt_records', 'attempt_bytes', 'tenant_quota')),
    CONSTRAINT step_log_streams_not_stored_valid CHECK (not_stored_reason IS NULL
        OR not_stored_reason IN ('service_busy')),        -- plain: the table is new in this migration, so no row
                                                           -- exists to validate (the NOT VALID rule of the migration
                                                           -- skill applies to a CHECK added to an existing table)
    CONSTRAINT step_log_streams_path_valid CHECK (instance_path_nodes IS NULL
        OR execution_instance_path_valid(instance_path_nodes, instance_path_indices)),
    CONSTRAINT step_log_streams_retention_valid CHECK (retention_days BETWEEN 1 AND 365),
    CONSTRAINT step_log_streams_removal_order CHECK ((deleted_at IS NULL OR removed_at IS NOT NULL)
        AND (purged_at IS NULL OR removed_at IS NOT NULL))
) WITH (fillfactor = 70);                                  -- room on each page for HOT updates (D041-111)
CREATE INDEX step_log_streams_run_idx ON step_log_streams (tenant_id, run_id, stream_id);  -- kept (D041-112)
CREATE INDEX step_log_streams_expiry_idx ON step_log_streams (expires_day) WHERE removed_at IS NULL;
CREATE INDEX step_log_streams_removal_idx ON step_log_streams (tenant_id, stream_id)
    WHERE removed_at IS NOT NULL AND purged_at IS NULL;
-- A BEFORE UPDATE trigger, enabled ALWAYS, refuses a change of tenant_id, stream_id, public_id, namespace_id, run_id,
-- step_id, node, the instance path, attempt, generation, source or retention_days, a lower last_sequence, and any of
-- final_at, deleted_at, removed_at, purged_at, last_stored_xid going from set to null, and a lower last_stored_xid, so no
-- writer can undo a delete or a removal or break the order guard.

CREATE TABLE step_log_lines (
    tenant_id     uuid        NOT NULL,
    stream_id     bigint      NOT NULL,
    sequence      bigint      NOT NULL,
    run_id        uuid        NOT NULL,
    stored_xid    xid8        NOT NULL DEFAULT pg_current_xact_id(),
    stored_at     timestamptz NOT NULL DEFAULT clock_timestamp(),
    worker_time   timestamptz,
    level         text        NOT NULL,
    kind          text        NOT NULL,
    truncated     boolean     NOT NULL DEFAULT false,
    message       text,                                    -- null for a marker
    exception     text,
    fields        text,                                    -- canonical JSON (section 5); null when none
    marker_reason text,
    script_execution_id uuid,                              -- for a script_marker: the script_executions row it points to
    PRIMARY KEY (tenant_id, stream_id, sequence),
    CONSTRAINT step_log_lines_stream_fk FOREIGN KEY (tenant_id, stream_id)
        REFERENCES step_log_streams (tenant_id, stream_id),
    CONSTRAINT step_log_lines_level_valid CHECK (level IN ('trace', 'debug', 'info', 'warn', 'error')),
    CONSTRAINT step_log_lines_kind_valid CHECK (
        (kind = 'line' AND marker_reason IS NULL AND script_execution_id IS NULL AND sequence < 4611686018427387904)
        OR (kind = 'ceiling_marker' AND message IS NULL AND exception IS NULL AND fields IS NULL
            AND marker_reason IN ('attempt_records', 'attempt_bytes') AND script_execution_id IS NULL
            AND sequence >= 4611686018427387904)
        OR (kind = 'script_marker' AND message IS NULL AND exception IS NULL AND fields IS NULL
            AND marker_reason IS NULL AND script_execution_id IS NOT NULL
            AND sequence >= 4611686018427387904)),
    CONSTRAINT step_log_lines_sizes CHECK (octet_length(coalesce(message, '')) <= 8192
        AND octet_length(coalesce(exception, '')) <= 8192 AND octet_length(coalesce(fields, '')) <= 2048)
);
-- One transaction stores lines of one stream only, so (stored_xid, sequence) is unique within a run and orders it; no
-- row identity exists, so none reaches a cursor.
CREATE INDEX step_log_lines_read_idx ON step_log_lines (tenant_id, run_id, stored_xid, sequence);
-- A read with `stream` scans only that stream (design section 5).
CREATE INDEX step_log_lines_stream_idx ON step_log_lines (tenant_id, stream_id, stored_xid, sequence);
-- A trigger refuses UPDATE; DELETE is the purge's. The engine never locks a row of either table.
-- No run usage table: there is no run ceiling (OD-2026-11-16-10). No classified-namespace record: every read needs the
-- PII reveal grant (OD-2026-11-16-8).
```

The lines reference their stream (same tenant), a row the engine never locks. No table references `runs` or `steps`
(D041-005). `step_log_lines.fields` is `text`, not `jsonb`, so that the size computed in Rust is the size stored and
measured (`octet_length_utf8`).

## 3. Storage classification and quota triggers

In `crates/infra-postgres/storage-classification.json` (one entry per column, as `script_executions.log` at line 3972):

| Column | Class | Measurement |
|---|---|---|
| `step_log_lines.message`, `.exception`, `.fields` | `step_log` | `octet_length_utf8` |
| `step_log_lines.level` | `step_log` | `fixed_octets`, 128: the per-row charge of OD-2026-11-15-6 on a `NOT NULL` text column, which the manifest test compares (it reads columns with `typlen = -1`, `durable_quotas.rs:323-343`) |
| every other variable-size column of the two tables | `protected_system` | evidence: closed sets of section 1, bounded identifiers |

The quota triggers are created on `step_log_lines` only: the manifest test requires them only on tables with
`content` or `log` columns (`tests/integration/security-multitenancy/durable_quotas.rs:345-411`), and on the counter
tables they would run `to_jsonb` on every update. Their shape is ADR-0027's Amendment of the Amendment of
2026-11-20 (OD-2026-11-20-1 chose it; its wording was accepted as O041-040 (a), OD-2026-11-20-16), not the per-row shape the generator writes for the other
classes:

- `step_log_lines_quota_insert`: `AFTER INSERT … REFERENCING NEW TABLE AS inserted FOR EACH STATEMENT`, a `SECURITY
  DEFINER` trigger function with the pinned `search_path` of its siblings, executable by neither `PUBLIC` nor
  `sagat_app`, which sums per tenant, over `inserted`, what `sagat_quota_measure` measures for each row's `message`,
  `exception`, `fields` (`octet_length_utf8`) and `level` (`fixed_octets`, 128), the same function the per-row release
  uses, and calls `sagat_quota_step_log_charge` (§3b) once per tenant, in tenant order, with that charge; `ENABLE ALWAYS`;
- no deferred flush on insert (ADR-0097, §3b): the charge goes to a lease in the statement trigger itself, so the
  constraint trigger `quota_transaction_deltas_step_log_flush` of the 2026-11-20 text is not created;
- `step_log_lines_quota_delete` and `step_log_lines_quota_flush`: the per-row `AFTER DELETE` delta and flush triggers of
  the generator's shape, so that the purge releases exactly what it deletes;
- no `UPDATE` trigger: lines are never updated (a trigger refuses it, §2).

The manifest test learns that the class `step_log` is measured by this shape, and checks it. The entries' owner
path is the migration file in T041-5 and becomes `crates/infra-postgres/src/step_logs.rs` in T041-6 (the test only
needs an existing file, `durable_quotas.rs:302-308`).

The class `step_log` and the kind `fixed_octets` are those of ADR-0027's Amendment of 2026-11-15 (OD-2026-11-15-5 and
-6; accepted, OD-2026-11-15-16): `fixed_octets` charges a constant per row whatever the value, one branch of
`sagat_quota_measure` (`20261024000000_durable_quotas.up.sql:152-174`); the class has its own usage column
`tenant_retained_usage.step_log_bytes`, its delta `quota_transaction_deltas.step_log_delta`, its policy fence
`quota-policy:tenant_step_log_bytes:<tenant>` taken after the content and log fences, its error
`tenant_step_log_quota_exceeded`, and `quota_accounting_control.step_log_enforcement_enabled`, true from the migration.

Who adds what. T041-4's migration adds `tenant_retained_usage.step_log_bytes` (`bigint NOT NULL DEFAULT 0`) with the
resource, so the usage view has a column to read. T041-5's migration adds the rest: `quota_transaction_deltas.step_log_delta`;
the routing of the class `step_log` in `sagat_quota_row_delta` (today every class but `content` goes to the log delta,
`20261024000000_durable_quotas.up.sql:231-238`), which raises on a class it does not know; `sagat_quota_add_delta`
dropped and created again with the step-log delta as a fifth argument defaulting to 0 (`CREATE OR REPLACE` with a new
argument would create a second function beside the four-argument one, and the present four-argument calls,
`:244-252, :268, :273`, would then be ambiguous), with its `REVOKE … FROM PUBLIC` (`:188`) and `FROM sagat_app` (`:569`) restated for the new signature;
the finalizer's loop filter (`OR step_log_delta <> 0`) and its step-log branch; the control flag; and the accessor
`sagat_quota_step_log_allowance`. The Amendment's locking accessor, `sagat_quota_lock_step_log_usage`, is not built: the
script marker is written in a transaction of its own (design section 7; a Note in ADR-0027, OD-2026-11-16-7). Every
function the migration creates or replaces keeps or has `SECURITY DEFINER`, its pinned `search_path` and its grants as
before, which T041-5 tests. The down migration restores the four-argument `sagat_quota_add_delta` and the bodies of the functions it
replaced.
`step_log_lines.script_execution_id` is a `uuid`, fixed-size, so it is not in the manifest.

## 3b. Exact leases of the step-log quota (ADR-0097, D041-108, D041-112, D041-117; T041-5)

Every step-log charge goes to a lease; nothing of the class `step_log` goes to `quota_transaction_deltas.step_log_delta`
on insert, so the deferred flush trigger `quota_transaction_deltas_step_log_flush` of §3 is **not** created, and a
transaction that only stores step-log rows queues no deferred quota call.

```sql
ALTER TABLE tenant_retained_usage ADD COLUMN step_log_leased_bytes bigint NOT NULL DEFAULT 0;
ALTER TABLE tenant_retained_usage ADD CONSTRAINT tenant_retained_usage_step_log_leased_nonnegative
    CHECK (step_log_leased_bytes >= 0) NOT VALID;          -- validated by task T041-5's second migration,
                                                           -- `<timestamp>_step_logs_validate`, as T041-4's CHECKs are
-- T041-4 adds no CHECK (step_log_bytes >= 0) (removed from its unmerged migrations, D041-113): S may be negative while
-- released bytes were charged in leases; U = S + Σ used >= 0 is checked by every release (D041-112).
-- One lease per tenant and server backend that charges for it (D041-117): `writer` is `pg_backend_pid()`. A backend
-- runs one transaction at a time, so no writer ever meets another's lock on its lease.
CREATE TABLE step_log_quota_leases (
    tenant_id  uuid        NOT NULL,                       -- no foreign key (D041-143): its check would take
                                                           -- FOR KEY SHARE on the usage row on a second charge of a
                                                           -- transaction; the usage row's AFTER DELETE trigger
                                                           -- `tenant_retained_usage_step_log_leases_cascade` removes a
                                                           -- tenant's leases
    writer     integer     NOT NULL,
    granted    bigint      NOT NULL DEFAULT 0,
    used       bigint      NOT NULL DEFAULT 0,
    charged_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (tenant_id, writer),
    CONSTRAINT step_log_quota_leases_used_within CHECK (0 <= used AND used <= granted)
) WITH (fillfactor = 50, autovacuum_vacuum_scale_factor = 0, autovacuum_vacuum_threshold = 1000);
-- No index besides the primary key: every charge sets `charged_at`, which an index would make a non-HOT update; the
-- idle reclaim scans one tenant's rows by the key's prefix, at most the backends that charged for it (D041-119).
```

S is `step_log_bytes` (the folded usage), G is `step_log_leased_bytes`; the usage is U = S + Σ `used` and the leased room
R = Σ (`granted` − `used`). Invariants, each tested: (I1) the lease `CHECK`; (I2) at every commit G = Σ `granted` of the
tenant's leases, both written only by a transaction holding the usage row; (I3) G ≥ 0 (`CHECK`) and U ≥ 0 (checked by the
release); (I4) a grant commits only with S + G ≤ L after it; (I5) a charge into a lease commits only with S + G ≤ L for
the L it read. L is the limit the finalizer's step-log branch works out (`quota_limits`, else `quota_ceilings`, else
1 GiB, bounded by the ceiling), read after the step-log fence. With `quota_accounting_control.step_log_enforcement_enabled`
off, I4 and I5 are not checked (a refill grants without the limit) and U stays exact.

`SECURITY DEFINER` functions of ADR-0027's pattern (fixed `search_path = pg_catalog, pg_temp`, every name written
`public.…`, trusted owner, `REVOKE ALL … FROM PUBLIC`); `EXECUTE` to `sagat_app` only on the six definer functions that ADR-0097 decision 1
lists (`sagat_quota_step_log_reserve`, `sagat_quota_step_log_top_up`, `sagat_quota_step_log_usage`,
`sagat_quota_step_log_counters`, `sagat_quota_step_log_idle_tenants`, `sagat_quota_step_log_reclaim_idle`), revoked
explicitly from the others; the trigger function of §3 calls the charge, the charge the refill, and the finalizer and
the limit editor the others (the top-up calls no refill: it folds and grants by itself, decision D041-134). Two invoker helpers (`SECURITY INVOKER`, `EXECUTE` to `sagat_app`,
revoked from `PUBLIC`; they read no table), so that SQL and its tests read one value each:
`sagat_quota_step_log_top_up_mark()`, `IMMUTABLE`, returns 65,536 (the top-up mark *M*, decision D041-124);
`sagat_quota_step_log_wait_slots()`, `STABLE`, returns *Q*, the replica's wait slots: `current_setting(
'sagat.step_log_wait_slots', true)` as an integer clamped to 1 to 64, 1 when unset, empty or not a number (decisions
D041-134 and D041-137); the store sets it per transaction, transaction-locally (`set_config(…, true)`), in a statement
it already sends (the carry's `p_wait_slots`, a batch's T1, the script marker's first statement), to max(1, ⌊N / 4⌋),
N its pool's maximum size (task T041-5), so that it travels with the transaction through a pooler in transaction mode:
5 at a pool of 20 (the Helm chart's), 2 at the binary's default of 10 and at 8. The tenant's slots are
*P* = ⌈*Q* / 2⌉: 3 and 1.

*Wait slots* (decisions D041-125 and D041-134). Every wait for a tenant's usage row on the step-log path (a bounded
refill: inside a report's carry, a batch or the script marker; the top-up never waits) first tries the row without
waiting, `SELECT … FOR UPDATE SKIP LOCKED` and, with no row, `SELECT EXISTS (…)` without a lock (so that no exception
block, and no subtransaction, is needed in a batch's trigger). Only when the row is held, or absent (its
`INSERT … ON CONFLICT DO NOTHING` can wait for a concurrent inserter), does it take, both tried in order and never
waited for:
one of the tenant's *P* slots on this replica, `pg_try_advisory_xact_lock(hashtextextended('sagat:step_log_wait:t:' ||
tenant || ':' || current_setting('application_name') || ':' || slot, 0))` for slot 1 to *P*; then one of the
replica's *Q* slots, `pg_try_advisory_xact_lock(hashtextextended('sagat:step_log_wait:r:' ||
current_setting('application_name') || ':' || slot, 0))` for slot 1 to *Q* (single 64-bit keys, as the other advisory
keys of `crates/infra-postgres`); with both it waits for the row within *b*; with either kind none free it raises
`55P03` (`lock_not_available`, the fixed text "step-log wait slots taken"), which each caller maps as it maps a wait past
*b*: the carry's block to the mark-only reason `service_busy`, a batch to `RETRY_LATER`, the marker to `skipped_quota`.
The refill runs in the insert's statement trigger, where nothing tells a batch from a carry, so batches, markers and
reports share the slots: on one replica all of a tenant's step-log waits on its usage row hold at most *P* of the pool's
connections and all tenants' at most *Q*, so on a replica's own pool step-log waits and Logs calls hold at most the Logs
cap ⌊N / 2⌋ plus *Q* and at least N − ⌊N / 2⌋ − max(1, ⌊N / 4⌋) connections (5 of 20, 3 of 10, 2 of 8) are never held
by them; behind a pooler in transaction mode the server backends are bounded only by the sizing rule of design section
2b. The slot keys and *Q* are the one case in which these functions read settings (`application_name`, which the pool
sets at connect and PgBouncer passes on; the transaction-local `sagat.step_log_wait_slots`, which the store sets); a
runtime-role session that changes them only changes which slots it shares or how many it takes on its own replica, and
the bound protects the pool against load, not against the runtime role, which is trusted code. A collision of two keys only shares slots
(more busy answers, never a wrong charge); a replica whose `application_name` is not per-replica shares its slots with
the replicas of the same name (fewer waits than intended, never more for one name). The advisory locks are released at
the end of the waiting transaction.

- `sagat_quota_step_log_charge(tenant, bytes)`: one statement takes the content, log and step-log fences shared (in that
  order: one select list, evaluated left to right); the limit; `SELECT step_log_bytes, step_log_leased_bytes` without a
  lock; when S + G ≤ L, `UPDATE public.step_log_quota_leases SET used = used + bytes, charged_at = clock_timestamp()
  WHERE tenant_id = tenant AND writer = (SELECT writer … WHERE tenant_id = tenant AND writer = pg_backend_pid() AND
  granted - used >= bytes FOR UPDATE SKIP LOCKED) RETURNING writer` (the writer's own lease, an uncorrelated scalar
  subquery run once; only a reclaim, holding the usage row, can hold it); no row, or S + G > L: the refill.
- `sagat_quota_step_log_refill_wait()`: the fallback's budget *b*, `interval '200 milliseconds'` (D041-119; design
  section 2b), `STABLE`; read by the refill, never a session setting (the built functions read none but the wait slots'
  two, above).
- `sagat_quota_step_log_refill(tenant, bytes, charge)`: reads L itself (every caller is bounded: a charge inside a
  report or a batch, or the marker's; the top-up does not call it); first tries the usage row without waiting and, when
  it is held or absent, takes a tenant and a replica wait slot (above),
  then sets `lock_timeout` to *b* for its lock of the usage row unless the transaction's own is lower (an ingest's
  200 ms; a `lock_timeout` of 0, which a report transaction runs with, means no timeout and is never lower) and restores
  the caller's value after it; then `SELECT … FOR UPDATE` of the tenant's usage row,
  and only when no row exists `INSERT … ON CONFLICT DO NOTHING` then the same select (no block: a wait past *b* raises
  `55P03` to the caller, the carry's block or the batch's mapping); when L − S − G < max(bytes, 1 MiB), every other
  lease holding a grant that no open transaction holds (`FOR UPDATE SKIP LOCKED`, writer order) gives its room back
  (S += `used`, G −= `granted`, both 0); the writer's own lease `FOR UPDATE NOWAIT` (only a holder of the usage row, or
  a limit change holding the fence exclusively, could hold it: nothing waits after the usage row), inserted when
  absent, is folded whole (S += `used`, G −= `granted`) and granted g = max(bytes, min(1 MiB, ⌊F / (2(A + 1))⌋)),
  F = L − S − G and A the tenant's other leases holding a grant, raising `tenant_step_log_quota_held` when g > F and
  the room missing is in leases it could not lock (their room read without a lock) and `tenant_step_log_quota_exceeded`
  otherwise (and, for a call with `bytes` = 0, `tenant_step_log_quota_exceeded` when g = 0: there is no room to grant);
  then `granted` = g, `used` = bytes (0 when not `charge`), `charged_at` = now, G += g. With
  `step_log_enforcement_enabled` off, the charge skips its S + G ≤ L test and the refill its I4 check.
- `sagat_quota_step_log_top_up(tenant) RETURNS TABLE (outcome text, room bigint)` (`EXECUTE` to `sagat_app`; decision
  D041-124, OD-2026-11-20-18; it replaces the ensure of decision D041-120): the store calls it with the principal's
  tenant only after a carrying report has committed and its carry returned `top_up = true` (section 8a), on that report's
  connection, as one statement in its own implicit transaction (autocommit), under no run lock; the report's answer does
  not wait for it. It takes no caller amount, so no caller can make it lease more than its share and it has no `invalid`
  answer. `contended` when the tenant's step-log fence cannot be had shared at once (tried); `room` when the writer's own
  lease already has room ≥ *M* (read without a lock); otherwise the usage row `FOR UPDATE SKIP LOCKED` only, so it
  opens no subtransaction (decision D041-134: no wait slot, no wait; no row, held or absent: `busy` at once; absent is
  unreachable after a charging carry, whose charge created the row); with it, reads L, S and G, computes the share
  min(1 MiB, ⌊F / (2(A + 1))⌋), and answers, in this order, `limit` when the share is 0, `room` when it is not above
  the writer's current room, and otherwise folds the writer's lease (S += `used`, G −= `granted`; inserted when absent) and grants it g = that share, no charge, never more,
  so the held-room bound of design section 2b holds unchanged: `made`. It never reclaims another lease, so a top-up never
  takes another writer's room. It never raises for these; it returns its answer and the writer's room after it. Near the
  limit the share can be below *M*, so a backend tops up after nearly every carry there, off the run lock, taking no wait
  slot. Any other error of its statement is the store's answer `error`: the store counts
  it and does nothing else (the report has already committed).
- `sagat_quota_step_log_idle_tenants(max integer) RETURNS SETOF uuid` (`EXECUTE` to `sagat_app`; the purge pass): at most
  `max` tenant ids that have a lease not charged for 10 minutes or whose backend has ended (its `writer` absent from
  `pg_stat_activity`); ids only.
- `sagat_quota_step_log_reclaim_idle(tenant) RETURNS TABLE (reclaimed integer, skipped boolean)` (`EXECUTE` to
  `sagat_app`; called by the purge pass, one tenant a transaction, decisions D041-117, D041-119, D041-120): returns
  `(0, false)` at once when, read without a lock, the tenant has no lease idle for 10 minutes or of an ended backend;
  tries the tenant's step-log fence shared (held: `(0, true)`, the tenant is skipped this pass, so that holding a lease
  always means holding that fence); then under the usage row every such lease, `SKIP LOCKED`, is folded (S += `used`,
  G −= `granted`) and deleted; the interval is fixed in the function; returns `(the count, false)`
  (`step_log_quota_refills_total{kind="idle"}` and `{kind="ended"}`).
- `sagat_quota_step_log_reserve(tenant, bytes)` (`EXECUTE` to `sagat_app`, called by the carry before any write,
  D041-113, D041-114): false for `bytes` ≤ 0 or when the tenant's step-log fence cannot be had shared at once (it tries it
  itself: holding a lease always means holding that fence, which the reclaim's `NOWAIT` relies on); otherwise, unless the
  quota is not enforced, S + G ≤ L read without a lock, and the writer's own lease with room ≥ bytes locked `FOR UPDATE
  SKIP LOCKED` for the transaction, nothing written. It returns `RETURNS TABLE (reserved boolean, room bigint)`, one
  row: whether it locked such a lease, and the writer's own lease's room (`granted` − `used`, read under the lock when it
  locked it, else without a lock; NULL when the writer has no lease of that tenant), as ADR-0097 decision 3 describes
  it. This is how the carry, a `SECURITY INVOKER` function that cannot read the lease table, learns the room it returns
  (section 8a). The carry's `bytes` is an upper bound of what CD's
  trigger will measure, computed before CC for either stream case: every sent record's `sagat_quota_measure` sizes plus 128
  a row, plus 128 for a possible ceiling marker (a property test of T041-36 compares it with the measured charge over new
  and existing streams, each ceiling, Unicode, JSON fields and NUL-replaced text). The charge of CD then finds
  that lease (a transaction's own lock is not skipped), so a reserved carry's charge cannot be refused. It can lock
  only the calling backend's own lease of the tenant it is given; the carry passes its report's tenant.
- `sagat_quota_step_log_reclaim()`: an `ENABLE ALWAYS` row trigger function on `quota_limits` and `quota_ceilings`, for
  rows of `tenant_step_log_bytes` (D041-113): it takes the step-log fence of the row's tenant exclusively itself (at once
  when the editor holds it, so no lease can be held), then the tenant's leases `FOR UPDATE NOWAIT`, then the usage row,
  and gives every lease's room back (S += Σ `used`, G = 0, leases zeroed), whoever wrote the row. Not executable by anyone.
- `sagat_quota_step_log_usage(tenant)` (`EXECUTE` to `sagat_app`, `STABLE`): S + Σ `used` in one snapshot, without a lock;
  the quota view of `tenant_step_log_bytes` and `sagat_quota_step_log_allowance` read it. The quota view calls it inside
  the one `SELECT` that also reads the membership role and the grants (`crates/infra-postgres/src/reveal_grants.rs`), so
  that the grant and the usage share the statement's snapshot (D041-141); `STABLE`, since a `VOLATILE` function's queries
  take snapshots of their own.
  `step_log_leased_bytes` is never shown.
- The finalizer's step-log branch (in `sagat_quota_flush_deltas`, ADR-0027's Amendment item 3), now reached only by a
  release (a negative `step_log_delta` from the per-row `AFTER DELETE` trigger): after the fences, under the usage row,
  Σ `used` read after the lock; S − released + Σ `used` < 0 raises `step_log_usage_underflow`; else S −= released. It
  takes no lease.

The usage the quota API shows for `tenant_step_log_bytes` and `sagat_quota_step_log_allowance(tenant)` returns is
S + Σ `used`. Lock order: L3 (the class fences, shared) → the writer's own lease (only tried, `SKIP LOCKED`) → L4 (the
usage row, by a top-up after a report, a fallback refill (at most *b* for each lock acquisition inside a report or batch,
behind a wait slot when the row is held), a release, an idle reclaim, a limit change and the content and log finalizer) → the writer's own lease again (by its refill, `NOWAIT`: nothing waits after
L4). No transaction waits for a lease it does not own,
and no holder of L4 waits after taking it for anything a waiter holds (design section 2b), so a refill's wait for L4
cannot deadlock.

Metrics (closed labels, no tenant; section 7 names each one's single producer): `step_log_quota_refills_total{kind}` and
the gauge `step_log_quota_lease_rows` come from SQL; `step_log_quota_top_ups_total{outcome}`,
`step_log_quota_fallbacks_total`, `step_log_quota_refusals_total{cause}` and `step_log_purge_failures_total{cause}`
come from the store. Events inside SQL never update a counter row (it would be a hot row again): the refill, the
reclaims and the release advance sequences (`step_log_quota_refills_seq`, one per `kind`; `REVOKE ALL … FROM sagat_app`
on each, in the privilege audit), which a definer accessor `sagat_quota_step_log_counters()` returns with the lease-row
count; only the replica that holds shard 0 publishes them (as the raw, monotonic values), so a sum across replicas is
never multiplied. A sequence is not transactional, so these count attempts, a refill whose transaction rolled back
included. No sequence counts refusals: the carry's function returns its refusal cause, which the store counts.

Budget line of one insert statement of *k* step-log rows charged into a free lease (pinned by
`tests/performance/scalability-ha/step_log_leases_tests.rs::a_step_log_insert_costs_its_budget_line` on the prototype):
rows written one lease row, no usage row, no delta row; nested statements 6 for the charge + *k* foreign-key checks
(`step_log_lines_stream_fk`); deferred calls at the commit 0. A refill adds one write of the usage row and one lease row
(plus one write per lease it reclaims near the limit, and the insert of the writer's lease when it had none). A short
step's carry of ten records on the fast path ran 26 nested statements and no subtransaction on the prototype
(`a_short_steps_carry_writes_its_new_stream_once`, `only_a_carry_that_needs_a_refill_opens_a_subtransaction`); the built
function's count (its returned `path`, `room` and `top_up`, the keys built inside it and the `p_mark_reason` check change
it) is recorded by task T041-36 with its derivation in the test comment, seen failing once, never taken from a first run.

The down migration refuses while a row of `step_log_lines` exists; otherwise U = 0, and it folds Σ `used` into S (so S = 0,
which T041-4's down requires), then drops the leases, the column, the functions and the triggers, and restores what it
replaced.

## 4. Grants (the migrations' `DO $grants$` blocks)

| Table | `sagat_app` |
|---|---|
| `step_log_lines` | `SELECT, INSERT, DELETE` |
| `step_log_streams` | `SELECT, INSERT, UPDATE` |
| `step_log_quota_leases` | none, revoked explicitly: only the `SECURITY DEFINER` functions of §3b write or read it (ADR-0097) |

Usage of the identity sequence of `step_log_streams`. No `TRUNCATE` and no grant on `namespaces`. The first new `SECURITY
DEFINER` function is the accessor of ADR-0027's Amendment, `sagat_quota_step_log_allowance(tenant)`, of the existing
pattern: a fixed `search_path`, a trusted owner, `REVOKE ALL … FROM PUBLIC` (as `20261024000000_durable_quotas.up.sql:294`
does, since PostgreSQL grants `EXECUTE` to `PUBLIC` on a new function) and `EXECUTE` to `sagat_app` (as `:572`). It
exists because the runtime role may not read `quota_accounting_control` (`:564-572`). Two more functions, of the 2026-11-20 revision (OD-2026-11-20-1): the statement-level quota trigger function of §3,
`sagat_quota_step_log_insert_trigger`
(`SECURITY DEFINER`, pinned `search_path`, `REVOKE ALL … FROM PUBLIC`, no `EXECUTE` to `sagat_app`, since only its trigger
calls it), and `sagat_step_log_carry` of §8a (`SECURITY INVOKER`, pinned `search_path`, `REVOKE ALL … FROM PUBLIC`,
`EXECUTE` to `sagat_app`), created by T041-36's migration. And the lease functions of §3b,
`sagat_quota_step_log_charge`, `sagat_quota_step_log_refill`, `sagat_quota_step_log_refill_wait` and the trigger
function `sagat_quota_step_log_reclaim` (`SECURITY DEFINER`, pinned `search_path`, `REVOKE ALL … FROM PUBLIC` and `FROM
sagat_app`), and `sagat_quota_step_log_reserve`, `sagat_quota_step_log_top_up`, `sagat_quota_step_log_idle_tenants`,
`sagat_quota_step_log_reclaim_idle`, `sagat_quota_step_log_usage` and `sagat_quota_step_log_counters` (`EXECUTE` to
`sagat_app`: the six definer functions the runtime role may execute); the two invoker helpers
`sagat_quota_step_log_top_up_mark()` (`IMMUTABLE`) and `sagat_quota_step_log_wait_slots()` (`STABLE`, decision
D041-134), both `SECURITY INVOKER`, `REVOKE ALL … FROM PUBLIC`, `EXECUTE` to `sagat_app`; `REVOKE ALL ON
step_log_quota_leases FROM sagat_app`; `REVOKE ALL` on every sequence the counters use from `sagat_app`; each function
and sequence in the privilege audit. The runtime role has `SELECT` on `tenant_retained_usage` today
(`20261024000000_durable_quotas.up.sql:566`), so it can read `step_log_bytes` (S, which may be negative) and
`step_log_leased_bytes` directly: a source-text check of T041-5 fails when any Rust source under `crates/` or `apps/`,
or any SQL outside the bodies of the §3b functions, the finalizer's step-log branch and the migrations' down files, reads
either column other than through `sagat_quota_step_log_usage` (the §3b functions, the finalizer and the down migration
must read them, so the check is scoped to everything else). No other grant. The down migration
refuses while a row of `step_log_lines` exists.

## 5. Canonical JSON of `fields`

A JSON object; keys sorted by their UTF-8 bytes; no whitespace; strings with the minimal escaping of RFC 8259 (`"`,
`\`, and U+0000 to U+001F as `\uXXXX`, the rest literal); `int_value` as a decimal integer; `double_value` in the
form of ECMAScript `Number::toString` (integral values below 10^21 without a fraction or an exponent, so `12345678.0`
is `12345678`); booleans `true`/`false`. Keys that collide after redaction keep the first in the order sent and get
`~2`, `~3` … appended. Fields are kept in key order until the next would pass 32 keys or 2,048 bytes; the rest are
dropped and `truncated` is set. The same form is the "decimal text" an SDK redacts in a number (FR-041-007): an SDK
formats the number this way, and if a secret value occurs in it the field becomes the string `[REDACTED]`.

## 6. Quota resources (T041-4)

All at `QuotaScope::Namespace` (`crates/domain/src/quota.rs:112-117`), settable per tenant and per namespace, each added
to `QuotaResource` with a new `minimum()` that `validate` and `validate_ceiling` check (the catalogue has a `maximum()`
only), to the CHECK constraints `quota_limits_resource_valid`, `quota_ceilings_resource_valid` and
`quota_limits_tenant_scoped` (the latest restatement is `20270108000000_pending_resets.up.sql:15, 28-30`; T041-4's
migration restates the three), and, in T041-3, to `Quota.resource` and `QuotaResourceName` in `openapi.yaml`.
Correction of T041-4 (2026-11-19): the migration also restates `quota_limits_value_range` and
`quota_ceilings_value_range` (last restated by `20261127000000_database_policies`) with the ranges of the table below,
since their `ELSE` branch caps a value at 10^9 and refused the 1 GiB default of `tenant_step_log_bytes` at the ceiling;
every CHECK is added `NOT VALID` and validated by `20270201000100_step_log_quotas_validate` (design 043, D043-145).

| Name | Kind | Unit | Range | Default (= default ceiling) | Meaning of 0 |
|---|---|---|---|---|---|
| `step_log_attempt_records` | `Policy` | items | 0 to 100,000 | 10,000 | capture is off in the namespace: a named exception, since `validate` and `validate_ceiling` refuse 0 for every `Policy` resource today (`quota.rs:547-549, 569`) |
| `step_log_attempt_bytes` | `Policy` | bytes | 1,024 to 16 MiB | 1 MiB | refused |
| `step_log_retention_days` | `Retention` | days | 1 to 365 | 14 | refused |

And one tenant resource, of ADR-0027's Amendment (OD-2026-11-15-5): `tenant_step_log_bytes`, `QuotaScope::Tenant`,
kind `Storage`, unit bytes, maximum that of `tenant_log_bytes`, default and default ceiling 1 GiB, with usage reported
from `tenant_retained_usage.step_log_bytes`.

There is no run resource (OD-2026-11-16-10). Each namespace resource stays configurable per namespace (OD-2026-11-16-9).
Changing any of them is audited by the quota catalogue as every quota change is. The ingest reads the three values of
the run's namespace in its transaction (design section 2, T3); a new stream keeps the retention it read
(`retention_days`), so a later change of the retention applies to new streams only (ADR-0027).

Admission defaults (decision D041-127, from task T041-35's fourth run, run 44834024933, workload (g): one
`AppendStepLogs` batch of ten lines, its answer awaited, then a carry of ten more; one pool, `SAGAT_DB_MAX_CONNECTIONS`,
the replica's database pool, which the benchmark builds as `API_POOL` = 20, design section 2, "Capacity"):
- `SAGAT_STEP_LOGS_TENANT_CALLS` (tickets per tenant per replica): 7, at most the cap. Gate (4)'s formula,
  ⌈rate × S p99 / 0.7⌉ per replica, on the medians of the rounds used: at 1 × 16, 275 batches/s × 16.4 ms / 0.7 = 6.44,
  so 7; at 8 × 128, 131 / 8 = 16.375 batches/s per replica × 180.4 ms / 0.7 = 4.22, so 5. The larger is the default.
  Headroom: the 0.7 use factor is 1 / 0.7 − 1 = 43% over the mean need at the p99 service time. Two of the eight valid
  rounds needed 10 at 1 × 16 (rounds 4 and 6), so one tenant at the benchmark's saturation can meet `RETRY_LATER` (the
  batch is resent; it is lost only after 10 s); the runbook names the setting.
- `SAGAT_GRPC_MAX_LOG_CALLS` (the Logs class cap per replica; a value above the default is refused at start,
  decision D041-150): half of `SAGAT_DB_MAX_CONNECTIONS`, rounded down, at least 1: 10 of a pool of 20 (the chart's; 5 of the binary's default of 10), the configuration the fourth run measured (cap and tickets 10) and the most the
  design allows (half the pool). A second tenant keeps 10 − 7 = 3 slots while the first holds its 7 tickets.
- `sagat.step_log_wait_slots` (*Q*, set by the store per transaction, decisions D041-134 and D041-137):
  max(1, ⌊N / 4⌋), 5 of 20; the tenant's wait slots *P* = ⌈*Q* / 2⌉, 3. Step-log waits and Logs calls then hold at most
  10 + 5 = 15 of the 20 connections (the Logs cap and the replica's wait slots), and at least 5 are never held by them.
  At the binary's default of 10: cap 5, tickets 5 (at most the cap), *Q* 2, *P* 1, at least 10 − 5 − 2 = 3 never held.
  At a pool of 8: cap 4, tickets 4, *Q* 2, *P* 1, at least 8 − 4 − 2 = 2 never held. Below a pool of 16 (cap 8) one
  tenant's tickets, min(7, cap), equal the cap, so one tenant can take the whole Logs class of a replica (a second
  tenant's batches answer `RETRY_LATER`; its carries and the reserve above are unaffected); the runbook states it. Gate (4) of SC-041-007 is judged at the cap and tickets of 10 the fourth run
  measured; task T041-37 measures ticket refusals at these defaults (decision D041-135).

## 7. Metrics (bare names; labels are closed sets, never tenant, namespace, run, node or worker)

Server, each metric with exactly one producer (decision D041-128):
- `step_log_batches_total{outcome}` (`stored`, `duplicate`, `final_only`, `stop_attempt`, `stop_capture`,
  `retry_later`, `invalid`): the ingest (task T041-6); `invalid` counts P4's refusals only, never a request the gRPC
  handler could not parse (decision D041-151).
- `step_log_carries_total{outcome}` (`written`, `mark_only`, `off`, `invalid`, `refused`, `removed`, `final_recorded`,
  `contended`, `failed`): the store, once per report the carry rode on (task T041-36). Out-of-order records are a drop
  reason, not an outcome.
- `step_log_lines_stored_total{level}`, `step_log_bytes_stored_total`, `step_log_lines_truncated_total`,
  `step_log_redactions_total{rule}` (`shape`, `field_name`): the application, from the rows a batch's T8 or a carry's
  call returned (tasks T041-6 and T041-36).
- `step_log_lines_dropped_total{reason}`, the label the domain enum `DropReason` of §1 (`worker_gap`, `attempt_records`,
  `attempt_bytes`, `tenant_quota`, `service_busy`, `out_of_order`, `invalid`, `secret`, `content_fallback`, `failed`):
  the application, from a batch's plan and from a carry's returned `dropped_by_worker` (`worker_gap`) and
  `dropped_by_server` (keyed by `DropReason`); `failed` counts the records of a carry whose call failed and whose report
  was re-run without it.
- `step_log_script_markers_total{outcome}` (`written`, `skipped_quota`, `skipped_off`, `skipped_finished`, `failed`): the
  script service (task T041-13).
- `step_log_ingest_seconds`, `step_log_lock_timeouts_total` (a batch's `55P03`, whatever lock it met), and
  `step_log_quota_retries_total`: the ingest.
- `step_log_ticket_refusals_total` (no label): the gRPC handler, one per batch answered `RETRY_LATER` because its
  tenant's tickets (P3) were all taken; the ingest never sees such a batch, so `step_log_batches_total` does not count
  it (decision D041-149). The Logs class itself is counted by the transport's `grpc_log_calls_in_flight` (a gauge) and
  `grpc_shed_total{class="logs"}`, and a batch refused by its own rate budgets by
  `http_rate_limited_total{scope="step_logs_subject"|"step_logs_tenant"}` (decision D041-150).
- `step_log_cross_tenant_check_failures_total` (no label): the ingest, when the check for a run of another tenant after
  a `LEASE` answer fails, so that a skipped cross-tenant audit is never silent (decision D041-148 (9)).
- `step_log_quota_top_ups_total{outcome}` (`room`, `made`, `limit`, `busy`, `contended`, `error`): the store,
  one per top-up, from its answer (task T041-36).
- `step_log_quota_fallbacks_total` (no label): the store, one per carry that returned `path = refill` (task T041-36).
  Every count taken from a carry's return (this one, the refusals' `cause`, the drops and the stored lines) is
  recorded only when that carry's transaction commits: a report whose commit raises (its content fallback, a failed
  `COMMIT`) counts its re-run's carry, never the rolled-back one (decision D041-137).
  A batch's and the marker's refills are their ordinary path and are counted only in `step_log_quota_refills_total`.
- `step_log_quota_refusals_total{cause}` (`limit`, `held`, `busy`): the store, from a carry's returned `cause` and from
  a batch's T8 raise by constraint name (`tenant_step_log_quota_exceeded`: `limit`; `tenant_step_log_quota_held`:
  `held`). A batch's `55P03` cannot be told apart from its other lock timeouts and is counted only in
  `step_log_lock_timeouts_total`, never as `busy`.
- `step_log_quota_refills_total{kind}` (`refill`, `reclaim`, `idle`, `ended`) and the gauge `step_log_quota_lease_rows`:
  SQL sequences and the lease-row count read through `sagat_quota_step_log_counters()` and published only by the
  replica that holds shard 0 (task T041-5 for the SQL, task T041-10 for the publisher in the purge pass). A sequence is
  not transactional, so the counts include attempts whose transactions rolled back.
- `step_log_purged_lines_total`, `step_log_purge_backlog` (the streams whose `expires_at` has passed or whose removal has
  begun and whose `purged_at` is null, counted by the purge pass at the start of each run) and
  `step_log_purge_failures_total{cause}` (`underflow`, `error`, `reclaim`; decision D041-145): the purge pass (task T041-10).
- `step_log_tails_open`, `step_log_tails_refused_total`: the tail (task T041-12).
- `database_oldest_transaction_age_seconds` (a gauge of the operations snapshot over `pg_stat_activity.xact_start` and
  `pg_prepared_xacts.prepared`; it sees other roles' sessions only when the deployment grants `pg_read_all_stats`,
  G041-016): the operations snapshot (task T041-11).

Removed by decision D041-128: `step_log_quota_ensures_total`, `step_log_quota_ensure_affinity_misses_total` (there is no
ensure), `step_log_quota_refill_budget_exceeded_total` (it counted the event `cause="busy"` counts), the `caller` label
of the fallbacks, and the carry outcomes `stored` (now `written`), `out_of_order` (a drop reason now), `quota_rerun`
(a report re-run after a guard's raise or a failed call counts its carry `failed`, once) and `mark_only_fallback` (never
defined). Each label set is a closed enum with `ALL` and `as_str` in `crates/telemetry/src/metrics.rs`, built
as `WorkflowPrimitiveOutcome` is (`metrics.rs:68-106`); `DropReason`'s `as_str` is the domain's, so the metric label, the
carry's keys and `p_mark_reason` read one set; a test requires every emitted value to be one of the enum's, and a test
of task T041-36 that every key the carry's function can return in `dropped_by_server` is a `DropReason`.

SDK (local to the worker process, each SDK's own metrics facility or log): dropped records by reason (`report_deadline`,
`buffer_full`,
`send_failed`, `after_ceiling`, `process_bound`, `secret_set_full`).

## 8. The ingest transaction, statement by statement (design section 2, T1 to T10)

Every statement that binds line text or `q` maps its database error with `store_error_without_text`
(`crates/infra-postgres/src/identity.rs:734`), because the database's error text can quote bound values. The lock
timeout of 200 ms is set by T1 (correction of T041-6, decision D041-146 (12)).

- **T1.** `SELECT set_config('lock_timeout', '200ms', true), pg_current_xact_id(), set_config('sagat.step_log_wait_slots',
  $q, true)`: the lock timeout local to the transaction (as `SET LOCAL`), the transaction's id and *Q*, in one select
  list evaluated left to right.
- **T2.** One `SELECT` of the `steps` row by tenant and `step_id`, joined with its `runs` row, matching the request's
  `run_id`, the instance path of `instance` and its node as `lease_unexpired_instance` matches them
  (`crates/infra-postgres/src/execution.rs:4783-4817`), with status `LEASED` or `RUNNING`, the same attempt and
  generation, the lease-token hash and `lease_expires_at > clock_timestamp()`; the row's `cancel_requested`, the
  run's `finished_at` and `stop_reason` and the node's kind are passed to `report_fence_allows` (OD-2026-11-16-11). It
  returns the namespace, the node, the instance path and
  `steps.activity`. No row: the run is checked with `run_exists_outside`, and a foreign run is audited as
  `access.denied.cross_tenant` through `record_denial`, outside this transaction, as `missing_task_error` does
  (`crates/application/src/worker.rs:2564-2586`). A batch that read a valid lease and commits just after the completion
  is still that attempt's lines; a stale holder's token never matches.
- **T3.** The `worker_sessions` row, without a lock, by tenant, id, the lease's namespace and the caller's subject, with
  the token hash equal, `expires_at > clock_timestamp()` and `accepted_protocol_minor >= 7`. The three namespace resources
  of §6, through the rule of `content_usage::admission_policy` (`content_usage.rs:95-106`: the tenant fence shared, then
  the namespace fence shared, then `effective_policy`): one statement takes the six fences, the three resources in the
  order of `QuotaResource`, and then the attempt's stream lock (`pg_advisory_xact_lock` on a key of tenant, run, step,
  attempt and generation, design section 2a), and one reads their values. The fence keys and the stream key come from
  the same SQL helpers the carry's function calls (§8a), so the two never build them apart.
- **T4.** `INSERT … ON CONFLICT (tenant_id, run_id, step_id, attempt, generation, source) DO UPDATE SET tenant_id =
  step_log_streams.tenant_id RETURNING …` on `step_log_streams`; a new row takes the namespace's
  `step_log_retention_days` as `retention_days`. The update changes no column (the trigger of §2 refuses a change of the
  columns it guards); it only locks the row to commit, so only batches of the same attempt wait on it. `removed_at` set
  or `expires_at <= clock_timestamp()` is `REMOVED`; `final_at` set is `FINAL_RECORDED`.
- **T5.** `plan_batch`: records at or below `last_sequence` are dropped (a resend, or a late batch the worker gave up
  on). The batch's high mark is the highest of its sequences and, for a carry (§8a), its `highest_sequence`; the sequences
  between `last_sequence` and that mark that the batch does not hold are a gap: those above `ceiling_sequence`, once a
  ceiling has been met, are counted as dropped by the server, the others as dropped by the worker, each saturating at
  2^53.
- **T6.** The attempt allowance from the stream row's `records` and `bytes` against `step_log_attempt_records` and
  `step_log_attempt_bytes`. The tenant allowance from `sagat_quota_step_log_allowance(tenant)`, which returns without a lock
  the usage S + Σ `used` of §3b, the limit worked out as the finalizer's branch works it out from
  `quota_limits`, `quota_ceilings` and the 1 GiB default, and whether it is enforced (the runtime role may not read
  `quota_accounting_control`, `20261024000000_durable_quotas.up.sql:561-572`, and the accessor keeps one limit rule).
  The attempt allowance counts text bytes; the tenant's counts text plus 128 bytes a row, a planned marker included. When a record does not
  fit, `ceiling_sequence` takes the stream's high mark and `ceiling_reason` the ceiling; when the attempt ceiling is
  first met, one `ceiling_marker` row is planned at `next_marker_sequence`. If the unlocked tenant read plus the kept
  bytes is over the limit, nothing of the batch is stored and the records are counted with `tenant_quota`.
- **T7.** Without a lock (OD-2026-11-16-7): the binding's patterns (`bound_patterns`, `execution.rs:4756`, read without
  `hold_worker_binding`) checked by `first_uncovered` against T2's `steps.activity`; and the caller's membership row and
  its role, a plain `SELECT` (no namespace secrets lock, no `FOR SHARE`), refusing the database-runner and runner roles.
  `current_worker_authorized` and its callers do not change, and
  `the_shared_secrets_lock_has_one_caller_and_the_scoped_path_never_upgrades`
  (`crates/infra-postgres/src/quotas.rs:1323-1344`) gains an assertion that `step_logs.rs` calls none of
  `share_secrets_lock(`, `lock_secrets(`, `hold_worker_binding(`, `bound_patterns(` (which takes the binding lock) and
  `sagat_lock_memberships`; the ingest's own query reads `worker_bindings` as `bound_patterns` does
  (`execution.rs:4766-4775`), without its lock.
- **T8.** One `INSERT … SELECT … FROM unnest(…) ON CONFLICT (tenant_id, stream_id, sequence) DO NOTHING RETURNING` the text
  size of each inserted row. Its statement trigger charges the rows to a lease (§3b); a refusal raises here
  (`tenant_step_log_quota_exceeded` or `tenant_step_log_quota_held`, counted `step_log_quota_refusals_total` with
  `cause` `limit` or `held`) and is mapped to `RETRY_LATER`, as is a refill's `55P03` (a wait past its lock timeout, or no
  wait slot free, §3b; counted as a lock timeout).
- **T9.** From the rows T8 returned: `last_sequence = GREATEST(last_sequence, high mark)`, records, bytes, truncated
  records, both drop counts, `ceiling_sequence`, `ceiling_reason`, `first_at`, `last_at = clock_timestamp()`,
  `expires_at = last_at + retention_days`, and `last_stored_xid = GREATEST(coalesce(last_stored_xid, '0'::xid8),
  pg_current_xact_id())` only when T8 inserted a row. A batch never sets `final_at` (D041-090): only the carry of §8a
  does.
- **T10.** `COMMIT`. No deferred quota call runs for the step-log rows (§3b).

A marker line (either kind) has the level `info`, and `minLevel` never hides a marker: the filter applies to
`kind = 'line'` only.

## 8a. The carry, in the report's transaction (design section 2a)

One call, `SELECT * FROM sagat_step_log_carry($1, …)`, made by the one method of `StepLogCarryTx` right before the
report transaction's commit, after its engine change and journal writes, and only when the report is recorded now
(OD-2026-11-20-1). Its error maps through `store_error_without_text` to `StoreError::StepLogCarry`. The identity is the
attempt the report's fence verified, captured before the engine applies the report.

```sql
CREATE FUNCTION sagat_step_log_carry(
    p_tenant uuid, p_namespace uuid, p_run uuid, p_step uuid, p_node text,
    p_path_nodes text[], p_path_indices integer[], p_attempt integer, p_generation bigint,
    p_sequences bigint[], p_levels text[], p_worker_times timestamptz[], p_messages text[],
    p_exceptions text[], p_fields text[], p_truncated boolean[], p_sizes integer[],
    p_highest bigint,
    p_wait_slots integer,         -- Q, the replica's wait slots (decision D041-137): set transaction-locally before
                                  -- any write, set_config('sagat.step_log_wait_slots', p_wait_slots::text, true)
    p_mark_reason text)           -- null for a full carry; else invalid, secret, content_fallback or failed (the application's
                                  -- mark-only reasons, a subset of DropReason); any other value raises at the top
                                  -- with a fixed text, before any lock
RETURNS TABLE (outcome text, kept integer, dropped_by_worker bigint,
               dropped_by_server jsonb,  -- the counts per DropReason (out_of_order, attempt_records, attempt_bytes,
                                         -- tenant_quota, service_busy, invalid, secret, content_fallback,
                                         -- failed); one
                                         -- carry can have two
               ceiling_reason text,      -- attempt_records, attempt_bytes or tenant_quota (a ceiling met)
               not_stored_reason text,   -- service_busy, or null (decision D041-129)
               cause text,               -- limit, held or busy for a mark-only answer of the quota; else null
               path text,                -- fast (its reservation held, no block), refill (its block ran: its
                                         -- fallback refill granted, or, in a race, the block's own charge found
                                         -- room after all), none (no charge planned, nothing written, or
                                         -- contended, off, removed, final_recorded, or mark-only before any grant)
               room bigint,              -- the room left in this writer's lease after the call's charge, measured
                                         -- by the database; null exactly when path = none
               top_up boolean)           -- room is not null and room < sagat_quota_step_log_top_up_mark() (65,536):
                                         -- the store runs the top-up after the commit (decision D041-124)
LANGUAGE plpgsql VOLATILE SECURITY INVOKER SET search_path = pg_catalog, pg_temp AS $$ … $$;
REVOKE ALL ON FUNCTION sagat_step_log_carry(…) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION sagat_step_log_carry(…) TO sagat_app;
```

As built by the migration `20270219000000_step_log_carry` (task T041-36, decision D041-155), the function is one of
six, each `SECURITY INVOKER`, revoked from `PUBLIC`, executable by `sagat_app` only, and pinned to `search_path =
pg_catalog, pg_temp` with every table and function of the schema named with `public.`:

- `sagat_step_log_policy_fence_keys(tenant, namespace)` and `sagat_step_log_stream_lock_key(tenant, run, step,
  attempt, generation)`: the keys of item 1. The ingest's T3 builds the same keys in Rust (`step_log_policy_fence_keys`,
  `stream_lock_key`, decision D041-146 (3)); a test requires the two builders equal for the same inputs, the edges of
  each part included.
- `sagat_step_log_policies(tenant, namespace)`: the three resources of item 2, resolved as `effective_policy` resolves
  them, with the platform's defaults and maxima of `QuotaResource`; the property test compares the values it plans with
  against the domain's `resolve` on the same stored rows.
- `sagat_step_log_carry_plan(…)`: item 5, the plan of `plan_batch` for a carry, in variables, with no statement for a
  full carry (a mark-only carry counts its distinct sequences in one statement, since they may come in any order).
- `sagat_step_log_carry_write(…)`: items 3, 4, 6 and 7, called as it is on the fast path and inside the block on the
  slow path, and again by the block's handler with the block's mark-only reason.
- `sagat_step_log_carry(…)`: item 0, items 1 and 2, the reservation, the write and the block.

What the build settles that the text below leaves open:

- Item 0 also refuses, with fixed texts, a full carry whose arrays differ in length, whose sequences are not strictly
  increasing in 1 to 2^62 − 1, whose `p_highest` is below its last record, or one of whose sizes is not the octets of
  its message, exception and fields (so that the stream's byte counters and the trigger's charge never disagree). A
  mark-only carry's sequences may come in any order with duplicates, and only they are read.
- The reservation is asked for every sent record's size plus 128 a row, plus 128 for a possible marker, as the text
  says: a bound on what any plan of the carry can charge. A carry that can charge nothing is a mark-only carry or one
  with no record; it takes neither the reservation nor the block. The new stream's plan is not used to decide it, since
  the existing stream's plan can charge where the new stream's would not (a tenant allowance that the whole carry
  exceeds and a remainder of it fits).
- Item 0 also refuses more than 64 records and a level outside the five, so that a caller's bug never runs an oversized
  carry under the run lock (decision D041-156).
- Inside the block, the write reserves the plan's exact charge before CD: when the writer's quota lease holds it (its
  room lay between the charge and the bound), no refill runs and `path` is `fast`, with `room` the lease's room less
  the charge; `path` is `refill` only when the charge refilled, so `step_log_quota_fallbacks_total` counts refills and
  nothing else (decision D041-156). A block whose plan charges nothing (all duplicates) returns `none` and a null
  `room`; a reserved carry that charged nothing returns `fast`, the reading this section gives and design section 2a's
  "`none`: it charged nothing" does not (decision D041-156). On the fast path `room` is the reservation's room less the
  plan's charge, and CD's inserted rows are checked equal to the plan's rows (an `internal_error` otherwise, which the
  store maps to `StoreError::StepLogCarry`).
- CC′ refuses an existing stream whose namespace is not `p_namespace`, with a fixed text: the fences and the ceilings
  would be another namespace's (decision D041-156). A missing accounting row (no row from the allowance) raises an
  `internal_error` rather than planning against nothing.
- A new stream's `public_id` is `gen_random_uuid()`, and its `first_at` is the carry's time, as T9 sets it for a batch.

Its body, each statement with a snapshot of its own (the function is `VOLATILE`, the transaction READ COMMITTED):

0. `p_mark_reason` not null and not one of `invalid`, `secret`, `content_fallback`, `failed`: raise (a fixed text, no value).
   Then `set_config('sagat.step_log_wait_slots', p_wait_slots::text, true)`, so that the refill of this transaction's
   inserts reads the replica's *Q* (decision D041-137; a value outside 1 to 64 is clamped where it is read).
1. **CA.** The keys are built inside the function, never passed by the caller (security review Nit S-4): the six
   policy fence keys of §6 from `p_tenant` and `p_namespace`, in `QuotaResource` order, tenant then namespace, by the
   same SQL helper the ingest's T3 statement calls, so that the two cannot drift; and the stream lock key from
   `p_tenant`, `p_run`, `p_step`, `p_attempt` and `p_generation`, by the helper the ingest's T3 also calls.
   `pg_try_advisory_xact_lock_shared(hashtextextended(k, 0))` for each fence key in order, then
   `pg_try_advisory_xact_lock(hashtextextended(<stream key>, 0))`, then the tenant's three quota class fences
   (`quota-policy:tenant_content_bytes:`, `…tenant_log_bytes:`, `…tenant_step_log_bytes:` and the tenant) with
   `pg_try_advisory_xact_lock_shared`, so that CD's charge never waits for them (ADR-0097). Any `false`: return `contended`.
2. **CB.** The `quota_limits` and `quota_ceilings` rows of the three resources for the tenant and the namespace, resolved
   as `effective_policy` resolves them (the namespace row, else the tenant row, else the default, bounded by the
   ceiling; the same rule the property test of T041-36 compares), and `sagat_quota_step_log_allowance(p_tenant)`.
   `step_log_attempt_records = 0`: return `off`.
3. **CC.** The plan of item 5 for a new stream (zero counters, `in_order` true), then `INSERT INTO step_log_streams (…)
   SELECT … WHERE NOT EXISTS (SELECT 1 FROM step_log_streams WHERE tenant_id = p_tenant AND run_id = p_run AND step_id =
   p_step AND attempt = p_attempt AND generation = p_generation AND source = 'worker') ON CONFLICT (tenant_id, run_id,
   step_id, attempt, generation, source) DO NOTHING RETURNING` the row, written once with what that plan stores (the
   counters, `last_sequence`, the drops, `ceiling_sequence`, `ceiling_reason`, `first_at`, `last_at`, `expires_at`,
   `final_at`, and `last_stored_xid` = `pg_current_xact_id()` when a row is kept; D041-111). A new stream's carry then runs
   CD and not CE.
4. **CC′.** When CC returned nothing: `SELECT … FOR UPDATE SKIP LOCKED` of the stream row with its counters,
   `removed_at`, `expires_at`, `final_at` and `last_stored_xid IS NULL OR last_stored_xid < pg_current_xact_id()` as
   `in_order`. No row: `contended`; removed, expired or final: `removed` or `final_recorded`.
5. **Plan**, in the function, exactly T5 and T6 of §8 for a carry (design section 2a): with `p_mark_reason` set, no
   record is kept, no marker is planned, each distinct sequence in (`last_sequence`, `p_highest`] that the carry sent is
   counted once as dropped by the server with that reason (an invalid carry's duplicate or out-of-range elements are not
   counted; the rest of the range is the gap of T5, the worker's below `ceiling_sequence` and the ceiling's reason above
   it, decision D041-140), and for
   `tenant_quota` the stream's `ceiling_reason` becomes `tenant_quota`. Otherwise the records whose sequence is above
   `last_sequence` (none above it when `in_order` is false, counted `out_of_order`) are kept in sequence order until the
   first that does not fit `step_log_attempt_records` or `step_log_attempt_bytes` from the stream's counters; the rest
   are dropped by the server, `ceiling_sequence` and `ceiling_reason` are set, and one `ceiling_marker` is planned at
   `next_marker_sequence` when the ceiling is first met; then, if the kept sizes plus 128 a row, the marker included, do
   not fit the tenant allowance, nothing is stored and all the records are counted `tenant_quota` (all or nothing). The
   gap from `last_sequence` to `p_highest` not covered by the sent records is dropped by the worker below
   `ceiling_sequence` and by the server above it, saturating at 2^53. T041-36's property test requires the same result
   as `plan_batch`, the drops by reason included.
6. **CD.** `INSERT INTO step_log_lines … SELECT … FROM unnest(…) WITH ORDINALITY … WHERE <kept> ON CONFLICT (tenant_id,
   stream_id, sequence) DO NOTHING RETURNING octet_length(…)`, and the marker. Not run when nothing is kept. The
   statement-level trigger of §3 charges the inserted rows once.
7. **CE.** For an existing stream only: the `UPDATE` of T9 from CD's rows, plus `final_at = clock_timestamp()` and
   `last_stored_xid` raised as T9 raises it; it runs also when nothing was kept. It changes `expires_day` only when the
   day of `expires_at` changes, so it is HOT on most days. Return `written` or `mark_only` with the kept count, the drops
   by the worker, the drops by the server per reason, the stream's `ceiling_reason` and `not_stored_reason`, `path`,
   `room` and `top_up`.

**The fast path and the block** (ADR-0095 decision 5, D041-112, D041-113, D041-124, D041-125). After item 5's plan
and before item 3's write, the function calls `sagat_quota_step_log_reserve(p_tenant, <every sent record's measured
sizes plus 128 a row, plus 128 for a possible ceiling marker>)` (§3b). When it returns true, items 3 to 7 run as they are:
CD's charge finds the reserved lease and cannot be refused, and no subtransaction is opened; `path` is `fast`, and `room`
is the reserve's room minus the charge CD's rows carry (their sizes, which C3 computed exactly as `sagat_quota_measure`
measures them, plus 128 a row), with no statement more. When it returns false, items 3 to 7 run inside one `BEGIN …
EXCEPTION WHEN program_limit_exceeded OR lock_not_available` block, in which CD's charge refills the writer's lease
(the fallback refill of §3b: the usage row tried `NOWAIT`, then a wait slot and at most *b* for each lock acquisition).
After CD, on this path only, one more call `sagat_quota_step_log_reserve(p_tenant, 1)` reads the writer's room after the
charge (its lease is already this transaction's, so it is found), and `path` is `refill`. The handler reads `GET STACKED
DIAGNOSTICS … RETURNED_SQLSTATE, CONSTRAINT_NAME` and re-raises anything but `tenant_step_log_quota_exceeded`,
`tenant_step_log_quota_held` and `55P03` (a wait past *b*, or no wait slot free; the only lock the function waits for);
for those it records the mark-only final mark, nothing charged: a new stream inserted with the records counted dropped by
the server, `tenant_quota` for the two refusals (and `ceiling_reason` = `tenant_quota`) or `service_busy` for `55P03` (and
`not_stored_reason` = `service_busy`, `ceiling_reason` left as it was: the busy loss is not a ceiling); an existing stream
(the block's rollback released the row CC′ locked) taken again `FOR UPDATE SKIP LOCKED` and updated the same way;
`contended` when that row is held. It returns `mark_only` (or `contended`) with the cause (`limit`, `held`, `busy`),
`path` `none`, `room` null and `top_up` false. A carry whose plan charges nothing (no record kept and no ceiling marker
planned: a planned charge of 0) writes with no block and no reservation and returns `path` `none`; a carry that keeps no
record but plans a ceiling marker charges the marker's 128 bytes and takes the reservation and, without one, the block.

A raise of a guard of the step-log quota on either path (an underflow or a quota lease `CHECK`, each with a constraint
name of its own, task T041-5) fails the call: the store maps it, by its constraint name, to `StoreError::StepLogRefused`,
and the application re-runs the report with a mark-only carry whose reason is `failed` (ADR-0095 decision 5), which
charges nothing. Any other raise (a lost connection, a deadlock the database breaks, a statement timeout, a bug) fails
the call too: the store maps it to `StoreError::StepLogCarry`, which keeps only the SQLSTATE, and the application
re-runs the report carrying none, the records counted `failed`. The two cross into the application as two internal
variants, `AppError::StepLogRefused` and `AppError::StepLogCarry`, that only the re-run wrapper consumes; a report runs
at most four transactions (design section 2a).

After the report commits with `top_up` true, the store runs `sagat_quota_step_log_top_up(tenant)` (§3b) on the same
connection, in its own implicit transaction, and counts its answer; the report's answer does not wait for it.

Added to the report (`work_budget.rs`): one top-level statement, no transaction and no top-level row, whatever the
function does; the step-log rows it wrote are the second measure of design section 2a. A top-up after the commit is one
statement in one transaction of its own, after the report, not part of its budget (design section 2a).

## 9. The script marker's transaction (design section 7)

`StepLogStore::record_script_marker(execution)`, called by the script service after `record_execution` has committed an
inserted execution of a run, in a transaction opened by `begin_safely` with the store's marker lock timeout (200 ms by
default, a constructor parameter); any error other than `tenant_step_log_quota_exceeded`, `tenant_step_log_quota_held`
and the refill's `55P03` (a wait past *b* or no wait slot free; all three counted `skipped_quota`) is caught by the script service and counted as `failed`, and never changes the report's
answer:
`SELECT pg_current_xact_id()`; the run's `finished_at` (no lock); the namespace's
`step_log_attempt_records` and `step_log_retention_days` under their shared fences; `INSERT INTO step_log_streams …
ON CONFLICT (tenant_id, public_id) DO NOTHING RETURNING` with `source = 'script'`, `public_id` the execution id and `expires_at`
the earlier of now plus the retention and the stored `script_executions.expires_at` of that `(tenant_id, id)` (no row returned: a marker
already exists, nothing more); `sagat_quota_step_log_allowance(tenant)`; the `script_marker` line, whose insert is charged
to a lease and may raise (§3b); `COMMIT`.
