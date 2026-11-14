# Step logs per run

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-13

--------------------------------------------------------------------------------

## Overview

The lines a step's code writes to its logger are sent by the worker to the platform, stored per attempt with
ceilings and a retention period, and read through a REST route, a live stream and a Studio screen. The owner
chose this on 2026-11-13 ("Logs de passo por run na 0.3.0 — Feature nova: os workers enviam as linhas de log
de cada passo, a plataforma guarda-as com retenção e limites, e há rota e ecrã no Studio", OD-2026-11-13-9).

The design treats a step log as a bounded diagnostic record kept **beside** the journal. It is never execution
state: no transition reads it, a lost line never changes a step's outcome, and the transactions that lease and
complete a step run no statement on it. What it must never be is a second place where a secret or another
tenant's data can be read, or a reason for a completion to wait.

Eleven choices had more than one defensible answer and were put to the owner as O041-001 to O041-011. The owner
accepted every recommendation on 2026-11-13 ("Aceitar todas", OD-2026-11-13-14). The owner accepted ADR-0084 and
ADR-0085 the same day (OD-2026-11-13-17). The architecture, security and correctness reviews of commit `2e90dbb8`
then found one design fault that blocks (the tenant's retained-usage row held for the whole ingest), one that
breaks isolation in one SDK (Python capture on the worker-wide logger) and a set of gaps in the fence, the
redaction, the ordering of ceilings and the end of the tail. This text is the revision that answers them; the
reports are in `docs/designs/041-step-logs.reviews/2026-11-13/`. Two questions the reviews raised were the
owner's and were answered on 2026-11-13 (OD-2026-11-13-18): in a namespace whose schemas are classified, reading
step logs also needs the namespace's PII reveal grant; and the `log` of a script execution is read under the
same rule, so viewers lose it.

## The Problem

Today a person who wants to know what a step's code did has three sources, and none is the code's own words:

- `GET /runs/{runId}/events`, `/attempts` and `/telemetry` say what the engine decided (leased, completed,
  failed with a code) and how long it took. They are rebuilt from `run_events`; there is no attempts table.
- The processes write JSON to stdout with `trace_id` and `run_id`. That is the platform's log, and the worker's
  log stays on the worker's machine, which the person reading the run in the Studio usually cannot reach.
- `sdks/SPEC.md` §6.3 gives an activity a context with "a cancellation token, a logger and `secrets()`", and
  says nothing more about the logger. Five SDKs expose one (`ActivityContext.log` in TypeScript and Python,
  `Logger *slog.Logger` in Go, `ILogger Logger` in C#, `ActivityLogger logger()` in Java) and each writes only
  to the host process. The Rust SDK's `ActivityContext` (`sdks/rust/src/activity.rs`) has no logger at all.

One kind of step already has a stored log. The script runner (`apps/sandbox-runner`) keeps a script's stderr,
redacts it, clips it and reports it; `script_executions.log` holds up to 1 MiB per attempt, counted against the
tenant quota `tenant_log_bytes` and purged after `script_retention_days`. It is read through
`GET /namespaces/{namespaceId}/script-executions/{executionId}` under `script:read`, which the viewer role
holds (`crates/api-http/src/routes/scripts.rs:425-446`, `ExecutionBody.log` in `dto_scripts.rs:596`).

Facts of the code that shape what can be built (read at `2e90dbb8`):

1. **Workers hold secret values.** `ResolveSecrets` returns them once per attempt, and the server does not keep
   them afterwards. The server therefore cannot recognise a secret value inside a line it receives; only the
   worker process can. Several attempts run in one worker process at once, each with its own secrets.
2. **Loggers are shared in some SDKs.** Python's activity logger is `_TaskLog`, a `LoggerAdapter` over the
   worker-wide logger (`sdks/python/sagat_octopus_orchestrator/worker/host.py:172`, built at `:712` over
   `self._log`, the process logger of `config.py:84`). A handler attached to that logger sees every attempt's
   lines.
3. **The hot path has a budget that fails a pull request.** `tests/integration/claim-transition-efficiency/
   work_budget.rs` holds a lease to 38 statements, 2 transactions and 5 rows and a completion to 50/1/11 or
   47/1/9, as equalities (constitution 4.4.0, CI gates). The engine locks the run row `FOR UPDATE`. A table
   with a foreign key to `runs` would make every insert take a `KEY SHARE` lock on that row.
4. **Retained bytes go through one row per tenant.** Every insert into a classified table fires the per-row
   trigger `sagat_quota_row_delta()`, which builds `to_jsonb(NEW)` and adds to `quota_transaction_deltas`; at
   commit the deferred `sagat_quota_flush_deltas()` takes the content and log policy fences (shared advisory
   locks) and then `tenant_retained_usage … FOR UPDATE`, checks the limit and raises
   `tenant_log_quota_exceeded` when it is passed (`20261024000000_durable_quotas.up.sql:190-370`).
   `sagat_quota_lock_log_usage()` (`:283-293`), which `content_usage::take_log_prefix` uses to fit a script log
   under the limit, takes the same row `FOR UPDATE` and holds it to commit. A completion that stores output
   takes that row in its own commit while it holds the run lock. A design that takes the row early in every log
   batch makes completions queue behind log ingest.
5. **The heartbeat fences more than the lease token.** `WorkerService::heartbeat`
   (`crates/application/src/worker.rs:1819`) authorizes `worker:execute`, and `ensure_bound` (`:2715`) checks
   the activity binding (`bound_patterns`), the managed-database owner and the current membership
   (`current_worker_authorized`, `crates/infra-postgres/src/execution.rs:1002`, which reads the membership row
   under the lock a revocation waits for and refuses the database-runner role). A `steps` row holds a lease
   when its status is `LEASED` or `RUNNING` (`steps_lease_consistent`) and the lease ends at
   `lease_expires_at`. Worker reports carry no session token; only `PollTasksRequest` does.
6. **Nothing deletes a run.** `run_events` and `payloads` are append-only by trigger and feature 020
   (retention and archive, ADR-0054) is a draft. "Purged with the run" is not available; logs need a retention
   of their own.

## Requirements

### Functional

- **FR-041-001 Capture.** When the worker session was accepted at protocol minor 1.7 or later and the policy
  the server returned says capture is on, every line an activity writes through the SDK's activity logger at or
  above the capture level MUST be sent to the platform. The logger MUST keep writing to the host's own logging
  as it does today; capture is added, it replaces nothing. Capture MUST happen in the per-attempt logger object
  and MUST NOT install a handler, sink or appender on a logger that other attempts share.
- **FR-041-002 Identity and order.** A stored line MUST be identified by tenant, run, instance path, node,
  attempt, generation and a sequence. The worker numbers its records from 1, one per logger call that passes
  the level floor, whether the record is later sent or dropped; markers the server adds take sequences from a
  range of their own (2^62 and above). A line MUST carry the worker's time and the server's time of storage.
  Within an attempt the worker's records are in sequence order. Across attempts of a run the order is the
  order in which the platform committed the lines; the worker's time is shown and never used to order.
- **FR-041-003 Levels and fields.** A line has one of five levels (`trace`, `debug`, `info`, `warn`, `error`),
  a text message and an optional flat map of fields whose values are strings, numbers or booleans. Each SDK
  maps its language's levels onto the five; a level above `error` maps to `error`. The capture level is `info`
  unless the worker's configuration says otherwise. A NUL character is replaced by U+FFFD; a number that is not
  finite is stored as the string `NaN`, `Infinity` or `-Infinity`.
- **FR-041-004 Transport.** Lines travel in batches through a new unary RPC, `AppendStepLogs`, added in
  protocol minor 1.7. A batch names one attempt with the identity and fence a heartbeat carries plus the
  worker's session token, and holds at most 500 records and 192 KiB. The server MUST answer only after the
  lines are committed and MUST store a record at most once however many times its batch arrives (identity:
  attempt, generation, sequence). After a batch marked `final` has been recorded for an attempt, every later
  batch for it MUST be answered `STOP_ATTEMPT` and store nothing.
- **FR-041-005 Logging never decides the step.** A call to the logger MUST NOT block on the network or on a
  full buffer, and MUST NOT throw into the activity. No failure of capture (refused batch, timeout, server
  down, ceiling reached, invalid line, bug in the capture path) may change the outcome, the output, the retry
  or the timing of a step beyond the bounded final flush of FR-041-015.
- **FR-041-006 Ceilings and markers.** These ceilings hold, each enforced by the server whatever the worker
  does: 8 KiB per message and 2 KiB and 32 keys per field map, measured after redaction; 1 MiB and 10,000
  records per attempt; 16 MiB and 100,000 records per run; the tenant's `tenant_log_bytes`. A message over its
  ceiling is cut at a character boundary and flagged `truncated`. Records past a ceiling are not stored and are
  counted on the attempt's stream. When the attempt ceiling is first met, the server stores one marker line;
  the run ceiling and the tenant quota are recorded only on the stream row, so that ten thousand iterations
  meeting the run ceiling add no lines. A marker holds no text: its reason and counts are in typed columns. A
  step MUST NOT fail, wait or retry because a ceiling was met.
- **FR-041-007 Redaction.** Before a line leaves the worker process the SDK MUST replace, in the message, in
  every field key, in every string field value, in the decimal text of every number field (which then becomes
  the string `[REDACTED]`), and in the exception text and stack it formats, each secret value held by **any
  attempt in flight in that process**, by the rule of `sagat_domain::scripts::redact` (values of at least 6
  bytes, longest first). Structured context the language attaches to a line (Python `extra`, Go `slog`
  attributes and groups, C# logger scopes, Java and TypeScript fields, Rust `tracing` fields) is captured only
  as fields and is redacted as fields. The server MUST then apply, in the application layer and before any
  size is measured, both existing scrubbers: the credential shapes of `sagat_domain::scripts` (PEM private
  keys, `AKIA`/`ASIA` keys, `Bearer` tokens) and those of `sagat_telemetry::redact` (URL user information,
  JWT-shaped words, `Basic` credentials, `sk-`, `ghp_` and `xox` tokens), with one placeholder, `[REDACTED]`, and
  without the 2,048-character cut of `scrub`; and it MUST replace the value of a field whose key
  `sagat_telemetry::redact::is_sensitive_name` matches. The content of a line MUST NOT appear in the journal, a
  projection, telemetry, an audit record, a metric, a span, an error body or the platform's own logs, and the
  `AppendStepLogs` request MUST NOT be printed by any log, `Debug` or trace of the server or the SDKs.
- **FR-041-008 Storage beside the journal.** Lines are stored in PostgreSQL in tables of their own. Storing a
  batch MUST NOT take, or wait for, a lock on a row of `runs`, `steps` or `run_events`, and leasing, heartbeat
  and completion MUST NOT read or write the log tables. A batch touches the tenant's `tenant_retained_usage`
  row only in its own commit, as every classified write does (the finalizer of ADR-0027), except within 2 MiB
  of the tenant's log limit, where it takes the row in the statement before its insert and holds it to its
  commit, under a lock timeout of 200 ms. A completion that stores output can therefore wait, on that row, for
  one log batch's commit, and near the limit for one batch's insert and commit; T041-6 measures it. The bytes
  are counted as `log` in the storage classification and released when the rows are deleted.
- **FR-041-009 Retention.** Lines are deleted when the attempt's last stored line is older than the
  `step_log_retention_days` in force at purge time (a new quota resource, 1 to 365, default 14). The stream row,
  which says the attempt had a log and holds its counters and the time its lines were purged, is kept, so that
  a reader is told "expired" and not shown an empty list.
- **FR-041-010 Read.** `GET /runs/{runId}/logs` returns lines in stored order with `cursor` and `limit`
  (default 50, at most 200, clamped, as every list). Filters: `stream` (one attempt's log; a stream that is not
  this run's is answered as missing), `node`, `attempt`, `minLevel`, `from` and `to` (server time) and `q` (a
  literal, case-insensitive substring of the message, at most 128 characters). The cursor is sealed: it names
  the run it was minted for and is refused for another. Every item carries the identity of FR-041-002, both
  times, level, message, fields, the `truncated` flag and, for a marker, its reason and counts.
- **FR-041-011 Streams.** `GET /runs/{runId}/logs/streams` lists, with `cursor` and `limit`, the attempts of the
  run that have a log: node, instance path, attempt, source (`worker` or `script`), records and bytes stored,
  records dropped by the SDK and by the server with the reason, the ceiling met, whether the attempt ended with
  its final flush, and the expiry time or the time the lines were purged. The counters are read at the
  statement's snapshot and can be ahead of the lines a read below the horizon of FR-041-010 returns.
- **FR-041-012 Live tail.** `GET /runs/{runId}/logs/stream` is a server-sent event stream of new lines with
  the filters of FR-041-010, resumable with `Last-Event-ID`, with the `reconnect` and ping behaviour and the
  per-subject and per-tenant stream caps of `/runs/{runId}/stream`. It never skips or repeats a line across a
  reconnect. It ends with `end` only when the run is terminal and every line that can still be committed for
  the run has been delivered (Technical Plan, section 5). Each poll authorizes the reader again.
- **FR-041-013 Who may read.** The three read routes need `run:read` and a new permission, `run:logs:read`,
  held by the admin, developer and operator roles and not by viewer, worker or runner. In a namespace whose
  schemas are classified (FR-041-021) the reader MUST also hold the namespace's PII reveal grant
  (`payload_reveal_grants`, checked as `has_effective_payload_reveal_grant` checks it) (OD-2026-11-13-18).
  Another tenant's run is answered 404 and recorded as `access.denied.cross_tenant`, as every run route. A read
  is audited as `run.logs_read`, at most once per subject and run in 300 seconds whatever its filters, committed
  before the first page of that window is returned.
- **FR-041-014 Mixed versions.** A worker accepted below 1.7 sends nothing and behaves exactly as today. A
  1.7 SDK against a server that accepts a lower minor, or whose policy says capture is off, sends nothing and
  logs locally. An SDK that gets `UNIMPLEMENTED` for `AppendStepLogs` (an older API replica) stops capture for
  that attempt only. No published workflow, run in progress or existing RPC changes meaning.
- **FR-041-015 The six SDKs.** Each SDK keeps its present logger type and adds capture behind it: a bounded
  buffer per attempt (1,000 records or 256 KiB, whichever is met first), a flush every second or at 256
  records, one batch in flight per attempt, at most two retries of the same unchanged batch, and a final flush
  before `CompleteStep` or `FailStep` that waits at most 2 seconds. The Rust SDK gains the logger its
  `ActivityContext` lacks. Records the SDK drops are reported by a `DROPPED` marker that names the first and
  the last sequence it covers.
- **FR-041-016 Steps the platform runs.** A script step's log stays where feature 005 stores it. When the
  script runner's report is recorded, the platform stores one marker in the run's log that names the script
  execution, so the run's log shows that the step produced output and where to read it. The marker holds no
  text and is not charged to the run's log usage. The database runner and the lake runner capture no log.
- **FR-041-017 Studio.** The run screen gains a Logs tab for the whole run and a Logs section for the selected
  step and attempt, with filters, search, a live tail that pauses when the reader scrolls up, markers shown as
  rows, the states "not allowed" (naming the permission or the grant that is missing), "no log", "expired",
  "deleted" and "ended without its final flush", and full keyboard operation.
- **FR-041-018 Observability.** The platform counts records accepted, dropped (by where and why), truncated
  and redacted, batches by outcome, ingest duration, lock-timeout retries, purged records and open tail streams,
  with labels that are closed enumerations only. It alerts on sustained shedding and on purge backlog, not on
  ceilings a customer's code meets.
- **FR-041-019 Lineage.** A log belongs to the run and attempt that wrote it. A retry is a new attempt and a
  new log. A reset, a rerun and a continue-as-new open a new run; the new run starts with no log and the
  earlier run keeps its own, reachable through the links the Studio already shows between those runs. Lines are
  never copied.
- **FR-041-020 Policy.** Capture on or off, the attempt and run ceilings and the retention are quota resources
  read at registration and at storage; changing one is audited by the quota catalogue as every quota change is.
  A change of the capture switch or of a ceiling applies to batches stored after it; a change of the retention
  applies to the next purge pass, so a shorter retention also removes existing lines.
- **FR-041-021 Classified namespace.** A namespace is classified when a workflow version published in it
  carries at least one `x-sagat-classification` annotation in a schema. The fact is recorded on the namespace
  in the publication transaction (it is never cleared) and is set for versions published before this feature by
  a backfill (OD-2026-11-13-18).
- **FR-041-022 Script-execution logs.** The `log` of `GET /namespaces/{namespaceId}/script-executions/
  {executionId}`, for an execution of a run, is returned only to a caller who may read that run's step logs
  under FR-041-013; anyone else who may read the execution gets it without the log and with `logWithheld: true`
  (OD-2026-11-13-18). Viewers lose the log they read today; the release notes say so.
- **FR-041-023 Deletion.** `DELETE /runs/{runId}/logs`, optionally with `stream`, needs `run:logs:delete`,
  held by the admin role only. It makes the lines unreadable at once, records `run.logs_purged` (run, number of
  streams, no text) in the same transaction, and leaves the deletion of the rows to the purge pass. It exists
  for the incident in which a secret or personal data was logged.

### Success criteria

- **SC-041-001** With the migration applied and a worker that logs on every step, `work_budget.rs` passes with
  its three constants unmodified. A batch for a run commits, under `lock_timeout = 50ms`, while another
  transaction holds that run's row `FOR UPDATE` and has updated the leased `steps` row. While an ingest
  transaction of tenant T is held after its inserts and before its commit, a completion with output of another
  run of T commits under `lock_timeout = 100ms`.
- **SC-041-002** An activity that writes 1,000,000 lines as fast as it can completes with the same outcome and
  output as one that writes none, the store holds no more than the attempt ceiling plus its marker, and the
  stream's counters account for every line not stored.
- **SC-041-003** In the fault tests of the Verification Plan (crash between accept and commit, crash between
  commit and answer, duplicate batch, lost batch, stale or expired lease, revoked binding, server unavailable)
  no record is stored twice and every record that is missing is covered by a counter or by the "ended without
  its final flush" state.
- **SC-041-004** A canary secret resolved by an attempt and written to the logger whole, inside a longer
  message, as a field key, as a field value and inside an exception, by that attempt and by another attempt
  running at the same time in the same worker process, appears nowhere in the stored lines, the three routes,
  the journal, telemetry, audit records or the platform's logs, in all six SDKs.
- **SC-041-005** The five conformance scenarios of the Technical Plan pass in TypeScript, Go, Python, C#, Java
  and Rust from the same scenario files and goldens.
- **SC-041-006** The Logs tab and section pass the axe run at WCAG 2.2 AA used by `frontend/e2e/tests/
  a11y.spec.ts`, reflow at 320 px, and a journey done with the keyboard only.
- **SC-041-007** On one machine, the same day, the transitions scenario of `tests/performance/scalability-ha`
  with every step writing ten `info` lines reaches at least 90% of its rate with capture off, in the rows
  "1 API replica, 16 workers" and "8 replicas, 128 workers", and the p99 of a completion that stores output is
  within 10% of its value with capture off. The machine, its load and the PostgreSQL configuration are stated;
  the figures are criteria to verify, not promises.

### User stories

- **US01** A developer opens a failed run in the Studio, selects the failed step and reads what its code logged
  in the attempt that failed and in the ones before it.
- **US02** An operator watches a long step of a running workflow and sees its lines arrive.
- **US03** A tenant administrator bounds what logs cost (bytes, days), turns capture off for a namespace whose
  code logs personal data, and deletes a run's log when a secret was logged.
- **US04** An SDK user changes nothing in their activity code and gets capture after upgrading the SDK.

## Goals

- What a step's code logged is readable from the run, by the people allowed to, during the run and for a
  stated number of days after.
- Capture adds no statement to a lease or a completion. A completion that stores output waits on a log batch
  only for that batch's commit on the tenant's usage row, and the wait is measured before the SDKs are built.
- Every line that is not there is accounted for by a count and a reason the reader can see.
- A secret the platform handed to any attempt of a worker process is not stored, in any of the six SDKs.

## Non-Goals

- **Not the platform's own logs.** The stdout JSON of the processes, their spans and the Collector pipeline
  belong to features 008 and 015. This feature does not export step logs to OpenTelemetry.
- **Not stdout or stderr of the worker process.** Only lines written through the SDK's activity logger are
  captured. A `print` in an activity is not, and lines written by a thread that outlives its activity are not.
- **Not a log search product.** No index on the text, no query across runs, no aggregation, no alerting on
  content. `q` is a substring match inside one run.
- **Not execution state.** A workflow cannot read a step's log, an expression cannot test it, and it is not an
  input to a compensation or a review.
- **Not an archive.** Legal hold, export to a customer's storage and retention beyond 365 days belong to
  feature 020.
- **No change to how script logs are written or kept.** `script_executions`, its ceiling and its retention stay
  as feature 005 built them; only who may read the `log` field changes (FR-041-022). Live lines from a running
  script are a follow-up (G041-003).
- **No classification of log content by field.** A log line has no schema. The classified-namespace rule of
  FR-041-021 decides who may read, not what is shown.
- **No download or export button** in the first delivery.

## Open Questions

Technical, each closed by a named task:

- **Does ingest still slow completions?** The revision takes the tenant usage row only at commit (FR-041-008).
  Each batch still adds a commit that takes the row, and a per-row trigger that builds `to_jsonb(NEW)` for up
  to 500 rows. T041-6 measures it as a spike before T041-7 is built; if SC-041-007 fails, the fallback is a
  grouped commit (Alternatives Considered), and after that a separate usage row, which needs an amendment of
  ADR-0027.
- **Does the stored-order cursor hold on PostgreSQL 16 and 18?** T041-11 proves it with held transactions on
  both.
- **Which scope do the new quota resources take?** Whether the namespace scope is available for the capture
  switch is settled in T041-4 against `crates/domain/src/quota.rs`. If it is not, the switch decided "per
  namespace" (O041-001) cannot be built as decided: T041-4 stops and the difference goes back to the owner.
- **Is 1.7 reachable where 1.6 is not?** `protocol_ceiling` offers 1.6 only when the store proves scoped and
  database support. 1.7 needs both proofs and the log tables; a deployment held lower captures nothing
  (G041-005).

## Owner decisions needed

None open. Recorded in `docs/owner-decisions.md` and as O items in `specs/041-step-logs/research.md`:

- **O041-001 to O041-011** were answered as recommended on 2026-11-13 ("Aceitar todas", OD-2026-11-13-14):
  - O041-001: capture is on by default at `info`, with a switch per namespace and an option per worker process.
    The release notes say it first (T041-25). Not chosen: off until a namespace asks; on for new namespaces only.
  - O041-002: `run:logs:read` for admin, developer and operator. Not chosen: also viewer; a per-namespace grant
    only. OD-2026-11-13-18 adds the PII grant in classified namespaces.
  - O041-003: one `run.logs_read` per subject and run per 300 seconds. Not chosen: one per request; none.
  - O041-004: PostgreSQL, one row per line, ordinary table. Not chosen for 0.3.0: compressed chunk rows (the
    next step if the measurement asks); the blob store (committed blobs are never purged today, G69); a
    partitioned table.
  - O041-005: 8 KiB a message; 1 MiB and 10,000 records an attempt; 16 MiB and 100,000 a run;
    `tenant_log_bytes` shared with script logs. Not chosen: a separate tenant quota.
  - O041-006: `step_log_retention_days`, default 14, 1 to 365. Not chosen: 7, 30, `script_retention_days`.
  - O041-007: at the attempt ceiling the first lines are kept, then a marker and a count. Not chosen: a ring.
  - O041-008: a new server-sent event route. Not chosen: polling only. The graph stream was never an option.
  - O041-009: a script step gets a marker that points to its execution. Not chosen: live script lines (G041-003);
    leaving script steps out.
  - O041-010: server, six SDKs, routes and Studio together in 0.3.0. Not chosen: SDKs one by one.
  - O041-011: the limit of redaction (G041-001) is accepted as stated in ADR-0085.
- **ADR-0084 and ADR-0085** were accepted on 2026-11-13 (OD-2026-11-13-17) and revised after review, before any
  code; each carries a Revision section. Their revision goes to the owner with this design (T041-2).
- **O041-012 and O041-013** were answered on 2026-11-13 (OD-2026-11-13-18):
  - O041-012 ("Exigir a concessão PII"): `run:logs:read` stays with admin, developer and operator, and in a
    classified namespace reading step logs also needs the namespace's PII reveal grant (FR-041-013, FR-041-021).
  - O041-013 ("Fechar ao viewer"): the `log` of a run's script execution is read under the same rule; viewers
    lose it and the release notes say so (FR-041-022).

## Critical User Journeys (CUJs)

### CUJ 1: Why did this step fail?

*   **Given**: a run whose step `charge` failed on its third attempt; the activity logs at `info` and `error`
    through the SDK's logger; the reader holds `run:logs:read`, and the namespace is not classified.
*   **When**: they open the run in the Studio, select `charge` and open Logs.
*   **Then**: they see the lines of attempt 3, can switch to attempts 1 and 2, each line with its level as text
    and shape, its worker time and its fields, and the secret the activity used shows as `[REDACTED]`.

### CUJ 2: Watching a long step

*   **Given**: a running step that logs a line every few seconds.
*   **When**: the reader opens the Logs tab with the tail on.
*   **Then**: lines appear within about a second of the worker's flush; scrolling up pauses the tail and a
    "Jump to latest" control resumes it; when the connection drops and returns, no line is missing or doubled;
    when the run ends, the stream ends after the last line that could still arrive.

### CUJ 3: A step that floods

*   **Given**: an activity with a loop that logs two million lines.
*   **When**: it runs.
*   **Then**: the step completes as it would without logging; the log holds the first lines up to the attempt
    ceiling and one marker "attempt ceiling reached"; the streams list says how many were dropped by the SDK's
    buffer and how many by the ceiling. No alert fires: it is the customer's code, shown on the run.

### CUJ 4: A worker that was not upgraded

*   **Given**: a worker on an SDK that offers protocol 1.5.
*   **When**: it runs steps on a server that supports 1.7.
*   **Then**: nothing changes for it; its steps show "no log" in the Studio, with the note that the worker did
    not send any.

### CUJ 5: A reader without the permission or the grant

*   **Given**: a viewer, who holds `run:read` and not `run:logs:read`; and a developer without the PII reveal
    grant of a classified namespace.
*   **When**: each opens a run.
*   **Then**: the Logs tab names what is missing and makes no request; a direct call to a route is refused with
    the typed error, and nothing about the log's existence or size is returned. The viewer's script-execution
    view shows "log withheld".

### CUJ 6: A collection's iterations

*   **Given**: a `forEach` over 200 items whose body step logs.
*   **When**: the reader opens the whole-run Logs tab and filters by that node, then picks iteration 17.
*   **Then**: the tab shows lines of all iterations in stored order, each labelled with its iteration; the
    filter narrows to the one attempt log of iteration 17.

### CUJ 7: A secret was logged

*   **Given**: a step's code that base64-encoded a secret into a log line (G041-001).
*   **When**: an administrator learns of it.
*   **Then**: `DELETE /runs/{runId}/logs?stream=…` makes the lines unreadable at once and is audited; the purge
    pass removes the rows; the runbook says to rotate the secret.

## Technical Plan (Architecture)

```text
activity code
   | logger.info(...)                      (unchanged API; never blocks, never throws)
   v
SDK capture in the per-attempt logger: level floor -> sequence -> redact by the process's live secret
   values -> cut -> bounded buffer per attempt -> DROPPED markers for gaps
   | AppendStepLogs (unary, 1.7): attempt identity + fence + session token, <=500 records, <=192 KiB
   v
api-grpc (Class::Logs, own permits) -> application::step_logs::ingest (authority, scrub, size, allowances)
   | one transaction, separate from lease/heartbeat/complete, lock_timeout 200 ms:
   |   xid -> authority reads -> lease read (no lock) -> lock stream -> run usage -> [near limit: tenant row]
   |   -> insert ... RETURNING -> counters -> commit (finalizer of ADR-0027)
   v
PostgreSQL: step_log_streams, step_log_lines, run_log_usage     (no foreign key to runs or steps)
   ^                                   ^
   | purge (housekeeping pass)         | GET /runs/{id}/logs, /logs/streams, /logs/stream; DELETE /runs/{id}/logs
                                       v
                                    Studio: Logs tab and step section
```

### 1. Wire: protocol minor 1.7

`specs/001-platform-foundation/contracts/worker.proto` (the canonical copy; the six SDK copies are
byte-identical and move with it, and `worker.baseline.json` records the addition) gains, additively:

- `rpc AppendStepLogs(AppendStepLogsRequest) returns (AppendStepLogsResponse)`.
- `AppendStepLogsRequest`: `worker_id` and `session_token` (as `PollTasksRequest` carries them), the fields
  `HeartbeatRequest` carries to name and fence an attempt (`task_id`, `generation`, `lease_token`, `run_id`,
  `step_id`, `attempt`, `instance`), `repeated StepLogRecord records`, and `bool final`.
- `StepLogRecord`: `uint64 sequence`; `int64 worker_time_epoch_ms`; `level`; `kind` (`LINE`, `DROPPED`);
  `string message`; `repeated StepLogField fields` (key and one of string, double, int64, bool); `bool
  truncated`; for `DROPPED`, `uint64 first_dropped_sequence`, `uint64 last_dropped_sequence` and a reason
  (`BUFFER_FULL`, `SEND_FAILED`, `STOPPED_BY_SERVER`). A `DROPPED` record has an empty message and no fields.
  It carries no count: the server derives the count from the range.
- `AppendStepLogsResponse`: `uint64 stored_through_sequence` and a verdict: `CONTINUE`; `STOP_ATTEMPT` (a
  ceiling was met or `final` was recorded: send nothing more for this attempt); `STOP_CAPTURE` (capture is off,
  or the lease, the binding, the membership or the session is no longer valid); `RETRY_LATER` with a delay (the
  server is shedding or a lock timed out).
- `RegisterWorkerResponse` gains `StepLogPolicy step_logs`: `enabled`, the line, field, batch, buffer and
  attempt ceilings and the flush interval. An SDK captures only when the accepted minor is at least 1.7 and
  `enabled` is true, and uses the server's numbers.
- The generated `Debug` of `AppendStepLogsRequest` and `StepLogRecord` is skipped in
  `crates/api-grpc/build.rs` and replaced by one that prints counts only, as is done for the secret-bearing
  messages (`crates/api-grpc/src/lib.rs`); the SDKs never log the request.

Negotiation is unchanged (ADR-0015): `negotiate_protocol_details` answers the lower of the two minors.
`PROTOCOL_MINOR` becomes 7, with `STEP_LOG_PROTOCOL_MINOR = 7` beside the others; `protocol_ceiling` answers 7
for an ordinary worker only when the store proves scoped and database support and has the log tables, and keeps
6 as the ceiling of the database runner. Minors are a line, so an ordinary worker at 1.7 has 1.6's
`ResolveDatabase` on the wire: it never calls it, and the server refuses it without `database:execute`, which
ordinary roles do not hold (`crates/application/src/worker/database.rs`). `sdks/SPEC.md` §7.1, which says
ordinary SDKs keep 1.5 and only the managed executor offers 1.6, is amended in T041-14 to say this, and the
`step-logs-negotiation` scenario asserts the refusal. The sandbox runner keeps offering `1` and the database
runner `1.6`.

**Why a separate unary RPC.** A field on `Heartbeat` would tie the size and the failure of a log batch to the
call that keeps the lease alive. A client stream would need acknowledgement and resend rules in six languages
for a gain the batch already gives. Riding on `CompleteStep` would add statements to the completion.

**Admission.** `AppendStepLogs` gets its own `Class::Logs` in `crates/api-grpc/src/limits.rs`, with its own
cap: it is shed before discovery and never draws on the reserve kept for lease calls. The class test lists it.

### 2. Ingest

A new application module, `sagat_application::step_logs`, with a port `StepLogStore` and its adapter in
`crates/infra-postgres/src/step_logs.rs`. Domain types (level, record kind, ceilings, marker reasons, the
validation of a record, the server's sequence range) live in `crates/domain/src/step_logs.rs` and import
nothing from the wire. Scrubbing, cutting and sizing are application code, so that the adapter receives final
text and the sizes it charges are the sizes of what it stores.

**Permits.** Ingest runs under a semaphore sized below the pool (default a quarter of it), with a per-tenant
sub-cap (default 4 permits), so that one tenant's flood cannot hold every permit. When no permit is free within
50 ms the answer is `RETRY_LATER`.

**Before the transaction**, in the application: `principal.authorize(Permission::WorkerExecute)`; the role must
not be `database-runner` or `runner` (`STOP_CAPTURE`); the batch is validated (sequences increase, no sequence
in the server's range, no unknown level or kind, a `DROPPED` record has a valid range below its own sequence),
and a malformed batch is refused whole as `INVALID_ARGUMENT`; NUL and non-finite numbers are mapped
(FR-041-003); each message, key and string value is scrubbed by the two scrubbers and the sensitive-name rule
(FR-041-007), then cut to its ceiling; then each record is sized exactly as `sagat_quota_measure` will measure
it (the UTF-8 octet length of `message` and of `fields`, which is stored as canonical JSON text so that the size
computed in Rust is the size the trigger measures).

**The transaction** (one per batch, opened with the project's `begin_safely` rule, ADR-0014, with
`SET LOCAL lock_timeout = '200ms'`; a lock timeout answers `RETRY_LATER`):

1. `SELECT pg_current_xact_id()`, so that the transaction's id is assigned before the lease is read (the tail
   relies on it, section 5).
2. **Session and authority.** The worker session is read by its id and token hash: its subject must be the
   caller's and its accepted minor at least 7. Then the checks every report makes, short of the run lock:
   `current_worker_authorized` (the membership row read under the lock a revocation waits for, which refuses
   the database-runner role) and the activity binding (`bound_patterns` and `first_uncovered` against the
   step's `activity`). A failure answers `STOP_CAPTURE`.
3. **The lease, without a lock.** One `SELECT` of the `steps` row by its primary key joined with its `runs` row:
   status `LEASED` or `RUNNING`, the same attempt and generation, the lease-token hash, `lease_expires_at >
   clock_timestamp()`, and the run neither finished nor stopping (`finished_at` and `stop_reason` null). No
   `FOR SHARE`, no `FOR UPDATE`. A mismatch answers `STOP_CAPTURE` and stores nothing. A batch that read a valid
   lease and commits just after the completion is still that attempt's lines; a stale holder's token never
   matches.
4. **Lock the stream.** `INSERT … ON CONFLICT DO UPDATE … RETURNING` on `step_log_streams`, keyed by (tenant,
   run, instance path, node, attempt, generation, source). The row is locked to commit; only batches of the same
   attempt wait on it, and an SDK sends one at a time. If `final_at` is set the answer is `STOP_ATTEMPT`.
5. **Drop duplicates.** Records with a sequence at or below the stream's `last_sequence` are removed.
6. **Fold the `DROPPED` markers.** For each, the server counts the sequences of its range that are above
   `last_sequence` and are not in this batch, adds them to `dropped_by_sdk` with saturation, and clamps the
   total at the attempt's record ceiling. The marker is stored as a line only while the attempt ceiling is not
   met.
7. **Allowances**, in this order, from counters and not by counting rows: the attempt (records and bytes on the
   stream row); the run (`run_log_usage`, read `FOR UPDATE`); the tenant. The tenant allowance reads
   `tenant_retained_usage.log_bytes` and the limit **without a lock**. When usage plus the batch is more than
   2 MiB below the limit, the batch goes on and the commit-time finalizer enforces the limit exactly. Otherwise
   the server takes the content fence, the log fence and the usage row (`sagat_quota_lock_log_usage`, as
   `take_log_prefix` does), as the last statement before the insert, and keeps the records that fit. If the
   finalizer raises `tenant_log_quota_exceeded` at commit (concurrent batches crossed the limit), the request
   runs once more on the locking path.
8. **Insert** the kept records and, when the attempt ceiling is first met, its one marker (under
   `next_marker_sequence`, which starts at 2^62), in one statement with `ON CONFLICT (tenant_id, stream_id,
   sequence) DO NOTHING RETURNING` the sizes. Markers have null `message` and `fields`, so they carry no
   `log` bytes.
9. **Counters** on the stream and on `run_log_usage` are updated from the rows the insert returned, never from
   the batch. `last_sequence` takes the batch's highest worker sequence; `final_at` is set when `final` is true,
   even when every record of the batch was past a ceiling. A ceiling met is recorded on the stream as
   `ceiling_reason`.
10. **Commit, then answer.** A crash before the commit leaves nothing and the SDK sends the batch again; a crash
    after it leaves the lines, and the resend stores nothing and gets the same answer.

The number of top-level statements per batch does not depend on the number of records. It is budgeted as an
equality in the feature's suite (T041-6).

**One lock order** for everything that writes step logs, which every new path follows and T041-6 tests: the
namespace secrets lock (shared) and the membership row, as every report → the stream row → the `run_log_usage`
row → the content policy fence → the log policy fence → the tenant's `tenant_retained_usage` row. The commit-time
finalizer takes the last three in the same order. `take_log_prefix` takes the content fence, the log fence and
the usage row in that order, so a script marker written after it (section 7) is compatible: it locks a stream
row that no ingest ever locks and does not touch `run_log_usage`.

### 3. Storage

Three tables, in one migration (the next free timestamp after `20270112000000_lake_blob_pin_release`, checked
against the other open branches before it is taken), plus one column on `namespaces` (FR-041-021):

- `step_log_streams`: `tenant_id`, `stream_id` (identity), `namespace_id`, `run_id`, `instance_path_nodes
  text[]`, `instance_path_indices integer[]`, `node`, `step_id`, `attempt`, `generation`, `source` (`worker`,
  `script`), `records`, `bytes`, `last_sequence`, `next_marker_sequence`, `dropped_by_sdk`,
  `dropped_by_server`, `ceiling_reason`, `truncated_records`, `final_at`, `first_at`, `last_at`,
  `deleted_at`, `purged_at`. Unique on (tenant, run, instance path, node, attempt, generation, source); indexed
  by (tenant, run) and by `last_at` where `purged_at` is null. For `source = script`, `generation` and the
  instance path are nullable (section 7).
- `step_log_lines`: `tenant_id`, `run_id`, `stream_id`, `sequence`, `id` (identity), `stored_xid xid8`,
  `stored_at`, `worker_time`, `level`, `kind`, `truncated`, `message text`, `fields text` (canonical JSON),
  `marker_reason`, `covered_first`, `covered_last`, `marker_count`. Primary key (tenant, stream, sequence);
  index (tenant, run, stored_xid, id) for reading. A trigger refuses `UPDATE`; `DELETE` is allowed for the purge.
- `run_log_usage`: `tenant_id`, `run_id`, `records`, `bytes`.
- `namespaces.has_classified_schemas boolean NOT NULL DEFAULT false`.

**No foreign key to `runs` or `steps`.** An insert that references `runs` takes `KEY SHARE` on the run row,
which conflicts with the `FOR UPDATE` the engine holds while it leases or completes. The run's existence in the
tenant is proven by step 3 of the ingest in the same transaction, and nothing deletes a run today. Feature 020
must delete a run's streams, lines and usage row when it deletes the run (G041-004, registered in feature 020's
research as G1618). `step_log_lines` references `step_log_streams` (same tenant), a row the engine never locks.

**Accounting.** In `crates/infra-postgres/storage-classification.json`, `step_log_lines.message` and `.fields`
are `log`; every other variable-size column of the three tables (`node`, the path arrays, `source`,
`ceiling_reason`, `marker_reason`, `level`, `kind`) is `protected_system`, so a marker charges no log bytes. The
migration creates, for the three tables, the measurement and finalizer triggers that the manifest generates
one-for-one (ADR-0027), so that `manifest_covers_every_variable_size_column_and_every_accounted_table_has_always_triggers`
passes unmodified.

**Grants.** In the migration's `DO $grants$` block: `sagat_app` gets `SELECT, INSERT, DELETE` on
`step_log_lines`, `SELECT, INSERT, UPDATE` on `step_log_streams` and `run_log_usage`, `UPDATE
(has_classified_schemas)` on `namespaces`, and usage of the identity sequences. No `UPDATE` on lines, no
`TRUNCATE`, no `SECURITY DEFINER` function. The runtime privilege audit covers the new statements. There is no
row-level security in this database; the tenant boundary is the `tenant_id` in every key and predicate, tested
by the cross-tenant battery. The down migration refuses to run while a line exists.

**Not partitioned.** No table in this database is partitioned. Dropping a partition would not fire the
triggers that release quota bytes, and retention differs per tenant. The lines table gets its own autovacuum
settings instead.

### 4. Retention, purge and deletion

A housekeeping pass (`crates/scheduler/src/passes.rs`, beside `purge_script_records`, through a new method of
`MaintenanceStore`) takes streams whose `deleted_at` is set, or whose `last_at` plus the retention in force for
their namespace is past, with `FOR UPDATE SKIP LOCKED`, deletes their lines in batches of `PURGE_BATCH`, and
sets `purged_at` when none is left. Deleting releases the quota through the triggers. A stream whose attempt
still holds its lease is skipped unless it is deleted. Expiry is computed at purge time, so a lower retention
reaches lines already stored. `run_log_usage` is not reduced: the run ceiling bounds what a run may ever write.

`DELETE /runs/{runId}/logs[?stream=]` (FR-041-023) sets `deleted_at` on the run's streams (or the one named)
and writes `run.logs_purged` in the same transaction; readers skip deleted streams at once. A batch for a
deleted stream answers `STOP_ATTEMPT`.

### 5. Reading

A second port, `StepLogReader`, used by the handlers in `crates/api-http/src/routes/execution.rs`. Each read
calls `admit` with a new `ApiOperation::RunLogsRead` that maps to `&[RunRead, RunLogsRead]`; the application
authorizes `run:logs:read` again and, when the run's namespace has `has_classified_schemas`, checks
`has_effective_payload_reveal_grant` in the read transaction (which holds the grant-set lock shared, so a
revocation cannot commit between the check and the audit). The permission is added to `Permission::ALL`, with
`run:logs:delete` (admin only), and to the role matrix test. `SAGAT_STEP_LOGS_READ_ENABLED` (default true) turns
the read routes off as a typed `FEATURE_DISABLED` without touching capture.

**Order and cursor.** Lines are read in (`stored_xid`, `id`) order, and only those whose `stored_xid` is below
`pg_snapshot_xmin(pg_current_snapshot())` computed in the same statement. Every transaction with a smaller id
has ended, so no line can later appear before the cursor: a page is final and a tail never skips. The cursor is
that pair and the run id, sealed with a server key; one minted for another run, or altered, is refused as
`VALIDATION_FAILED`. A long write transaction anywhere in the database holds the tail back until it ends; the
lines are not lost, and the response says how far behind the horizon is.

**End of the tail.** Because ingest assigns its transaction id before it reads the lease (step 1), every batch
that saw the lease live has an id below any snapshot taken after the run became terminal. At the first poll that
sees the run terminal, the tail records X = `pg_snapshot_xmax(pg_current_snapshot())`; it sends `end` only when
a later poll finds `pg_snapshot_xmin` at or above X and one more page is empty.

**Filters** are predicates on indexed columns except `q`, which is `position(lower($q) in lower(message)) > 0`
over the lines of one run (at most 16 MiB by the run ceiling), under the route's statement timeout, as a bound
parameter. `stream` is matched together with the run and tenant.

**Audit.** `run.logs_read` is a new action in `crates/domain/src/audit.rs`. The first read by a subject of a run
in a 300-second window writes the record in the read transaction before any line is returned ("no audit, no
lines"); later reads in the window do not write, whatever their filters. The record holds the run, the subject
and the names of the first read's filters, never a line or the value of `q`.

**Tail.** `GET /runs/{runId}/logs/stream` reuses the loop of `follow` in `execution.rs`: server-sent events,
`event: log` with up to 200 lines and the cursor as `id`, a `: ping` every 15 seconds, `event: end` as above,
`event: reconnect` at token expiry or the 15-minute limit, resume by `Last-Event-ID`, polling every 0.5 s and
every 2 s once idle for 30 s, each poll authorizing `RunLogsRead` and the grant again through the reader, and
the same caps (20 per subject, 200 per tenant) as the graph stream, shared with it.

**Contract.** Four operations in `specs/001-platform-foundation/contracts/openapi.yaml`, tagged
`x-implemented: '041'`: `listRunLogs`, `listRunLogStreams`, `streamRunLogs`, `deleteRunLogs`. The digest test
`operations_outside_the_approved_additions_and_event_extensions_are_byte_stable` gains `041` in its list of
approved additions. `getScriptExecution` changes (FR-041-022): its `log` becomes optional and `logWithheld` is
added. That operation is inside the digest, so T041-29 changes the expected digest after reading the diff, and
states the break in the release notes.

### 6. The six SDKs

`sdks/SPEC.md` §6.3 gains the logger's semantics and a new section defines capture; §7.1 and §9 are amended for
1.7; §10 keeps its rule that a logger that throws never kills a heartbeat or a task, and adds that capture never
does either.

| SDK | Logger today (kept) | Capture is added in the per-attempt object |
|---|---|---|
| TypeScript | `ActivityContext.log: ActivityLogger` | the attempt's `ActivityLogger` instance, behind its four methods |
| Go | `Logger *slog.Logger` | a per-attempt `slog.Handler` that wraps the host's handler; never installed on the host's logger |
| Python | `ActivityContext.log` (`_TaskLog`, a `LoggerAdapter`) | `_TaskLog.log` and `process`, from the raw message and arguments, before the identifiers are appended; never a `logging.Handler` on the shared logger |
| C# | `ILogger Logger` | a per-attempt `ILogger` decorator, with scopes captured as fields |
| Java | `ActivityLogger logger()` | a per-attempt decorator of `ActivityLogger` |
| Rust | none | a new `ActivityContext::log()` with the five levels, which also emits `tracing` events |

Common rules, each a line of the specification and a conformance assertion:

- The process keeps one set of the secret values held by every attempt in flight: values are added when
  `ResolveSecrets` returns and removed when the attempt ends. Every captured line is redacted with that set
  (FR-041-007), so a value of attempt A is removed from a line of attempt B.
- The logger call takes the next sequence, formats, redacts, cuts to the line ceiling and appends to the
  attempt's buffer, without waiting. A full buffer drops the record and remembers its sequence.
- A sender per attempt flushes every second, or at 256 records, one batch in flight. A failed send is retried
  twice with the same bytes, then the batch is dropped. Every gap in the sequences sent is reported by a
  `DROPPED` record with its first and last sequence and the reason.
- `STOP_ATTEMPT` stops sending for the attempt; `STOP_CAPTURE` stops for the attempt and, when it was a
  session failure, for the session; `RETRY_LATER` waits the given delay; `UNIMPLEMENTED` stops for the attempt.
- Before `CompleteStep` or `FailStep`, the SDK flushes what remains with `final = true`, waiting at most
  2 seconds; then it reports the step whatever the flush did.
- The buffer is in memory only and dropped with the attempt. Capture uses no thread or task that can keep the
  process alive after shutdown, and worker shutdown flushes each attempt once within the shutdown deadline.
- The SDK never logs a batch, and nothing about capture is written at a level the host logs by default except
  one line per process when the server stops capture.

Conformance scenarios, one file and one golden each, run by all six through `tests/contract/sdk-conformance`.
The harness gains a step that reads `GET /runs/{id}/logs` and compares it with the golden, and a server fault
hook for the RPC:

1. `step-logs-capture`: the five levels, the floor, fields of each type, NUL and non-finite numbers, order by
   sequence, two attempts of one step, an iteration of a root collection, and language context (scopes, `extra`,
   `slog` attributes) captured as fields.
2. `step-logs-redaction`: two attempts in flight at once with different secrets; each logs its own and the
   other's whole, inside a message, as a field key, as a field value, as a number equal to a numeric secret and
   inside an exception; `mustNotLeak` for both.
3. `step-logs-limits`: a message over the ceiling is cut and flagged; a flood meets the buffer and the attempt
   ceiling; the step completes with its normal output; the counts add up.
4. `step-logs-transport-faults`: a lost response (same batch twice), a refused batch, `RETRY_LATER`,
   `UNIMPLEMENTED`, the server down for the whole attempt; no duplicate; the step completes.
5. `step-logs-negotiation`: a worker pinned to 1.5 sends nothing; a 1.7 worker told `enabled = false` sends
   nothing; both log locally; a 1.7 ordinary worker calling `ResolveDatabase` is refused.

### 7. Steps the platform runs

`record_execution` (`crates/infra-postgres/src/scripts.rs`), which stores the script runner's report, finds the
step by (tenant, run, node) and reads the `step.leased` event of that attempt; it does not carry an instance
path. After `take_log_prefix` and the insert into `script_executions`, it also inserts a `source = script`
stream for that attempt and one marker line: kind `script_output`, the execution id, the log's size and its
`log_truncated` flag, no text. The stream's generation is the one of that `step.leased` event; its instance path
is the one `script_executions` records when the report carries it and null otherwise (G041-007). The marker
does not touch `run_log_usage` and charges no log bytes; its stream row is never locked by an ingest, so it
cannot close a cycle with one (section 2, lock order; tested in T041-13). The Studio shows the marker as a row
that opens the script execution's log under FR-041-022. The script's text is stored once, counted once, redacted
by the sandbox as today.

### 8. Studio

In `frontend/apps/studio/src/features/runs/`:

- A fourth tab, **Logs**, in `RunDetailPage.tsx` beside Graph, Table and Timeline, and a **Logs** section in
  `NodePanel.tsx` for the selected step with an attempt selector fed by the streams route (paged).
- `LogView.tsx`: rows windowed with `useWindowedRows`; each row shows time (worker time, with the stored time
  on demand and a note when the two differ by more than five minutes), level as text and shape as well as
  colour, the step and iteration in the whole-run view, the message as text, never as HTML, never auto-linked,
  with terminal escape sequences removed and bidirectional controls and zero-width characters shown as visible
  escapes, and fields in an expandable part.
- Filters: level, step, attempt, time range; a search field that sends `q`; all reflected in the URL except
  the search text.
- Tail: `followLogs()` beside `followRun()` in `src/api/follow.ts`, same parser (`readSse`), same fallback to
  polling the list route. The tail pauses when the reader leaves the end; "Jump to latest" resumes. New lines
  are announced through `Announcer` as a count at most every ten seconds.
- States: `NotAllowed` naming `run:logs:read` or the PII reveal grant, with no request made; "no log" with the
  reason when known; "expired on …"; "deleted"; "ended without its final flush: the last lines may be missing";
  markers as rows with text; for a script execution read without the right, "log withheld".
- Keyboard: the tab follows the existing `Tabs` pattern; the list is one tab stop with arrow, Page, Home and
  End movement and Enter to expand a row; focus is not moved by arriving lines.
- Types come from `npm run api`; `npm run check:api` stays green.

### 9. Observability

Metrics (bare names as the code uses; labels are closed enumerations, never tenant, namespace, run, node or
worker):

- `step_log_batches_total{outcome}`: `stored`, `duplicate`, `stopped_attempt`, `stopped_capture`,
  `retry_later`, `invalid`.
- `step_log_lines_accepted_total{level}`, `step_log_bytes_accepted_total`.
- `step_log_lines_dropped_total{where, reason}`: `where` is `sdk` or `server`; reasons `buffer_full`,
  `send_failed`, `stopped_by_server`, `attempt_ceiling`, `run_ceiling`, `tenant_quota`.
- `step_log_lines_truncated_total`, `step_log_redactions_total{rule}` (`shape`, `field_name`).
- `step_log_ingest_seconds`, `step_log_lock_timeouts_total`, `step_log_quota_retries_total`,
  `step_log_purged_lines_total`, `step_log_purge_backlog` (streams past expiry), `run_log_streams_open`,
  `run_log_streams_refused_total`.

Alerts in `infra/observability/alerts.yaml`, with entries in `docs/runbooks/alerts.md`:
`SagatStepLogIngestShedding` (`retry_later` above 5% of batches for ten minutes) and `SagatStepLogPurgeBehind`
(purge backlog growing for an hour). Drops at a ceiling are the customer's code and are shown on the run, not
alerted. The ingest has a span, `worker.append_step_logs`, with counts and the verdict and no text. A new
runbook, `docs/runbooks/step-logs.md`, covers the ceilings, the switch, the purge, what each marker means, that
a change of the switch or a ceiling is not retroactive and a change of retention is, and the incident path
(CUJ 7: delete the lines, rotate the secret).

### 10. Rollout and rollback

The migration is additive. `SAGAT_STEP_LOGS_ENABLED` makes the policy answer `enabled = false` and the RPC
answer `STOP_CAPTURE` without touching the database; it defaults to **false** until the purge pass (T041-10) is
merged, so that nothing is stored that nothing deletes, and to true from then on.
`SAGAT_STEP_LOGS_READ_ENABLED` turns the read routes off. Old and new binaries run together: an old API replica
answers `UNIMPLEMENTED`, which an SDK treats as a stop for that attempt. Rolling back the binary leaves the
tables unread; rolling back the migration needs the lines purged first.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| A step's code logs a secret the platform cannot recognise (transformed, short, obtained elsewhere) | G041-001, accepted (O041-011). Value redaction across the process's live attempts, two shape scrubbers, a permission of its own, the PII grant in classified namespaces, the namespace switch, the delete route and its runbook |
| Log batches make completions wait on the tenant usage row | The row is taken only at commit (and near the limit, for the insert, under a 200 ms lock timeout); a barrier test (SC-041-001); the spike in T041-6 before the RPC is built; the grouped commit as a fallback |
| Each logging step adds a transaction and up to 500 trigger calls | Per-tenant permits and shedding; a step that logs nothing sends nothing; T041-9 measures trigger cost and commit latency of a full batch |
| The final flush delays the completion of every step that logs | One call, bounded at 2 seconds, skipped when the buffer is empty; measured in T041-9 |
| A revoked worker keeps writing | The ingest checks membership, binding, session and lease expiry as reports do; tests for each |
| A long write transaction holds the tail back | Lines arrive late, never out of order or missing; the response states the delay; tested with a held transaction |
| The tail ends before a late batch commits | The end rule waits for the snapshot horizon to pass the terminal poll's `xmax` |
| Ingest and the script marker deadlock | One lock order; the marker locks no row ingest locks; a barrier test |
| A flood fills the table before the ceilings act | Ceilings from counters before the insert, in the same transaction; `DROPPED` counts clamped |
| The missing foreign key leaves log rows of a deleted run | Nothing deletes a run today; G041-004 is registered in feature 020 (G1618); a test asserts every stream's run exists |
| The Studio renders log text as markup, links or reordered text | Text only, no links, escape sequences removed, bidirectional and zero-width characters made visible; tested |
| Purge falls behind ingest | Purge rate against ingest rate measured in T041-9; `SagatStepLogPurgeBehind` |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | A log is not execution state: no transition reads it, and lease, heartbeat and completion run no statement on it (FR-041-008, SC-041-001). The lease and fencing token, its expiry, the binding and the membership decide whether a batch is accepted; a stale attempt is refused, never merged (Ingest, steps 2-3). The journal and its sequence are unchanged |
| II. Modular domain, ports and adapters | Pass | Domain types in `crates/domain/src/step_logs.rs` with no wire or SQL import; ports `StepLogStore` and `StepLogReader`; scrubbing and sizing in the application; adapters in `infra-postgres`, `api-grpc`, `api-http`. `scripts/check-architecture.py` runs on every slice |
| III. External effects are at-least-once; sagas compensate | Pass | Delivery of a batch is at-least-once and made idempotent by (attempt, generation, sequence); nothing here is an external effect |
| IV. Compatibility is versioned and pinned | Pass with conditions | Additive minor 1.7 (ADR-0015); older workers and servers behave as today; four new operations; the six SDKs pass the same five scenarios. The condition: `getScriptExecution` withholds `log` from callers without the step-log right (OD-2026-11-13-18), a documented break of one field for viewers, with the digest changed in T041-29 |
| V. Code, tenant boundary and secrets are isolated | Pass with conditions | Tenant in every key and predicate, 404 and audit for another tenant's run, a permission of its own, the PII grant in classified namespaces, reads audited, an audited delete. Values handed to any live attempt of a process are removed before a line leaves it. The condition is G041-001, accepted by the owner (O041-011, ADR-0085) |
| VI. Observability and audit are part of the feature | Pass | FR-041-018: metrics with closed labels, a span without text, typed errors, alerts on shedding and purge, `run.logs_read` and `run.logs_purged` |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | No broker; a batch is safe to deliver twice. Backpressure is designed (bounded buffer, permits per tenant, `RETRY_LATER`, counted drops). No dead-letter path: a dropped line is counted, not parked (ADR-0084) |
| VIII. Failure is tested and contracts are verified, honestly | Pass | The Verification Plan names the crash, duplicate, lost-batch, stale and expired lease, revocation, flood, old-worker, lock-order, tail-end and restart tests, with barriers and held locks, each seen failing first. What is not proven is listed as gaps G041-001 to G041-007 |
| Delivery Constraints | Pass | Rust, tonic, Axum, SQLx, PostgreSQL as the only store; no new component; SC-041-007 is a criterion on a stated machine |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| Condition on principle V: free text written by customer code is stored and may hold a transformed secret | The feature is the storage of that text; the server never holds the secret values | Re-resolving the attempt's secrets on the server for every batch: a disclosure per batch, each to audit |
| Condition on principle IV: one field of `getScriptExecution` is withheld from viewers | The owner closed the script log to callers without the step-log right (OD-2026-11-13-18) | Leaving it readable under `script:read`, which contradicts ADR-0085 |
| Tables with no foreign key to `runs` | A foreign key makes each insert take a lock that conflicts with the engine's run lock | Keeping the convention |
| An unlocked tenant-allowance path, with the locking path near the limit | Taking the tenant usage row early in every batch queues completions behind ingest | Always `sagat_quota_lock_log_usage`, as script logs do: one per step there, many per step here |
| A new RPC, protocol minor and session token on it | The batch must not share the fate of the heartbeat or the cost of the completion; the session's minor and subject must be checkable | A field on `Heartbeat` or `CompleteStep`; an RPC with no session |
| A cursor of transaction id and row id below the snapshot horizon, and an end rule on `xmax` | A tail over concurrent writers otherwise skips a late commit or ends before it | A per-run sequence taken under the run lock |
| A second server-sent event route | The graph stream has one resume point and a wider audience | Log events on `/runs/{runId}/stream` |
| A stream row per attempt beside the lines | Ceilings and drop counts from counters; the "expired", "deleted" and "no final flush" states | Deriving them from the lines on every batch and read |
| Two permissions (`run:logs:read`, `run:logs:delete`) and a namespace flag | Logs can hold personal data; viewers hold `run:read`; deletion is an incident act; the grant applies only where classification is used | Reusing `run:read`; requiring the grant everywhere |
| Capture code in six SDKs | Only the worker process holds the secret values and the logger | Capturing the worker's stdout from outside |

## Alternatives Considered

- **Chunk rows in PostgreSQL** (one row per batch, compressed). About a tenth of the rows and trigger calls.
  Filters, search and per-line identity move into the application. Not chosen for 0.3.0 (O041-004); the next
  step if the measurement asks.
- **The blob store** (ADR-0023) with an index in PostgreSQL. Cheapest per byte; committed blobs are never
  purged today (G69), and the tail and the filters would read objects.
- **A partitioned table dropped by time.** It does not release quota bytes, does not fit a retention that
  differs per tenant, and would be the first partitioned table of the database.
- **Log lines as journal events.** The journal is never purged and is written under the run lock.
- **Sending logs to the OpenTelemetry Collector.** No tenant boundary, ceiling or retention the platform can
  enforce. Export is a later feature.
- **A grouped commit on the server** (several requests' batches in one transaction). Fewer commits and one
  update of the tenant usage row per group; a queue and a failure that touches several requests. Held back
  until T041-6's spike or T041-9 shows it is needed; it changes no contract.
- **Taking the tenant allowance with the row lock in every batch**, as `take_log_prefix` does for a script.
  It is exact but holds the row a completion needs for the whole ingest (the reviews' blocker).
- **Capture through a handler on the language's shared logger.** Simplest in Python and Go; it sees every
  attempt's lines and redacts them with the wrong secrets.
- **Keeping the last lines at the ceiling** (a ring). Not chosen: O041-007.
- **Capture off unless a namespace asks.** Not chosen: O041-001.
- **Accepting batches after the lease has ended**, with a log capability that outlives the lease. A second
  token with its own lifetime and revocation, to save at most one flush interval (G041-002).

## Non-Functional Requirements

- **NFR-041-PERF** SC-041-001 by test on every pull request that touches the hot-path files (the selector rule
  of design 038 picks the budget suite, and the lock tests live in that suite); SC-041-007 by the spike of
  T041-6 and the benchmark of T041-9, with the machine, its load and the PostgreSQL configuration stated, in
  `specs/041-step-logs/validation-report.md`. The ingest's statement count is budgeted as an equality.
- **NFR-041-SEC** No grant beyond those of the Technical Plan, no `SECURITY DEFINER`, no role membership. The
  cross-tenant battery covers the RPC and the four routes. The canary test (SC-041-004) runs in the conformance
  suite and in the server's integration suite. Line text never reaches a metric, span, audit record, error or
  the platform's log; a test greps the captured output of the suites for the canary.
- **NFR-041-OPS** Additive migration; capture off by default until the purge pass exists; the two switches; old
  and new replicas and workers run together; the runbook and the alerts ship with the code.
- **NFR-041-A11Y** WCAG 2.2 AA as the Studio's axe run checks it, reflow at 320 px, keyboard-only journey,
  level by text and shape, no focus theft, announcements throttled (SC-041-006).
- **NFR-041-COST** The stored size of a line is its message and fields plus a fixed overhead that T041-9
  measures and the runbook states.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Six slices; each task is one reviewable pull request unless it says otherwise, and each slice leaves `develop`
releasable. A task that builds what an owner answer decided carries an `Owner-gate:` line. Task numbers are never
reused; T041-28 to T041-30 were added by the revision of 2026-11-13 and sit in the slice they belong to.

Slice A, decisions and contracts (nothing runs yet):

- [ ] T041-1 Take this design through `egm-review` and commit its report beside the three repository reviews of
  2026-11-13; a second round of the repository reviewers reads this revision.
  Refs: FR-041-001, FR-041-013
- [ ] T041-2 Put the revisions of ADR-0084 and ADR-0085 (after review, before any code) to the owner with this
  design, and correct them for what the second round finds.
  Refs: FR-041-007, FR-041-008
  Owner-gate: OD-2026-11-13-14
- [ ] T041-3 Add `AppendStepLogs`, its messages (with the session token and the `DROPPED` range) and
  `StepLogPolicy` to `worker.proto` and its six copies, update `worker.baseline.json`, skip their generated
  `Debug`; add the four operations to `openapi.yaml` tagged `x-implemented: '041'` with the digest test's list
  extended; regenerate the Studio types. Contract checks pass; nothing is served.
  Refs: FR-041-004, FR-041-010, FR-041-011, FR-041-012, FR-041-014, FR-041-023

Slice B, the server stores (capture off by default until T041-10):

- [ ] T041-4 Domain types and validation (`crates/domain/src/step_logs.rs`), the quota resources
  (`step_log_retention_days`, the attempt and run ceilings, the capture switch) at the scope the catalogue
  allows, and the permissions `run:logs:read` and `run:logs:delete` with the role matrix test.
  Refs: FR-041-003, FR-041-006, FR-041-013, FR-041-020, FR-041-023
  Owner-gate: OD-2026-11-13-14
- [ ] T041-5 The migration (three tables, indexes, the no-update trigger, the generated quota triggers for the
  three tables, storage classification with markers and every variable-size column classified, grants, a down
  that refuses data loss), following `.claude/skills/migration`; the privilege audit and the manifest test pass
  unmodified; in `tests/integration/claim-transition-efficiency/`, a test that no foreign key leads from the log
  tables to `runs` or `steps`, with its row in `docs/engineering/architecture-principles.md`.
  Refs: FR-041-008
  Owner-gate: OD-2026-11-13-14
- [ ] T041-6 `StepLogStore` and its adapter: the ingest transaction of section 2 (transaction id first, session
  and authority reads, the lock-free lease read, stream lock, duplicates, `DROPPED` folding, the three
  allowances with the unlocked tenant path and the locking path near the limit, insert with `RETURNING`,
  counters, `final`), the application's scrub, cut and size, the lock timeout; its statement budget as an
  equality. In `tests/integration/claim-transition-efficiency/`: the lock-free test (a transaction holds
  `runs … FOR UPDATE` and an `UPDATE` of the leased `steps` row; a batch commits under `lock_timeout = 50ms`), the
  tenant-row barrier test (an ingest held after its inserts; a completion with output of the same tenant
  commits under `lock_timeout = 100ms`), and `work_budget.rs` unmodified, with their rows in
  `docs/engineering/architecture-principles.md`. Before T041-7 starts, a spike runs the transitions scenario with
  ten lines per step through the store directly and records SC-041-007's two figures; if they fail, the grouped
  commit becomes a task before T041-7.
  Refs: FR-041-002, FR-041-004, FR-041-006, FR-041-007, FR-041-008, SC-041-001, SC-041-007
  Owner-gate: OD-2026-11-13-14
- [ ] T041-7 The gRPC handler and `Class::Logs` with its permits and per-tenant sub-cap, minor 1.7 in
  `negotiate_protocol_details` and `protocol_ceiling` (6 kept for the database runner), the policy in
  `RegisterWorkerResponse`, `RETRY_LATER`, `SAGAT_STEP_LOGS_ENABLED` defaulting to false, the metrics and the
  span.
  Refs: FR-041-001, FR-041-004, FR-041-005, FR-041-014, FR-041-018
  Owner-gate: OD-2026-11-13-14
- [ ] T041-8 Server fault and security tests with barriers and held locks: crash before commit, crash after
  commit and before the answer, the same batch twice at once, a stale lease, an expired lease not yet reclaimed,
  a binding removed, a membership revoked, a database-runner token, a session of another subject or below 1.7,
  a batch after `final`, a batch for another tenant's run, `DROPPED` ranges that lie, two attempts of one run
  and two runs of one tenant at the run and tenant ceilings at once, a value redaction makes longer at the tenant
  boundary, a flood from a client that ignores every verdict, a restart of the API in the middle of an attempt.
  Refs: SC-041-002, SC-041-003, NFR-041-SEC
- [ ] T041-9 Measure: the transitions scenario with and without ten lines per step through the RPC, the p99 of a
  completion with output, the per-row trigger cost and the commit latency of a 500-record batch, the purge rate
  against the ingest rate, the added time of the final flush, the stored overhead per line; write the dated
  section of the validation report. If SC-041-007 fails, stop and add the grouped commit as a follow-up task
  before slice D.
  Refs: SC-041-007, NFR-041-PERF, NFR-041-COST

Slice C, the server serves:

- [ ] T041-10 Retention and deletion storage: the housekeeping pass with expiry computed at purge time, deleted
  streams first, the kept stream row with `purged_at`, the purge metrics; a test that a purge releases the quota
  bytes, skips a leased attempt, and applies a lowered retention to existing lines; `SAGAT_STEP_LOGS_ENABLED`
  defaults to true from this task on.
  Refs: FR-041-009, FR-041-018, FR-041-020
  Owner-gate: OD-2026-11-13-14
- [ ] T041-28 The classified-namespace flag: `namespaces.has_classified_schemas` set by publication from the
  classification walk it already runs, a backfill over existing versions, and a test that a version with one
  annotation sets it and one without does not.
  Refs: FR-041-021
  Owner-gate: OD-2026-11-13-18
- [ ] T041-11 `StepLogReader`, `GET /runs/{runId}/logs` and `/logs/streams` (paged): order and sealed cursor
  below the snapshot horizon computed in the same statement (with held-transaction tests on PostgreSQL 16 and
  18), filters with the run-scoped `stream`, `q`, the `[RunRead, RunLogsRead]` admission, the PII grant in a
  classified namespace, the throttled `run.logs_read` audit, `SAGAT_STEP_LOGS_READ_ENABLED`, the cross-tenant
  battery, the runtime-role run.
  Refs: FR-041-010, FR-041-011, FR-041-013, FR-041-002
  Owner-gate: OD-2026-11-13-14, OD-2026-11-13-18
- [ ] T041-12 `GET /runs/{runId}/logs/stream`: events, resume, reconnect, caps shared with the graph stream,
  authorization and grant on every poll, and the end rule on `xmax`; a test that a reconnect neither skips nor
  repeats, and one that holds a writing ingest transaction across the run's terminal transition and sees its
  lines before `end`.
  Refs: FR-041-012
  Owner-gate: OD-2026-11-13-14, OD-2026-11-13-18
- [ ] T041-13 The script-output marker in `record_execution`, with the generation of its `step.leased` event and
  the nullable instance path; the `script` source in the streams route; a barrier test of an ingest near the
  tenant limit and a `record_execution` of the same run that both commit.
  Refs: FR-041-016
  Owner-gate: OD-2026-11-13-14
- [ ] T041-29 `getScriptExecution` withholds `log` (`logWithheld: true`) from a caller without the step-log right
  of FR-041-013; the OpenAPI change and the digest moved after reading the diff; a viewer test; the release-note
  line.
  Refs: FR-041-022
  Owner-gate: OD-2026-11-13-18
- [ ] T041-30 `DELETE /runs/{runId}/logs[?stream=]` under `run:logs:delete`, with `run.logs_purged` in the same
  transaction, readers skipping deleted streams at once, `STOP_ATTEMPT` for a deleted stream, and the runbook's
  incident path.
  Refs: FR-041-023

Slice D, the SDKs (one pull request per SDK after the specification; they can be built in parallel):

- [ ] T041-14 `sdks/SPEC.md`: the logger's semantics, the capture section (per-attempt object, the process's
  live secret set, keys, numbers, exceptions, language context), §7.1 and §9 for 1.7; the five scenarios and
  their goldens; the harness step that reads the logs route and the fault hook for the RPC. Scenarios fail in
  all six for the right reason.
  Refs: FR-041-007, FR-041-015, SC-041-005
- [ ] T041-15 TypeScript: capture in the attempt's `ActivityLogger`, redaction by the process's live secrets,
  buffer, sender, `DROPPED` ranges, final flush.
  Refs: FR-041-005, FR-041-007, FR-041-015, SC-041-004
- [ ] T041-16 Go: capture in a per-attempt `slog.Handler`, attributes and groups as fields.
  Refs: FR-041-005, FR-041-007, FR-041-015, SC-041-004
- [ ] T041-17 Python: capture in `_TaskLog` from the raw message and arguments, `extra` as fields; no handler on
  the shared logger.
  Refs: FR-041-005, FR-041-007, FR-041-015, SC-041-004
- [ ] T041-18 C#: capture in a per-attempt `ILogger` decorator, scopes as fields.
  Refs: FR-041-005, FR-041-007, FR-041-015, SC-041-004
- [ ] T041-19 Java: capture in a per-attempt decorator of `ActivityLogger`.
  Refs: FR-041-005, FR-041-007, FR-041-015, SC-041-004
- [ ] T041-20 Rust: add `ActivityContext::log()` and capture; the four lock files move together.
  Refs: FR-041-005, FR-041-007, FR-041-015, SC-041-004
- [ ] T041-21 Run the five scenarios in the six SDKs on the merged tree and record the matrix; the flood
  scenario's counts add up in each.
  Refs: SC-041-002, SC-041-004, SC-041-005
  Owner-gate: OD-2026-11-13-14

Slice E, the Studio:

- [ ] T041-22 `LogView`, the Logs tab and the step section over the list and streams routes: filters, search,
  states (with the missing permission or grant named, "deleted", "log withheld"), markers, windowing, text-only
  rendering without links and with bidirectional and zero-width characters made visible; unit tests.
  Refs: FR-041-017, FR-041-019, FR-041-022
  Owner-gate: OD-2026-11-13-18
- [ ] T041-23 The tail (`followLogs`, pause and resume, announcements, polling fallback), with a test that a
  dropped connection neither skips nor repeats.
  Refs: FR-041-017, FR-041-012
- [ ] T041-24 Accessibility and journey: the axe run and reflow in `a11y.spec.ts`, a keyboard-only journey, and
  the Rust-driven browser journey for CUJ 1, CUJ 2 and CUJ 5.
  Refs: SC-041-006, NFR-041-A11Y

Slice F, operations and close:

- [ ] T041-25 Alerts with their runbook entries, `docs/runbooks/step-logs.md` (with what is and is not
  retroactive and the incident path), the reference documentation (generated files regenerated after reading
  the diff), `docs/traceability.md`, the quickstart demo of the happy and the failure scenario, and the release
  notes that state O041-001's default first and the withheld script log for viewers.
  Refs: FR-041-018, NFR-041-OPS, FR-041-022
  Owner-gate: OD-2026-11-13-14, OD-2026-11-13-18
- [ ] T041-26 End-to-end lineage and mixed versions on the Compose stack: retry, reset, rerun and
  continue-as-new each leave logs where FR-041-019 says; a 1.5 worker beside a 1.7 worker; an old API replica
  beside a new one; both switches.
  Refs: FR-041-014, FR-041-019, SC-041-003
- [ ] T041-27 Independent security and correctness review of the merged slices B to E, each finding proven by a
  failing test before it is fixed, and the drift report against this design.
  Refs: NFR-041-SEC, SC-041-004

#### Follow-up tasks

None yet.

### Verification Plan

Every pull request runs `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
the three checkers, the suites of what it touches and, for slices B and C, the same suites as the runtime role
(`SAGAT_TEST_AS_RUNTIME_ROLE=1`). The feature's integration suite is a new target of `sagat-tests`,
`integration_step_logs`, with real PostgreSQL; the tests that guard the hot path live in
`tests/integration/claim-transition-efficiency/`, which `scripts/select-test-targets.py` selects for every change
under `crates/infra-postgres/`. Each test below is written first and seen failing for the stated reason; races use
barriers or held locks, never sleeps.

| What is proven | How |
|---|---|
| The hot path is unchanged (SC-041-001) | `work_budget.rs` with its constants unmodified while the worker logs; a batch commits under `lock_timeout = 50ms` while another transaction holds the run row `FOR UPDATE` and has updated the leased step row |
| Completions do not queue behind ingest (SC-041-001) | An ingest transaction held after its inserts; a completion with output of the same tenant commits under `lock_timeout = 100ms`. Near the limit the bound is the 200 ms ingest lock timeout, measured |
| Crash between accept and store | A fault point before the commit: nothing stored, the resend stores all, once |
| Crash between store and answer | A fault point after the commit: the resend stores nothing and gets the same answer |
| Duplicate batch | The same batch twice at a barrier: the second waits on the stream row and inserts nothing; counters rise once |
| Lost batch and lying markers | The SDK gives up on a batch: the next `DROPPED` range covers it; a range that overlaps stored sequences, or claims 2^60 records, counts only unseen sequences and is clamped |
| A worker that floods (SC-041-002) | `step-logs-limits` for the SDKs; a client that ignores verdicts for the server: stored bytes stay at the ceilings, per-tenant permits hold, another tenant's completion latency is measured during the flood |
| Ceilings under concurrency | Two attempts of one run at the run ceiling, and two runs of one tenant at the tenant limit, at a barrier: the ceilings hold exactly; a value that redaction lengthens is charged at its stored size |
| Revocation and fence | Binding removed, membership revoked, lease expired but not reclaimed, run stopping, database-runner token, foreign session or session below 1.7: each answers `STOP_CAPTURE` and stores nothing |
| An old worker, an old server | `step-logs-negotiation`; an API replica without the RPC beside one with it |
| Redelivery does not duplicate lines | A task delivered again under a new generation writes a new stream; the same generation's resend inserts nothing; rebuilding the projections of a run changes no log row |
| Reset, rerun, continue-as-new, retry | T041-26 |
| Tail order and end | Transaction A stays open, B stores a line and commits: the line waits until A ends, then both arrive in cursor order; an ingest transaction held across the terminal transition is delivered before `end`; a reconnect at any point neither skips nor repeats |
| Lock order | An ingest near the tenant limit and a `record_execution` of the same run at a barrier both commit; no deadlock is reported |
| Redaction (SC-041-004) | `step-logs-redaction` with two concurrent attempts in six SDKs; server tests for each shape of both scrubbers and each sensitive field name; the canary is searched for in the stored rows, the routes' bodies, `run_events`, telemetry, `audit_records` and the captured process output |
| Tenant boundary | The cross-tenant battery on the RPC and the four routes: 404, `access.denied.cross_tenant`, no count or size disclosed; a cursor or `stream` of another run is refused |
| Permission, grant and audit | A viewer is refused on the read routes and gets the script execution without its log; a developer without the grant is refused in a classified namespace and allowed in another; the first read in a window writes `run.logs_read` before returning and a failure to write it returns no line; only admin may delete, and the delete is audited |
| Retention | A stream past expiry is purged in batches, quota bytes return, the stream row stays with `purged_at`, a leased attempt is skipped, a lowered retention reaches existing lines |
| Runtime privileges and storage classification | The privilege audit and manifest tests, unmodified, pass with the new statements and columns |
| Studio (SC-041-006) | Unit tests of the states and markers; the axe run; the keyboard journey; a line holding markup, a URL, escape sequences and bidirectional controls is shown as inert text |

What this plan does not prove is registered as gaps in `specs/041-step-logs/research.md`: G041-001 (a
transformed secret), G041-002 (the tail of an attempt that lost its lease or its network), G041-003 (script
lines are not live), G041-004 (feature 020 must delete streams with their run; G1618 there), G041-005 (a
deployment held below protocol 1.6 captures nothing), G041-006 (SC-041-007 is measured on one machine),
G041-007 (a script marker inside a collection may lack its iteration).

### Estimate

An estimate by reading, not a measurement. Server (domain, application, adapter, gRPC, HTTP, purge, delete,
flag, tests): about 5,500 to 6,500 lines of Rust, most of them tests, in twelve pull requests. SDKs: 500 to 800
lines each with their unit tests, plus the specification, five scenarios and the harness change, in seven pull
requests. Studio: about 1,300 to 1,600 lines with tests, in three. Contracts, ADRs, runbook and documents: four.
That is 30 tasks and roughly 26 to 28 pull requests. A pull request's CI takes up to two hours and they land one
at a time where they touch `research.md` or the contracts, so the calendar is set by the queue: about three and
a half to four working weeks for one session working alone, less with the six SDK tasks in parallel after
T041-14. The places most likely to move it are the spike of T041-6 (if the grouped commit is needed, add a week)
and the cursor and end rule on two PostgreSQL versions.
