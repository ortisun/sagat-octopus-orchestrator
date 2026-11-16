# Step logs per run

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-15

--------------------------------------------------------------------------------

## Overview

The lines a step's code writes to its logger are sent by the worker to the platform, stored per attempt with
ceilings and a retention period, and read through a REST route, a live tail and a Studio screen. The owner chose this
on 2026-11-13 (OD-2026-11-13-9: "step logs per run in 0.3.0 — a new feature: workers send the log lines of each step,
the platform keeps them with retention and limits, and there is a route and a Studio screen").

The design treats a step log as a bounded diagnostic record kept **beside** the journal. It is never execution state:
no transition reads it, a lost line never changes a step's outcome, and the transactions that lease and complete a step
run no statement on it. It must never be a second place where a secret or another tenant's data can be read. A log batch
can delay a completion with output of the same tenant only by being queued ahead of it on the tenant's usage row or on the
namespace's secrets lock, at most the tenant's log tickets on every replica (section 2). In time, the bound is that of
SC-041-007: the p99 of a completion that stores output stays within 10% of its value with capture off, which T041-31
measures before the schema is fixed and T041-9 measures again at the end.

The status stays `Draft` until the approval record of `docs/designs/constitution-4.0.0-egm.md` section 3 is written. The
owner's acceptance of the design for release 0.3.0 (OD-2026-11-15-17) decides scope; it is not the written acceptance of
review findings that the approval rule asks for.

The decisions, gaps and owner questions are registered in [specs/041-step-logs/research.md](../../specs/041-step-logs/research.md);
the contract this design freezes is [specs/041-step-logs/contracts.md](../../specs/041-step-logs/contracts.md) and the
schema detail is [specs/041-step-logs/data-model.md](../../specs/041-step-logs/data-model.md). The architecture decisions
are [ADR-0084](../adr/ADR-0084-step-logs-are-a-bounded-lossy-per-attempt-record-beside-the-journal.md) and
[ADR-0085](../adr/ADR-0085-step-log-text-is-read-under-its-own-permission-and-redacted-by-value-in-the-worker.md). How
each finding of the reviews of 2026-11-15 was answered is in
[resolution.md](041-step-logs.reviews/2026-11-15/resolution.md) of that review folder; the reports of 2026-11-13 carry their answers inline.

## The Problem

Today a person who wants to know what a step's code did has three sources, and none is the code's own words:

- `GET /runs/{runId}/events`, `/attempts` and `/telemetry` say what the engine decided (leased, completed, failed with
  a code) and how long it took. They are rebuilt from `run_events`; there is no attempts table.
- The platform's processes write JSON to stdout with `trace_id` and `run_id`. That is the platform's log. The worker's
  log stays on the worker's machine, which the person reading the run in the Studio usually cannot reach.
- `sdks/SPEC.md` §6.3 gives an activity a context with "a cancellation token, a logger and `secrets()`", and says
  nothing more about the logger. Five SDKs expose one and each writes only to the host process; the Rust SDK has none
  (section 6 lists each API with its file).

One kind of step already has a stored log. The script runner (`apps/sandbox-runner`) keeps a script's stderr, redacts
it, clips it and reports it; `script_executions.log` holds up to 1 MiB per attempt, counted against the tenant quota
`tenant_log_bytes` and purged after `script_retention_days`. It is read through
`GET /namespaces/{namespaceId}/script-executions/{executionId}` under `script:read`, which the viewer role holds
(`crates/api-http/src/routes/scripts.rs:426-448`, `ExecutionBody.log` in `dto_scripts.rs:582`).

Facts of the code that shape what can be built (read at `b1b2c249`):

1. **Workers hold secret values.** `ResolveSecrets` returns them once per attempt, and the server does not keep them
   afterwards. Only the worker process can recognise a secret value inside a line. Several attempts run in one worker
   process at once, each with its own secrets.
2. **Loggers are shared in some SDKs.** Python's activity logger is `_TaskLog`, a `LoggerAdapter` over the worker-wide
   logger (`sdks/python/sagat_octopus_orchestrator/worker/host.py:172-178`, built at `:712`). A handler attached to that
   logger sees every attempt's lines.
3. **The hot path has a budget that fails a pull request.** `tests/integration/claim-transition-efficiency/work_budget.rs`
   holds a lease to 38 statements, 2 transactions and 5 rows, a completion with a next step to 50 statements, 1
   transaction and 11 rows, and a completion that ends the run to 47, 1 and 9 (`work_budget.rs:223-225`), as equalities (constitution 4.4.0, CI gates). The engine locks the run row `FOR UPDATE`. A
   table with a foreign key to `runs` would make every insert take a `KEY SHARE` lock on that row.
4. **Retained bytes go through one row per tenant.** Every insert into a table with `log` or `content` columns fires the
   per-row trigger `sagat_quota_row_delta()` and queues one deferred constraint-trigger call of
   `sagat_quota_flush_deltas()` per row (`20261024000000_durable_quotas.up.sql:446-558`). At commit the first call takes
   the content and log policy fences and `tenant_retained_usage … FOR UPDATE` (`:348`) and the other calls run while it is
   held. A completion that stores output takes that row in its own commit. `content_usage::take_log_prefix` takes the
   same row early and cuts text to what is left under the limit; dispatch, connector and script errors are written
   through it (`dispatch.rs:219`, `connectors.rs:626`, `scripts.rs:1161-1855`).
5. **A report is fenced by more than the lease token.** `ensure_bound` (`crates/application/src/worker.rs:2715`) checks
   the activity binding (`bound_patterns` under `hold_worker_binding`, `first_uncovered`) and the current membership
   (`current_worker_authorized`, `crates/infra-postgres/src/execution.rs:1017`: the namespace secrets lock shared, then
   the membership row `FOR SHARE`, refusing the database-runner role). A `steps` row holds a lease when its status is
   `LEASED` or `RUNNING` and the lease ends at `lease_expires_at`. A worker session is a `worker_sessions` row matched by
   tenant, namespace, id, token hash and subject, unexpired (`worker_session_room`, `execution.rs:671`); registering again
   under the same name keeps the row and its id and replaces the token hash (`workers.rs:328-344`).
6. **Nothing deletes a run.** `run_events` and `payloads` are append-only by trigger, and feature 020 (retention and
   archive, ADR-0054) is a draft. Logs need a retention of their own.

## Requirements

### Functional

- **FR-041-001 Capture.** When the worker session was accepted at protocol minor 1.7 or later and the policy the server
  returned at registration says capture is on (FR-041-020), every line an activity writes through the SDK's activity
  logger at or above the capture level MUST be sent to the platform. The logger MUST keep writing to the host's own
  logging as it does today. Capture MUST happen in the per-attempt logger object, MUST NOT depend on the host logger's
  own enabled check, and MUST NOT install a handler, sink, appender or `tracing` layer on anything other attempts share.
- **FR-041-002 Identity and order.** A stored line is identified by tenant, run, step, attempt, generation and a
  sequence, and shows its node and instance path. The worker numbers its records from 1, one per logger call that passes
  the capture level, whether the record is later sent or dropped; the server's markers take sequences from 2^62 up.
  A line carries the worker's time and the server's time of storage. Within an attempt the records are in sequence
  order. Across attempts the order is the order of the storing transactions' ids, then the sequence (section 5); the
  worker's time is shown and never used to order.
- **FR-041-003 Levels and fields.** A line has one of five levels (`trace`, `debug`, `info`, `warn`, `error`), a text
  message, an optional exception text and an optional flat map of fields whose values are strings, 64-bit integers,
  finite doubles or booleans. Each SDK maps its language's levels onto the five by the table of section 6. The capture
  level is `info` unless the worker's option says otherwise. A NUL character becomes U+FFFD; a double that is not finite
  is stored as the string `NaN`, `Infinity` or `-Infinity`.
- **FR-041-004 Transport.** Lines travel in batches through a new unary RPC, `AppendStepLogs`, added in protocol minor
  1.7 (contracts.md §1). A batch names one attempt with the identity and fence a heartbeat carries plus the worker's
  session id and current session token, and holds at most 500 records and 192 KiB. The server MUST answer only after the
  lines are committed and MUST store a record at most once however many times its batch arrives (identity: attempt,
  generation, sequence). After the final batch of an attempt is recorded, every later batch for it MUST store nothing
  and be answered `STOP_ATTEMPT` with the reason `FINAL_RECORDED`, which a resent final batch takes as its success.
- **FR-041-005 Logging never decides the step.** A call to the logger MUST NOT block on the network or on a full buffer,
  and MUST NOT throw into the activity. No failure of capture (refused batch, timeout, server down, ceiling reached,
  invalid line, bug in the capture path) may change the outcome, the output, the retry or the timing of a step beyond the
  bounded final flush of FR-041-015.
- **FR-041-006 Ceilings and drop counts.** These ceilings hold, enforced by the server from counters whatever the worker
  does: 8 KiB per message and per exception text, and 2 KiB and 32 keys per field map, measured after redaction; the
  attempt and run ceilings of FR-041-020 (defaults 10,000 records and 1 MiB an attempt, 100,000 records and 16 MiB a
  run, the bytes being those of the stored text); the tenant's step-log quota, `tenant_step_log_bytes`, a resource of its own (OD-2026-11-15-5). A message or exception over its ceiling is cut at a character boundary and the
  line is flagged `truncated`. Records past a ceiling are not stored and are counted on the attempt's stream. When the
  attempt ceiling is first met the server stores one marker line, which holds no text; the run ceiling and the tenant
  quota are recorded only on the stream row. Every sequence the worker numbered and the server never stored is counted
  once, as dropped by the worker (a gap in the sequences it sent) or by the server (a ceiling, including the records the
  worker stopped sending after it). A step MUST NOT fail, wait or retry because a ceiling was met.
- **FR-041-007 Redaction.** Redaction is the SDK's for values and the server's for shapes.
  - (a) Before a line leaves the worker process, the SDK MUST replace each secret value held in the process's secret set
    (section 6) by the rule of `sagat_domain::scripts::redact` (values of at least 6 bytes, longest first, the
    placeholder `[REDACTED]`), in the message, in every field key, in every string field value, in the canonical
    decimal text of every number field (data-model.md §5; a number whose text holds a secret becomes the string
    `[REDACTED]`), and in the exception text and stack it formats. The algorithm is the SDK's; the result MUST equal
    `redact` on the conformance goldens.
  - (b) Context the language attaches to a line (section 6) is captured only as fields and is redacted as fields.
  - (c) The server MUST apply, in the application layer and before any size is measured, the credential shapes of
    `sagat_domain::scripts` and of `sagat_telemetry::redact` through public entry points that take the placeholder and do
    not cut (section 2), and MUST replace with the string `[REDACTED]` the value of any type of a field whose key
    `sagat_telemetry::redact::is_sensitive_name` matches.
  - (d) The content of a line MUST NOT appear in the journal, a projection, telemetry, an audit record, a metric, a span,
    an error body or the platform's own logs, and the `AppendStepLogs` request MUST NOT be printed by any log, `Debug`
    or trace of the server or the SDKs.
  - What this cannot catch is G041-001: a value the code transformed, a value shorter than 6 bytes, a value obtained
    outside `ResolveSecrets`, and anything a client that does not use an official SDK sends.
- **FR-041-008 Storage beside the journal.** Lines are stored in PostgreSQL in tables of their own. Storing a batch MUST
  NOT take, or wait for, a lock on a row of `runs`, `steps` or `run_events`, and leasing, heartbeat and completion MUST
  NOT read or write the log tables. A batch MUST NOT take the tenant's `tenant_retained_usage` row before its commit.
  The batches of one run wait for each other on the run's usage row, which each holds from its allowance step to its
  commit (section 2; measured by T041-31, G041-009). The ceilings of an attempt and a run count text bytes (the UTF-8
  octets of message, exception and fields); the bytes charged to the tenant, each row's text plus a fixed 128 bytes
  (OD-2026-11-15-6), are counted in a storage class of their own, `step_log` (ADR-0027 Amendment of 2026-11-15), and released when the rows are
  deleted.
- **FR-041-009 Retention.** An attempt's lines are deleted once its stream expires: the server's time of its last
  recorded batch plus the `step_log_retention_days` the stream took from its namespace when it was created (FR-041-020).
  As ADR-0027 requires, lowering the retention does not shorten what is stored; the delete route (FR-041-023) is the way
  to remove lines early. The stream row, which says the attempt had a log and holds its counters and the time its lines
  were removed, is kept, so that a reader is told "expired" and not shown an empty list. Once removal has begun, the
  stream's lines are no longer returned and a later batch for it stores nothing.
- **FR-041-010 Read.** `GET /runs/{runId}/logs` returns lines in stored order with `cursor` and `limit` (default 50, at
  most 200, clamped, as every list), and only lines below the read horizon: the oldest transaction still in progress when
  the reading statement runs (section 5). Filters: `stream` (the public id of one stream of this run, an attempt log or a
  script execution's marker; another is answered as missing), `node`, `attempt`, `minLevel`, `from` and `to` (storage time), and `q` (a literal, case-insensitive substring
  of the message, at most 128 characters). The cursor names the run it was minted for and is refused for another. A page
  says whether lines are held back by the horizon (`heldBack`) and whether the run's log is complete (`complete`).
  Every item carries the identity of FR-041-002, both times, level, message, exception, fields, `truncated` and, for a
  marker, its reason (contracts.md §2).
- **FR-041-011 Streams.** `GET /runs/{runId}/logs/streams` lists, with `cursor` and `limit`, the attempt logs of the run
  (`source = worker`: node, instance path, attempt, state, records and bytes stored, dropped by the worker and by the
  server, the ceiling met, first and last time, expiry or removal time) and, with `source = script`, the run's script
  executions (FR-041-016). The counters are read at the statement's snapshot and can be ahead of the lines a read below
  the horizon of FR-041-010 returns.
- **FR-041-012 Live tail.** `GET /runs/{runId}/logs/stream` is a server-sent event stream of new lines with the filters
  of FR-041-010, resumable with `Last-Event-ID`, with the `reconnect` and ping behaviour and the per-subject and
  per-tenant caps of `/runs/{runId}/stream`, shared with it. It never skips or repeats a line across a reconnect. It ends
  with `end` only when the run is finished and every line that can still be committed for the run has been delivered,
  by the rule of D1958 (a decision of feature 018: a transaction id taken after the end is seen, and an end declared only
  once every older transaction has ended; section 5). The reader's right is checked when the tail opens and again every 30 seconds
  (`StreamLimits::recheck`); a reader who loses it is cut within that time.
- **FR-041-013 Who may read.** The three read routes need `run:read` and a new permission, `run:logs:read`, held by the
  admin, developer and operator roles and not by viewer, worker or runner. In a classified namespace (FR-041-021) the
  reader MUST also hold the namespace's PII reveal grant, checked as `has_effective_payload_reveal_grant` checks it
  (OD-2026-11-13-18). Another tenant's run is answered 404 and recorded as `access.denied.cross_tenant`, as every run
  route. A read is audited as `run.logs_read` at most once per subject and run in 300 seconds whatever its filters; the
  record of a window commits before the first line of that window is returned, and a failure to write it returns no line.
- **FR-041-014 Mixed versions.** A worker accepted below 1.7 sends nothing and behaves exactly as today. A 1.7 SDK
  against a server that accepts a lower minor, or whose policy says capture is off, sends nothing and logs locally. An SDK
  that gets `UNIMPLEMENTED` for `AppendStepLogs` (an older API replica) stops capture for that attempt only. No published
  workflow, run in progress or existing RPC changes meaning.
- **FR-041-015 The six SDKs.** Each SDK keeps its present logger type, adds the calls of section 6 that it lacks, and
  adds capture behind it: a bounded buffer per attempt (1,000 records or 256 KiB, whichever is met first) and per process
  (16 MiB across attempts), a flush every second or at 256 records, one batch in flight per attempt, the answers of
  contracts.md §4, and a final batch before `CompleteStep` or `FailStep` that waits at most 2 seconds. These numbers are
  constants of `sdks/SPEC.md`, not values the server sends. The Rust SDK gains the logger its `ActivityContext` lacks.
- **FR-041-016 Steps the platform runs.** A script step's log stays where feature 005 stores it and is never copied.
  When the script runner's report is recorded, the platform also stores one marker line, with no text, that points to
  the script execution, so that the whole-run Logs tab shows, in stored order, that the step produced output and opens
  it under FR-041-022 (OD-2026-11-15-9; section 7 says when the marker is written and when it is not). The marker is keyed
  on the script execution, of which there is one per (run, node, attempt): it shows the node and the attempt, and no
  iteration (G041-007). The script runner's report does not change. The streams route also lists the run's script
  executions as `source = script` items with their own expiry. The database runner and the lake runner capture no log.
- **FR-041-017 Studio.** The run screen gains a Logs tab for the whole run and a Logs section for the selected step and
  attempt, with filters, search, a live tail that pauses when the reader scrolls up, markers (the attempt ceiling's and the script output's) shown as rows, and the states
  "not allowed" (naming the permission or the grant that is missing), "no log", "expired", "deleted", "ended without its
  final flush" (the attempt's lease is gone and no final batch was stored), "some lines are delayed" (`heldBack`) and
  "log withheld" for a script execution, with full keyboard operation.
- **FR-041-018 Observability.** The platform counts records stored, dropped (by reason), truncated and redacted, batches
  by outcome, ingest duration, lock-timeout retries, purged records, open tails and the age of the database's oldest open
  transaction, with labels that are closed enumerations only (data-model.md §7). It alerts on sustained shedding, on purge
  backlog and on an old open transaction, not on ceilings a customer's code meets.
- **FR-041-019 Lineage.** A log belongs to the run and attempt that wrote it. A retry is a new attempt and a new log. A
  reset, a rerun and a continue-as-new open a new run; the new run starts with no log and the earlier run keeps its own,
  reachable through the links the Studio already shows between those runs. Lines are never copied.
- **FR-041-020 Policy.** Capture in a namespace, the attempt and run ceilings and the retention are five quota resources
  at namespace scope (data-model.md §6); `step_log_attempt_records = 0` turns capture off in the namespace. Capture
  happens only when all of these hold: the deployment switch `SAGAT_STEP_LOGS_ENABLED` is on, the namespace's
  `step_log_attempt_records` is above 0, the worker's own option is on, and the session was accepted at 1.7 or later.
  The server reads the first two at registration (the policy's `enabled`) and again in every batch; a change of a
  ceiling applies to the batches stored after it, and a change of the retention to the streams created after it. The
  execution engine and the control plane both read `SAGAT_STEP_LOGS_ENABLED`, as both read `SAGAT_SCHEDULES_ENABLED`
  today: the engine for capture, the control plane for the script marker (section 7).
  Changing a resource is audited by the quota catalogue as every quota change is.
- **FR-041-021 Classified namespace.** A namespace is classified when a workflow version published in it carries at
  least one `x-sagat-classification` annotation in a schema (OD-2026-11-13-18). The fact is recorded as a row of an
  insert-only table that nothing clears, by a database trigger on every insert into `workflow_versions`, whatever binary
  inserts it, and by the migration for the versions that exist (section 3). The test is a superset of the annotation:
  a document whose canonical JSON holds the text `"x-sagat-classification"` anywhere counts, so it errs toward
  classified (G041-010).
- **FR-041-022 Script-execution logs.** The `log` of `GET /namespaces/{namespaceId}/script-executions/{executionId}`, for
  an execution of a run, is returned only to a caller who may read that run's step logs under FR-041-013. Anyone else who
  may read the execution gets `log` empty, `logTruncated` false and `logWithheld: true`, and the listing shows
  `logTruncated` false to them (OD-2026-11-13-18), whatever `SAGAT_STEP_LOGS_READ_ENABLED` says. Viewers lose the log
  they read today; the release notes say so.
- **FR-041-023 Deletion.** `DELETE /runs/{runId}/logs`, optionally with `stream`, needs `run:logs:delete`, held by the
  admin role only. It makes the lines committed when it runs unreadable at once (a batch or a marker that commits after it
  is stored, and a second delete removes it), records `run.logs_deleted` (run, number of streams, no text) in
  the same transaction, answers `204` with no body, and leaves the removal of the rows to the purge. It exists for the incident in which a secret or
  personal data was logged. No route removes a script execution's log before its retention (G041-013); the incident
  runbook says so.

### Success criteria

- **SC-041-001 The hot path.** With the migration applied and the ingest running between the measuring windows,
  `work_budget.rs` passes with its constants unmodified. A batch for a run commits, under `lock_timeout = 50ms`, while
  another transaction holds that run's row `FOR UPDATE` and has updated the leased `steps` row. What a completion with
  output can wait for is shown by the queue, not by a time. One ingest transaction of tenant T runs its quota flush
  (`SET CONSTRAINTS ALL IMMEDIATE`) and is held, so it holds the tenant usage row; K − 1 more ingests of T queue behind it
  on that row (K = the tenant's tickets on one replica); then a completion with output of another run of T queues
  after them. The ingests are released one at a time. At every stage the completion is blocked only by those ingests
  (`pg_blocking_pids`), and it commits after the last of them. The same holds with a `lock_secrets` waiter queued on the
  namespace's secrets lock. The bound in time is SC-041-007's completion p99.
- **SC-041-002 A flood.** An activity that writes 1,000,000 lines as fast as it can completes with the same outcome and
  output as one that writes none; the store holds at most the attempt ceiling plus its marker; and for the stream,
  lines stored (the marker excluded) + dropped by the worker + dropped by the server = the highest sequence the logger
  assigned (for highest sequences below 2^53, where the drop counts saturate).
- **SC-041-003 Faults.** In the fault tests of the Verification Plan no record is stored twice, and every record of an
  attempt that has a stream and is missing is covered by a counter or by the "ended without its final flush" state. A
  stream whose removal began while its lease lives answers its next batch `STOP_ATTEMPT` (`REMOVED`) and stores
  nothing, and the reader sees it as expired or deleted. An attempt whose capture stopped before its first batch was
  stored has no stream (G041-014).
- **SC-041-004 Redaction.** A canary secret resolved by an attempt and written to the logger whole, inside a longer
  message, as a field key, as a field value, as a number and inside an exception, by that attempt and by another attempt
  of the same worker process (at the same time and after the first ended), appears nowhere in the stored lines, the
  routes' bodies, the journal, telemetry, audit records or the platform's captured output, in all six SDKs.
- **SC-041-005 Conformance.** The five scenarios of section 6 pass in TypeScript, Go, Python, C#, Java and Rust from
  the same scenario files and goldens.
- **SC-041-006 Accessibility.** The Logs tab and section pass the axe run at WCAG 2.2 AA of
  `frontend/e2e/tests/a11y.spec.ts`, reflow at 320 px, and a journey done with the keyboard only.
- **SC-041-007 Throughput.** On one machine, the same day, the transitions scenario of `tests/performance/scalability-ha`
  with every step writing ten `info` lines reaches at least 90% of its rate with capture off, in the rows "1 API replica,
  16 workers" and "8 replicas, 128 workers", and the p99 of a completion that stores output is within 10% of its value
  with capture off. The machine, its load and the PostgreSQL configuration are stated; the figures are criteria to
  verify, not promises.

### User stories

- **US01** A developer opens a failed run in the Studio, selects the failed step and reads what its code logged in the
  attempt that failed and in the ones before it (CUJ 1, CUJ 6).
- **US02** An operator watches a long step of a running workflow and sees its lines arrive (CUJ 2).
- **US03** A tenant administrator bounds what logs cost (bytes, days), turns capture off for a namespace whose code logs
  personal data, and deletes a run's log when a secret was logged (CUJ 3, CUJ 7, CUJ 8).
- **US04** An SDK user changes nothing in their activity code and gets capture after upgrading the SDK (CUJ 4).

## Goals

- What a step's code logged is readable from the run, by the people allowed to, during the run and for a stated number
  of days after (FR-041-010 to FR-041-013, SC-041-005).
- Capture adds no statement to a lease or a completion. A completion with output waits on log ingest only for the
  ingests queued ahead of it on the tenant's usage row and the namespace's secrets lock (section 2), and its p99 stays
  within 10% of its value with capture off (SC-041-001, SC-041-007, T041-31).
- Every step that logged waits, before it reports, for its final flush, at most 2 seconds (FR-041-015). That flush
  meets `RETRY_LATER` when the tenant's tickets are taken; no capacity is reserved for it, and T041-31 measures how often
  it happens and what it adds to the completion.
- The Studio shows a run's log and each step's log, live and after the run, to keyboard and screen-reader users too
  (FR-041-017, SC-041-006).
- Every line of an attempt log that is not there is accounted for by a count and a reason the reader can see
  (SC-041-002, SC-041-003; G041-014 for an attempt whose capture stopped before it had a stream).
- A secret value the platform handed to an attempt of a worker process, written verbatim and at least 6 bytes long, is
  not stored by an official SDK (SC-041-004; the limits are G041-001).

## Non-Goals

- **Not the platform's own logs.** The stdout JSON of the processes, their spans and the Collector pipeline belong to
  features 008 and 015. This feature does not export step logs to OpenTelemetry.
- **Not stdout or stderr of the worker process.** Only lines written through the SDK's activity logger are captured. A
  `print` in an activity is not, and lines written by a thread that outlives its activity are not.
- **Not a log search product.** No index on the text, no query across runs, no aggregation, no alerting on content. `q`
  is a substring match inside one run.
- **Not execution state.** A workflow cannot read a step's log, an expression cannot test it, and it is not an input to a
  compensation or a review.
- **Not an archive.** Legal hold, export to a customer's storage and retention beyond 365 days belong to feature 020.
- **No change to how script logs are written or kept.** `script_executions`, its ceiling and its retention stay as
  feature 005 built them; only who may read the `log` field changes (FR-041-022). Live lines from a running script are a
  follow-up (G041-003).
- **No classification of log content by field.** A log line has no schema. The classified-namespace rule of FR-041-021
  decides who may read, not what is shown.
- **No download or export button** in the first delivery.

## Open Questions

Technical, each closed by the task named:

- **Does a completion's p99 stay within 10% of its value with capture off (SC-041-007)?** T041-31 measures it on a
  scratch schema before T041-5 starts, with the workload of that task; a failure stops the feature for a design change
  and a new review (one contingency, one insertion point).
- **Does the read horizon and its end rule hold on PostgreSQL 16 and 18?** T041-11 and T041-12, with held transactions
  on both, each in a disposable container of its own (the horizon is cluster-wide), the case of a held ingest whose id
  is the largest below the end horizon included.

## Owner decisions needed

Open, and asked only on a result of the spike T041-31 (each registered in research.md; the recommendation is the first
option):

- **O041-020 An approximate run ceiling** (asked only if the fan-out case of T041-31 fails). (a) Keep the exact run
  ceiling, so that the batches of one run wait for each other on `run_log_usage`. (b) An approximate ceiling: each batch
  sums the counters of the run's streams without a lock, so a run can pass its ceiling by the batches in flight.
  Recommended: (a) unless the fan-out fails, then (b). Trade-off: (a) a wide `forEach` of logging iterations meets
  `RETRY_LATER` and drops; (b) the run ceiling is a target, not a bound.
- **O041-021 Chunk rows** (asked only if T041-31 shows the per-line rows fail SC-041-007 and chunk rows pass). (a) Keep
  one row per line (O041-004). (b) One row per batch, with the reads, filters and search done in the application.
  Recommended: whichever T041-31 shows passes; (a) if both do.

Answered, recorded in `docs/owner-decisions.md` and in research.md:

- **O041-001 to O041-011** were answered as recommended on 2026-11-13 ("Aceitar todas", "accept them all",
  OD-2026-11-13-14). Each rejected option and its reason:
  - O041-001: capture on by default at `info`, a namespace switch and a worker option. Rejected: off until a namespace
    asks (no one would find it), on for new namespaces only (two behaviours to explain).
  - O041-002: `run:logs:read` for admin, developer and operator. Rejected: viewers too (logs can hold personal data); a
    per-namespace grant only (a grant to manage for every reader everywhere).
  - O041-003: one `run.logs_read` per subject and run per 300 seconds. Rejected: one per request (a tail would write one
    every poll); none (principle VI).
  - O041-004: PostgreSQL, one row per line. Rejected for 0.3.0: chunk rows (filters and identity move into Rust; the next
    step if T041-31 asks, O041-021); the blob store (committed blobs are never purged today, G69 of ADR-0023); a
    partitioned table (does not release quota bytes).
  - O041-005: the ceilings of FR-041-006, `tenant_log_bytes` shared with script logs. Rejected: a separate tenant quota
    (reopened by O041-015).
  - O041-006: `step_log_retention_days`, default 14, 1 to 365. Rejected: 7 (too short for a weekly look), 30 (twice the
    cost), `script_retention_days` (365 by default).
  - O041-007: at the attempt ceiling the first lines are kept, then a marker and a count. Rejected: a ring (the start of
    an attempt explains its failure more often, and a ring rewrites stored rows).
  - O041-008: a new server-sent event route. Rejected: polling only (a second of latency at best, a request per second
    per reader).
  - O041-009: a script step shows where its output is. Rejected: live script lines (G041-003); leaving script steps out.
  - O041-010: server, six SDKs, routes and Studio together in 0.3.0. Rejected: SDKs one by one.
  - O041-011: the limit of redaction (G041-001) is accepted as stated in ADR-0085.
- **O041-012 and O041-013** were answered on 2026-11-13 (OD-2026-11-13-18): in a classified namespace reading step logs
  also needs the namespace's PII reveal grant ("Exigir a concessão PII", "require the PII grant"); and the `log` of a
  run's script execution is read under the same rule ("Fechar ao viewer", "close it to viewers").
- **ADR-0084 and ADR-0085** were accepted on 2026-11-13 (OD-2026-11-13-17) in the text of commit `35b752b3`.
- **O041-014 to O041-019, O041-022 and O041-023** were answered on 2026-11-15 (OD-2026-11-15-4 to -11):
  - O041-014, "Aceitar as revisões" ("accept the revisions"): both revisions of ADR-0084 and ADR-0085 are accepted in
    place. Rejected: superseding ADRs.
  - O041-015, "Quota separada" ("a separate quota"), against the recommendation: step logs get their own tenant quota,
    `tenant_step_log_bytes` (D041-053; ADR-0027 Amendment, O041-024). Rejected: stopping at 90% of `tenant_log_bytes`;
    sharing it as O041-005 had decided.
  - O041-016, "Cobrar 128 bytes/linha" ("charge 128 bytes per line"): every row is charged 128 bytes on top of its text
    (D041-054). Rejected: a written acceptance of uncharged rows; chunk rows.
  - O041-017, "Toda a vida do processo" ("the whole life of the process"): the secret set lives for the process,
    bounded, and capture stops when it is full (D041-055). Rejected: keeping a value only while an attempt holds it.
  - O041-018, "041, com registo" ("041, with a register"): 041 holds minor 1.7 and `sdks/SPEC.md` §9 has a register of
    minors (D041-056). Rejected: an allocation of all five drafts now; merge order with no register.
  - O041-019, "Sim, com marcador" ("yes, with a marker"), against the recommendation: a script step's output is a marker
    row of the whole-run tab, written when the runner's report is recorded (D041-057, section 7). Rejected: the streams
    list and the step section only.
  - O041-022, "Emendar o gate de CI" ("amend the CI gate"): constitution 4.5.0 runs the step-log suites when step-log
    code changes (D041-058, T041-32). Rejected: no rule.
  - O041-023, "Aceitar" ("accept"): the cursor that shows the storing transaction's id is accepted (G041-008).
    Rejected: an encrypted cursor.
- **O041-024**, "Emenda no ADR-0027" ("an amendment in ADR-0027"): the Amendment of 2026-11-15 stays a section of ADR-0027
  and is accepted, an exception to `docs/adr/README.md` rule 2 (OD-2026-11-15-16). Rejected: a new ADR refining ADR-0027.
- **The design**, "Aceitar e construir" ("accept and build"): accepted with constitution 4.5.0 for release 0.3.0
  (OD-2026-11-15-17).

## Critical User Journeys (CUJs)

### CUJ 1: Why did this step fail? (US01)

*   **Given**: a run whose step `charge` failed on its third attempt; the activity logs at `info` and `error` through the
    SDK's logger; the reader holds `run:logs:read`, and the namespace is not classified.
*   **When**: they open the run in the Studio, select `charge` and open Logs.
*   **Then**: they see the lines of attempt 3 and can switch to attempts 1 and 2; each line shows its level as text and
    shape, its worker time, its fields and its exception; a secret the activity resolved and logged verbatim shows as
    `[REDACTED]`.

### CUJ 2: Watching a long step (US02)

*   **Given**: a running step that logs a line every few seconds.
*   **When**: the reader opens the Logs tab with the tail on.
*   **Then**: lines appear within the worker's flush interval (1 s) plus the tail's poll interval (0.5 s); scrolling
    up pauses the tail and a "Jump to latest" control resumes it; when the connection drops and returns, no line is
    missing or doubled; when the run ends, the tail ends after the last line that could still arrive.

### CUJ 3: A step that floods (US03)

*   **Given**: an activity with a loop that logs 1,000,000 lines.
*   **When**: it runs.
*   **Then**: the step completes as it would without logging; the log holds the first 10,000 lines and one marker
    "attempt ceiling reached"; the streams list says how many lines the worker dropped (its buffer was full) and how many
    the server dropped (the ceiling), and the three numbers add up to what the code logged. No alert fires: it is the
    customer's code, shown on the run.

### CUJ 4: A worker that was not upgraded (US04)

*   **Given**: a worker on an SDK that offers protocol 1.5.
*   **When**: it runs steps on a server that supports 1.7.
*   **Then**: nothing changes for it; its steps show "no log" in the Studio (the streams route lists no stream for them).

### CUJ 5: A reader without the permission or the grant (US01)

*   **Given**: a viewer, who holds `run:read` and not `run:logs:read`; and a developer without the PII reveal grant of a
    classified namespace.
*   **When**: each opens a run.
*   **Then**: for the viewer the Logs tab says `run:logs:read` is missing and makes no request (the Studio reads the
    session's permissions); for the developer the first request is refused `403 FORBIDDEN` with `PII_REVEAL_NOT_GRANTED`
    and the tab names the grant. Neither learns whether a log exists or how large it is. The viewer's script-execution
    view shows "log withheld".

### CUJ 6: A collection's iterations (US01)

*   **Given**: a `forEach` over 200 items whose body step logs.
*   **When**: the reader opens the whole-run Logs tab and filters by that node, then picks the iteration 17 attempt log
    from the streams list.
*   **Then**: the tab shows lines of all iterations in stored order, each labelled with its iteration; picking one stream
    narrows to that attempt log.

### CUJ 7: A secret was logged (US03)

*   **Given**: a step's code that base64-encoded a secret into a log line (G041-001).
*   **When**: an administrator learns of it.
*   **Then**: `DELETE /runs/{runId}/logs?stream=…` answers `204`, makes the lines unreadable at once and is audited as
    `run.logs_deleted`; the purge removes the rows; the runbook says to rotate the secret, and that a script execution's
    log holding it stays until its own retention (G041-013).

### CUJ 8: Bounding the cost (US03)

*   **Given**: a tenant administrator whose namespace `batch` logs a lot and namespace `hr` logs personal data.
*   **When**: they set `step_log_retention_days` to 3 for `batch` and `step_log_attempt_records` to 0 for `hr`.
*   **Then**: attempt logs of `batch` created from then on expire 3 days after their last batch, and those already stored
    keep the retention they were created with; workers of `hr` registering after the change get
    `enabled = false`, and a batch already in flight there is answered `STOP_CAPTURE` with `CAPTURE_OFF`; both changes are
    in the quota audit.

## Technical Plan (Architecture)

**Terms.** *Attempt* and *generation*: a step's attempt number counts retries; the generation is the fencing number a
lease carries (`steps.generation`), which a reclaim raises, so two holders of one attempt never share it. *Instance
path*: the ordered (scope node, iteration index) pairs that place a step inside collections and loops
(`ExecutionInstanceIdentity`, `worker.proto:90-93`); a root step has an empty path. *Stream*: a row of
`step_log_streams` and the lines that reference it. A stream of `source = worker` is an *attempt log*, the lines of one
(step, attempt, generation); a stream of `source = script` holds the one marker of one script execution, and its public
id is that execution's id. The `stream` parameter of the routes takes a stream's public id. A *tail* is a reader's open
server-sent event connection, never a stream. *Storage classification*: the class each column has in `storage-classification.json` (`log`, `content`,
`protected_system`), which decides what the quota triggers charge; it is unrelated to the *data classification* of
payload fields (`x-sagat-classification`, ADR-0033), which decides what a reader may see. *Store proofs*: the methods of
`WorkerStore` that say a deployment can serve a protocol minor (`supports_scoped_worker_protocol`,
`supports_database_worker_protocol`). *Namespace secrets lock*: the advisory lock `quota:secrets:{tenant}:{namespace}`
(`crates/infra-postgres/src/quotas.rs:307`), taken shared by every report and exclusively by secret and database-catalog
changes. *Policy fences* (content, log and, with ADR-0027's Amendment of 2026-11-15, step-log): the advisory locks the
quota finalizer takes before the tenant usage row, so that a change of a tenant's limits waits for writes in flight.

```text
activity code
   | logger call (unchanged API plus the calls of section 6; never blocks, never throws)
   v
SDK capture in the per-attempt logger: level -> sequence -> format -> redact with the process's secret set
   -> cut -> bounded buffer (attempt and process)
   | AppendStepLogs (unary, 1.7): attempt identity + fence + session id and token, <=500 records, <=192 KiB
   v
api-grpc: Class::Logs admission -> per-tenant ticket -> application::step_logs (validate, scrub, cut, size, plan)
   | one transaction, separate from lease/heartbeat/complete, lock_timeout 200 ms:
   |   xid -> lease read (no lock) -> session and namespace policy -> stream row -> plan (gaps, duplicates)
   |   -> run usage row -> tenant usage read (no lock) -> binding, secrets lock, membership -> insert -> counters
   |   -> commit (ADR-0027 finalizer: fences, tenant row)
   v
PostgreSQL: step_log_streams, step_log_lines, run_log_usage, classified_namespaces   (no foreign key to runs or steps)
   ^ purge pass (scheduler)        ^ GET /runs/{runId}/logs, /logs/streams, /logs/stream; DELETE /runs/{runId}/logs
                                   |    (script_executions read for source = script; record_execution writes the script marker)
                                   v
                                Studio: Logs tab and step section
```

### 1. Wire: protocol minor 1.7

`specs/001-platform-foundation/contracts/worker.proto` (the canonical copy; the six SDK copies are byte-identical) gains
the rpc, messages and enums of contracts.md §1, additively. The verdicts are: `CONTINUE`; `FINAL_ONLY` (a ceiling was
met: send no more records, but still send the empty final batch, so that the server learns the highest sequence and
records the final flush); `STOP_ATTEMPT` with a reason (`FINAL_RECORDED`, `LEASE`, `AUTHORITY`, `REMOVED`);
`STOP_CAPTURE` with a reason (`CAPTURE_OFF`, `SESSION`); `RETRY_LATER` with `retry_after_ms`. A record has no kind: a
gap in the sequences is the server's evidence of a drop, and the final batch carries `highest_sequence` so that drops at
the end are counted too.

`RegisterWorkerResponse` gains `StepLogPolicy step_logs = 4` with one field, `enabled`, computed at registration from the
deployment switch and the namespace's `step_log_attempt_records` (a session belongs to one namespace,
`RegisterWorkerRequest.namespace`).

`crates/api-grpc/build.rs` generates the server without default method stubs (`:31-34`), so the new rpc must be answered
by every implementation of the generated trait. In T041-3 these answer `UNIMPLEMENTED`: `WorkerGrpc` in
`crates/api-grpc/src/service.rs:435`, `Stub` in `tests/contract/durable-execution/grpc/limits.rs:44` and
`tests/contract/security-multitenancy/grpc/stub.rs:19`, `Proxy` in `tests/integration/database-runner/proxy.rs:69` and
`sdks/rust/conformance/src/faults/grpc.rs:70`, `Fake` in `apps/database-runner/src/testing.rs:98`, and `Service` in
`sdks/rust/src/worker_tests.rs:42`. From T041-7 on, `WorkerGrpc` answers it in a file of its own,
`crates/api-grpc/src/step_logs.rs`, so that the test selector can match the step-log code by path (T041-32). The
committed Go and Python stubs are regenerated with the tool versions they record (T041-3 names them).
`AppendStepLogsRequest`, `StepLogRecord`, `StepLogField` and its `oneof` join `skip_debug` (`build.rs:17-25`), with
`Debug` impls that print counts only, written beside each generated module (`crates/api-grpc/src/lib.rs`, the private
`protocol` module of `sdks/rust/src/lib.rs:39-42`, the `pb` module of `sdks/rust/conformance/src/faults/grpc.rs:4-7`).

**Negotiation.** The rule is unchanged (ADR-0015): `negotiate_protocol_details` (`crates/application/src/worker.rs:169`)
answers the lower of the two minors; what changes is the server's own minor. `PROTOCOL_MINOR` (`worker.rs:69`) becomes 7, with `STEP_LOG_PROTOCOL_MINOR = 7` beside the
others. `protocol_ceiling` (`worker.rs:82`) answers 7 for an ordinary worker only when the store proves scoped and
database support and a new proof, `WorkerStore::supports_step_log_protocol()`, beside `supports_scoped_worker_protocol`
(`worker.rs:283-290`), which is true when the migration of T041-5 is applied; it keeps 6 for the database runner. The
ceiling tests (`worker.rs:3035-3044`) gain the 7 cases.

Minors are a line, so an ordinary worker at 1.7 has 1.6's `ResolveDatabase` on the wire. It never calls it. The server
refuses it to a caller who does not own the managed executor and its live capability (`crates/application/src/worker/database.rs:22, 63`); an admin token holds `database:execute` (`crates/domain/src/identity.rs:585`), so for admin the
refusal rests on that ownership. `sdks/SPEC.md` §7.1 says ordinary SDKs keep 1.5; T041-7, which makes the server offer
1.7, amends it, and the `step-logs-negotiation` scenario asserts the refusal for a worker token and for an admin token.

**Why a separate unary RPC.** A field on `Heartbeat` would tie the size and the failure of a log batch to the call that
keeps the lease alive. A client stream would need acknowledgement and resend rules in six languages for a gain the batch
already gives. Riding on `CompleteStep` would add statements to the completion.

**Admission.** `AppendStepLogs` gets a fourth class, `Class::Logs`, in `crates/api-grpc/src/limits.rs:489-508`, with its
own counter outside `Shared.in_flight`, so that a flood of log calls never moves the threshold at which discovery is
shed (`limits.rs:548-563, 595`). Its cap is `SAGAT_GRPC_MAX_LOG_CALLS` (default a quarter of `SAGAT_DB_MAX_CONNECTIONS`,
at least 1: 5 of the execution engine's default pool of 20, `apps/execution-engine/src/cli.rs:55`), per replica; a call
over it is shed `RESOURCE_EXHAUSTED` with `retry-after-ms`, as shedding is today (`limits.rs:675-678`). The cap is what
keeps ingest from holding the connections lease calls need: at most that many of the pool's connections are in a log
transaction at once. The tenant is known only after authentication, so the per-tenant share is taken in the handler: a
ticket set built as `TaskStreams` is (`limits.rs:47-129`), `SAGAT_STEP_LOGS_TENANT_CALLS` per tenant per replica
(default half the cap, at least 1), taken without waiting; none free answers `RETRY_LATER` with 200 ms. The class test
`only_the_calls_that_keep_leased_work_alive_are_reserved_capacity` (`limits.rs:706`) lists `Logs`, and new tests show
that a full Logs class leaves discovery admission unchanged and the p99 of lease calls within 10% of its value without
it. No capacity is reserved for final batches: a final batch that meets `RETRY_LATER` or `RESOURCE_EXHAUSTED` waits
within its 2 seconds and the step reports anyway (section 6); T041-31 counts how often that happens.

### 2. Ingest

A new application module, `sagat_application::step_logs`, with a port `StepLogTx` that only reads and writes, and its
adapter in `crates/infra-postgres/src/step_logs.rs`. The rules are in the domain and the application, not in the
adapter: `crates/domain/src/step_logs.rs` holds the types (level, stop reason, ceilings, the server's sequence range), the
validation of a batch, and a pure `plan_batch(stream counters, policy, batch) -> Plan` that removes duplicates, counts
gaps, applies the attempt and run allowances and decides the marker and the verdict, with property tests. The
application reuses `first_uncovered` (`crates/domain/src/binding.rs:224`) and the role rule of `ensure_bound`.

**Before the transaction**, in the application:

1. `principal.authorize(Permission::WorkerExecute)`; a token of the database-runner or runner role is refused
   `PERMISSION_DENIED`.
2. `SAGAT_STEP_LOGS_ENABLED=false` answers `STOP_CAPTURE` (`CAPTURE_OFF`) without the database.
3. The tenant's ticket (section 1), or `RETRY_LATER`.
4. `validate_batch`: at most 500 records and 192 KiB encoded, sequences strictly increasing in 1 to 2^62-1, a known
   level, `highest_sequence` at least the last record's when `final_batch`; otherwise `INVALID_ARGUMENT` for the whole
   batch.
5. Normalise (FR-041-003), scrub with the server's shapes and the sensitive-name rule (FR-041-007 (c)), cut each message
   and exception to 8 KiB at a character boundary, build the fields' canonical JSON and keep fields up to 32 keys and
   2 KiB (data-model.md §5), and size each record exactly as `sagat_quota_measure` will measure it: the UTF-8 octets of
   `message`, `exception` and `fields`, plus the fixed row charge of 128 bytes (OD-2026-11-15-6).

The server's shapes are reached through two public entry points added in T041-6, each with a test that it equals what
it replaces: `sagat_telemetry::redact::scrub_shapes_with(text, placeholder)` (today `scrub_shapes` is private, hard-codes
`[redacted]` and is reached only through `scrub`, which cuts at 2,048 characters, `redact.rs:13, 46-66`) and
`sagat_domain::scripts::redact_credentials` made `pub` (`scripts.rs:779`). No second list of shapes or names is written.

**The transaction** (one per batch, opened by the project's `begin_safely` rule, ADR-0014, with
`SET LOCAL lock_timeout = '200ms'`; a lock timeout rolls back and answers `RETRY_LATER`). The control flow, so that a
caller whose binding or membership is gone learns only `AUTHORITY`, whatever else is true (D041-050):

- `LEASE` (step 2) ends the transaction at once: without the lease row there is no activity to check authority against.
- `SESSION`, `CAPTURE_OFF` (step 3), `REMOVED` and `FINAL_RECORDED` (step 4) are noted, and the transaction skips
  to step 7. If step 7 passes, the noted answer is given and the transaction rolls back, so nothing is stored or
  counted; if it fails, the answer is `AUTHORITY`.
- `FINAL_ONLY` (step 6) is not a stop: the transaction goes on through steps 7 to 10, so that the counters record the
  drops, and then answers `FINAL_ONLY` (or `AUTHORITY` if step 7 fails).

Every statement that binds line text or `q` maps its database error with `store_error_without_text`
(`crates/infra-postgres/src/identity.rs:734`), because the database's error text can quote bound values:

1. **Id first.** `SELECT pg_current_xact_id()`, so that the transaction's id is assigned before the lease is read (the
   end rule relies on it, section 5).
2. **The lease, without a lock.** One `SELECT` of the `steps` row by tenant and `step_id`, joined with its `runs` row,
   matching the request's `run_id`, the instance path of `instance` and its node as `lease_unexpired_instance` matches
   them (`crates/infra-postgres/src/execution.rs:4783-4817`), with status `LEASED` or `RUNNING`, the same attempt and
   generation, the lease-token hash, `lease_expires_at > clock_timestamp()`, and the run neither finished nor stopping
   (`finished_at` and `stop_reason` null). It returns the namespace, the node, the instance path and `steps.activity`;
   every later step takes the run, namespace, node and path from this row, never from the request. No row: the run is
   checked with `run_exists_outside`, and a foreign run is audited as `access.denied.cross_tenant` through
   `record_denial`, outside this transaction, as `missing_task_error` does (`crates/application/src/worker.rs:2564-2586`);
   either way, and for a mismatch, the answer is `STOP_ATTEMPT` (`LEASE`) and nothing else runs. A batch that read a
   valid lease and commits just after the completion is still that attempt's lines; a stale holder's token never
   matches.
3. **Session and policy.** The `worker_sessions` row, read without a lock, by tenant, id, the lease's namespace and the
   caller's subject, with the token hash equal, `expires_at > clock_timestamp()` and `accepted_protocol_minor >= 7`;
   otherwise `SESSION`. The five step-log resources of that namespace are read without a fence (a change applies to the
   batches that start after it commits; none of them is a tenant limit the finalizer enforces);
   `step_log_attempt_records = 0` means `CAPTURE_OFF`, decided before any row is written.
4. **The stream row.** `INSERT … ON CONFLICT (tenant_id, run_id, step_id, attempt, generation, source) DO UPDATE … RETURNING`
   on `step_log_streams` (a new row takes the namespace's retention, `retention_days`); the row is locked to commit and
   only batches of the same attempt wait on it (an SDK sends one at a time). `removed_at` set means `REMOVED`;
   `final_at` set means `FINAL_RECORDED`.
5. **Plan** (`plan_batch`). Records at or below `last_sequence` are dropped (a resend, or a late batch the worker gave up
   on). The batch's high mark is the highest of its sequences and, for a final batch, `highest_sequence`; the sequences
   between `last_sequence` and that mark that the batch does not hold are a gap: those above `ceiling_sequence`, once a
   ceiling has been met, are counted as dropped by the server, the others as dropped by the worker, each saturating at
   2^53.
6. **Allowances**, from counters, in this order: the attempt (lines and bytes on the stream row, against the namespace's
   attempt ceilings); the run (`run_log_usage`, created or locked by `INSERT … ON CONFLICT (tenant_id, run_id) DO UPDATE … RETURNING`, held to commit, so the batches of one run wait for each other here); the tenant
   (`sagat_quota_step_log_allowance(tenant)`, a `SECURITY DEFINER` accessor of the ADR-0027 Amendment that returns,
   **without a lock**, `tenant_retained_usage.step_log_bytes`, the limit worked out as the finalizer's branch works it out
   from `quota_limits`, `quota_ceilings` and the 1 GiB default, and whether it is enforced; the runtime role may not read
   `quota_accounting_control`, `20261024000000_durable_quotas.up.sql:561-572`, and the accessor keeps the finalizer's
   limit rule in one place). The attempt and run allowances count text bytes; the tenant's counts text plus 128 bytes a
   row. Records are kept in
   sequence order until the first that does not fit the attempt or run allowance; the rest are counted as dropped by
   the server with that ceiling as the reason, `ceiling_sequence` takes the stream's high mark, and when the attempt
   ceiling is first met one marker row (`ceiling_marker`, at `next_marker_sequence`) is planned. If the unlocked tenant
   read plus the kept bytes is over that limit, nothing
   of the batch is stored, the records are counted with `tenant_quota`, `ceiling_sequence` takes the high mark, and the
   answer is `FINAL_ONLY`.
7. **Authority, last before the insert.** `hold_worker_binding` (a shared advisory lock, `bindings.rs:15`), the binding's
   patterns (`bound_patterns`, `execution.rs:4756`) checked by `first_uncovered` against the `steps.activity` of step 2,
   and the free function `current_worker_authorized` of `crates/infra-postgres/src/execution.rs:1017` (the namespace
   secrets lock shared, then the membership row `FOR SHARE`), which gains an argument naming the roles it refuses. Its
   two present callers, the `current_worker_authorized` methods of `execution.rs:4628` and `execution/path.rs:183`,
   pass the database-runner role as today, so the trait method of the application and its callers do not change; the
   ingest's adapter passes the database-runner and runner roles. `share_secrets_lock` keeps its one caller, so
   `the_shared_secrets_lock_has_one_caller_and_the_scoped_path_never_upgrades` (`crates/infra-postgres/src/quotas.rs:1323-1344`) passes unmodified. A failure answers `STOP_ATTEMPT` (`AUTHORITY`). These locks are held to
   commit, so they are taken as late as possible. A noted answer of steps 3 and 4 is given now, and the transaction
   rolls back.
8. **Insert** the kept records and the marker in one `INSERT … SELECT … FROM unnest(…) ON CONFLICT (tenant_id, stream_id, sequence) DO NOTHING RETURNING` the text size of each inserted row (the octets the attempt and run ceilings count; the tenant's 128 bytes a row are the finalizer's).
9. **Counters** on the stream and on `run_log_usage` from the rows the insert returned, never from the batch:
   `last_sequence = GREATEST(last_sequence, high mark)`, lines, bytes, truncated lines, both drop counts,
   `ceiling_sequence` and `ceiling_reason`, `first_at`, `last_at = clock_timestamp()` (on every recorded batch, a line
   stored or not), `expires_at = last_at + retention_days`, and `final_at` when `final_batch` is true, even when nothing
   was stored.
10. **Commit, then answer.** The finalizer of ADR-0027 runs at commit. If it raises `tenant_step_log_quota_exceeded`
    (batches of other attempts crossed the limit at the same time), the transaction has rolled back with nothing stored
    or counted, and the answer is `RETRY_LATER` with 200 ms (D041-061). The SDK sends the same batch again; its step 6
    then reads the tenant over the limit and answers `FINAL_ONLY` with the records counted as `tenant_quota`. If the
    resends do not succeed within the batch's 10 seconds, the SDK drops the batch and the server counts it later as a gap
    (dropped by the worker); a final batch that is never stored leaves the stream "ended without its final flush", as any
    lost final batch does (G041-002). ADR-0084 decision 3 holds: the batch stores no line, and its records are counted
    on its stream by the resend's plan or by a later batch's gap. A property test of `plan_batch` holds that lines stored + dropped = `last_sequence` over
    any sequence of plans and refusals. A crash before the commit leaves nothing and the SDK sends the batch again; a
    crash after it leaves the lines, and the resend stores nothing (its records are at or below `last_sequence`) and
    gets `CONTINUE`, or `FINAL_RECORDED` for a final batch.

The number of top-level statements per batch does not depend on the number of records; it is budgeted as an equality in
the feature's suite (T041-6).

**One lock order** for everything that writes step logs: the stream row → the `run_log_usage` row → the binding lock
(shared) → the namespace secrets lock (shared) → the membership row (`FOR SHARE`) → at commit, the policy fences of the
classes the transaction charges, in ADR-0027's order (content, log, then step-log) → the tenant's
`tenant_retained_usage` row. An ingest charges the step-log class only, so it takes the step-log fence alone. Reports
take the binding, the secrets lock and the membership in the same order (`worker.rs:2729-2733`) and never take a stream
or run-usage row; the finalizer takes the fences and the usage row in the same order for every writer. The purge and the
delete route take stream rows (in `stream_id` order) and nothing earlier in the order. The script marker (section 7)
takes the script execution's advisory lock, inserts a stream row no other transaction can see, then takes the fences and
the usage row through `take_log_prefix`: the same order, with no exception. The run-usage row is taken before the
authority locks, so a run's batches wait for each other for the authority reads as well as the insert and the commit;
moving it after them would make every batch hold the namespace secrets lock, which delays the namespace's other writers,
while it waits for the run row (G041-009 and O041-020 cover the cost). The lock-order test grows with the tasks that add
a writer: T041-6 runs an ingest, a report and a binding change; T041-10 adds the purge, T041-13 the script marker and
T041-30 the delete route.

**What a completion can wait for.** A completion with output of tenant T takes T's usage row in its own commit. It waits
for the ingest commits queued ahead of it on that row: at most the tenant's tickets on every replica (tickets × R for R
API replicas, the tickets being at most the Logs cap), each holding the row from its first deferred flush call to the end
of its commit, which runs the remaining flush calls of up to 500 rows. A completion's report also takes the namespace
secrets lock shared; when an exclusive taker (`lock_secrets`, `execution/database.rs:96`) is queued, it waits for the
ingests of that namespace that hold the shared lock, each from step 7 to the end of its commit. That is the bound in
structure; SC-041-001's barrier tests (T041-6) prove it, and the bound in time is SC-041-007's completion p99, which
T041-31 measures before the schema is fixed and T041-9 at the end.

### 3. Storage

Four tables, in one migration of T041-5 (data-model.md §2 gives the DDL): `step_log_streams` (one row per attempt log,
with its counters and expiry), `step_log_lines` (one row per stored line or marker), `run_log_usage` (one row per run
that logged) and `classified_namespaces` (one row per classified namespace, insert-only, with its trigger).

**No foreign key to `runs` or `steps`.** An insert that references `runs` takes `KEY SHARE` on the run row, which
conflicts with the `FOR UPDATE` the engine holds while it leases or completes. The run's existence in the tenant is proven
by step 2 of the ingest in the same transaction, and nothing deletes a run today. Feature 020 must delete a run's streams,
lines and usage row when it deletes the run (G041-004, registered in feature 020's research as G1618). `step_log_lines`
references `step_log_streams` (same tenant), a row the engine never locks.

**Accounting** (ADR-0027 Amendment of 2026-11-15, accepted, OD-2026-11-15-16). `step_log_lines.message`,
`.exception` and `.fields` are of the new class `step_log` in `crates/infra-postgres/storage-classification.json`, and
`.level` carries the fixed charge of 128 bytes (`fixed_octets`, OD-2026-11-15-6); every other variable-size column of
the four tables is `protected_system`, so a marker costs its 128 bytes and nothing more. They count against
`tenant_retained_usage.step_log_bytes` and `tenant_step_log_bytes`, not against `tenant_log_bytes`, so step logs never
leave a script log, a dispatch error or a connector error stored empty (OD-2026-11-15-5). The usage column
`tenant_retained_usage.step_log_bytes` comes with the resource in T041-4, so that its usage view has a column to read.
T041-5's migration adds the delta, the class, the kind, the routing of the class in `sagat_quota_row_delta`, the new
argument of `sagat_quota_add_delta`, the loop filter and the branch of the finalizer, the control flag and the two
accessors (data-model.md §3), and creates the triggers the manifest generates on `step_log_lines` only, the one table
with accounted columns; the manifest test learns the class and the kind and otherwise passes unmodified.

**Grants** are those of data-model.md §4: no `UPDATE` on lines, no grant on `namespaces`, no `TRUNCATE`, and no
`SECURITY DEFINER` function beyond the two accessors of ADR-0027's Amendment (`sagat_quota_step_log_allowance`,
`sagat_quota_lock_step_log_usage`). The runtime privilege audit covers the new statements. There is no row-level security in
this database; the tenant boundary is the `tenant_id` in every key and predicate, tested by the cross-tenant battery
(`tests/contract/security-multitenancy/rest/battery.rs`). The down migration refuses while a line or a classified
namespace exists.

**The classified-namespace record (FR-041-021).** An `AFTER INSERT` trigger on `workflow_versions`, enabled `ALWAYS` as
the table's other triggers are (`20261009000000_definitions.up.sql:76-78`), with a fixed `search_path` and qualified
names, inserts
`(tenant, namespace)` into `classified_namespaces` with `ON CONFLICT DO NOTHING` when the version's `document` holds the
UTF-8 text `"x-sagat-classification"` (`position(… in document) > 0`). `document` is the canonical JSON the version froze
(`crates/application/src/definitions.rs:479-480`), so every annotation the publication analysis finds appears in it
literally; the test can also match the text where it is not an annotation (a description that quotes it), which counts
the namespace as classified and errs closed (G041-010). The trigger fires for every insert, whatever binary makes it
(both `crates/infra-postgres/src/definitions.rs:547` and `execution/ad_hoc.rs:34` insert versions), so a rolling upgrade
or a rollback cannot leave a classified version unrecorded. The migration creates the trigger and runs the same test once over the versions that
exist in one transaction, so no version inserted in between is missed. A test holds that, on the classification fixtures and on versions published with `classification_rules = 0`, every version the publication analysis (`crates/infra-schema/src/classification/publication.rs:33-55`) finds
classified records its namespace.

### 4. Retention, purge and deletion

A stream takes `step_log_retention_days` from its namespace when it is created and keeps it (`retention_days`); its
`expires_at` is the server's time of its last recorded batch plus that retention, written by step 9 of every batch. This
is ADR-0027's retention rule ("lowering policy does not rewrite stored expiry … or authorize retroactive deletion",
`docs/adr/ADR-0027-*.md:93`): a lowered retention applies to the streams created after it, and the delete route is the
way to remove lines early.

A housekeeping pass (`crates/scheduler/src/passes.rs`, beside `purge_script_records`, through a new method of
`MaintenanceStore`, `crates/application/src/maintenance.rs:117`) removes lines in two moves:

1. **Mark.** Streams with `removed_at` null and `expires_at` past (`step_log_streams_expiry_idx`) are locked
   `FOR UPDATE SKIP LOCKED`, a page at a time, and get `removed_at`. From then on readers show the stream as expired and
   return none of its lines, and step 4 of the ingest answers `REMOVED` under the same row lock, so no batch can add a
   line to a stream whose removal has begun. The delete route sets `deleted_at` and `removed_at` together.
2. **Delete.** For streams with `removed_at` set and `purged_at` null (`step_log_streams_removal_idx`), at most 500 lines
   per transaction are deleted; when none is left, `purged_at` is set. Deleting releases the quota through the triggers,
   and 500 rows per transaction bound how long the purge holds the tenant usage row.

A stream whose attempt still writes has a recent `expires_at` and is not marked; one that stopped writing for longer
than its retention is marked even if its lease lives, and its next batch is answered `STOP_ATTEMPT` (`REMOVED`).
`run_log_usage` is not reduced: the run ceiling bounds what a run may ever write. Stream and usage rows stay until
feature 020 deletes the run (G041-004); T041-9 measures their size per row.

`DELETE /runs/{runId}/logs[?stream=]` (FR-041-023) locks the run's streams (or the one named) `ORDER BY stream_id FOR UPDATE`, sets `deleted_at` and `removed_at`, writes `run.logs_deleted` in the same transaction and answers `204` with no
body, so the answer says nothing about how many streams the run has; readers skip deleted streams at once. It is admitted as a new `ApiOperation::RunLogsDelete` (`crates/application/src/api_admission.rs:32`) that maps to `&[RunRead, RunLogsDelete]`.

### 5. Reading

A second port, `StepLogReader`, used by new handlers in a file of their own, `crates/api-http/src/routes/step_logs.rs`,
so that the test selector can match them by path (T041-32). Each read calls `admit` with a new
`ApiOperation::RunLogsRead` that maps to `&[RunRead, RunLogsRead]`; the application authorizes `run:logs:read` again
and, when the run's namespace is classified (a row of `classified_namespaces`), checks
`has_effective_payload_reveal_grant` (`crates/infra-postgres/src/reveal_grants.rs:40`) in the read transaction, which
holds the grant-set lock shared, so that a revocation cannot commit between the check and the audit. This check is one
application function, `step_logs::may_read(principal, tx, run)`, built in T041-11; it takes the read transaction, and
every route that discloses step-log text or a script execution's log calls it inside its own read transaction and writes
the throttled `run.logs_read` record in that transaction before it answers (T041-11, T041-13, T041-29). The permissions
`run:logs:read` and `run:logs:delete` join `Permission::ALL` and the role matrix test. `SAGAT_STEP_LOGS_READ_ENABLED`
(default true) turns the read routes off (`422 VALIDATION_FAILED` `STEP_LOGS_NOT_AVAILABLE`) without touching capture;
it is the operator's way to shed read load during the rollout, and it never opens a script log FR-041-022 closes. The
other refusals are the error table of contracts.md §2.

**A stream's state** (contracts.md §2) is worked out by the read from the stream row and, for a worker stream, the
`steps` row of its `step_id`, read without a lock: `deleted` when `deleted_at` is set; `expired` when `removed_at` is
set otherwise; `ended` when `final_at` is set; `ended_without_final` when none of those is set and the `steps` row no
longer holds this attempt's lease (its status is not `LEASED` or `RUNNING`, its attempt or generation differs, or its
`lease_expires_at` has passed); `open` otherwise. FR-041-008 forbids the lease, heartbeat and completion from reading the
log tables, not the read routes from reading `steps`. A script stream is `ended` or `expired`.

**Order and horizon.** Lines are read in (`stored_xid`, `sequence`) order (one transaction stores lines of one stream
only, so the pair is unique within a run), and only those whose `stored_xid` is below
`pg_snapshot_xmin(pg_current_snapshot())` computed in the same statement. Every transaction with a smaller id has ended,
so no line can later appear before the cursor: a page is final and a tail never skips. `heldBack` is true when lines of
the run are visible to the statement but at or above the horizon. The horizon is held back by any open transaction of the
cluster, prepared ones included; the lines are not lost, and `database_oldest_transaction_age_seconds` with its alert
(section 9) tells the operator. The deployment is expected to keep `max_prepared_transactions = 0` and to set
`idle_in_transaction_session_timeout`; the runbook says so.

**Cursor.** The cursor is plain, as every cursor of the API (`crates/api-http/src/routes/primitives.rs:193-209`): the
`base64url` JSON of contracts.md §3, holding the run id, the position (`stored_xid`, `sequence`) and the end horizon once
there is one. A cursor of another run, or one that does not parse, is refused `400 BAD_REQUEST`. The position exposes the
database's transaction counter to a reader (G041-008, O041-023); no row identity is in it, and the stream's public id is
a UUID.

**End of a read (the rule of D1958).** When a read sees the run finished and its cursor has no end horizon, it ends its
read transaction and takes `end = pg_current_xact_id()` in a new, short transaction of its own, so that the id is
assigned after the statement that saw the run finished (the read transaction may already hold an id, for instance from
the audit insert, assigned before it). Every ingest that saw the lease
live took its id before it read the lease (step 1), and that read did not see the run finished, so its id is below
`end`. A read whose cursor holds `end` answers `complete: true`, and the tail sends `event: end`, only when the run is
finished, `pg_snapshot_xmin(pg_current_snapshot()) > end` in the reading statement, and the page is empty; an `end` in a
cursor for a run that is not finished is ignored, so a forged cursor cannot end a running run's tail. The comparison is
strict, as in D1958 (`crates/infra-postgres/src/batch.rs:453-459, 654-656`), where the old horizon `xmax` let a
transaction open with that id pass `>=` ("held 916, horizon 916, xmin 916"). T041-11 builds the rule for the list
route's `complete`; T041-12 uses the same function for the tail and tests an ingest with the largest id below `end` held
open across the end, and a first read of a new audit window that sees the run finished.

**Filters.** The read index (`tenant_id, run_id, stored_xid, sequence`) serves every read; `stream`, `node`, `attempt`,
`minLevel`, `from` and `to` are evaluated on the run's rows in that order, and `q` is
`position(lower($q) in lower(message)) > 0` as a bound parameter. A read scans at most the run's rows (the run ceiling,
100,000 by default) under the route's statement timeout. `stream` is matched by public id together with the run and the
tenant.

**Audit.** `run.logs_read` is a new `AuditAction` in `crates/domain/src/audit.rs`, added to `collapses_repeats()` and
to `repeat_window()` with 300 seconds (`audit.rs:259, 287`). It is written by `insert_audit`
(`crates/infra-postgres/src/identity.rs:239`), which takes an advisory lock on (tenant, actor, action, resource) and
writes only when no identical record exists in the window, so two first reads on two replicas write one record. The
resource is the run and the record holds no filter, no line and not the value of `q`, so that every read of the window is
the same event. It is written in the read transaction before the page is returned ("no audit, no lines").

**Tail.** `GET /runs/{runId}/logs/stream` (contracts.md §2) follows the loop of `follow` in `execution.rs:1340`, refactored
to take the poll function and the event encoder as parameters so that the graph stream and the tail share one loop:
`event: log` with up to 200 lines and the cursor as `id`; `event: progress` with the cursor when the scanned position
moved and no line matched the filters, so that a filtered tail does not rescan on reconnect; a `: ping` every 15 seconds
otherwise; `event: end` by the rule above; `event: reconnect` at token expiry or after 15 minutes; resume by
`Last-Event-ID`; polling every 0.5 s and every 2 s once idle for 30 s; the reader's right and grant checked again every
`StreamLimits::recheck` (30 s, `crates/api-http/src/streams.rs:34-50`); the caps of the graph stream (20 per subject, 200
per tenant), shared with it.

**Contract.** The four operations are tagged `x-planned: '041'` until their route task serves them, and the battery's
`body_of` (`battery.rs:29`) gains them when they are served. The digest test
`operations_outside_the_approved_additions_and_event_extensions_are_byte_stable`
(`tests/contract/payload-classification/openapi.rs:343-388`) gains `x-planned == "041"` and `x-implemented == "041"` in
its list of approved additions. The `ScriptExecution` change of FR-041-022 is additive (`log` stays required; an optional
`logWithheld`); the digest hashes path operations only, so it moves only for the descriptions of `getScriptExecution`
and `listScriptExecutions`, which T041-29 changes and records in the digest's comment. No check today fails when a
component schema used by an existing operation changes; T041-29 says so in its pull request.
`recordScriptExecution` does not change: the script marker is keyed on the execution the report creates (section 7).
The `Quota` description gains one absence of `used`: `tenant_step_log_bytes` shows no `used` to a caller without
`run:logs:read`, so that a viewer, who holds `quota:read`, cannot watch how much a tenant's runs log (ADR-0085 decision 6;
T041-4).

### 6. The six SDKs

`sdks/SPEC.md` §6.3 gains the logger's semantics and a new section defines capture; §9 (versioning) is amended for 1.7
and gains the register of which feature holds each minor (O041-018); its two headings numbered 10 are renumbered; §10 keeps its rule that a logger that throws never kills a
heartbeat or a task, and adds that capture never does either.

**The logger each SDK has today, and what it gains** (T041-14 writes this into `sdks/SPEC.md`; each SDK task builds its
row):

| SDK | Today | Gains (public API) | Capture point and the host's enabled check |
|---|---|---|---|
| TypeScript | `ActivityLogger`: `debug/info/warn/error(message, fields?)` (`sdks/typescript/src/worker/activity.ts:87-92`); one object per attempt that merges `{task}` (`host.ts:555-559`) | `trace(message, fields?)`, and an optional `error?: unknown` third parameter on all five | the attempt's object, before it calls the host `Logger`, which has no enabled method (`config.ts:7-12`) |
| Go | `*slog.Logger` from `TaskFrom(ctx).Logger` (`worker/activity.go:195, 265-268`), built with `With("task","activity","run","node","attempt")` (`task.go:176`) | `worker.LevelTrace = slog.Level(-8)` | a per-attempt `slog.Handler` wrapping the host's; `Enabled` is true at or above the capture level or when the host is enabled (`sdks/go/internal/safelog/safelog.go:24-31` passes through today), and `Handle` forwards to the host only when the host is enabled |
| Python | `_TaskLog(LoggerAdapter)` per attempt (`worker/host.py:172-178, 712-720`); `process()` replaces `extra` with `{"sagat": fields}` and appends `key=value` to the message | `TRACE = 5` (registered with `logging.addLevelName`) and `_TaskLog.trace(msg, *args, **kwargs)` | `_TaskLog.log`, before `LoggerAdapter.log` checks `isEnabledFor`, from the raw message and arguments, before `process` runs |
| C# | `ILogger Logger` per attempt, category `SagatOctopusOrchestrator.Activity.<activity>`, wrapped in `SafeLogger` (`Workers/ActivityExecutionContext.cs:154`, `WorkerHost.cs:814, 953-957`) | nothing: `ILogger` has `Trace`, exceptions and state | a per-attempt decorator inside `SafeLogger`; `IsEnabled` is true at or above the capture level or when the inner logger is (`Internal/Safe.cs:103-114`), and `Log` forwards only when the inner logger is enabled |
| Java | `ActivityLogger`: abstract `log(System.Logger.Level, String)` and `debug/info/warn/error(String)` (`worker/ActivityLogger.java:7-24`); a lambda per context prefixing `task=<id> ` (`WorkerHost.java:804`, `Hooks.java:29-35`) | default methods (binary compatible): `trace(String)`; `log(Level, String, Map<String, ?>, Throwable)` whose default calls `log(level, message)`; `trace/debug/info/warn/error(String, Map<String, ?>)`; `warn(String, Throwable)`; `error(String, Throwable)`; `error(String, Map<String, ?>, Throwable)` | the SDK's per-attempt implementation of the four-argument `log`, before the `task=` prefix; `Hooks.log` has no `isLoggable` |
| Rust | none (`sdks/rust/src/activity.rs:55-62`) | `ActivityContext::log(&self) -> &StepLogger`; `StepLogger::{trace, debug, info, warn, error}(&self, message: impl AsRef<str>)` and `StepLogger::log(&self, level: Level, message: impl AsRef<str>, fields: &[(&str, FieldValue)], error: Option<&(dyn std::error::Error + 'static)>)`; `FieldValue::{Str, Int, Float, Bool}` with `From` impls; `Level` of five | `StepLogger` itself; it also emits a `tracing` event with the message at the same level (target `sagat_octopus_orchestrator::activity`); no `tracing` layer is installed, so only these calls are captured |

| SDK | Level mapping onto the five | Exception text | Language context as fields | Identifiers not captured | Worker option |
|---|---|---|---|---|---|
| TypeScript | identity | an `Error`: `name: message` and `stack`; anything else: `String(value)` | the `fields` argument | the `task` key the SDK merges | `WorkerConfig.stepLogs?: {capture?: boolean; level?: StepLogLevel}` (`host.ts:23`) |
| Go | below -4 trace; -4 to -1 debug; 0 to 3 info; 4 to 7 warn; 8 and above error | an attribute whose value is an `error`: `err.Error()` (`%+v` when it implements `fmt.Formatter`), also kept as a field | attributes of the call and of `With` by the activity; groups flattened as `group.key` | the five attributes the SDK adds with `With` | `worker.WithStepLogs(capture bool, level slog.Level)` (the pattern of `WithMaxInflight`, `host.go:112`) |
| Python | below 10 trace; 10-19 debug; 20-29 info; 30-39 warn; 40 and above error (`CRITICAL` included) | `exc_info` (and `exception()`) through `traceback.format_exception`; `stack_info=True` appended | the caller's `extra`, taken before `process` replaces it | `task`, `run`, `node`, `attempt` | `WorkerConfig.step_logs_capture: bool = True`, `step_logs_level: int = logging.INFO` (`host.py:104`) |
| C# | `Trace`, `Debug`, `Information`→info, `Warning`→warn, `Error` and `Critical`→error; `None` is not captured | the `Exception` argument's `ToString()` (type, message, stack, inner exceptions) | a state that is `IReadOnlyList<KeyValuePair<string, object?>>` without `{OriginalFormat}`; scopes begun on the activity logger, kept per attempt in an `AsyncLocal` stack | none (the host's `sagat.*` scope is on another logger, `WorkerHost.cs:703-708`) | `WorkerOptions.StepLogs` with `Capture` and `Level` (`SagatOptions.cs:162`) |
| Java | `ALL` and `TRACE`→trace, `DEBUG`, `INFO`, `WARNING`→warn, `ERROR`; `OFF` is not captured | the `Throwable`'s stack trace with its causes | the `Map` argument | the `task=` prefix | `WorkerConfig.Builder.stepLogs(boolean capture, System.Logger.Level level)` (`WorkerConfig.java:142`) |
| Rust | identity | the error's `Display`, then each `source()` joined by `\nCaused by: ` | the `fields` argument | none | `WorkerOptions.step_logs: StepLogOptions {capture: bool, level: Level}` (`worker.rs:28`) |

Field values in every SDK: strings, integers (a language integer that fits 64 bits; a larger one as a string), finite
doubles, booleans; a null or absent value is dropped; any other value is captured as a string (its JSON text where the
language has one, else its string form), then redacted and cut as a string. The message is formatted as the language
formats it for its host (Python `msg % args`, C# the formatter's result) before redaction.

The cost of a logger call is bounded per SDK: each SDK task (T041-15 to T041-20) adds a micro-benchmark of one call
with an 8 KiB message and a secret set of 1,024 values, whose p99 must be under 1 ms on the machine of the validation
report; an SDK that does not meet it replaces its plain search by a multi-pattern matcher in the same task.

Common rules, each a line of `sdks/SPEC.md` and a conformance assertion:

- **The process's secret set** (OD-2026-11-15-7): every value `ResolveSecrets` returns in the
  process is added and kept for the life of the process, up to 1,024 values or 256 KiB; when the set is full, capture
  stops for the process and one local line says so; each attempt that has a stream still sends its empty final batch
  with `highest_sequence`, so its drops are counted. The SDKs add the value where they resolve it (Java
  `WorkerHost.java:977-992`, TypeScript `host.ts:896-913`, Go `task.go:456-460`, Python `host.py:1111-1150`, C#
  `WorkerHost.cs:1299-1351`, Rust `capabilities.rs:111-113`). Every captured line is redacted with the whole set
  (FR-041-007 (a)).
- A logger call at or above the capture level takes the next sequence, formats, redacts, cuts to the line ceilings and
  appends to the attempt's buffer, without waiting. A full attempt buffer, or a full process bound (16 MiB), drops the
  record; the gap is the server's evidence and the SDK counts the reason locally (data-model.md §7).
- A sender per attempt flushes every second, or at 256 records, one batch in flight. Its answers are those of
  contracts.md §4: two retries of the same bytes for `UNAVAILABLE` and `DEADLINE_EXCEEDED` (after 200 ms and 1 s, ±20%),
  then the batch is dropped; `RETRY_LATER` and `RESOURCE_EXHAUSTED` wait the given delay and do not count, up to 10 s of
  a batch's age; after `FINAL_ONLY` nothing but the final batch is sent, and `FINAL_ONLY` or `FINAL_RECORDED` in answer
  to the final batch is its success.
- Before `CompleteStep` or `FailStep`, an attempt that assigned at least one sequence sends one final batch (its
  remaining records, or none) with `final_batch = true` and `highest_sequence`, waiting at most 2 seconds; then it
  reports the step whatever the flush did. An attempt that assigned none sends nothing (no stream, "no log").
- After `STOP_CAPTURE` (`CAPTURE_OFF`, or `SESSION` when the token sent is the one held) capture stays off for the
  session until the next successful registration, which brings a new policy.
- The buffer is in memory only and dropped with the attempt. Capture uses no thread or task that can keep the process
  alive after shutdown; worker shutdown sends each attempt's final batch once within the shutdown deadline.
- The SDK never logs a batch, and nothing about capture is written at a level the host logs by default except one line
  per process when the server stops capture or the secret set is full.

**Conformance** (T041-14 extends `tests/contract/sdk-conformance`): activity behaviours `logs` (a list of level, message,
fields, exception and a repeat count) and `logsSecret` (log the resolved secret in the six positions of SC-041-004), a
step op `stepLogs` that reads `GET /runs/{runId}/logs` and `/logs/streams` and compares them, normalised (times, ids and
cursors removed), with the golden; a fault action `verdict` on the gRPC channel that answers `AppendStepLogs` with a
given verdict, implemented by each SDK's fault plan (`sdks/go/conformance/faults.go`, `sdks/python/conformance/faults.py`,
`sdks/typescript/conformance/faults.ts`, `sdks/java/conformance/.../FaultPlan.java`, `sdks/dotnet/conformance/FaultPlan.cs`, `sdks/rust/conformance/src/faults.rs`); and an evidence key `mustNotStore`, searched in the stored lines,
the routes' bodies and the server's captured output (`mustNotLeak` searches only errors and `mustNotAppear` not the worker
log, `harness.rs:942-1040`). Five scenarios, one file and one golden each:

1. `step-logs-capture`: the five levels and the capture level, fields of each type, a non-scalar field, NUL and
   non-finite doubles, an exception, order by sequence, two attempts of one step, an iteration of a root collection,
   language context captured as fields, a field map over 32 keys and over 2 KiB, a host logger set above the capture
   level.
2. `step-logs-redaction`: two attempts in flight at once with different secrets, and a third that starts after the
   first ended; each logs its own and the others' secrets whole, inside a message, as a field key, as a field value, as
   a number equal to a numeric secret (12345678 and 1e15) and inside an exception; `mustNotStore` for all. Fails before
   the SDK task: the scenario's `stepLogs` op finds no line.
3. `step-logs-limits`: a message over 8 KiB is cut and flagged; a flood meets the buffer and the attempt ceiling; the
   step completes with its normal output; stored + dropped by the worker + dropped by the server = the highest
   sequence.
4. `step-logs-transport-faults`: a lost response (the same batch twice), `INVALID_ARGUMENT`, `RETRY_LATER`,
   `RESOURCE_EXHAUSTED`, `UNIMPLEMENTED`, `FINAL_ONLY`, a re-registration in the middle of an attempt, the server down for
   the whole attempt; no duplicate; the step completes.
5. `step-logs-negotiation`: a worker pinned to 1.5 sends nothing; a 1.7 worker told `enabled = false` sends nothing;
   both log locally; a 1.7 ordinary worker calling `ResolveDatabase` is refused, with a worker token and with an admin
   token.

Before an SDK's task lands, each scenario fails in that SDK because its runner refuses the new behaviours and the
`stepLogs` op as unknown; after it, a failing scenario names the assertion of its golden that did not hold.

The server's own canary test (SC-041-004 on the server side) is T041-8's: it captures the server's `tracing` output in
`integration_step_logs` and searches it.

### 7. Steps the platform runs

A script step's output is shown in two ways (OD-2026-11-15-9): `listRunLogStreams` with `source = script` lists the
run's `script_executions` by (tenant, run) through `script_executions_step_uniq` (`20261014000000_scripts.up.sql:250-278`),
paged on its own, each with its execution id, node, attempt, own `expiresAt` and `logTruncated`, under FR-041-022; and a
**marker line** in the run's log, at the time the report was recorded, that the whole-run tab shows as a row opening the
execution's log under FR-041-022. No script text is ever copied.

**The marker is keyed on the execution** (D041-060). `script_executions_step_uniq` is unique on (tenant, run, node,
attempt) (`20261014000000_scripts.up.sql:277`), and `record_execution` inserts at most one execution per key under its
advisory lock (`crates/infra-postgres/src/scripts.rs:1783-1796`). The marker follows the execution: one stream of
`source = script` per inserted execution, whose public id is the execution's id, with the execution's node and attempt
and no step, generation or instance path, and one marker line in it. Inside a collection the runner gives every
iteration the same node (`apps/sandbox-runner/src/lib.rs:1309-1315`), so only the first iteration's report of an attempt
is recorded and the others return as repeats (`scripts.rs:1792-1797`), as today; the marker shows the node and the
attempt and no iteration (G041-007). Widening the key belongs to feature 005. The runner's report, `ExecutionReport`
and `recordScriptExecution` do not change.

**When it is written.** The decision is a pure function of the domain, `script_marker_decision`, beside `plan_batch` in
`crates/domain/src/step_logs.rs`, over facts the transaction reads: the execution is being inserted (not a repeat, which
returns early under its advisory lock); it is an execution of a run (not a test run, `run_id` null); the control plane's
`SAGAT_STEP_LOGS_ENABLED` is on and the namespace's `step_log_attempt_records` above 0; the run's `finished_at` is null
in this transaction. A run already finished gets no marker, so that a marker can never commit after a tail has sent
`end`. Each outcome is counted (`step_log_script_markers_total`, data-model.md §7), which is where a skipped marker is
seen.

**The transaction** is `record_execution`'s own, in this order, which keeps ADR-0084 decision 11 without an exception:

1. `SELECT pg_current_xact_id()` first, as the ingest does, so that a marker whose transaction saw the run unfinished has
   an id below any end horizon (section 5).
2. The existing advisory lock on (tenant, run, node, attempt), the repeat check, the `step.leased` read and the digest
   and retention checks, unchanged. No other step-log path takes that advisory lock.
3. The facts of the decision, read without a lock: the run's `finished_at` and the namespace's capture resource.
4. The marker stream: a plain `INSERT` into `step_log_streams` with `source = script`, `public_id` the execution id that
   `record_execution` generates before its insert, `last_at` now and `expires_at` the earlier of now plus the namespace's
   `step_log_retention_days` and the execution's own `expires_at`, so that the marker never outlives the execution it
   opens. No other transaction can see or lock that row before the commit.
5. `take_log_prefix`, which takes the content fence, the log fence and, with a new argument, the step-log fence, then the
   tenant usage row `FOR UPDATE` through `sagat_quota_lock_step_log_usage` (ADR-0027 Amendment), in ADR-0027's order.
6. Under that lock, the step-log usage and limit are exact (nothing in the transaction has charged the step-log class
   yet): if the marker's 128 bytes fit, one line of kind `script_marker` with the execution id is inserted at the
   stream's `next_marker_sequence`; if they do not, no line is inserted, the stream records `tenant_quota` and the
   metric counts `skipped_quota`. The script record never fails for lack of step-log quota (ADR-0027 Amendment, item 1).
7. The existing insert of `script_executions` and its audits. Under the advisory lock it always inserts; if it ever
   reports no row, the transaction rolls back and the call answers as a repeat, so no marker points to nothing.
8. Commit: the finalizer charges the script log to `tenant_log_bytes` and the marker to `tenant_step_log_bytes`, under the
   fences it already holds.

The marker does not touch `run_log_usage`: it is one row per script attempt and is not a worker line. `listRunLogStreams`
with `source = worker` does not list marker streams; with `source = script` its items are the executions, whose ids are
the marker streams' public ids. The purge and the delete route remove marker lines as any other; the execution and its
log are untouched.

**Tests (T041-13).** One marker per inserted execution, with the execution's node and attempt; with two iterations of
one attempt in a collection, one execution and one marker are kept and the second report answers as a repeat with no
marker; a repeat report writes none; a test run, a finished run, capture off in the namespace and the control plane's
switch off write none; at the step-log limit the execution is recorded and no line is stored; the marker stream expires
no later than the execution; filtering `listRunLogs` by the execution id returns its marker; as the runtime role
(`SAGAT_TEST_AS_RUNTIME_ROLE=1`); at barriers, a `record_execution`, an ingest of the same run, a delete of the run's
logs and a change of `tenant_step_log_bytes` all commit with no deadlock; a `record_execution` held across the run's
terminal transition is delivered by the tail before `end`, or writes no marker if it read the run finished.
`script_marker_decision` has unit tests of each outcome.

### 8. Studio

In `frontend/apps/studio/src/features/runs/`:

- A fourth tab, **Logs**, in `RunDetailPage.tsx` beside Graph, Table and Timeline, and a **Logs** section in
  `NodePanel.tsx` for the selected step with an attempt selector fed by the streams route (paged, both sources).
- `LogView.tsx`: rows windowed with `useWindowedRows`; each row shows time (worker time, with the stored time on demand
  and a note when the two differ by more than five minutes), level as text and shape as well as colour, the step and
  iteration in the whole-run view, the message and the exception as text, never as HTML, never auto-linked, with terminal
  escape sequences removed and bidirectional controls and zero-width characters shown as visible escapes, and fields in
  an expandable part.
- Filters: level, step, attempt, time range; a search field that sends `q`; all reflected in the URL except the search
  text.
- Tail: `followLogs()` beside `followRun()` in `src/api/follow.ts`, with the same parser (`readSse`) and the same
  fallback to polling the list route, which stops when `complete` is true (contracts.md §3). The tail pauses when the
  reader leaves the end; "Jump to latest" resumes. New lines are announced through `Announcer` as a count at most every
  ten seconds.
- States (FR-041-017): `NotAllowed` naming `run:logs:read` from the session's permissions with no request, or naming the
  PII reveal grant from a `403` `PII_REVEAL_NOT_GRANTED`; "no log"; "expired on …"; "deleted"; "ended without its final
  flush: the last lines may be missing"; "some lines are delayed" while `heldBack`; markers as rows with text; "log
  withheld" for a script execution.
- Keyboard: the tab follows the existing `Tabs` pattern; the list is one tab stop with arrow, Page, Home and End movement
  and Enter to expand a row; focus is not moved by arriving lines.
- Types come from `npm run api`; `npm run check:api` stays green.

### 9. Observability

The metrics of data-model.md §7, with closed labels. Alerts in `infra/observability/alerts.yaml`, with entries in
`docs/runbooks/alerts.md`: `SagatStepLogIngestShedding` (`retry_later` above 5% of batches for ten minutes),
`SagatStepLogPurgeBehind` (purge backlog growing for an hour) and `SagatDatabaseOldTransaction` (the oldest open
transaction older than five minutes). That gauge, `database_oldest_transaction_age_seconds`, is a read of the operations
snapshot built as the `count_and_age` reads of `crates/infra-postgres/src/operations.rs:437-448`, over `pg_stat_activity`
(`xact_start`) and `pg_prepared_xacts` (`prepared`). The runtime role sees `xact_start` only for its own role's sessions
unless the deployment grants it `pg_read_all_stats`, so a long transaction of another role (a backup, a person in
`psql`) can hold the read horizon unseen by the alert (G041-016); the reader still sees `heldBack`, and the runbook
names the grant. Drops at a ceiling are the customer's code and are shown on the run, not alerted. The ingest has a
span, `worker.append_step_logs`, with counts and the verdict and no text. The runbook `docs/runbooks/step-logs.md`
(created by T041-10, extended by T041-30 and completed by T041-25) covers the six resources, the deployment switches, the
purge, what each state and marker means, that a change of retention applies to new streams and a change of capture or a
ceiling to new batches, the classified-namespace record and its superset test, the PostgreSQL settings of section 5, and
the incident path (CUJ 7: delete the lines, rotate the secret; a script execution's log is removed only by its own
retention, G041-013). `docs/architecture/overview.md` and the storage documentation gain the new tables.

### 10. Rollout and rollback

The migrations are additive. `SAGAT_STEP_LOGS_ENABLED` (default true from the task that serves the RPC on, since the
purge lands before it) makes the policy answer `enabled = false` and the RPC answer `STOP_CAPTURE` without touching the
database, and makes the control plane write no script marker. `SAGAT_STEP_LOGS_READ_ENABLED` (default true) turns the
read routes off. Old and new binaries run together: an old API replica answers `UNIMPLEMENTED`, which an SDK treats as a
stop for that attempt. An old replica refuses to list quotas once a step-log quota row exists, because it cannot parse
the resource name (`crates/infra-postgres/src/quotas.rs:1302-1308`); the runbook says to finish the upgrade before
setting a step-log quota. The classified-namespace record is kept by a database trigger, so an old binary that publishes
during the upgrade or after a rollback is recorded too. Rolling back the binary leaves the tables unread; rolling back
the migration needs the lines purged first.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| A step's code logs a secret the platform cannot recognise (transformed, short, obtained elsewhere) | G041-001, accepted (O041-011). Value redaction with the process's secret set, two shape scrubbers, a permission of its own, the PII grant in classified namespaces, the namespace switch, the delete route and its runbook |
| Capture is on by default and sends text a customer wrote for their own machines | O041-001 chosen by the owner; the release notes say it first; the namespace switch and the worker option; the runbook |
| The six SDKs' redaction drifts apart | One rule (`redact`) and one canonical number form in `sdks/SPEC.md`; `step-logs-redaction` with the same goldens in six languages |
| A hand-written 1.7 client skips redaction | ADR-0085 rule 12 states it; the server's shapes and sensitive names still apply; the permission and the grant bound who reads |
| Log batches make completions wait | The tenant row only at commit; authority last before the insert; per-tenant tickets; the queue bound of section 2, proven by the barrier tests of T041-6; the time bound of SC-041-007, measured by T041-31 before the schema and T041-9 at the end |
| A wide fan-out serialises on `run_log_usage` | Measured in T041-31 (200 attempts of one run, 64 at a time); O041-020 if it fails |
| Each logging step adds a transaction and up to 500 trigger calls | Per-tenant tickets and shedding; a step that logs nothing sends nothing; T041-9 measures trigger cost and commit latency of a full batch |
| The final flush delays the completion of every step that logs | One call, bounded at 2 seconds; measured in T041-9 |
| A revoked or re-registered worker | Binding, membership, session (namespace, expiry, token) and lease checked as reports do; a re-registered worker resends with its new token; T041-8 tests each |
| A long or prepared transaction holds the read horizon | Lines arrive late, never out of order or missing; `heldBack`; the oldest-transaction gauge and alert; the runbook's settings |
| The tail ends before a late batch commits | The end rule of D1958 with a strict comparison; the equal-id case tested on PostgreSQL 16 and 18 |
| A flood fills the table before the ceilings act | Ceilings from counters before the insert, in the same transaction; drop counts derived from sequences |
| Rows without log bytes escape the tenant quota | Every row is charged 128 bytes on top of its text (OD-2026-11-15-6, D041-054) |
| Step logs fill a quota other writers need | Step logs have their own quota, `tenant_step_log_bytes`; `tenant_log_bytes` keeps script, dispatch and connector logs (OD-2026-11-15-5, D041-053) |
| The script marker makes the script report fail or deadlock | Written only when it fits the step-log quota, under the locks `take_log_prefix` already takes in ADR-0027's order; keyed on the execution the report inserts, so it needs nothing from the runner; a barrier test with an ingest, a delete and a policy change (section 7) |
| A classified version is published without being recorded (an old binary, a rollback) | The record is a database trigger on `workflow_versions`, not code in the binary; the migration records the versions that exist; a superset test that errs closed (G041-010); checked in T041-28 and T041-26 |
| The membership row is locked `FOR SHARE` by every batch, which creates MultiXacts | Heartbeats already make the same read; the MultiXact rate is measured in T041-31 |
| Each line upserts the transaction's `quota_transaction_deltas` row, which bloats while the horizon is held | T041-31 runs one workload with a held transaction and records the table's size and the commit time of other writers |
| Stream and usage rows grow with every logging attempt until feature 020 deletes runs | One small row per attempt and per run, of the order of the `steps` rows that are never purged either; the size per row is measured in T041-9 and stated in the runbook; G041-004 |
| An ordinary 1.7 SDK carries `ResolveDatabase` on the wire | The server refuses it without managed-executor ownership; `step-logs-negotiation` asserts it for a worker and an admin token |
| Viewers lose the script log they read today | OD-2026-11-13-18; the release notes; a viewer test in T041-29 |
| The missing foreign key leaves log rows of a deleted run | Nothing deletes a run today; G041-004 is registered in feature 020 (G1618); T041-26 asserts that every stream's run exists after its scenarios |
| The Studio renders log text as markup, links or reordered text | Text only, no links, escape sequences removed, bidirectional and zero-width characters made visible; tested |
| Purge falls behind ingest | 500 lines per transaction, purge rate against ingest rate measured in T041-9; `SagatStepLogPurgeBehind` |
| The cursor shows the database's transaction counter (G041-008) | Accepted by the owner as stated (OD-2026-11-15-11); no row identity and no tenant's data are in it |
| A viewer watches the step-log quota to learn how much a tenant's runs log | `used` of `tenant_step_log_bytes` is shown only to a caller with `run:logs:read` (D041-066; G041-015 closed) |
| A long transaction of another role holds the read horizon unseen by the alert | `heldBack` on every page; the runbook's `pg_read_all_stats` grant for the gauge (G041-016) |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | A log is not execution state: no transition reads it, and lease, heartbeat and completion run no statement on it (FR-041-008, SC-041-001). The lease and fencing token, its expiry, the session, the binding and the membership decide whether a batch is accepted; a stale attempt is refused, never merged (section 2, steps 2, 3 and 7). The journal and its sequence are unchanged |
| II. Modular domain, ports and adapters | Pass | Domain types and the pure `plan_batch` and `script_marker_decision` in `crates/domain/src/step_logs.rs` with no wire or SQL import; ports `StepLogTx` and `StepLogReader`; scrubbing and sizing in the application; adapters in `infra-postgres`, `api-grpc`, `api-http`. `scripts/check-architecture.py` checks the layering on every pull request, and T041-5 adds two architecture tests (no foreign key to `runs` or `steps`; the hot-path modules never name the log tables) |
| III. External effects are at-least-once; sagas compensate | Pass | Delivery of a batch is at-least-once and made idempotent by (attempt, generation, sequence); nothing here is an external effect |
| IV. Compatibility is versioned and pinned | Pass with conditions | Additive minor 1.7 (ADR-0015; held by 041 in the register of `sdks/SPEC.md` §9, OD-2026-11-15-8); older workers and servers behave as today; four new operations; the six SDKs pass the same five scenarios; the `ScriptExecution` schema change is additive. The condition: viewers get `getScriptExecution` without its `log` (OD-2026-11-13-18), a documented change of behaviour |
| V. Code, tenant boundary and secrets are isolated | Pass with conditions | Tenant in every key and predicate, 404 and audit for another tenant's run, a permission of its own, the PII grant in classified namespaces (recorded by a trigger that errs closed), reads audited, an audited delete. Values the process resolved, written verbatim and at least 6 bytes long, are removed by an official SDK before a line leaves it. Conditions: G041-001, accepted by the owner (O041-011, ADR-0085); and G041-008 (the cursor shows the transaction counter), accepted by the owner (OD-2026-11-15-11). The rows without text are charged 128 bytes each (OD-2026-11-15-6), so no row escapes the step-log quota; the step-log quota's usage is shown only to readers of step logs (D041-066), so G041-015 is closed |
| VI. Observability and audit are part of the feature | Pass | FR-041-018: metrics with closed labels, a span without text, typed errors, alerts on shedding, purge and old transactions, `run.logs_read` and `run.logs_deleted` |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | No broker, inbox or outbox is involved: a batch is a synchronous call whose answer the worker waits for, and it is safe to deliver twice. Backpressure is designed (bounded buffers, admission and tickets, `RETRY_LATER`, counted drops). A dropped line is counted, not parked, so the dead-letter rule has nothing to apply to (ADR-0084 decision 7) |
| VIII. Failure is tested and contracts are verified, honestly | Pass | The Verification Plan names the crash, duplicate, lost-batch, stale and expired lease, revocation, re-registration, flood, old-worker, lock-order, tail-end and restart tests, with barriers and held locks; each is written first and seen failing on its own assertion before the code it tests exists. What is not proven is listed as gaps G041-001 to G041-016 |
| Delivery Constraints | Pass | Rust, tonic, Axum, SQLx, PostgreSQL as the only store; no new component; SC-041-007 is a criterion on a stated machine |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| Condition on principle V: free text written by customer code is stored and may hold a transformed secret | The feature is the storage of that text; the server never holds the secret values | Re-resolving the attempt's secrets on the server for every batch: a disclosure per batch, each to audit |
| Condition on principle IV: viewers get the script execution without its log | The owner closed the script log to callers without the step-log right (OD-2026-11-13-18) | Leaving it readable under `script:read`, which contradicts ADR-0085 rule 6 |
| Condition on principle V: the read cursor shows the database's transaction counter (G041-008) | The read order is the storing transactions' ids; the owner accepted the plain cursor (OD-2026-11-15-11) | A cursor encrypted with a server key, which no other cursor of the API needs (D041-035) |
| Tables with no foreign key to `runs` | A foreign key makes each insert take a lock that conflicts with the engine's run lock | Keeping the convention |
| A new RPC, protocol minor and session token on it | The batch must not share the fate of the heartbeat or the cost of the completion; the session's namespace, minor and subject must be checkable | A field on `Heartbeat` or `CompleteStep`; an RPC with no session |
| A read horizon on transaction ids, and an end horizon in the cursor | A tail over concurrent writers otherwise skips a late commit or ends before it | A per-run sequence taken under the run lock |
| A second server-sent event route | The graph stream has one resume point and a wider audience | Log events on `/runs/{runId}/stream` |
| A stream row per attempt beside the lines | Ceilings and drop counts from counters; the "expired", "deleted" and "no final flush" states | Deriving them from the lines on every batch and read |
| Two permissions and a classified-namespace record kept by a trigger | Logs can hold personal data; viewers hold `run:read`; deletion is an incident act; the grant applies only where classification is used | Reusing `run:read`; requiring the grant everywhere |
| Capture code and new logger calls in six SDKs | Only the worker process holds the secret values and the logger | Capturing the worker's stdout from outside |

## Alternatives Considered

The options the ADRs weighed, with their reasons, are in ADR-0084 ("Considered options", 1 to 10) and ADR-0085 (1 to
11); the owner's rejected options are in "Owner decisions needed". The ones this design weighed on its own:

- **A field on `Heartbeat`, a client stream, or riding on `CompleteStep`** for transport. Rejected: section 1, "Why a
  separate unary RPC".
- **A `DROPPED` record kind** that names the range an SDK lost. Rejected (D041-028): the server already sees every gap
  in the sequences, so the kind only added a record type, a fold step and a "lying range" case in six SDKs.
- **Tuning numbers in the policy** (buffer, flush, batch). Rejected (D041-033): each would be a permanent 1.7 field
  that six SDKs must honour, for a choice nobody needs to make per server.
- **A locking path near the tenant limit** (`sagat_quota_lock_log_usage`, keep what fits). Rejected (D041-029): it holds
  the row a completion needs; a lossy log can drop one counted batch at the limit.
- **A grouped commit on the server** (several requests' batches in one transaction). Rejected as a planned fallback: it
  lengthens each hold of the tenant row while cutting their number. T041-31 decides whether any change is needed.
- **A sealed cursor.** Rejected (D041-035): key management no other cursor of the API needs; the run id in a plain
  cursor is compared instead.
- **Script output through the streams list only, with no marker row.** Proposed by the design (D041-030) and not chosen
  by the owner (OD-2026-11-15-9): the whole-run tab would not show when a script step produced output.
- **Step logs inside `tenant_log_bytes`, stopping at 90% of it.** Proposed by the design and not chosen by the owner
  (OD-2026-11-15-5): a separate quota is exact and leaves the other log writers their whole allowance.
- **Recording the classified namespace in the binary, with a backfill gated by the instance registry.** Rejected
  (D041-037): the registry cannot see binaries older than itself and a requirement can be removed, so a version an old
  binary publishes could stay unrecorded; a database trigger records every insert.
- **Expiry computed at purge time from the retention in force.** Rejected (D041-049): ADR-0027 forbids retroactive
  deletion by a lowered policy; the delete route is the early removal.
- **A column on `namespaces` for the classified flag.** Rejected (D041-037): an `UPDATE` grant on `namespaces` lets the
  runtime role lock any namespace row and clear the flag.
- **Re-authorizing the tail on every poll.** Rejected (D041-041): a membership lock twice a second per tail, where the
  graph stream re-checks every 30 seconds.
- **Capture through a handler on the language's shared logger.** Rejected (ADR-0085 option 5): it sees every attempt's
  lines and redacts them with the wrong secrets.
- **A script marker that carries the runner's iteration**, reported by the runner and verified against a `step.leased`
  event, with a resend without the fields for an older control plane. Rejected (D041-060): `script_executions` keeps one
  record per (run, node, attempt), so the iteration could name only the first report of an attempt, and the runner
  fields, the fallback and the verification cost more than they show; the marker is keyed on the execution.
- **A second transaction after the finalizer's raise**, re-planned with a tenant allowance of 0. Rejected (D041-061):
  it only attributes the drops to `tenant_quota`; `RETRY_LATER` and the resend's own plan do the same with one
  transaction fewer, and a batch that is never stored is still counted as a gap.
- **Sending logs to the OpenTelemetry Collector.** Rejected: no tenant boundary, ceiling or retention the platform can
  enforce. Export is a later feature.

## Non-Functional Requirements

- **NFR-041-PERF (performance)** SC-041-001 by test on every pull request that touches the hot-path files (the selector
  rule of design 038 picks `integration_claim_transition_efficiency`, where the lock tests live); SC-041-007 by T041-31
  before the schema and T041-9 at the end, with the machine, its load and the PostgreSQL configuration stated, in
  `specs/041-step-logs/validation-report.md`. The ingest's statement count is budgeted as an equality.
- **NFR-041-SEC (security)** No grant beyond those of data-model.md §4, no `SECURITY DEFINER` function beyond the
  two accessors of ADR-0027's Amendment, no role membership. The
  cross-tenant battery covers the RPC and the four routes. The canary (SC-041-004) runs in the conformance suite and in
  the server's integration suite. Line text never reaches a metric, span, audit record, error or the platform's log; T041-8
  searches the server's captured output for the canary.
- **NFR-041-OPS (operations)** Additive migrations; the purge lands before capture can be served; the two switches; old
  and new replicas and workers run together; the runbook and the alerts ship with the code.
- **NFR-041-A11Y (accessibility)** WCAG 2.2 AA as the Studio's axe run checks it, reflow at 320 px, keyboard-only journey,
  level by text and shape, no focus theft, announcements throttled (SC-041-006).
- **NFR-041-COST (cost)** The stored size of a line is its message, exception and fields plus a fixed overhead that T041-9
  measures and the runbook states; each row is charged its text plus 128 bytes (OD-2026-11-15-6).

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Six slices. Each task is one pull request, except T041-2 (an owner answer, no code). Each slice leaves `develop`
releasable. A task that builds what an owner answer decided carries an `Owner-gate:` line citing its row of
`docs/owner-decisions.md`; a task that waits for an owner question still open says "Waits for" and gains its
`Owner-gate:` line when the answer is recorded. Task numbers are never reused; T041-28 to T041-30 were added on
2026-11-13 and T041-31 and T041-32 on 2026-11-15, each in the slice it belongs to, and tasks run in the order they are
listed. Each test is written first, in the pull request of the task that builds what it tests, and is seen failing on
its own assertion (not on a compile error or a missing fixture) before the code it tests exists; the pull request names,
for each new test, the assertion that failed first. "The checkers" are `scripts/check-architecture.py`,
`scripts/check-agent-config.py` and `scripts/check-design-docs.py`. Paths are relative to the repository root.

Slice A, decisions and contracts (nothing runs yet):

- [ ] T041-1 Review this design with `egm-review` (its five stages, in order) and the three repository reviewers,
  answer every finding in the design and in `resolution.md` of the review folder, and repeat on the committed text until
  a complete run leaves no Blocker and no Major open. Done when that run's four reports are cited by the approval block
  (Status `Approved`) and the checkers pass.
  Refs: FR-041-001, FR-041-013
- [ ] T041-2 Put O041-014 to O041-024 to the owner and record each answer as a row of `docs/owner-decisions.md` and in
  research.md; correct the ADRs' Revision sections and this design for the answers. Done when every question that is not
  conditional on T041-31 (all but O041-020 and O041-021) has a row with Status `Decided`, and every task that builds one
  of those answers cites its row on an `Owner-gate:` line.
  Refs: FR-041-007, FR-041-008
  Owner-gate: OD-2026-11-15-17
- [ ] T041-31 Spike, before the schema is fixed. A log driver in `tests/performance/scalability-ha/main.rs`, enabled
  only by `SAGAT_PERF_STEP_LOG_SPIKE=1`, works in the benchmark's own disposable database; without the variable the
  benchmark runs as today, and nothing it creates reaches a migration.
  The scratch schema `spike_041` has two variants. The per-line variant copies the DDL of data-model.md §2 for
  `step_log_streams`, `step_log_lines` and `run_log_usage`. The chunk variant has the same streams and usage tables and a
  line table with one row per batch: the stream, the first and last sequence, `stored_xid`, and the batch's records as
  one `text` column of JSON lines. Each line table gets the two triggers ADR-0027 generates, charging its text columns to
  the existing `log` class (`sagat_quota_row_delta('log', 'octet_length_utf8', 'message', …)` and the deferred
  `sagat_quota_flush_deltas`, created as `20261024000000_durable_quotas.up.sql:444-558` creates them), so every commit
  takes a policy fence and the tenant usage row as the `step_log` class will.
  The prototype of the ingest is the SQL of section 2, steps 1 to 10, written in the driver against those tables, with
  the lease, session and authority reads against the real tables, and admission modelled by a semaphore of
  `SAGAT_STEP_LOGS_TENANT_CALLS` tickets per tenant per replica that answers `RETRY_LATER` when none is free. The engine
  completes steps with a 1 KiB output (the harness completes in process with `{}` today, `main.rs:1437-1447`).
  Workloads, each run with both variants: ten 200-byte `info` lines per step in batches of ten, then a final batch
  before the completion; full batches of 500 from the tickets × R concurrent attempts of one tenant (R = 1 and 8); a
  fan-out of 200 attempts of one run, 64 at a time; the first workload again while one transaction stays open for a
  minute. Recorded: SC-041-007's two figures against the same workload with the driver off; the completion's wait on the
  tenant row and on the secrets lock; the share of batches and of final batches answered `RETRY_LATER` and what the
  final flush adds to a completion; the MultiXact rate of the membership row; the size of `quota_transaction_deltas` and
  the commit time of other writers under the held transaction.
  Done when the per-line variant meets SC-041-007's two figures and fewer than 1% of the fan-out's batches meet
  `RETRY_LATER`; the chunk variant is measured only to inform O041-021. The figures, the machine, its load and the
  PostgreSQL configuration go to `specs/041-step-logs/validation-report.md`. If a figure fails, stop: the design changes
  and is reviewed again before T041-5 starts, and only then do O041-020 and O041-021 go to the owner.
  Refs: NFR-041-PERF, SC-041-001, SC-041-007
  Files: modify `tests/performance/scalability-ha/main.rs`; create `specs/041-step-logs/validation-report.md`.
- [ ] T041-3 Contracts. Copy contracts.md §1 into `worker.proto` and its six SDK copies and update `worker.baseline.json`
  after reading the diff; answer `UNIMPLEMENTED` in the seven implementations of section 1; regenerate the Go stubs with
  protoc v7.36.2, protoc-gen-go v1.36.11 and protoc-gen-go-grpc v1.6.1, and the Python stubs with the protobuf runtime
  7.35.1 (the versions the committed stubs record); add the three messages and the `oneof` of section 1 to `skip_debug` in `crates/api-grpc/build.rs`
  and to the prost configuration of `sdks/rust/build.rs` and `sdks/rust/conformance/build.rs`, with `Debug` impls that
  print counts, beside each generated module (section 1). Copy contracts.md §2 into `openapi.yaml`: the operations tagged
  `x-planned: '041'`, the `ScriptExecution`, `Quota` and `Error.details` changes, the six resources in `Quota.resource`
  and `QuotaResourceName`. Add the four operations to the battery's `not_built` list (`battery.rs:584-591`); extend the
  digest test's approved additions with `041`; regenerate `docs/reference/api.md` (`SAGAT_DOCS_BLESS=1`,
  `contract_developer_experience`) and the Studio types, each after reading the diff. Done when the contract checks and
  the gRPC compatibility test pass and nothing new is served.
  Refs: FR-041-004, FR-041-010, FR-041-011, FR-041-012, FR-041-014, FR-041-022, FR-041-023
  Owner-gate: OD-2026-11-15-4, OD-2026-11-15-8, OD-2026-11-15-9, OD-2026-11-15-16
  Files: modify `specs/001-platform-foundation/contracts/{worker.proto,worker.baseline.json,openapi.yaml}`, the six SDK
  proto copies, `crates/api-grpc/build.rs`, `crates/api-grpc/src/{lib.rs,service.rs}`, `sdks/rust/build.rs`,
  `sdks/rust/src/lib.rs`, `sdks/rust/src/worker_tests.rs`, `sdks/rust/conformance/build.rs`,
  `sdks/rust/conformance/src/faults/grpc.rs`, `tests/contract/durable-execution/grpc/limits.rs`,
  `tests/contract/security-multitenancy/grpc/stub.rs`, `tests/integration/database-runner/proxy.rs`,
  `apps/database-runner/src/testing.rs`, `sdks/go/internal/workerpb/*.pb.go`,
  `sdks/python/sagat_octopus_orchestrator/_proto/worker_pb2*.py`, `tests/contract/payload-classification/openapi.rs`,
  `tests/contract/security-multitenancy/rest/battery.rs`, `docs/reference/api.md`, `frontend/apps/studio/src/api/`
  generated types.

Slice B, the server stores:

- [ ] T041-4 Domain and quota resources. `crates/domain/src/step_logs.rs`: the types, `validate_batch` and `plan_batch`,
  with property tests (lines stored + dropped = `last_sequence` over any sequence of plans and refusals). The six quota
  resources of data-model.md §6 (the five namespace resources and the tenant resource `tenant_step_log_bytes` of the
  ADR-0027 Amendment): `QuotaResource` variants, a `minimum()` per resource checked by `validate` and
  `validate_ceiling`, and the one named exception that lets the `Policy` resource `step_log_attempt_records` take 0, with
  tests. A migration that restates `quota_limits_resource_valid`, `quota_ceilings_resource_valid` and
  `quota_limits_tenant_scoped` and adds `tenant_retained_usage.step_log_bytes` (`bigint NOT NULL DEFAULT 0`), so that the
  usage arm of `tenant_step_log_bytes` has a column to read before T041-5 fills it. The exhaustive matches that list every
  resource gain the new ones: the usage of `crates/infra-postgres/src/quotas.rs:1211-1287` (`tenant_step_log_bytes` reads
  the new column; the five namespace resources join the arm that has no scalar usage) and the cases of
  `tests/integration/security-multitenancy/quotas.rs:507-598`. The quota view of `crates/application/src/quotas.rs`
  leaves `used` of `tenant_step_log_bytes` empty for a principal without `run:logs:read` (D041-066). The permissions
  `run:logs:read` and `run:logs:delete` with the role matrix test; `AuditAction::RunLogsRead` and `RunLogsDeleted`, the
  first in `collapses_repeats` and `repeat_window`. Done when the domain's tests, the quota suites of
  `integration_security_multitenancy` and the role matrix test pass.
  Refs: FR-041-002, FR-041-003, FR-041-006, FR-041-013, FR-041-020, FR-041-023
  Owner-gate: OD-2026-11-13-14, OD-2026-11-15-4, OD-2026-11-15-5, OD-2026-11-15-16
  Files: create `crates/domain/src/step_logs.rs` and one migration under `crates/infra-postgres/migrations/`; modify
  `crates/domain/src/{lib.rs,quota.rs,identity.rs,audit.rs}`, `crates/application/src/quotas.rs`,
  `crates/infra-postgres/src/quotas.rs`, `tests/integration/security-multitenancy/quotas.rs`.
- [ ] T041-5 The migration of data-model.md §2 to §4: the tables, indexes and the no-update trigger; the rest of the
  ADR-0027 Amendment's accounting (data-model.md §3: the `step_log_delta`, the class `step_log` and its routing in
  `sagat_quota_row_delta`, the kind `fixed_octets` in `sagat_quota_measure`, `sagat_quota_add_delta` dropped and
  recreated with its fifth argument, the finalizer's loop filter and branch, `step_log_enforcement_enabled`, and the two
  `SECURITY DEFINER` accessors), with the manifest test taught the class and the kind and a test of the new branch; the
  quota triggers on `step_log_lines`; the storage-classification entries with the migration as owner path; the
  `workflow_versions` trigger that records classified namespaces, and the statement that records the existing versions;
  grants; a down that refuses data loss and restores the functions it replaced. It follows `.claude/skills/migration`,
  with the upgrade-boundary oracle that `scripts/check-upgrade-boundary.py` checks; the privilege audit passes, extended
  to the new column and functions. Two architecture tests, each with its row in
  `docs/engineering/architecture-principles.md`: in `step_log_isolation.rs`, no foreign key leads from the log tables to
  `runs` or `steps`; and, in the style of `crates/application/tests/managed_kind_sites.rs`, a source-text test that the
  production code (outside `#[cfg(test)]`) of `crates/application/src/worker.rs`, `crates/application/src/worker/`,
  `crates/application/src/execution*`, `crates/infra-postgres/src/execution.rs` and `crates/infra-postgres/src/execution/`
  never names `step_log_lines`, `step_log_streams`, `run_log_usage` or `StepLogTx` (ADR-0084 decisions 1 and 2). Done
  when the migration's up and down tests, the manifest test, the privilege audit and the two architecture tests pass.
  Refs: FR-041-008, FR-041-021
  Owner-gate: OD-2026-11-13-14, OD-2026-11-13-18, OD-2026-11-15-4, OD-2026-11-15-5, OD-2026-11-15-6, OD-2026-11-15-9, OD-2026-11-15-16
  Files: create a migration (`.up.sql`, `.down.sql`), `tests/integration/claim-transition-efficiency/step_log_isolation.rs`,
  `crates/application/tests/step_log_sites.rs`; modify `crates/infra-postgres/src/identity.rs` (the typed refusal of
  `tenant_step_log_quota_exceeded` in `classify`, `:787-791`), `crates/infra-postgres/src/content_usage.rs`
  (`retained_usage`, `:146-173`), `crates/application/src/quotas.rs` (`RetainedUsage`, `:33-38`),
  `crates/infra-postgres/storage-classification.json`, `tests/integration/security-multitenancy/durable_quotas.rs`,
  `tests/integration/platform-foundation/runtime_privileges.rs`, `tests/integration/claim-transition-efficiency/main.rs`,
  `docs/engineering/architecture-principles.md`.
