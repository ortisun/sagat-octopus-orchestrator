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
| `ceiling_reason` | `attempt_records`, `attempt_bytes`, `run_records`, `run_bytes`, `tenant_quota` | none |

The `DROPPED` record kind of the 2026-11-13 revision is gone (D041-028). The `script` source is back for the script
marker of OD-2026-11-15-9 (D041-057): one stream per script attempt, written only by `record_execution`. A
stream's state on the routes: `deleted` when `deleted_at` is set, `expired` when `removed_at` is set otherwise, `ended`
when `final_at` is set, `ended_without_final` when the attempt's lease is gone without it, `open` otherwise.

## 2. Tables (one migration, T041-5)

Timestamp: later than every file in `crates/infra-postgres/migrations/` and on the other open branches when T041-5 is
written (today the last is `20270116000000_batch_selection_failures`).

```sql
CREATE TABLE step_log_streams (
    tenant_id             uuid        NOT NULL,
    stream_id             bigint      GENERATED ALWAYS AS IDENTITY,
    public_id             uuid        NOT NULL,            -- what the routes show; the identity is never exposed
    namespace_id          uuid        NOT NULL,
    run_id                uuid        NOT NULL,
    step_id               uuid        NOT NULL,            -- steps.step_id: (version, run, path, node)
    node                  text        NOT NULL,
    instance_path_nodes   text[]      NOT NULL DEFAULT '{}',   -- copied from the steps row the lease read saw
    instance_path_indices integer[]   NOT NULL DEFAULT '{}',
    attempt               integer     NOT NULL,
    generation            bigint      NOT NULL,
    source                text        NOT NULL DEFAULT 'worker',  -- 'script': the marker stream of a script attempt
    records               integer     NOT NULL DEFAULT 0,  -- worker lines stored; the marker is not counted here
    bytes                 bigint      NOT NULL DEFAULT 0,  -- text bytes stored (the ceilings' measure; the tenant
                                                           -- quota charges 128 more per row)
    last_sequence         bigint      NOT NULL DEFAULT 0,  -- highest worker sequence seen; only rises (GREATEST)
    ceiling_sequence      bigint,                          -- last_sequence when a ceiling was first met
    next_marker_sequence  bigint      NOT NULL DEFAULT 4611686018427387904,  -- 2^62
    dropped_by_worker     bigint      NOT NULL DEFAULT 0,  -- saturating at 2^53
    dropped_by_server     bigint      NOT NULL DEFAULT 0,
    ceiling_reason        text,
    truncated_records     integer     NOT NULL DEFAULT 0,
    retention_days        smallint    NOT NULL,            -- step_log_retention_days of the namespace at creation (ADR-0027)
    final_at              timestamptz,
    first_at              timestamptz,                     -- server time of the first recorded batch
    last_at               timestamptz NOT NULL,            -- server time (clock_timestamp()) of the last recorded batch
    expires_at            timestamptz NOT NULL,            -- last_at + retention_days, written with every batch
    deleted_at            timestamptz,                     -- the delete route
    removed_at            timestamptz,                     -- removal began (purge mark or delete): no line is served
    purged_at             timestamptz,                     -- no line is left
    PRIMARY KEY (tenant_id, stream_id),
    CONSTRAINT step_log_streams_public_uniq UNIQUE (tenant_id, public_id),
    CONSTRAINT step_log_streams_attempt_uniq UNIQUE (tenant_id, run_id, step_id, attempt, generation, source),
    CONSTRAINT step_log_streams_source_valid CHECK (source IN ('worker', 'script')),
    CONSTRAINT step_log_streams_attempt_valid CHECK (attempt >= 1 AND generation >= 0),
    CONSTRAINT step_log_streams_counters_valid CHECK (records >= 0 AND bytes >= 0 AND last_sequence >= 0
        AND dropped_by_worker BETWEEN 0 AND 9007199254740992 AND dropped_by_server >= 0),
    CONSTRAINT step_log_streams_ceiling_valid CHECK (ceiling_reason IS NULL OR ceiling_reason IN
        ('attempt_records', 'attempt_bytes', 'run_records', 'run_bytes', 'tenant_quota')),
    CONSTRAINT step_log_streams_path_valid CHECK (execution_instance_path_valid(instance_path_nodes, instance_path_indices)),
    CONSTRAINT step_log_streams_retention_valid CHECK (retention_days BETWEEN 1 AND 365),
    CONSTRAINT step_log_streams_removal_order CHECK ((deleted_at IS NULL OR removed_at IS NOT NULL)
        AND (purged_at IS NULL OR removed_at IS NOT NULL))
);
CREATE INDEX step_log_streams_run_idx ON step_log_streams (tenant_id, run_id, stream_id);
CREATE INDEX step_log_streams_expiry_idx ON step_log_streams (expires_at) WHERE removed_at IS NULL;
CREATE INDEX step_log_streams_removal_idx ON step_log_streams (tenant_id, stream_id)
    WHERE removed_at IS NOT NULL AND purged_at IS NULL;

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
-- A trigger refuses UPDATE; DELETE is the purge's. The engine never locks a row of either table.

CREATE TABLE run_log_usage (
    tenant_id uuid    NOT NULL,
    run_id    uuid    NOT NULL,
    records   integer NOT NULL DEFAULT 0,
    bytes     bigint  NOT NULL DEFAULT 0,
    PRIMARY KEY (tenant_id, run_id),
    CONSTRAINT run_log_usage_counters_valid CHECK (records >= 0 AND bytes >= 0)
);
-- Created by the first batch of the run: INSERT ... ON CONFLICT (tenant_id, run_id) DO UPDATE ... RETURNING, which
-- locks it to commit (design section 2, step 6).

CREATE TABLE classified_namespaces (
    tenant_id    uuid        NOT NULL,
    namespace_id uuid        NOT NULL,
    recorded_at  timestamptz NOT NULL DEFAULT clock_timestamp(),
    recorded_by  text        NOT NULL,                     -- 'trigger' or 'migration'
    PRIMARY KEY (tenant_id, namespace_id),
    CONSTRAINT classified_namespaces_by_valid CHECK (recorded_by IN ('trigger', 'migration'))
);
-- Insert-only: ON CONFLICT DO NOTHING. sagat_app has SELECT, INSERT and nothing else, so a row is never cleared.

-- The record of FR-041-021: a trigger on every insert into workflow_versions, whatever binary makes it. document is the
-- canonical JSON of the version (crates/application/src/definitions.rs:479-480); the test is a superset of an annotation.
CREATE FUNCTION sagat_record_classified_namespace() RETURNS trigger LANGUAGE plpgsql
    SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    IF position(convert_to('"x-sagat-classification"', 'UTF8') IN NEW.document) > 0 THEN
        INSERT INTO public.classified_namespaces (tenant_id, namespace_id, recorded_by)
        SELECT w.tenant_id, w.namespace_id, 'trigger' FROM public.workflows w
         WHERE w.tenant_id = NEW.tenant_id AND w.id = NEW.workflow_id
        ON CONFLICT DO NOTHING;
    END IF;
    RETURN NULL;
END $$;
CREATE TRIGGER workflow_versions_record_classified AFTER INSERT ON workflow_versions
    FOR EACH ROW EXECUTE FUNCTION sagat_record_classified_namespace();
ALTER TABLE workflow_versions ENABLE ALWAYS TRIGGER workflow_versions_record_classified;  -- as the table's other triggers
-- The trigger and the statement below run in the migration's one transaction, so no version inserted between them is missed.
-- And once, for the versions that exist:
INSERT INTO classified_namespaces (tenant_id, namespace_id, recorded_by)
SELECT DISTINCT w.tenant_id, w.namespace_id, 'migration'
  FROM workflow_versions v JOIN workflows w ON w.tenant_id = v.tenant_id AND w.id = v.workflow_id
 WHERE position(convert_to('"x-sagat-classification"', 'UTF8') IN v.document) > 0
ON CONFLICT DO NOTHING;
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
| every other variable-size column of the four tables | `protected_system` | evidence: closed sets of section 1, bounded identifiers |

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
`step_log_lines.script_execution_id` is a `uuid`, fixed-size, so it is not in the manifest.

## 4. Grants (the migration's `DO $grants$` block)

| Table | `sagat_app` |
|---|---|
| `step_log_lines` | `SELECT, INSERT, DELETE` |
| `step_log_streams` | `SELECT, INSERT, UPDATE` |
| `run_log_usage` | `SELECT, INSERT, UPDATE` |
| `classified_namespaces` | `SELECT, INSERT` |

Usage of the two identity sequences. No `TRUNCATE` and no grant on `namespaces`. The only `SECURITY DEFINER` functions
are the two accessors of ADR-0027's Amendment, `sagat_quota_step_log_allowance(tenant)` and
`sagat_quota_lock_step_log_usage(tenant)`, of the existing pattern (fixed `search_path`, trusted owner, `EXECUTE` to
`sagat_app`), because the runtime role reads neither `tenant_retained_usage` with a lock nor `quota_accounting_control`
(`20261024000000_durable_quotas.up.sql:564-572`); the trigger function of `workflow_versions` runs as the inserting role.
No other grant (the
2026-11-13 column `namespaces.has_classified_schemas` is replaced by `classified_namespaces`, security M3 of
2026-11-15). The trigger function runs as the inserting role, which holds `INSERT` on `classified_namespaces`. The down
migration refuses while a row of `step_log_lines` or `classified_namespaces` exists. The migration's one-time statement
reads every `workflow_versions.document`; its duration on a large catalogue is measured on the upgrade fixture of
`scripts/check-upgrade-boundary.py` and stated in the runbook.

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

| Name | Kind | Unit | Range | Default (= default ceiling) | Meaning of 0 |
|---|---|---|---|---|---|
| `step_log_attempt_records` | `Policy` | items | 0 to 100,000 | 10,000 | capture is off in the namespace: a named exception, since `validate` and `validate_ceiling` refuse 0 for every `Policy` resource today (`quota.rs:547-549, 569`) |
| `step_log_attempt_bytes` | `Policy` | bytes | 1,024 to 16 MiB | 1 MiB | refused |
| `step_log_run_records` | `Policy` | items | 1 to 1,000,000 | 100,000 | refused |
| `step_log_run_bytes` | `Policy` | bytes | 1,024 to 256 MiB | 16 MiB | refused |
| `step_log_retention_days` | `Retention` | days | 1 to 365 | 14 | refused |

And one tenant resource, of ADR-0027's Amendment (OD-2026-11-15-5): `tenant_step_log_bytes`, `QuotaScope::Tenant`,
kind `Storage`, unit bytes, maximum that of `tenant_log_bytes`, default and default ceiling 1 GiB, with usage reported
from `tenant_retained_usage.step_log_bytes`.

Changing any of them is audited by the quota catalogue as every quota change is. The ingest reads the five values of the
run's namespace in its transaction (design section 2, step 3); a new stream keeps the retention it read
(`retention_days`), so a later change of the retention applies to new streams only (ADR-0027).

## 7. Metrics (bare names; labels are closed sets, never tenant, namespace, run, node or worker)

Server: `step_log_batches_total{outcome}` (`stored`, `duplicate`, `final_only`, `stop_attempt`, `stop_capture`,
`retry_later`, `invalid`); `step_log_lines_stored_total{level}`; `step_log_bytes_stored_total`;
`step_log_lines_dropped_total{reason}` (`worker_gap`, `attempt_records`, `attempt_bytes`, `run_records`, `run_bytes`,
`tenant_quota`); `step_log_script_markers_total{outcome}` (`written`, `skipped_quota`, `skipped_identity`,
`skipped_off`, `skipped_finished`); `step_log_lines_truncated_total`; `step_log_redactions_total{rule}` (`shape`, `field_name`);
`step_log_ingest_seconds`; `step_log_lock_timeouts_total`; `step_log_quota_retries_total`;
`step_log_purged_lines_total`; `step_log_purge_backlog`; `step_log_tails_open`; `step_log_tails_refused_total`;
`database_oldest_transaction_age_seconds` (a gauge of the operations snapshot, database-wide). Each label set is a
closed enum with `ALL` and `as_str` in `crates/telemetry/src/metrics.rs`, built as `WorkflowPrimitiveOutcome` is
(`metrics.rs:68-106`), with a test that every emitted value is one of the enum's.

SDK (local to the worker process, each SDK's own metrics facility or log): dropped records by reason (`buffer_full`,
`send_failed`, `after_ceiling`, `process_bound`).
