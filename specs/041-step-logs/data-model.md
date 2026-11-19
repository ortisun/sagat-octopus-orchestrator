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
| `ceiling_reason` | `attempt_records`, `attempt_bytes`, `tenant_quota` | none |

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
    truncated_records     integer     NOT NULL DEFAULT 0,
    retention_days        smallint    NOT NULL,            -- step_log_retention_days of the namespace at creation (ADR-0027)
    last_stored_xid       xid8,                            -- highest id of a transaction that stored a line of this stream
                                                           -- (design section 2a, the order guard); never lowered; null
                                                           -- before the first
    final_at              timestamptz,                     -- set only by the attempt's carry (its final mark)
    first_at              timestamptz,                     -- server time of the first recorded batch or carry
    last_at               timestamptz NOT NULL,            -- server time (clock_timestamp()) of the last recorded batch or carry
    expires_at            timestamptz NOT NULL,            -- last_at + retention_days, written with every batch and carry
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
    CONSTRAINT step_log_streams_path_valid CHECK (instance_path_nodes IS NULL
        OR execution_instance_path_valid(instance_path_nodes, instance_path_indices)),
    CONSTRAINT step_log_streams_retention_valid CHECK (retention_days BETWEEN 1 AND 365),
    CONSTRAINT step_log_streams_removal_order CHECK ((deleted_at IS NULL OR removed_at IS NOT NULL)
        AND (purged_at IS NULL OR removed_at IS NOT NULL))
);
CREATE INDEX step_log_streams_run_idx ON step_log_streams (tenant_id, run_id, stream_id);
CREATE INDEX step_log_streams_expiry_idx ON step_log_streams (expires_at) WHERE removed_at IS NULL;
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

The measurement and finalizer triggers that ADR-0027 generates are created on `step_log_lines` only: the manifest test
requires them only on tables with `content` or `log` columns (`tests/integration/security-multitenancy/durable_quotas.rs:345-411`), and on the counter tables they would run `to_jsonb` on every update. The entries' owner
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

## 4. Grants (the migration's `DO $grants$` block)

| Table | `sagat_app` |
|---|---|
| `step_log_lines` | `SELECT, INSERT, DELETE` |
| `step_log_streams` | `SELECT, INSERT, UPDATE` |

Usage of the identity sequence of `step_log_streams`. No `TRUNCATE` and no grant on `namespaces`. The only new `SECURITY
DEFINER` function is the accessor of ADR-0027's Amendment, `sagat_quota_step_log_allowance(tenant)`, of the existing
pattern: a fixed `search_path`, a trusted owner, `REVOKE ALL … FROM PUBLIC` (as `20261024000000_durable_quotas.up.sql:294`
does, since PostgreSQL grants `EXECUTE` to `PUBLIC` on a new function) and `EXECUTE` to `sagat_app` (as `:572`). It
exists because the runtime role may not read `quota_accounting_control` (`:564-572`). No other grant. The down migration
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