- [ ] T041-6 The ingest of section 2.
  Builds: `sagat_application::step_logs` (the checks before the transaction, scrub, cut, size, the control flow of
  section 2); `StepLogTx` and its adapter; the two public scrubber entry points; the refused-roles argument of the free
  function `current_worker_authorized`; `WorkerStore::supports_step_log_protocol` and its implementation
  (`crates/infra-postgres/src/workers.rs:199-206`); `store_error_without_text` on every statement that binds line text;
  the statement budget of the ingest as an equality; the `integration_step_logs` target (`tests/Cargo.toml`); and, in
  `tests/support/step_logs.rs`, a session fixture (`worker_sessions` rows inserted at minor 7, since the server
  negotiates 7 only from T041-7) and a `StepLogTx` wrapper with three hooks: fail before the commit, commit and then
  fail as a lost answer would, and hold after the quota flush (`SET CONSTRAINTS ALL IMMEDIATE`, with the transaction's
  lock timeout raised to 60 s) until the test releases it. Its selector rule is T041-32.
  Tests, in `tests/integration/claim-transition-efficiency/step_log_isolation.rs`: the lock-free test of SC-041-001; the
  barrier tests of SC-041-001: one ingest held by the wrapper after its flush, K − 1 ingests seen blocked by it on the
  tenant usage row (`pg_blocking_pids`), then a completion with output of another run of the tenant, through the path
  `work_budget.rs` drives, seen blocked; the test commits the ingests one at a time and asserts after each commit that
  the completion is still blocked, and only by ingest backends, and that it commits after the K-th; the same with one
  ingest held after step 7 and a `lock_secrets` waiter queued behind it; `work_budget.rs` unmodified; the lock-order
  test with an ingest, a report and a binding change (section 2).
  Tests, in `integration_step_logs`: the session cases (expired, other namespace, other subject, below 1.7, replaced
  token); the authority cases (binding removed, membership revoked, runner, database-runner), each answered `AUTHORITY`
  even where another answer was due; a lease sent with another run's id; duplicates; gaps; a late batch after a newer
  one; the tenant limit by the unlocked read (`FINAL_ONLY`, counted) and by the finalizer's raise (`RETRY_LATER`, nothing
  counted, then the resend's `FINAL_ONLY`), with a gap in the batch and a final batch after it, asserting the counts;
  capture off in the namespace (no row written). Done when these pass, also as the runtime role.
  Refs: FR-041-002, FR-041-004, FR-041-006, FR-041-007, FR-041-008, SC-041-001
  Owner-gate: OD-2026-11-13-14, OD-2026-11-15-4, OD-2026-11-15-5, OD-2026-11-15-6, OD-2026-11-15-16
  Files: create `crates/application/src/step_logs.rs`, `crates/infra-postgres/src/step_logs.rs`,
  `tests/integration/step-logs/main.rs` and its modules, `tests/support/step_logs.rs`; modify
  `crates/application/src/{lib.rs,worker.rs}`, `crates/infra-postgres/src/{lib.rs,execution.rs,execution/path.rs,workers.rs}`,
  `crates/telemetry/src/redact.rs`, `crates/domain/src/scripts.rs`, `crates/infra-postgres/storage-classification.json`
  (owner path), `tests/Cargo.toml`, `tests/support/mod.rs`, `tests/integration/claim-transition-efficiency/step_log_isolation.rs`.