Admission defaults (set by T041-35 from its workload (g); one pool, `SAGAT_DB_MAX_CONNECTIONS`, the replica's database pool, which the benchmark builds as `API_POOL` = 20, design section 2, "Capacity"; placeholders until then):
`SAGAT_GRPC_MAX_LOG_CALLS` a quarter of `SAGAT_DB_MAX_CONNECTIONS`, at least 1; `SAGAT_STEP_LOGS_TENANT_CALLS` half of
it, rounded down, at least 1.

## 7. Metrics (bare names; labels are closed sets, never tenant, namespace, run, node or worker)

Server: `step_log_batches_total{outcome}` (`stored`, `duplicate`, `final_only`, `stop_attempt`, `stop_capture`,
`retry_later`, `invalid`); `step_log_carries_total{outcome}` (`stored`, `mark_only`, `off`, `invalid`, `removed`,
`final_recorded`, `out_of_order`, `quota_rerun`, `failed`, `mark_only_fallback`, `refused`), counted once per report the
carry rode on, by T041-36 (design section 2a), which also counts a carry's lines in the line, byte, truncation, redaction
and drop metrics below; `step_log_lines_stored_total{level}`; `step_log_bytes_stored_total`;
`step_log_lines_dropped_total{reason}` (`worker_gap`, `attempt_records`, `attempt_bytes`, `tenant_quota`,
`out_of_order`); `step_log_script_markers_total{outcome}` (`written`, `skipped_quota`, `skipped_off`,
`skipped_finished`, `failed`); `step_log_lines_truncated_total`; `step_log_redactions_total{rule}` (`shape`, `field_name`);
`step_log_ingest_seconds`; `step_log_lock_timeouts_total`; `step_log_quota_retries_total`;
`step_log_purged_lines_total`; `step_log_purge_backlog` (the streams whose `expires_at` has passed or whose removal has
begun and whose `purged_at` is null, counted by the purge pass at the start of each run); `step_log_tails_open`; `step_log_tails_refused_total`;
`database_oldest_transaction_age_seconds` (a gauge of the operations snapshot over `pg_stat_activity.xact_start` and
`pg_prepared_xacts.prepared`; it sees other roles' sessions only when the deployment grants `pg_read_all_stats`,
G041-016). Each label set is a
closed enum with `ALL` and `as_str` in `crates/telemetry/src/metrics.rs`, built as `WorkflowPrimitiveOutcome` is
(`metrics.rs:68-106`), with a test that every emitted value is one of the enum's. Owners: T041-36 the carries' and its
`out_of_order` drop reason; T041-6 the ingest's metrics
(batches, lines stored, bytes, drops, truncations, redactions, ingest seconds, lock timeouts, quota retries); T041-10 the
purge's; T041-11 the gauge; T041-12 the tails'; T041-13 the markers'.

SDK (local to the worker process, each SDK's own metrics facility or log): dropped records by reason (`report_deadline`,
`buffer_full`,
`send_failed`, `after_ceiling`, `process_bound`, `secret_set_full`).

## 8. The ingest transaction, statement by statement (design section 2, T1 to T10)

Every statement that binds line text or `q` maps its database error with `store_error_without_text`
(`crates/infra-postgres/src/identity.rs:734`), because the database's error text can quote bound values. The lock
timeout is `SET LOCAL lock_timeout = '200ms'`.

- **T1.** `SELECT pg_current_xact_id()`.
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
  attempt and generation, design section 2a), and one reads their values.
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
  `tenant_retained_usage.step_log_bytes`, the limit worked out as the finalizer's branch works it out from
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
  size of each inserted row.
- **T9.** From the rows T8 returned: `last_sequence = GREATEST(last_sequence, high mark)`, records, bytes, truncated
  records, both drop counts, `ceiling_sequence`, `ceiling_reason`, `first_at`, `last_at = clock_timestamp()`,
  `expires_at = last_at + retention_days`, and `last_stored_xid = GREATEST(coalesce(last_stored_xid, '0'::xid8),
  pg_current_xact_id())` only when T8 inserted a row. A batch never sets `final_at` (D041-090): only the carry of §8a
  does.
- **T10.** `COMMIT`. `tenant_step_log_quota_exceeded` from the finalizer is mapped to `RETRY_LATER`.

A marker line (either kind) has the level `info`, and `minLevel` never hides a marker: the filter applies to
`kind = 'line'` only.

## 8a. The carry, in the report's transaction (design section 2a, CA to CE)

Run by the six methods of `StepLogCarryTx` right before the report transaction's commit, after its engine change and
journal writes, and only when the report is recorded now; at most five run for a short step, six for a long one. Every
statement binds only values C2 and C3 checked and maps its database error with `store_error_without_text` to
`StoreError::StepLogCarry`; those that write begin with `INSERT` or `UPDATE` (never `WITH`), so that `work_budget.rs`
counts their rows. No lock is waited for: each is tried, and a refused one ends the carry (`contended`) with the report's
transaction untouched. The identity is the attempt the report's fence verified (tenant, namespace, run, `steps.step_id`,
node, instance path, attempt, generation), captured before the engine applies the report.

- **CA.** One `SELECT`: `pg_try_advisory_xact_lock_shared` on the six policy fences of the three namespace resources of
  §6 (tenant fence then namespace fence, the resources in the order of `QuotaResource`, the keys `content_usage` uses),
  then `pg_try_advisory_xact_lock` on the attempt's stream lock (the key the ingest's T3 takes). Any `false`: stop
  (`contended`).