- [ ] T041-10 Retention and deletion storage: the two moves of section 4 (mark by `expires_at`, then delete 500 lines per
  transaction, `purged_at` kept), the purge metrics; tests that a purge releases the quota bytes, that a lowered retention
  leaves existing streams' expiry as it was and applies to new streams, that an idle stream whose lease lives is marked
  and its next batch is `REMOVED`, that a batch held before step 4 while the purge marks the stream stores nothing, and
  that a stream being deleted over several transactions is shown as expired with no line from the first of them; the
  purge joins the lock-order test of T041-6. The runbook `docs/runbooks/step-logs.md` is created here with the purge,
  the retention rule and the switches. Done when these tests pass, also as the runtime role.
  Refs: FR-041-009, FR-041-018, FR-041-020
  Owner-gate: OD-2026-11-13-14, OD-2026-11-15-4
  Files: modify `crates/scheduler/src/passes.rs`, `crates/application/src/maintenance.rs`,
  `crates/infra-postgres/src/{maintenance.rs,step_logs.rs}`, `crates/telemetry/src/metrics.rs`,
  `tests/integration/claim-transition-efficiency/step_log_isolation.rs`; create `docs/runbooks/step-logs.md`; tests in
  `tests/integration/step-logs/`.
- [ ] T041-7 The gRPC handler, in `crates/api-grpc/src/step_logs.rs`, called from `WorkerGrpc` in `service.rs`;
  `Class::Logs` and the per-tenant tickets with their configuration keys and tests; minor 1.7 in
  `negotiate_protocol_details` and `protocol_ceiling` (6 kept for the database runner) with the ceiling tests; the policy
  in `RegisterWorkerResponse`; `SAGAT_STEP_LOGS_ENABLED` (default true) in the execution engine; the metrics and the span;
  `sdks/SPEC.md` §7.1 (managed database operations) and the register of minors in §9 (O041-018); the golden
  `tests/contract/sdk-conformance/expected/protocol-negotiation.json` updated after reading its diff. Done when the
  ceiling, admission and configuration tests and `sdk_conformance` with `protocol-negotiation` pass.
  Refs: FR-041-001, FR-041-004, FR-041-005, FR-041-014, FR-041-018
  Owner-gate: OD-2026-11-13-14, OD-2026-11-15-4, OD-2026-11-15-8
  Files: create `crates/api-grpc/src/step_logs.rs`; modify `crates/api-grpc/src/{lib.rs,service.rs,limits.rs}`,
  `crates/application/src/worker.rs`, `apps/execution-engine/src/{cli.rs,settings.rs}`,
  `apps/execution-engine/tests/config.rs`, `crates/telemetry/src/metrics.rs`, `sdks/SPEC.md`, the golden.
- [ ] T041-8 Server fault and security tests with barriers and held locks. Crash points use the `StepLogTx` wrapper of
  T041-6, driving the application's ingest. A lost response through the RPC uses the gRPC proxy of
  `tests/integration/database-runner/proxy.rs`, which this task moves to `tests/support/worker_proxy.rs` (the
  database-runner target declares it from there) and teaches a `lose_append_ack` fault that forwards `AppendStepLogs`
  and drops its answer. Cases: crash before the commit; crash after the commit and before the answer; the same batch
  twice at once (`RETRY_LATER`, then a resend that stores nothing); a stale lease; an expired lease not yet reclaimed; a
  batch after the final one; a batch for another tenant's run (audited, the audit kept after the rollback); two attempts
  of one run and two runs of one tenant at the run and tenant ceilings at once; a value that redaction lengthens at the
  tenant boundary; a client that ignores every verdict (stored bytes stay at the ceilings; another tenant's completion
  latency recorded); a restart of the API in the middle of an attempt (the attempt's next batch is stored, no
  duplicate); a database error forced on a statement that binds line text; and the server canary: the suite's captured
  `tracing` output holds no canary. Done when these pass, also as the runtime role.
  Refs: NFR-041-SEC, SC-041-002, SC-041-003
  Files: create modules under `tests/integration/step-logs/` and `tests/support/worker_proxy.rs` (moved from
  `tests/integration/database-runner/proxy.rs`); modify `tests/integration/database-runner/main.rs`,
  `tests/support/mod.rs`.