- **CB.** One `SELECT`, whose snapshot begins after CA's locks are held: the `quota_limits` and `quota_ceilings` rows of
  the three resources (the read `effective_policy` makes, batched as T041-6's variant of `admission_policy`; the
  resolution is the application's) and `sagat_quota_step_log_allowance(tenant)`. `step_log_attempt_records = 0`: stop
  (`off`).
- **CC.** `INSERT INTO step_log_streams (…) SELECT … WHERE NOT EXISTS (SELECT 1 FROM step_log_streams WHERE tenant_id = $1
  AND run_id = $2 AND step_id = $3 AND attempt = $4 AND generation = $5 AND source = 'worker') ON CONFLICT (tenant_id,
  run_id, step_id, attempt, generation, source) DO NOTHING RETURNING` the new stream row (it takes the namespace's
  retention). The `NOT EXISTS` sees an existing row under the statement's snapshot even while the purge or the delete
  route updates it, so no row is offered and the speculative insertion's unique check, which waits for an in-progress
  updater of the conflicting row, never runs; with the stream lock held no other transaction is inserting the row. So
  this never waits. A row returned: a new stream, zero counters, `in_order` true; go to the plan.
- **CC′.** Only when CC returned nothing: `SELECT … FROM step_log_streams WHERE <key> FOR UPDATE SKIP LOCKED`, returning
  the counters, `removed_at`, `expires_at`, `final_at` and `last_stored_xid IS NULL OR last_stored_xid <
  pg_current_xact_id()` as `in_order`. No row: stop (`contended`). `removed_at` set, `expires_at <= clock_timestamp()` or
  `final_at` set: stop (`removed`, `final_recorded`).
- **Plan** (domain, no statement): `plan_batch` with `final = true`, the carry's `highest_sequence` and `in_order`; when
  `in_order` is false, no record above `last_sequence` is kept and those are counted as dropped by the server with the
  reason `out_of_order`. A mark-only carry keeps no record and plans no marker.
- **CD.** The `INSERT … SELECT … FROM unnest(…) ON CONFLICT (tenant_id, stream_id, sequence) DO NOTHING RETURNING` of T8,
  for the kept lines and the planned marker. Not run when nothing is kept.
- **CE.** The `UPDATE` of T9 from CD's rows, plus `final_at = clock_timestamp()`, and `last_stored_xid` raised as T9
  raises it; it runs also when nothing was kept.
- **Commit.** The report's `COMMIT`. Any raise of the finalizer's step-log branch (each with a constraint name of its own,
  T041-5) is mapped to `StoreError::StepLogRefused`, which the application answers with a re-run of the report carrying
  a mark-only carry (the records counted `tenant_quota`); `StoreError::StepLogCarry` with a re-run carrying none. Both
  cross into the application as internal variants (`AppError::StepLogRefused`, `AppError::StepLogCarry`) that only the
  re-run wrapper consumes.

Top-level statements added to the report (`work_budget.rs`'s measure): contended at CA 1; capture off 2; a short step 4
with no row kept (2 rows) and 5 with *k* kept (2 + *k* rows); a long step 5 with none kept (1 row) and 6 with *k* kept
(1 + *k* rows); an existing stream held, removed or final 4 (no row).

## 9. The script marker's transaction (design section 7)

`StepLogStore::record_script_marker(execution)`, called by the script service after `record_execution` has committed an
inserted execution of a run, in a transaction opened by `begin_safely` with the store's marker lock timeout (200 ms by
default, a constructor parameter); any error other than `tenant_step_log_quota_exceeded` is caught by the script service
and counted as `failed`, and never changes the report's answer:
`SELECT pg_current_xact_id()`; the run's `finished_at` (no lock); the namespace's
`step_log_attempt_records` and `step_log_retention_days` under their shared fences; `INSERT INTO step_log_streams …
ON CONFLICT (tenant_id, public_id) DO NOTHING RETURNING` with `source = 'script'`, `public_id` the execution id and `expires_at`
the earlier of now plus the retention and the stored `script_executions.expires_at` of that `(tenant_id, id)` (no row returned: a marker
already exists, nothing more); `sagat_quota_step_log_allowance(tenant)`; the `script_marker` line;
`COMMIT`, with `tenant_step_log_quota_exceeded` counted as `skipped_quota`.