Slice C, the server serves:

- [ ] T041-28 The classified-namespace record of section 3 is created by T041-5 and read by `step_logs::may_read`
  (T041-11); this task adds its tests:
  a version with one annotation records its namespace and one without does not, through both insert sites and through a
  plain `INSERT` as an old binary would make it; on the classification fixtures and on versions with
  `classification_rules = 0`, every version the publication analysis finds classified records its namespace; the
  migration's statement records the existing versions; the trigger is enabled `ALWAYS` (`pg_trigger.tgenabled = 'A'`)
  and fires under `session_replication_role = replica`; a temporary table named `classified_namespaces` does not capture
  the record; the runtime role cannot delete or update a row. Done when these pass.
  Refs: FR-041-021
  Owner-gate: OD-2026-11-13-18, OD-2026-11-15-4
  Files: tests in `tests/integration/step-logs/` and `tests/integration/security-multitenancy/`.
- [ ] T041-11 `StepLogReader`, `listRunLogs` and `listRunLogStreams` for `source = worker` (the `source = script` items
  are T041-13's), in `crates/api-http/src/routes/step_logs.rs`, switched to `x-implemented: '041'`, removed from the
  battery's `not_built` list and added to its `body_of`: the horizon, `heldBack`, the stream states of section 5, the
  cursor, the end rule of D1958 for `complete` (its end horizon taken in a transaction of its own), the filters, the
  `[RunRead, RunLogsRead]` admission, `step_logs::may_read` (the PII grant in a classified namespace), the throttled audit
  (with a race of two first reads that writes one record), `SAGAT_STEP_LOGS_READ_ENABLED` in the control plane's
  configuration with its test, `STEP_LOGS_NOT_AVAILABLE` in the remediation table, the gauge
  `database_oldest_transaction_age_seconds` (section 9), the cross-tenant battery and the runtime-role run
  (`SAGAT_TEST_AS_RUNTIME_ROLE=1`); held-transaction tests on PostgreSQL 16 and 18, each on a server of its own: the test
  support gains `isolated_server(tag)`, which starts a PostgreSQL container that no other test of the process shares (as
  `start_server` does, `tests/support/mod.rs:136-176`), and `fresh_db_on(&server)`, the tag being `16-alpine` or the
  value of `SAGAT_TEST_POSTGRES_TAG`; a CI step in `.github/workflows/ci.yml` runs them with `18-alpine` when
  `integration_step_logs` is selected; `docs/reference/api.md` regenerated after reading the diff. Done when these tests
  pass on both versions.
  Refs: FR-041-002, FR-041-010, FR-041-011, FR-041-013
  Owner-gate: OD-2026-11-13-14, OD-2026-11-13-18, OD-2026-11-15-4, OD-2026-11-15-11
  Files: create `crates/api-http/src/routes/step_logs.rs`; modify `crates/api-http/src/routes/mod.rs`,
  `crates/api-http/src/error.rs`, `crates/application/src/{api_admission.rs,step_logs.rs}`,
  `crates/infra-postgres/src/{step_logs.rs,operations.rs}`, `crates/telemetry/src/metrics.rs`, `openapi.yaml`,
  `tests/contract/security-multitenancy/rest/battery.rs`, `tests/support/mod.rs`, `.github/workflows/ci.yml`,
  `apps/control-plane/src/{config.rs,cli.rs}`, `apps/control-plane/tests/config.rs`, `docs/reference/api.md`.
- [ ] T041-12 `streamRunLogs`, switched to `x-implemented: '041'` and removed from `not_built`: the `follow` loop of
  `crates/api-http/src/routes/execution.rs:1340` shared with the graph stream, events, resume, `progress`, reconnect,
  caps shared with the graph stream, the re-check every 30 s, and the end rule built in T041-11; tests that a reconnect
  neither skips nor repeats, that an ingest transaction held across the run's terminal transition is delivered before
  `end`, that an ingest with the largest id below `end` held open keeps the tail open until it ends, that a first read of
  a new audit window that sees the run finished does not end early, and that a forged `end` for a running run is
  ignored; a contract test that the graph stream emits only `graph`, `reconnect` and `end` (ADR-0084 decision 10);
  `docs/reference/api.md` regenerated. Done when these pass.
  Refs: FR-041-012
  Owner-gate: OD-2026-11-13-14, OD-2026-11-13-18, OD-2026-11-15-4
  Files: modify `crates/api-http/src/{routes/step_logs.rs,routes/execution.rs,streams.rs}`, `openapi.yaml`,
  `tests/contract/security-multitenancy/rest/battery.rs`, `docs/reference/api.md`; tests in `tests/integration/step-logs/`.
- [ ] T041-13 Script steps (section 7): the `source = script` items of `listRunLogStreams`, through `step_logs::may_read`
  of T041-11, with a test that a viewer and a reader without the grant see no script item; the marker write in
  `record_execution` in the order of section 7, keyed on the execution; `script_marker_decision`; a variant of
  `take_log_prefix` that also takes the step-log fence and locks through `sagat_quota_lock_step_log_usage`, used by
  `record_execution` only (the present callers, `dispatch.rs:219`, `connectors.rs:626` and `scripts.rs:1161-1855`, are
  unchanged); `SAGAT_STEP_LOGS_ENABLED` read by the control plane, with its test; the metric
  `step_log_script_markers_total`; the marker joins the lock-order test; the tests of section 7. Done when these pass,
  also as the runtime role.
  Refs: FR-041-016
  Owner-gate: OD-2026-11-13-14, OD-2026-11-15-9, OD-2026-11-15-16
  Files: modify `crates/domain/src/step_logs.rs`, `crates/infra-postgres/src/{step_logs.rs,scripts.rs,content_usage.rs}`,
  `crates/application/src/{step_logs.rs,scripts.rs}`, `crates/api-http/src/routes/step_logs.rs`,
  `crates/telemetry/src/metrics.rs`, `apps/control-plane/src/{config.rs,cli.rs}`, `apps/control-plane/tests/config.rs`,
  `tests/integration/claim-transition-efficiency/step_log_isolation.rs`; tests in `tests/integration/step-logs/` and
  `tests/integration/inline-code-sandbox/`.
- [ ] T041-29 `getScriptExecution` and `listScriptExecutions` withhold the log of FR-041-022 (`log` empty, `logTruncated`
  false, `logWithheld: true`) through `step_logs::may_read` of T041-11, called in the route's read transaction for each
  execution of a run; when a log is returned, the throttled `run.logs_read` record is written in that transaction first;
  the descriptions, the digest's comment and `docs/reference/api.md` changed after reading the diff; a viewer test, a
  test that the read switch does not change the answer and a test that a returned log is audited; the release-note
  line. Done when these pass.
  Refs: FR-041-022
  Owner-gate: OD-2026-11-13-18, OD-2026-11-15-4
  Files: modify `crates/api-http/src/{routes/scripts.rs,dto_scripts.rs}`, `crates/application/src/scripts.rs`,
  `openapi.yaml`, `tests/contract/payload-classification/openapi.rs`, `docs/reference/api.md`,
  `docs/releases/unreleased.md`.
- [ ] T041-30 `deleteRunLogs`, switched to `x-implemented: '041'` and removed from `not_built`: `run:logs:delete`
  (`ApiOperation::RunLogsDelete`), `run.logs_deleted` in the same transaction, streams locked in `stream_id` order,
  `204` with no body, readers skipping deleted streams at once, `REMOVED` for a deleted stream, a `stream` of another run
  answered 404; the delete joins the lock-order test; the incident path in the runbook; `docs/reference/api.md`
  regenerated after reading the diff. Done when these pass, also as the runtime role.
  Refs: FR-041-023
  Owner-gate: OD-2026-11-15-4
  Files: modify `crates/api-http/src/routes/step_logs.rs`, `crates/application/src/{api_admission.rs,step_logs.rs}`,
  `crates/infra-postgres/src/step_logs.rs`, `openapi.yaml`, `tests/contract/security-multitenancy/rest/battery.rs`,
  `tests/integration/claim-transition-efficiency/step_log_isolation.rs`, `docs/reference/api.md`,
  `docs/runbooks/step-logs.md`.
- [ ] T041-32 The CI gate of constitution 4.5.0 (OD-2026-11-15-10), landed after T041-6 (which creates the target) and
  before T041-7. A `HARNESSES` entry of `scripts/select-test-targets.py` selects `integration_step_logs` and
  `integration_claim_transition_efficiency` for the server's step-log modules, matched by path
  (`crates/[a-z-]+/src/(.*/)?step_logs(\.rs$|/)`, which covers the domain, application, adapter, gRPC and HTTP modules
  of D041-063, and `crates/api-grpc/src/limits\.rs`); another selects `integration_step_logs` and `sdk_conformance` for
  `specs/001-platform-foundation/contracts/worker.proto`, carved out of `SHARED_INPUTS` (`select-test-targets.py:79`),
  and for the six capture files the SDK tasks create (`sdks/typescript/src/worker/step-logs.ts`,
  `sdks/go/worker/steplogs.go`, `sdks/python/sagat_octopus_orchestrator/worker/_step_logs.py`,
  `sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/Workers/StepLogCapture.cs`, `StepLogCapture.java` under
  `sdks/java/sdk/src/main/`, `sdks/rust/src/step_logs.rs`); its cases in `scripts/tests/test_select_test_targets.py`;
  `docs/test-strategy.md`, `tests/AGENTS.md` and `scripts/README.md`. Done when the selector's tests pass.
  Refs: NFR-041-PERF, SC-041-001
  Owner-gate: OD-2026-11-15-10
  Files: modify `scripts/select-test-targets.py`, `scripts/tests/test_select_test_targets.py`, `docs/test-strategy.md`,
  `tests/AGENTS.md`, `scripts/README.md`.

Slice D, the SDKs (one pull request per SDK after the specification; they can be built in parallel). Each SDK task
builds the row of section 6 for its language, its buffer, sender and final batch, its protocol constant at 1.7, and, in
its conformance runner, the behaviours `logs` and `logsSecret`, the `stepLogs` op and the `verdict` fault; it adds the
micro-benchmark of section 6. Each is done when the five scenarios pass in that SDK
(`SAGAT_SDK_SCENARIO=step-logs-<name> cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored`), the SDK's
own checks of `sdks/AGENTS.md` pass, and the micro-benchmark meets its bound.

- [ ] T041-14 `sdks/SPEC.md`: the logger's semantics, the two tables of section 6, the capture section (per-attempt
  object, the secret set, keys, numbers, exceptions, language context, the answers of contracts.md §4), §9 for 1.7, the
  two headings numbered 10 renumbered; the harness additions of section 6 (the scenario schema of the behaviours and of
  the `stepLogs` op, which each SDK runner executes with a plain HTTP request, and `mustNotStore` in the harness); the
  five scenarios and their goldens. Until an SDK's task lands, its runner refuses the new behaviours and op as unknown,
  and that is the failure each scenario shows first. Done when the harness's own tests pass and each scenario fails in
  every SDK for that reason.
  Refs: FR-041-007, FR-041-015, SC-041-005
  Owner-gate: OD-2026-11-15-4, OD-2026-11-15-7
  Files: modify `sdks/SPEC.md`, `tests/contract/sdk-conformance/{harness.rs,normalize.rs,README.md}`; create five
  scenario files and their goldens.
- [ ] T041-15 TypeScript: the row of section 6, the protocol constant to 1.7 (`src/config.ts:48`).
  Refs: FR-041-005, FR-041-007, FR-041-015, SC-041-004
  Owner-gate: OD-2026-11-15-7
  Files: modify `sdks/typescript/src/worker/{activity.ts,host.ts}`, `sdks/typescript/src/config.ts`, `sdks/typescript/conformance/{faults.ts,runner.ts}`; create `sdks/typescript/src/worker/step-logs.ts`, its tests and its micro-benchmark.
- [ ] T041-16 Go: the row of section 6, `ProtocolVersion` to 1.7 (`version.go:9`).
  Refs: FR-041-005, FR-041-007, FR-041-015, SC-041-004
  Owner-gate: OD-2026-11-15-7
  Files: modify `sdks/go/worker/{activity.go,task.go,host.go}`, `sdks/go/version.go`, `sdks/go/conformance/{faults.go,run.go,worker.go}`; create `sdks/go/worker/steplogs.go`, its tests and its benchmark.
- [ ] T041-17 Python: the row of section 6, `PROTOCOL_VERSION` to 1.7 (`config.py:25`).
  Refs: FR-041-005, FR-041-007, FR-041-015, SC-041-004
  Owner-gate: OD-2026-11-15-7
  Files: modify `sdks/python/sagat_octopus_orchestrator/worker/{host.py,activity.py}`, `sdks/python/sagat_octopus_orchestrator/config.py`, `sdks/python/conformance/{faults.py,runner.py}`; create `sdks/python/sagat_octopus_orchestrator/worker/_step_logs.py`, its tests and its micro-benchmark.
- [ ] T041-18 C#: the row of section 6, `SdkInfo.ProtocolVersion` to 1.7 (`SdkInfo.cs:12`).
  Refs: FR-041-005, FR-041-007, FR-041-015, SC-041-004
  Owner-gate: OD-2026-11-15-7
  Files: modify `sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/{Workers/WorkerHost.cs,Workers/ActivityExecutionContext.cs,Internal/Safe.cs,SagatOptions.cs,SdkInfo.cs}`, `sdks/dotnet/conformance/{FaultPlan.cs,RunMode.cs,Behaviors.cs}`; create `Workers/StepLogCapture.cs`, its tests and its micro-benchmark.
- [ ] T041-19 Java: the row of section 6, `SagatSdk.PROTOCOL_VERSION` to 1.7 (`SagatSdk.java:12`).
  Refs: FR-041-005, FR-041-007, FR-041-015, SC-041-004
  Owner-gate: OD-2026-11-15-7
  Files: modify `sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/worker/{ActivityLogger.java,WorkerHost.java,WorkerConfig.java}`, `sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/{Hooks.java,SagatSdk.java}`, the conformance `FaultPlan.java`, `Runner.java` and `Behaviors.java`; create `StepLogCapture.java`, its tests and its micro-benchmark.
- [ ] T041-20 Rust: `ActivityContext::log()` and the row of section 6, the default `protocol_version` to 1.7
  (`src/worker.rs:58`) and the refusal of an accepted minor above 6 (`src/worker.rs:468`) raised to 7;
  `Cargo.lock`, `sdks/rust/Cargo.lock`, `sdks/rust/conformance/Cargo.lock` and `sdks/rust/consumer/Cargo.lock` updated in
  the same pull request.
  Refs: FR-041-005, FR-041-007, FR-041-015, SC-041-004
  Owner-gate: OD-2026-11-15-7
  Files: modify `sdks/rust/src/{lib.rs,activity.rs,worker.rs,capabilities.rs}`, `sdks/rust/conformance/src/{faults.rs,runtime.rs}`, the four lock files; create `sdks/rust/src/step_logs.rs`, its tests and its benchmark.
- [ ] T041-21 Run the five scenarios in the six SDKs on the merged tree and record the matrix in the validation report;
  the counts of `step-logs-limits` add up in each. Done when the matrix is all green and recorded.
  Refs: SC-041-002, SC-041-004, SC-041-005
  Owner-gate: OD-2026-11-13-14
  Files: modify `specs/041-step-logs/validation-report.md`.

Slice E, the Studio:

- [ ] T041-22 `LogView`, the Logs tab and the step section over the list and streams routes: filters, search, the states of
  FR-041-017 (the missing permission from the session, the missing grant from the refusal), markers, windowing, text-only
  rendering without links and with bidirectional and zero-width characters made visible; unit tests. Done when the
  Studio's unit tests and `npm run check:api` pass.
  Refs: FR-041-017, FR-041-019, FR-041-022
  Owner-gate: OD-2026-11-13-18, OD-2026-11-15-9
  Files: create `frontend/apps/studio/src/features/runs/LogView.tsx` and its tests; modify `frontend/apps/studio/src/features/runs/{RunDetailPage.tsx,NodePanel.tsx}`.
- [ ] T041-23 The tail (`followLogs`, pause and resume, announcements, the polling fallback that stops on `complete`),
  with a test that a dropped connection neither skips nor repeats. Done when the Studio's unit tests pass.
  Refs: FR-041-012, FR-041-017
  Files: modify `frontend/apps/studio/src/api/follow.ts`, `frontend/apps/studio/src/features/runs/LogView.tsx`; tests.
- [ ] T041-24 Accessibility and journey: the axe run and reflow in `a11y.spec.ts`, a keyboard-only journey, and the
  browser journey of `e2e_workflow_studio` (`tests/e2e/workflow-studio/`) for CUJ 1, CUJ 2 and CUJ 5. Done when the axe
  run and both journeys pass.
  Refs: SC-041-006, NFR-041-A11Y
  Files: modify `frontend/e2e/tests/a11y.spec.ts`; create a journey module under `tests/e2e/workflow-studio/`.

Slice F, measure, operate and close:

- [ ] T041-9 Measure through the RPC. A new `#[ignore]` target `perf_step_logs` starts the platform with
  `platform_with` of `tests/support/worker.rs` and drives the workers of the same module (`Platform::worker`), which this
  task teaches to send `AppendStepLogs` by the rules of section 6 (a batch every second or at 256 records, one in
  flight, a final batch before the completion), through the transitions workload of `scalability-ha` with ten 200-byte
  `info` lines per step; the baseline is the same workload with `SAGAT_STEP_LOGS_ENABLED=false`, as in SC-041-007. It
  records SC-041-007's two figures, the trigger cost and commit latency of a 500-record batch, the purge rate against
  the ingest rate and the completion p99 during a purge, the added time of the final flush, and the stored size per line
  and per stream and usage row, in the dated section of the validation report. Done when the figures are recorded; a
  failure of SC-041-007 stops the release of the feature for a design change.
  Refs: NFR-041-COST, NFR-041-PERF, SC-041-007
  Files: create `tests/performance/step-logs/main.rs` and its `[[test]]` entry in `tests/Cargo.toml`; modify
  `tests/support/worker.rs`, `specs/041-step-logs/validation-report.md`.
- [ ] T041-25 Alerts with their runbook entries, the runbook `docs/runbooks/step-logs.md` completed, the reference
  documentation (generated files regenerated after reading the diff), `docs/architecture/overview.md`,
  `docs/traceability.md`, a quickstart `docs/quickstarts/step-logs.md` with the happy and the failure scenario, and the release notes, whose first
  line on this feature says that capture is on by default (O041-001) and which say that viewers no longer read script
  logs. Done when the alert rules' check and the documentation build pass.
  Refs: FR-041-018, FR-041-022, NFR-041-OPS
  Owner-gate: OD-2026-11-13-14, OD-2026-11-13-18
  Files: modify `infra/observability/alerts.yaml`, `docs/runbooks/alerts.md`, `docs/runbooks/step-logs.md`, `docs/architecture/overview.md`, `docs/traceability.md`, `docs/reference/` (generated), `docs/releases/unreleased.md`, `docs/quickstarts/README.md`; create `docs/quickstarts/step-logs.md`.
- [ ] T041-26 End-to-end lineage, switches and mixed versions. (a) A new target `e2e_step_logs`
  (`tests/e2e/step-logs/main.rs`) on the platform of `tests/support/worker.rs`, expecting: a retry leaves one attempt
  log per attempt; a reset, a rerun and a continue-as-new each start a new run with no log while the source keeps its
  own; a worker that offers 1.5 beside one that offers 1.7 (`Worker::register_version`, `tests/support/worker.rs:369-372`): the first is
  accepted at 1.5 and stores nothing, the second stores its lines; `SAGAT_STEP_LOGS_ENABLED=false` answers
  `enabled = false` at registration, `STOP_CAPTURE` (`CAPTURE_OFF`) to a batch, and no script marker;
  `SAGAT_STEP_LOGS_READ_ENABLED=false` answers the read routes `422 STEP_LOGS_NOT_AVAILABLE` while capture goes on; at
  the end every stream's run exists. (b) An older API replica: a scenario `step-logs-mixed-version` of
  `scripts/test-release-upgrade-profile.py`, whose released phase runs the v0.2.0 tree (`RELEASE`) against the migrated
  schema, expecting: a session accepted at 1.7 by the current replica gets `UNIMPLEMENTED` from the released replica's
  `AppendStepLogs`; a version with a classification annotation published by the released control plane records its
  namespace; the released replica's quota listing is refused once a step-log quota row exists (section 10). Done when
  both pass.
  Refs: FR-041-014, FR-041-019, FR-041-021, SC-041-003
  Files: create `tests/e2e/step-logs/main.rs` and its `[[test]]` entry in `tests/Cargo.toml`,
  `tests/integration/workflow-primitives/release_upgrade_step_logs.rs`, `tests/fixtures/release-upgrade/step_logs.rs`;
  modify `scripts/test-release-upgrade-profile.py`, `.github/workflows/ci.yml` (the scenario list of `released-upgrade`).
- [ ] T041-27 Review of the merged slices B to E by the `security-reviewer` and `correctness-reviewer` agents, each
  finding proven by a failing test before it is fixed, and the drift report of `egm-drift-report` against this design,
  written to `docs/designs/041-step-logs.reviews/<date>/`. Done when every finding has an answer and every fix its test.
  Refs: NFR-041-SEC, SC-041-004
  Files: create `docs/designs/041-step-logs.reviews/<date>/{security,correctness,drift}.md`.

#### Follow-up tasks

None yet.

### Verification Plan

Every pull request runs `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the
checkers, the suites the selector picks and, for slices B and C, the same suites as the runtime role
(`SAGAT_TEST_AS_RUNTIME_ROLE=1`). The feature's integration suite is a new target of `sagat-tests`,
`integration_step_logs`, with real PostgreSQL, picked by the selector rule of T041-32 (constitution 4.5.0); the tests
that guard the hot path
live in `tests/integration/claim-transition-efficiency/`, which `scripts/select-test-targets.py:71` selects for every
change under `crates/infra-postgres/` and the worker and execution modules. Races use barriers or held locks, never
sleeps.

| What is proven | How |
|---|---|
| The hot path is unchanged (SC-041-001) | `work_budget.rs` with its constants unmodified while ingest runs between its windows; a batch commits under `lock_timeout = 50ms` while another transaction holds the run row `FOR UPDATE` and has updated the leased step row; a source-text test that the worker and execution modules never name the log tables |
| What a completion waits for (SC-041-001) | One ingest of a tenant held by the test wrapper after `SET CONSTRAINTS ALL IMMEDIATE` (it holds the usage row), K − 1 more seen blocked behind it with their lock timeout raised, then a completion with output of the same tenant seen blocked; the ingests are committed one at a time, and after each the completion is still blocked, only by ingest backends (`pg_blocking_pids`), until it commits after the K-th; the same with one ingest held after step 7 and a `lock_secrets` waiter queued behind it. The time bound is SC-041-007's |
| Crash between accept and store | The `StepLogTx` wrapper fails before the commit: nothing stored, the resend stores all, once |
| Crash between store and answer | The wrapper commits and then fails, and the proxy of `tests/support/worker_proxy.rs` drops the answer of `AppendStepLogs`: the resend stores nothing and gets `CONTINUE` (`FINAL_RECORDED` for a final batch) |
| Duplicate batch | The same batch twice at a barrier: the second waits on the stream row and gets `RETRY_LATER` at the lock timeout; its resend stores nothing; counters rise once |
| Gaps and late batches | A batch the SDK gave up on is counted as dropped by the worker once; a late batch after a newer one stores nothing and does not lower `last_sequence`; a final batch's `highest_sequence` counts the drops at the end |
| A worker that floods (SC-041-002) | `step-logs-limits` for the SDKs; a client that ignores verdicts for the server: stored bytes stay at the ceilings, tickets hold, another tenant's completion latency is recorded; lines stored + dropped = highest sequence |
| Ceilings under concurrency | Two attempts of one run at the run ceiling, and two runs of one tenant at the tenant limit, at a barrier: the ceilings hold exactly; the finalizer's raise answers `RETRY_LATER` and stores and counts nothing, and the resend, with a gap in the batch and a final batch after it, keeps lines stored + dropped = highest sequence; a value that redaction lengthens is charged at its stored size |
| Revocation, session and fence | Binding removed, membership revoked, lease expired but not reclaimed, run stopping, runner and database-runner tokens, an expired session, a session of another namespace or subject or below 1.7, a replaced token, a valid lease sent with another run's id: each stores nothing and gets its verdict; a revoked caller gets `AUTHORITY` whatever else was due |
| An old worker, an old server | `step-logs-negotiation`; an API replica without the RPC beside one with it (T041-26) |
| Redelivery does not duplicate lines | A task delivered again under a new generation writes a new stream; the same generation's resend inserts nothing; rebuilding the projections of a run changes no log row |
| Reset, rerun, continue-as-new, retry | `e2e_step_logs` (T041-26): each new run starts with no log and the source keeps its own |
| Read horizon and end | Transaction A stays open, B stores a line and commits: the line waits until A ends, then both arrive in cursor order and `heldBack` was true meanwhile; an ingest held across the terminal transition is delivered before `end`; an ingest with the largest id below `end` held open keeps the tail open; a first read of a new audit window that sees the run finished does not end early; a forged `end` for a running run is ignored; a reconnect at any point neither skips nor repeats; on PostgreSQL 16 and 18, each in a container of its own |
| Lock order | An ingest, a report of the same namespace and a binding change (T041-6), then a purge (T041-10), a script marker (T041-13) and a delete (T041-30), at barriers, all commit; no deadlock is reported |
| Redaction (SC-041-004) | `step-logs-redaction` in six SDKs; server tests for each shape of both scrubbers and each sensitive field name, of any value type; the canary is searched for in the stored rows, the routes' bodies, `run_events`, telemetry, `audit_records` and the captured process output |
| Tenant boundary | The cross-tenant battery on the RPC and the four routes: 404, `access.denied.cross_tenant`, no count or size disclosed; a cursor or `stream` of another run is refused |
| Permission, grant and audit | A viewer is refused on the read routes and gets the script execution without its log; a developer without the grant is refused in a classified namespace (recorded by the trigger, also for a version inserted as an old binary would) and allowed in another; two first reads in a window write one `run.logs_read` before returning, and a failure to write it returns no line; only admin may delete, and the delete is audited |
| Retention | A stream past its expiry is marked, then purged 500 lines at a time and shown as expired from the first of them; quota bytes return; the stream row stays with `purged_at`; a lowered retention leaves existing streams' expiry as it was; a batch racing the mark stores nothing |
| Script marker (OD-2026-11-15-9) | The tests of section 7 (T041-13): one marker per inserted execution, with its node and attempt; one per (run, node, attempt) with two iterations; none for a repeat, a test run, a finished run, capture off or the switch off; the record kept at the step-log limit; the marker found by the execution id; no deadlock with an ingest, a delete or a policy change; delivered before `end` |
| Separate step-log quota (OD-2026-11-15-5, -6) | A tenant at its `tenant_step_log_bytes` limit stores no step-log line while a script log, a dispatch error and a connector error are stored in full under `tenant_log_bytes`; an empty line is charged 128 bytes; the finalizer's new branch raises at the limit (T041-5, T041-6) |
| Runtime privileges and storage classification | The privilege audit and manifest tests pass with the new statements and columns |
| Studio (SC-041-006) | Unit tests of the states and markers; the axe run; the keyboard journey; a line holding markup, a URL, escape sequences and bidirectional controls is shown as inert text |

What this plan does not prove is registered as gaps in research.md: G041-001 (a transformed or foreign secret),
G041-002 (the tail of an attempt that lost its lease or its network), G041-003 (script lines are not live), G041-004
(feature 020 must delete streams with their run; G1618 in feature 020's research), G041-005 (a deployment whose
protocol ceiling a missing store proof holds below 1.7 captures nothing), G041-006 (SC-041-007 is measured on one machine), G041-007 (a script execution in a collection is
listed without its iteration), G041-008 (the cursor shows the transaction counter), G041-009 (a wide fan-out
serialises on `run_log_usage`), G041-010 (the classified-namespace test over-matches), G041-011 (moot: step logs have their own quota), G041-012 (any open transaction of the cluster holds the read horizon), G041-013 (no route
removes a script execution's log early), G041-014 (an attempt whose capture stopped before its first batch has no
stream), G041-015 (closed by D041-066: the step-log quota's usage is shown only to readers of step logs), G041-016 (the
oldest-transaction gauge sees other roles' transactions only with `pg_read_all_stats`).

### Estimate

An estimate by reading, not a measurement. Server and contracts (T041-3 to T041-8, T041-10 to T041-13, T041-28 to T041-32):
about 6,500 to 7,500 lines of Rust, most of them tests, in 15 pull requests. SDKs (T041-14 to T041-21): 600 to 900 lines
each with their unit tests, plus the specification, five scenarios and the harness change, in 8. Studio (T041-22 to
T041-24): about 1,300 to 1,600 lines with tests, in 3. Measure, operate and close (T041-1, T041-9, T041-25 to T041-27):
5. That is 32 tasks and 31 pull requests (T041-2 has none). A pull request's CI takes up to two hours and they land one at
a time where they touch research.md or the contracts, so the calendar is set by the queue: about four to four and a half
working weeks for one session working alone, less with the six SDK tasks in parallel after T041-14. The places most
likely to move it are T041-31 (a failure stops for a design change) and the read horizon on two PostgreSQL versions.
