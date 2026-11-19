# Step logs per run

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-19
*   **Owner-acceptance:** OD-2026-11-15-17 (the design, for 0.3.0); OD-2026-11-16-7 (the approval path); OD-2026-11-19-1 (the direction of the redesign; its text is pending review)

--------------------------------------------------------------------------------

## Overview

The lines a step's code writes to its logger are sent by the worker to the platform, stored per attempt with
ceilings and a retention period, and read through a REST route, a live tail and a Studio screen. The owner chose this
on 2026-11-13 (OD-2026-11-13-9: "step logs per run in 0.3.0 — a new feature: workers send the log lines of each step,
the platform keeps them with retention and limits, and there is a route and a Studio screen").

The design treats a step log as a bounded diagnostic record kept **beside** the journal. It is never execution state:
no transition reads it, a lost line never changes a step's outcome, and the transaction that leases a step runs no
statement on it. The last records of an attempt ride on its report (`CompleteStep` or `FailStep`) and are written by the
report's own transaction, which they add at most five statements to and no transaction; when the carry makes that
transaction fail, the report is recorded by one re-run (section 2a). A long step also streams its earlier records
through an RPC of their own, `AppendStepLogs`. A step log MUST NOT be a second place where another tenant's data can be
read, or where a secret can be read beyond the limit of G041-001 (a value the code transformed, a short one, or one
obtained outside the platform). Log writing can delay a report of the same tenant only by the waits listed in section 2,
"What a report can wait for": the tenant's usage row at the commit and, for a report that carries records, the
step-log policy fences and its own attempt's stream row, each bounded at 200 ms.

**Revision of 2026-11-19 (Draft).** The design was approved on 2026-11-16 (commit `f9d4a65f`; the four reports of
`041-step-logs.reviews/2026-11-16/`). Its spike before the schema, T041-31, failed SC-041-007: with an `AppendStepLogs`
transaction per batch and a final batch before every report, the median rate with capture on was 79.2% (1 replica, 16
workers) and 87.0% (8 replicas, 128 workers) of the rate with capture off on a quiet machine, against 90%
(`specs/041-step-logs/validation-report.md`). The owner chose to remove the per-step transaction ("Redesenhar
(CompleteStep)", "redesign (CompleteStep)", OD-2026-11-19-1, O041-036 option (d)): the last records travel on the report,
so a short step needs no transaction of its own. The change is
[ADR-0095](../adr/ADR-0095-the-last-step-log-records-of-an-attempt-ride-on-its-report.md) (Proposed, O041-038). The
revision changes the body of an approved doc, so by the approval rule of `docs/designs/constitution-4.0.0-egm.md`
(section 4, "Amending an approved doc") the design is `Draft` until it is reviewed again. Changed: the Overview,
FR-041-004/005/008/009/014/015/018/020, SC-041-001/003, the Goals, the Open Questions, the owner decisions, the Terms,
the diagram, sections 1, 2, 2a (new), 3, 4, 5, 6, 9, 10, the Risks, the Constitution Check, Complexity Tracking, the
Alternatives, the NFRs, the plan (T041-3 to T041-9, T041-14, T041-32, T041-33; T041-35 and T041-36 new; T041-34
withdrawn), the Verification Plan and the Estimate. The reviews of the revision, dated by their folder, are in
`041-step-logs.reviews/2026-11-20-redesign/`. A new spike, T041-35, with its acceptance rule set in its task before its
first round, replaces T041-31 as the gate before the schema.

Before this revision, the owner's acceptance of the design for release 0.3.0 (OD-2026-11-15-17) decides scope. The review findings of the
approval of 2026-11-16 were either fixed and re-read by the three repository reviewers or accepted by the owner, one row
each, on the approval path the owner chose ("Corrigir e aceitar o resto", OD-2026-11-16-7).

The decisions, gaps and owner questions are registered in [specs/041-step-logs/research.md](../../specs/041-step-logs/research.md);
the contract this design freezes is [specs/041-step-logs/contracts.md](../../specs/041-step-logs/contracts.md) and the
schema detail is [specs/041-step-logs/data-model.md](../../specs/041-step-logs/data-model.md). The architecture decisions
are [ADR-0084](../adr/ADR-0084-step-logs-are-a-bounded-lossy-per-attempt-record-beside-the-journal.md),
[ADR-0085](../adr/ADR-0085-step-log-text-is-read-under-its-own-permission-and-redacted-by-value-in-the-worker.md) and,
proposed, [ADR-0095](../adr/ADR-0095-the-last-step-log-records-of-an-attempt-ride-on-its-report.md). How
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

Each requirement is binding as written, whether or not it says MUST.

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
- **FR-041-004 Transport.** Lines travel in protocol minor 1.7 (contracts.md §1) in two ways (OD-2026-11-19-1). The
  **carry**: the attempt's last records (at most 64 records, and at most 16 KiB for the encoded `StepLogCarry`) and the
  highest sequence its logger assigned travel as an optional `StepLogCarry` on the attempt's own `CompleteStep` or
  `FailStep`, and are stored by the transaction that records the report (section 2a); the carry holds the attempt's
  *final mark*. A **batch**: while the attempt runs, the SDK sends the records it flushes through a unary RPC,
  `AppendStepLogs`, that names one attempt with the identity and fence a heartbeat carries plus the worker's session id
  and current session token, and holds 1 to 500 records and at most 192 KiB. The server MUST answer a batch only after its
  lines are committed, and MUST store a record at most once however many times its batch or its report arrives
  (identity: attempt, generation, sequence). After the final mark of an attempt is recorded, a later batch for it MUST
  store nothing and be answered `STOP_ATTEMPT`: `LEASE` when it read the lease after the report committed, as almost
  every one does, and `FINAL_RECORDED` when it read the lease before.
- **FR-041-005 Logging never decides the step.** A call to the logger MUST NOT block on the network or on a full buffer,
  and MUST NOT throw into the activity. No failure of capture (refused batch, timeout, server down, ceiling reached,
  invalid line, a carry the server cannot store, bug in the capture path) may change the outcome, the output or the retry
  of a step, nor its timing beyond the bounded wait of FR-041-015, the carry's own statements and, when the carry makes
  the report's transaction fail, one re-run of the report (section 2a). The one residual: a re-run whose lease expired
  after the first transaction failed is refused as a late report would be (G041-021).
- **FR-041-006 Ceilings and drop counts.** The server enforces these from counters, regardless of what the worker sends:
  - per line: 8 KiB per message and per exception text, 2 KiB and 32 keys per field map, measured after redaction; a
    message or exception over its ceiling is cut at a character boundary and the line is flagged `truncated`;
  - per attempt: the attempt ceilings of FR-041-020 (defaults 10,000 records and 1 MiB of stored text);
  - per tenant: the step-log quota `tenant_step_log_bytes`, a resource of its own (OD-2026-11-15-5).
  There is no run ceiling: the attempt ceilings and the tenant quota bound what a run stores (OD-2026-11-16-10).
  Records past a ceiling are not stored and are counted on the attempt's stream. When the attempt ceiling is first met
  the server stores one marker line, which holds no text; the tenant quota is recorded only on the stream row. Every
  sequence the worker numbered and the server never stored is counted once, as dropped by the worker (a gap in the
  sequences it sent) or by the server (a ceiling, including the records the worker stopped sending after it). A step
  MUST NOT fail, wait or retry because a ceiling was met.
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
    an error body or the platform's own logs, and neither the `AppendStepLogs` request nor a `CompleteStep` or `FailStep`
    request that holds a carry MUST be printed by any log, `Debug` or trace of the server or the SDKs.
  - (e) What this cannot catch is G041-001: a value the code transformed, a value shorter than 6 bytes, a value obtained
    outside `ResolveSecrets`, and anything a client that does not use an official SDK sends.
- **FR-041-008 Storage beside the journal.** Lines are stored in PostgreSQL in tables of their own. Storing a batch MUST
  NOT take, or wait for, a lock on a row of `runs`, `steps` or `run_events`; leasing and heartbeat MUST NOT read or write
  the log tables; a report MUST NOT read or write them except to store its own carry, through the `StepLogCarryTx` port
  (five methods, one statement each, section 2a), at the cost section 2a states as an equality. Neither a batch nor a
  carry may take the tenant's `tenant_retained_usage` row before its commit.
  The attempt ceilings count text bytes (the UTF-8 octets of message, exception and fields); the bytes charged to the tenant, each row's text plus a fixed 128 bytes
  (OD-2026-11-15-6), are counted in a storage class of their own, `step_log` (ADR-0027 Amendment of 2026-11-15), and released when the rows are
  deleted.
- **FR-041-009 Retention.** An attempt's lines are deleted once its stream expires: the server's time of its last
  recorded batch or carry plus the `step_log_retention_days` the stream took from its namespace when it was created (FR-041-020).
  As ADR-0027 requires, lowering the retention does not shorten what is stored; the delete route (FR-041-023) is the way
  to remove lines early. The stream row, which says the attempt had a log and holds its counters and the time its lines
  were removed, is kept, so that a reader is told "expired" and not shown an empty list. Once removal has begun, the
  stream's lines are no longer returned and a later batch for it stores nothing.
- **FR-041-010 Read.** `GET /runs/{runId}/logs` returns lines in stored order with `cursor` and `limit` (default 50, at
  most 200, clamped, as every list), and only lines below the read horizon: the oldest transaction still in progress when
  the reading statement runs (section 5). Filters: `stream` (the public id of one stream of this run, an attempt log or a
  script execution's marker; another is answered as missing), `node`, `attempt`, `minLevel`, `from` and `to`
  (storage time), and `q` (a literal, case-insensitive substring of the message, at most 128 characters). A request
  scans at most 5,000 rows and its cursor moves past every row it scanned, matched or not; a read with `stream` scans
  that stream's rows only. The cursor names the run it was minted for and is refused for another. A page says whether
  lines are held back by the horizon (`heldBack`) and whether the run's log is complete (`complete`): complete only
  when the run is finished, every line that can still be stored has been, and the request's scan ended because no row
  below the horizon was left in the index range it reads (the run's, or the stream's with `stream`): neither the page
  limit nor the 5,000-row bound stopped it, and whether or not the filters matched (section 5).
  Every item carries the identity of FR-041-002, both times, level, message, exception, fields, `truncated` and, for a
  marker, its reason (contracts.md §2).
- **FR-041-011 Streams.** `GET /runs/{runId}/logs/streams` lists, with `cursor` and `limit`, the attempt logs of the run
  (`source = worker`: node, instance path, attempt, state, records and bytes stored, dropped by the worker and by the
  server, the ceiling met, first and last time, expiry or removal time) and, with `source = script`, the run's script
  executions (FR-041-016), whose state comes from the execution itself: `expired` once the execution's own expiry has
  passed, `ended` before. The counters are read at the statement's snapshot and can be ahead of the lines a read below
  the horizon of FR-041-010 returns.
- **FR-041-012 Live tail.** `GET /runs/{runId}/logs/stream` is a server-sent event stream of new lines with the filters
  of FR-041-010, resumable with `Last-Event-ID`, with the `reconnect` and ping behaviour and the per-subject and
  per-tenant caps of `/runs/{runId}/stream`, shared with it. It never skips or repeats a line across a reconnect. It ends
  with `end` only when the run is finished and every line that can still be committed for the run has been delivered,
  by the rule of D1958 (a decision of feature 018: a transaction id taken after the end is seen, and an end declared only
  once every older transaction has ended; section 5). The reader's right is checked when the tail opens and again every 30 seconds
  (`StreamLimits::recheck`), and its PII reveal grant at every poll; a reader who loses the right is cut within 30
  seconds, and one who loses the grant at the next poll.
- **FR-041-013 Who may read.** The three read routes need `run:read` and a new permission, `run:logs:read`, held by the
  admin, developer and operator roles and not by viewer, worker or runner. In every namespace the reader MUST also hold
  the namespace's PII reveal grant, checked as `has_effective_payload_reveal_grant` checks it (OD-2026-11-16-8, which
  replaces the classified-namespace condition of OD-2026-11-13-18). Another tenant's run is answered 404 and recorded as `access.denied.cross_tenant`, as every run
  route. A read is audited as `run.logs_read` at most once per subject and run in 300 seconds whatever its filters; the
  record of a window commits before the first line of that window is returned, a tail's included, and a failure to
  write it returns no line.
- **FR-041-014 Mixed versions.** A worker accepted below 1.7 sends nothing and behaves as today. A 1.7 SDK
  against a server that accepts a lower minor, or whose policy says capture is off, sends nothing and logs locally. An SDK
  that gets `UNIMPLEMENTED` for `AppendStepLogs` (an older API replica) stops sending batches for that attempt only and
  still sends its carry. An API replica older than 1.7 ignores a carry as an unknown field and records the report; the
  carry is lost and the stream, if the attempt had one, shows "ended without its final flush". No published workflow,
  run in progress or existing RPC changes meaning.
- **FR-041-015 The six SDKs.** Each SDK keeps its present logger type, adds the calls of section 6 that it lacks, and
  adds capture behind it: a bounded buffer per attempt (1,000 records or 256 KiB, whichever is met first) and per process
  (16 MiB across attempts), a flush through `AppendStepLogs` one second after the attempt's first captured record and
  every second after, or at 256 records, one batch in flight per attempt, the answers of contracts.md §4, and the carry
  on the report. Before it reports, an SDK waits for its batch in flight, if any, and sends what does not fit one carry
  as batches, at most 2 seconds in all; a short step (see Terms) waits for nothing. These numbers are constants of
  `sdks/SPEC.md`, not values the server sends. The Rust SDK gains the logger its `ActivityContext` lacks.
- **FR-041-016 Steps the platform runs.** A script step's log stays where feature 005 stores it and is never copied.
  Right after the script runner's report is recorded, in a short transaction of its own, the platform also stores one
  marker line, with no text, that points to
  the script execution, so that the whole-run Logs tab shows, in stored order, that the step produced output and opens
  it under FR-041-022 (OD-2026-11-15-9; section 7 says when the marker is written and when it is not). The marker is keyed
  on the script execution, of which there is one per (run, node, attempt): it shows the node and the attempt, and no
  iteration (G041-007). A crash between the two transactions loses that one marker, as ADR-0084's lossy record allows.
  The script runner's report does not change. The streams route also lists the run's script
  executions as `source = script` items with their own expiry. The database runner and the lake runner capture no log.
- **FR-041-017 Studio.** The run screen gains a Logs tab for the whole run and a Logs section for the selected step and
  attempt, with filters, search, a live tail that pauses when the reader scrolls up, markers (the attempt ceiling's and the script output's) shown as rows, and the states
  "not allowed" (naming the permission or the grant that is missing), "no log", "expired", "deleted", "ended without its
  final flush" (the attempt's lease is gone and no final mark was stored), "some lines are delayed" (`heldBack`) and
  "log withheld" for a script execution, with full keyboard operation.
- **FR-041-018 Observability.** The platform counts records stored, dropped (by reason), truncated and redacted, batches
  and carries by outcome, ingest duration, lock-timeout retries, purged records, open tails and the age of the database's oldest open
  transaction, with labels that are closed enumerations only (data-model.md §7). It alerts on sustained shedding, on purge
  backlog and on an old open transaction, not on ceilings a customer's code meets.
- **FR-041-019 Lineage.** A log belongs to the run and attempt that wrote it. A retry is a new attempt and a new log. A
  reset, a rerun and a continue-as-new open a new run; the new run starts with no log and the earlier run keeps its own,
  reachable through the links the Studio already shows between those runs. Lines are never copied.
- **FR-041-020 Policy.** Three quota resources at namespace scope hold the policy, each configurable per namespace
  (data-model.md §6; OD-2026-11-16-9): `step_log_attempt_records` and `step_log_attempt_bytes` (the attempt ceilings) and
  `step_log_retention_days`; `step_log_attempt_records = 0` is the namespace's capture switch, kept as it is. A fourth
  resource, `tenant_step_log_bytes`, is the tenant quota of FR-041-006. A *ceiling* in this design is the limit in force
  of one of the two attempt-ceiling resources (a `quota_limits` row or its default), not an operator's `quota_ceilings`
  row. Capture happens only when all of these hold: the deployment switch `SAGAT_STEP_LOGS_ENABLED` is on, the
  namespace's `step_log_attempt_records` is above 0, the worker's own option is on, and the session was accepted at 1.7
  or later (a carry is exempt from the last condition, D041-092: the server cannot see a report's session). The server
  reads the first two at registration (the policy's `enabled`) and again in every batch and every carry; a change of a
  ceiling applies to the batches and carries stored after it, and a change of the retention to the streams created after
  it.
  The execution engine and the control plane both read `SAGAT_STEP_LOGS_ENABLED`, as both read `SAGAT_SCHEDULES_ENABLED`
  today: the engine for capture, the control plane for the script marker (section 7).
  Changing a resource is audited by the quota catalogue as every quota change is.
- **FR-041-021 The PII reveal grant everywhere.** Feature 016 treats a payload field with no `x-sagat-classification`
  annotation as `pii` (`crates/domain/src/classification.rs:41-42`), and an activity's log lines can hold payload values.
  So reading step logs needs the namespace's PII reveal grant in every namespace, whether or not its schemas carry an
  annotation ("Sim, sempre", OD-2026-11-16-8, which changes OD-2026-11-13-18). No classified-namespace record is kept.
- **FR-041-022 Script-execution logs.** The `log` of `GET /namespaces/{namespaceId}/script-executions/{executionId}`, for
  an execution of a run, is returned only to a caller who may read that run's step logs under FR-041-013. Anyone else who
  may read the execution gets `log` empty, `logTruncated` false and `logWithheld: true`, and the listing shows
  `logTruncated` false to them (OD-2026-11-13-18). An execution with no run (a test run of a script) is unchanged. Viewers lose the log
  they read today; the release notes say so.
- **FR-041-023 Deletion.** `DELETE /runs/{runId}/logs`, optionally with `stream`, needs `run:logs:delete`, held by the
  admin role only. It makes the lines committed when it runs unreadable at once; a later batch for a deleted stream is
  answered `REMOVED` and stores nothing, while a batch or a marker that creates a new stream after it (a later attempt)
  is stored, and a second delete removes it. It records `run.logs_deleted` (run, number of streams changed, 0 included,
  no text) in the same transaction, every time, answers `204` with no body, and leaves the removal of the rows to the purge. It exists for the incident in which a secret or
  personal data was logged. No route removes a script execution's log before its retention (G041-013); the incident
  runbook says so.

### Success criteria

- **SC-041-001 The hot path.**
  - With the migration applied, `work_budget.rs` passes with its three constants unmodified for leases and for reports
    without a carry: the migration adds no trigger, statement or lock to them. Six new equalities hold (section 2a,
    "What a report pays"), as statements, transactions and rows:

    | Report | Completion with a next step | Completion that ends the run |
    |---|---|---|
    | carries ten records, all kept | 55, 1, 23 | 52, 1, 21 |
    | carries a mark-only carry | 54, 1, 13 | 51, 1, 11 |
    | carries records, capture off in the namespace | 53, 1, 11 | 50, 1, 9 |
  - A batch for a run commits, under `lock_timeout = 50ms`, while another transaction holds that run's row `FOR UPDATE`
    and has updated the leased `steps` row.
  - One ingest of tenant T is held after its quota flush (`SET CONSTRAINTS ALL IMMEDIATE`), so it holds the tenant usage
    row; K − 1 more ingests of T queue behind it on that row (K = the tenant's tickets on one replica); then a completion
    with output of another run of T, whose output has a digest not yet stored, queues after them. When the ingests are
    committed one at a time, after each commit the completion is still blocked and `pg_blocking_pids` of it names ingest
    backends only, and it commits after the K-th.
  - One ingest of the namespace is held after T7: a `lock_secrets` of the namespace and a completion of the namespace
    both commit while it is held, since the ingest takes no secrets lock and no membership lock (OD-2026-11-16-7).
  - A carrying report whose commit fails because the finalizer finds the tenant's step-log limit exceeded, and a
    carrying report whose carry statement fails, are each recorded by one re-run with the same outcome and output as
    without a carry (section 2a).
  - A carrying report waits at most 200 ms for each step-log lock (the policy fences, its stream row): a change of a
    step-log resource held exclusively, or an ingest of the same attempt held after T7, makes the carry fail into its
    re-run, and the report commits.
  - The bound in time is SC-041-007's.
- **SC-041-002 A flood.** An activity that writes 1,000,000 lines as fast as it can completes with the same outcome and
  output as one that writes none; the store holds at most the attempt ceiling plus its marker; and for the stream,
  lines stored (the marker excluded) + dropped by the worker + dropped by the server = the highest sequence the logger
  assigned (the drop counts saturate at 2^53, so this holds for highest sequences below it).
- **SC-041-003 Faults.** In the fault tests of the Verification Plan no record is stored twice, and every record of an
  attempt that has a stream and is missing is covered by a counter or by the "ended without its final flush" state. An
  attempt whose carry was lost before a stream existed (an API replica that predates 1.7, a failed carry statement, a
  carry ignored by C1) has no stream (G041-014). A
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
  attempt that failed and in the ones before it, and is told when they may not (CUJ 1, CUJ 5, CUJ 6).
- **US02** An operator watches a long step of a running workflow and sees its lines arrive (CUJ 2).
- **US03** A tenant administrator bounds what logs cost (bytes, days), turns capture off for a namespace whose code logs
  personal data, and deletes a run's log when a secret was logged (CUJ 3, CUJ 7, CUJ 8).
- **US04** An SDK user changes nothing in their activity code and gets capture after upgrading the SDK (CUJ 4).

## Goals

- What a step's code logged is readable from the run, by the people allowed to, during the run and for a stated number
  of days after (FR-041-010 to FR-041-013, SC-041-005).
- Capture adds no statement to a lease or a heartbeat, no transaction to a short step, and at most five statements to
  the report that carries an attempt's last records (one re-run when the carry makes that report's transaction fail); a
  short step reports without waiting, and a long one waits at most 2 seconds for its batches before it reports
  (FR-041-015). The waits a report can meet are those of section 2, "What a report can wait for"; their effect must keep
  the completion p99 within 10% and the rate within 90% of their values with capture off (SC-041-007), which T041-35
  measures before the schema.
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
- **No classification of log content by field.** A log line has no schema. The PII reveal grant of FR-041-021 decides
  who may read, not what is shown.
- **No download or export button** in the first delivery.

## Open Questions

Technical, each closed by the task named:

- **Does a completion's p99 stay within 10% of its value with capture off, and the rate within 90% (SC-041-007)?**
  T041-31 measured the approved transport and it failed (OD-2026-11-19-1). T041-35 measures the carry on a scratch schema
  before T041-5 starts, with the workloads and acceptance rule recorded in T041-35. If it fails, the feature stops for a
  design change and a new review, and no task after T041-35 starts until that review approves the change. T041-35 does
  not reach the scoped report path (G041-019) nor a database on another host (G041-022).
- **Does the read horizon and its end rule hold on PostgreSQL 16 and 18?** T041-11 and T041-12, with held transactions
  on both, each in a disposable container of its own (the horizon is cluster-wide).

## Owner decisions needed

### Open

- **O041-038 Accept ADR-0095** (asked once T041-35 passes; T041-3 and T041-36 wait for it). (a) Accept it as written,
  which supersedes ADR-0084 decisions 2, 6 and 11 and decision 5's session clause for a carry, and the terminal-report
  clause of ADR-0027's Amendment item 1; (b) accept with changes; (c) reject, and the design changes again. Recommended
  (a) if T041-35 passes.

Conditional on the spike T041-35, asked only if its condition is met:

- **O041-026 A step-log usage row of its own** (asked by T041-35, only if its completion wait on the tenant row fails
  SC-041-007). (a) Keep `tenant_retained_usage.step_log_bytes`, as the ADR-0027 Amendment was accepted
  (OD-2026-11-15-16). (b) A per-tenant row of its own for the step-log class; the Amendment is revised. With the carry, a
  report that carries records takes the step-log usage row at its commit whatever its output, so (b) moves the queue
  of a tenant's logging reports to a row of their own rather than removing it. Recommended: (a) if T041-35 passes; (b)
  if T041-35 shows that more than half of a failing p99 excess is spent waiting on the tenant row.

### Answered

Recorded in `docs/owner-decisions.md` and in research.md:

- **O041-001 to O041-011** were answered as recommended on 2026-11-13 ("Aceitar todas", "accept them all",
  OD-2026-11-13-14). Each rejected option and its reason:
  - O041-001: capture on by default at `info`, a namespace switch and a worker option. Rejected: off until a namespace
    asks (no one would find it), on for new namespaces only (two behaviours to explain).
  - O041-002: `run:logs:read` for admin, developer and operator. Rejected: viewers too (logs can hold personal data); a
    per-namespace grant only (a grant to manage for every reader everywhere).
  - O041-003: one `run.logs_read` per subject and run per 300 seconds. Rejected: one per request (a tail would write one
    every poll); none (principle VI).
  - O041-004: PostgreSQL, one row per line. Rejected for 0.3.0: chunk rows (filters and identity move into Rust; the next
    step if T041-31 asks, O041-021, which was not asked: OD-2026-11-19-1); the blob store (committed blobs are never purged today, G69 of ADR-0023); a
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
- **O041-012 and O041-013** were answered on 2026-11-13 (OD-2026-11-13-18; the first part changed by OD-2026-11-16-8,
  which needs the grant in every namespace): in a classified namespace reading step logs
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
- **O041-028 to O041-035**, the questions of the third approval run, were answered on 2026-11-16 (OD-2026-11-16-7 to
  -10):
  - O041-028, "Sim, sempre" ("yes, always"): every step-log read needs `run:logs:read` and the namespace's PII reveal
    grant, in every namespace (OD-2026-11-16-8, which changes OD-2026-11-13-18). Rejected: the grant only in a
    classified namespace; the grant where a version has `classification_rules = 1` or no annotation.
  - O041-029, "Configuráveis" ("configurable"), against the recommendation: the attempt ceilings stay configurable per
    namespace, and so does the capture switch (OD-2026-11-16-9). Rejected: constants with a plain capture switch.
  - O041-030, "Retirar" ("remove"): no run ceiling; the attempt ceilings and the step-log quota bound the total
    (OD-2026-11-16-10). It answers O041-020 and O041-027. Rejected: an unlocked sum; an exact run ceiling.
  - O041-021 (chunk rows), not asked: T041-31 measured chunk rows and they failed SC-041-007 too (OD-2026-11-19-1); one
    row per line stays (O041-004).
  - O041-031 to O041-035, accepted as recommended on the approval path "Corrigir e aceitar o resto" ("fix and accept
    the rest", OD-2026-11-16-7): the script marker in its own transaction after the report commits; the ingest reads the
    binding and the membership without the namespace secrets lock; O041-025 restated to the code's stop rule; G041-017
    accepted in writing; a Note on ADR-0085.
- **O041-025**, "Sim, como os reports" ("yes, as the reports"), option (a), answered 2026-11-16 (OD-2026-11-16-11).
- **O041-036**, "Redesenhar (CompleteStep)" ("redesign (CompleteStep)"), option (d), answered 2026-11-19
  (OD-2026-11-19-1): cut what a step pays by carrying its lines on the report, then a new spike. Rejected: (a) only a
  quiet rerun (it was run and confirmed the failure); (b) restating the workload to one ingest per step (its medians,
  89.4% and 89.9%, missed the criterion); (c) a lower criterion; (e) deferring step logs from 0.3.0. O041-037 follows
  from it and is recorded as a decision (D041-091), not an owner answer: T041-3 and T041-4 stay held and unticked until
  this revision is approved, and T041-3's 1.7 contract changes (D041-090).


## Critical User Journeys (CUJs)

### CUJ 1: Why did this step fail? (US01)

*   **Given**: a run whose step `charge` failed on its third attempt; the activity logs at `info` and `error` through the
    SDK's logger; the reader holds `run:logs:read` and the namespace's PII reveal grant.
*   **When**: they open the run in the Studio, select `charge` and open Logs.
*   **Then**: they see the lines of attempt 3 and can switch to attempts 1 and 2; each line shows its level as text and
    shape, its worker time, its fields and its exception; a secret the activity resolved and logged verbatim shows as
    `[REDACTED]`.

### CUJ 2: Watching a long step (US02)

*   **Given**: a running step that logs a line every few seconds.
*   **When**: the reader opens the Logs tab with the tail on.
*   **Then**: lines usually appear within the worker's flush interval (1 s) plus the tail's poll interval (0.5 s), later
    when an older transaction holds the read horizon (the Studio shows "some lines are delayed"); scrolling
    up pauses the tail and a "Jump to latest" control resumes it; when the connection drops and returns, no line is
    missing or doubled; when the run ends, the tail ends after the last line that could still arrive.

### CUJ 3: A step that floods (US03)

*   **Given**: an activity with a loop that logs 1,000,000 lines.
*   **When**: it runs.
*   **Then**: the step completes as it would without logging; the log holds the first 10,000 lines that reached the server and one
    marker "attempt ceiling reached"; the streams list says how many lines the worker dropped (its buffer was full) and how
    many the server dropped (the ceiling), and lines stored plus the two drop counts equal what the code logged. No alert fires: it is the
    customer's code, shown on the run.

### CUJ 4: A worker that was not upgraded (US04)

*   **Given**: a worker on an SDK that offers protocol 1.5.
*   **When**: it runs steps on a server that supports 1.7.
*   **Then**: nothing changes for it; its steps show "no log" in the Studio (the streams route lists no stream for them).

### CUJ 5: A reader without the permission or the grant (US01)

*   **Given**: a viewer, who holds `run:read` and not `run:logs:read`; and a developer without the namespace's PII reveal
    grant, in a namespace whose schemas carry no annotation.
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

**Terms** (used from the Overview on):

- *Attempt* and *generation*: a step's attempt number counts retries; the generation is the fencing number a lease
  carries (`steps.generation`), which a reclaim raises, so two holders of one attempt never share it.
- *Instance path*: the ordered (scope node, iteration index) pairs that place a step inside collections and loops
  (`ExecutionInstanceIdentity`, `worker.proto:90-93`); a root step has an empty path.
- *Stream*: a row of `step_log_streams` and the lines that reference it. A stream of `source = worker` is an *attempt
  log*, the lines of one (step, attempt, generation); a stream of `source = script` holds the one marker of one script
  execution, and its public id is that execution's id. The `stream` parameter of the routes takes a stream's public id.
- *Tail*: a reader's open server-sent event connection to `GET /runs/{runId}/logs/stream`, never a stream; the list of
  streams is `GET /runs/{runId}/logs/streams`. The *graph stream* is the run's existing event route,
  `GET /runs/{runId}/stream`.
- *Activity binding*: the record that lets a worker subject run the activities matching its patterns in a namespace
  (`bound_patterns`, `first_uncovered`); a report, and a batch, is refused when the binding no longer covers the step.
- *Runtime role*: `sagat_app`, the least-privilege PostgreSQL role the services run as.
- *Usage row*: the tenant's row of `tenant_retained_usage`, which the quota finalizer locks at commit.
- *Storage classification*: the class each column has in `storage-classification.json` (`log`, `content`,
  `protected_system`, and with this feature `step_log`), which decides what the quota triggers charge; it is unrelated to
  the *data classification* of payload fields (`x-sagat-classification`, ADR-0033), which decides what a reader may see.
- *Store proofs*: the methods of `WorkerStore` that say a deployment can serve a protocol minor
  (`supports_scoped_worker_protocol`, `supports_database_worker_protocol`).
- *Namespace secrets lock*: the advisory lock `quota:secrets:{tenant}:{namespace}` (`crates/infra-postgres/src/quotas.rs:307`),
  taken shared by every report and exclusively by secret and database-catalog changes.
- *Policy fences*: the advisory locks of a quota resource's policy; a change of the resource takes its fence
  exclusively, and a writer that relies on the resource takes it shared, so the change waits for writes in flight. The
  finalizer takes the fences of the classes a transaction charges (content, log and step-log) before the usage row.
- *Admission classes and discovery*: the gRPC server sheds calls by class (`crates/api-grpc/src/limits.rs:489-508`);
  *discovery* is the class of polls for new work, shed first under load.
- *Tickets*: the per-tenant share of the Logs class on one replica (section 1); *R* is the number of API replicas.
- *Design 038*: `docs/designs/038-claim-transition-efficiency.md`, which set the hot-path budget and its selector rule.
- *Carry*: the optional `StepLogCarry` of a `CompleteStep` or `FailStep` request: the attempt's last records and the
  highest sequence its logger assigned. *Final mark*: that highest sequence once a carry records it, with `final_at`, on
  the stream row; every stored carry records one, and a *mark-only carry* records it and no line. *A carrying report*
  is a report that holds a carry. *Batch* is kept for `AppendStepLogs`. *Report transaction*: a transaction that records
  an attempt's outcome and commits (section 2a lists them).
- *Short step* and *long step*: a short step reports before its first flush (one second after its first captured
  record, or 256 records buffered) and its remaining records fit one carry (64 records, 16 KiB encoded); it sends no
  batch and waits for nothing before its report. Every other step is a long step. "Ended without its final flush", the
  name of a stream's state, means that no final mark was recorded.

```text
activity code
   | logger call (unchanged API plus the calls of section 6; never blocks, never throws)
   v
SDK capture in the per-attempt logger: level -> sequence -> format -> redact with the process's secret set
   -> cut -> bounded buffer (attempt and process)
   |                                                      |
   | while the attempt runs, after its                    | at the end: the rest (<=64 records, <=16 KiB)
   | first flush (1 s or 256 records):                    | and the highest sequence, as the StepLogCarry
   | AppendStepLogs (unary, 1.7): attempt identity        | of its CompleteStep or FailStep (1.7)
   | + fence + session id and token, <=500 records,       v
   | <=192 KiB                                            |
   v                                                api-grpc -> WorkerService::complete / fail
api-grpc: Class::Logs admission -> per-tenant          | the report transaction (run row, lease, binding, membership,
   ticket -> application::step_logs                   |   engine) and, before its commit, at most five statements
   (validate, scrub, cut, size, plan)                  |   under a 200 ms lock timeout: fences -> values -> stream
   | one transaction of its own, lock_timeout 200 ms:  |   row -> insert -> counters -> commit (finalizer: fences,
   |   xid -> lease read (no lock) -> session,         |   tenant row); a refused or failed carry: the report is
   |   policy fences and values -> stream row -> plan  |   re-run once with a mark-only carry or none (section 2a)
   |   -> tenant usage read (no lock) -> binding and   |
   |   membership read (no lock) -> insert -> counters |
   |   -> commit (ADR-0027 finalizer: fences, tenant row)
   v                                                   v
PostgreSQL: step_log_streams, step_log_lines   (no foreign key to runs or steps)
   ^ purge pass (scheduler)        ^ GET /runs/{runId}/logs, /logs/streams, /logs/stream; DELETE /runs/{runId}/logs
                                   |    (script_executions read for source = script; after record_execution, the script marker)
                                   v
                                Studio: Logs tab and step section
```

### 1. Wire: protocol minor 1.7

`specs/001-platform-foundation/contracts/worker.proto` (the canonical copy; the six SDK copies are byte-identical) gains
the rpc, messages and enums of contracts.md §1, additively, and `CompleteStepRequest` and `FailStepRequest` gain
`StepLogCarry step_logs = 13` (D041-090). The verdicts of a batch are: `CONTINUE`; `FINAL_ONLY` (a ceiling was met: send
no more records; the report still carries the final mark, with no record, so that the server learns the highest sequence);
`STOP_ATTEMPT` with a reason (`FINAL_RECORDED`, `LEASE`, `AUTHORITY`, `REMOVED`); `STOP_CAPTURE` with a reason
(`CAPTURE_OFF`, `SESSION`); `RETRY_LATER` with `retry_after_ms`. A record has no kind: a gap in the sequences is the
server's evidence of a drop, and the carry's `highest_sequence` counts the drops at the end too. A carry gets no verdict:
nothing follows it, and the report's answer (`StepCommandResponse`) does not change.

**The carry's size.** A carry holds at most 64 records, and its encoded `StepLogCarry` at most 16 KiB. The bound comes
from the time a carry runs under the run lock (section 2a). It also fits the message cap: the server decodes at most
`MAX_MESSAGE_BYTES` = `MAX_PAYLOAD_BYTES` (256 KiB) + 64 KiB per worker message (`crates/api-grpc/src/server.rs:14`),
an output may take the 256 KiB, and 16 KiB of the remaining 64 KiB leaves the report's other fields room. The SDK sends
what does not fit as batches before the report (section 6). The decoding cap does not change.

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
`AppendStepLogsRequest`, `StepLogCarry`, `StepLogRecord`, `StepLogField` and its `oneof` join `skip_debug` (`build.rs:17-25`), with
`Debug` impls that print counts only, written beside each generated module (`crates/api-grpc/src/lib.rs`, the private
`protocol` module of `sdks/rust/src/lib.rs:39-42`, the `pb` module of `sdks/rust/conformance/src/faults/grpc.rs:4-7`).

**Negotiation.** The rule is unchanged (ADR-0015): `negotiate_protocol_details` (`crates/application/src/worker.rs:169`)
answers the lower of the two minors; what changes is the server's own minor. `PROTOCOL_MINOR` (`worker.rs:69`) becomes 7, with `STEP_LOG_PROTOCOL_MINOR = 7` beside the
others. `protocol_ceiling` (`worker.rs:82`) answers 7 for an ordinary worker only when the store proves scoped and
database support and a new proof, `WorkerStore::supports_step_log_protocol()`, beside `supports_scoped_worker_protocol`
(`worker.rs:283-290`), which is true when the migration of T041-5 is applied; it keeps 6 for the database runner. The
ceiling tests (`worker.rs:3035-3044`) gain the 7 cases.

Protocol minors are cumulative, so an ordinary worker at 1.7 has 1.6's `ResolveDatabase` on the wire. It never calls it. The server
refuses it to a caller who does not own the managed executor and its live capability (`crates/application/src/worker/database.rs:22, 63`); an admin token holds `database:execute` (`crates/domain/src/identity.rs:585`), so for admin the
refusal rests on that ownership. `sdks/SPEC.md` §7.1 says ordinary SDKs keep 1.5; T041-7, which makes the server offer
1.7, amends it, and the `step-logs-negotiation` scenario asserts the refusal for a worker token and for an admin token.

**Why the report carries the last records, and a unary RPC the rest.** T041-31 measured a transaction of its own per
batch and a final batch before every report: what a step paid was the extra transactions, not the rows
(validation-report.md, 2026-11-19). The report's transaction already exists, holds the attempt's fence and its authority
checks under their locks, and commits once; a carry adds to it at most five statements, *k* trigger calls and *k*
deferred flush calls, and no transaction or round trip of the worker (section 2a). A long step must still be readable
while it runs (CUJ 2), so its earlier records stream through `AppendStepLogs`, about one batch for each second it logs. A field on `Heartbeat` was
weighed again and rejected: a heartbeat renews the lease under the run row, so records there would add statements under
the run lock once a second per long step, where `AppendStepLogs` takes no run row, and a batch's size and failure would
ride on the call that keeps the lease alive. A client stream would need acknowledgement and resend rules in six
languages for a gain the batch already gives.

**Admission.** `AppendStepLogs` gets a fourth class, `Class::Logs`, in `crates/api-grpc/src/limits.rs:489-508`, with its
own counter outside `Shared.in_flight`, so that a flood of log calls never moves the threshold at which discovery is
shed (`limits.rs:548-563, 595`). Its cap is `SAGAT_GRPC_MAX_LOG_CALLS` (default a quarter of `SAGAT_DB_MAX_CONNECTIONS`,
at least 1: 5 of the execution engine's default pool of 20, `apps/execution-engine/src/cli.rs:55`), per replica; a call
over it is shed `RESOURCE_EXHAUSTED` with `retry-after-ms`, as shedding is today (`limits.rs:675-678`). The cap is what
keeps ingest from holding the connections that lease calls need: at most that many of the pool's connections are in a
log transaction at once. The tenant is known only after authentication, so the per-tenant share is taken in the handler: a
ticket set built as `TaskStreams` is (`crates/api-grpc/src/limits.rs:47-129`), `SAGAT_STEP_LOGS_TENANT_CALLS` per tenant
per replica (default half the cap rounded down, at least 1: 2 at the default cap of 5). A batch that finds no ticket free
is answered `RETRY_LATER` with 200 ms at once: no call waits for a ticket, so a call holds its class slot only while it
runs and one tenant cannot fill the Logs class with waiting calls. The SDK resends a batch until the batch is 10 seconds
old or, once the activity has ended, until its 2-second wait before the report ends, whichever comes first. A carry is
not admitted in this class: it is part of its report, whose class is the reserved one of calls that keep leased work
alive; its cost there is bounded by its 64 records and its 200 ms lock timeout (section 2a). Both defaults are placeholders until T041-35 measures the
ingest's service time (section 2, "Capacity") and records the defaults it sets in data-model.md §6. The class test `only_the_calls_that_keep_leased_work_alive_are_reserved_capacity` (`limits.rs:706`) lists
`Logs`. T041-7's tests show, with admission counters and no timing, that a full Logs class leaves discovery admission
(the class that sheds polls for new work) unchanged, and that a second tenant's batch is admitted while the first
tenant's tickets are all taken; the p99 of lease calls with a full Logs class is measured by T041-9.

### 2. Ingest

A new application module, `sagat_application::step_logs`, holds the ingest service, `StepLogIngest<S: StepLogStore>`,
and its port: `StepLogStore::begin()` opens a `StepLogTx`, whose methods are the reads and writes of the steps T1 to T10
below, one method each, and which holds no rule. Its adapter is `crates/infra-postgres/src/step_logs.rs`. The rules are
in the domain and the application: `crates/domain/src/step_logs.rs` holds the types (level, stop reason, ceilings, the
server's sequence range), the validation of a batch, and a pure `plan_batch(stream counters, policy, batch) -> Plan`
that removes duplicates, counts gaps, applies the attempt allowance and decides the marker and the verdict, with
property tests. The stop rule of a report becomes one domain function, `report_fence_allows`, in
`crates/domain/src/execution/state.rs`, which `attempt_authority_fence` and T2 both call (O041-025, answered (a), OD-2026-11-16-11). The application reuses `first_uncovered` (`crates/domain/src/binding.rs:224`) and the role rule of
`ensure_bound` (`crates/application/src/worker.rs:2715`), which it cannot call because `ensure_bound` works on the run
transaction. `WorkerGrpc` (`crates/api-grpc/src/service.rs:53-84`) gains a field holding the service, built where the
execution engine builds `WorkerGrpc` (`apps/execution-engine/src/lib.rs:775`) and, for tests, in
`tests/support/api.rs:616`.

**Pre-checks**, in the application, before any statement (P1 to P5):

- P1. `principal.authorize(Permission::WorkerExecute)`; a token whose role is database-runner or runner is refused
  `PERMISSION_DENIED`. (A membership whose role is one of those, read at T7, is answered `AUTHORITY`.)
- P2. `SAGAT_STEP_LOGS_ENABLED=false` answers `STOP_CAPTURE` (`CAPTURE_OFF`).
- P3. The tenant's ticket, taken by the gRPC handler before it calls the service (section 1), or `RETRY_LATER`.
- P4. `validate_batch`: 1 to 500 records and at most 192 KiB encoded, sequences strictly increasing in 1 to 2^62 − 1, a
  known level; otherwise `INVALID_ARGUMENT`. (A batch carries no final mark: the carry does, section 2a.)
- P5. Normalise (FR-041-003); scrub the message, the exception text, every field key and every string field value with
  the server's shapes, and replace the value of a field whose key is a sensitive name (FR-041-007 (c)); cut each message
  and exception to 8 KiB at a character boundary; build the fields' canonical JSON and keep fields up to 32 keys and
  2 KiB (data-model.md §5); and size each record exactly as `sagat_quota_measure` will measure it.

The server's shapes are reached through two public entry points added in T041-6, each with a test that it equals what
it replaces: `sagat_telemetry::redact::scrub_shapes_with(text, placeholder)` (today `scrub_shapes` in
`crates/telemetry/src/redact.rs:46-66` is private, hard-codes `[redacted]` and is reached only through `scrub`, which
cuts at 2,048 characters) and `sagat_domain::scripts::redact_credentials` (`crates/domain/src/scripts.rs:779`) made
`pub`. No second list of shapes or names is written.

**The transaction** (T1 to T10), one per batch, opened by the project's `begin_safely` rule (ADR-0014) with a lock
timeout of 200 ms; a lock timeout rolls back and answers `RETRY_LATER`. Each step is one method of `StepLogTx`; the
statements are in data-model.md §8.

- T1. The transaction's id first, so that it is assigned before the lease is read (the end rule of section 5 relies on
  it).
- T2. The lease, read without a lock: the `steps` row of the request's step, attempt, generation, instance and
  lease-token hash, `LEASED` or `RUNNING`, unexpired, and the run, both passed to `report_fence_allows` (the step not
  cancel-requested; the run not finished; the run not stopping unless the node is a compensation or reconciliation
  node; OD-2026-11-16-11). Every later step takes the run, namespace, node, path and activity from this row, never from
  the request. No row, or a refusal: `LEASE`, and a run of another tenant is audited as `access.denied.cross_tenant`
  outside the transaction.
- T3. The session (tenant, id, the lease's namespace, the caller's subject, token, unexpired, accepted at 1.7 or later),
  else `SESSION`; and the three step-log resources of the namespace, read under their policy fences taken shared,
  tenant then namespace, as `content_usage::admission_policy` reads a policy (`crates/infra-postgres/src/content_usage.rs:95-106`),
  so that a change of a resource waits for the batches that read the old value. `step_log_attempt_records = 0` is
  `CAPTURE_OFF`.
- T4. The stream row, created or locked to commit. Removal begun, or `expires_at` passed, is `REMOVED`; a final mark
  already recorded (by the attempt's carry) is `FINAL_RECORDED`.
- T5. The plan (`plan_batch`): duplicates dropped, gaps counted (data-model.md §8).
- T6. The allowances, from counters: the attempt's (lines and bytes on the stream row, against the attempt ceilings);
  then the tenant's, `sagat_quota_step_log_allowance(tenant)`, read without a lock. Records are kept in sequence order
  until the first that does not fit the attempt allowance; the rest are dropped by the server, and when the attempt
  ceiling is first met one marker is planned. If the kept bytes, the planned marker's 128 bytes included, do not fit the
  tenant allowance, nothing is stored and the records are counted as `tenant_quota`. Either way the answer is
  `FINAL_ONLY` from then on.
- T7. Authority, read without a lock (OD-2026-11-16-7): the binding's patterns checked against the lease's activity by
  `first_uncovered`, and the caller's membership and its role, refusing the database-runner and runner roles. A failure
  answers `AUTHORITY`. The ingest takes no binding lock, no namespace secrets lock and no membership lock, so it never
  delays a report, a lease, a heartbeat, a secret change or a managed-database admission; a batch that commits just
  after its worker's binding or membership is revoked is still stored, a cost the owner accepted for a lossy record.
- T8. Insert the kept records and the marker, at most once each (attempt, generation, sequence).
- T9. Counters on the stream row, from the rows the insert returned, never from the batch; `last_at` and `expires_at` on
  every recorded batch, and `last_stored_xid = GREATEST(last_stored_xid, this transaction's id)` when T8 inserted a row
  (the order guard of section 2a; a trigger refuses a lower value).
- T10. Commit, then answer.

The control flow, so that a caller whose binding or membership is gone learns only `AUTHORITY` from the transaction,
regardless of what else is true (D041-050):

- `LEASE` (T2) ends the transaction at once: there is no activity to check authority against.
- `SESSION`, `CAPTURE_OFF` (T3), `REMOVED` and `FINAL_RECORDED` (T4) are noted and the transaction skips to T7. If T7
  passes, the noted answer is given; either way the transaction rolls back, so nothing is stored or counted.
- A failure at T7 answers `AUTHORITY` and rolls back. The gRPC handler then calls `spawn_audit_access_denied` itself,
  since the answer is a verdict and not an error status (`crates/api-grpc/src/service.rs:367-385` audits only errors).
- `FINAL_ONLY` (T6) is not a stop: the transaction goes on through T7 to T10, so that the counters record the drops.
- `stored_through_sequence` is sent only with `CONTINUE`, `FINAL_ONLY` and `FINAL_RECORDED`, and is 0 otherwise.
- A batch never sets `final_at`; only a carry does (section 2a).

At T10 the finalizer of ADR-0027 runs. If it raises `tenant_step_log_quota_exceeded` (batches of other attempts crossed
the limit at the same time), nothing was stored or counted, and the answer is `RETRY_LATER` with 200 ms (D041-061). The
SDK sends the same batch again, and its T6 reads the tenant over the limit and counts the records as `tenant_quota`. A
batch the SDK gives up on after its 10 seconds is counted later as a gap, dropped by the worker; a stream whose final
mark is never stored shows "ended without its final flush" (G041-002). ADR-0084 decision 3 holds: the batch stores no
line, and its records are counted on its stream. A crash before the commit leaves nothing and the SDK sends the batch
again; a crash after it leaves the lines, and the resend stores nothing and gets `CONTINUE`, or `STOP_ATTEMPT` once the
report is recorded (FR-041-004). The counting rule, held by a property test of `plan_batch` over any sequence of batches, carries
and refusals: for a stream, lines stored (markers excluded) + dropped by the worker + dropped by the server =
`last_sequence`, the highest worker sequence the server has recorded (once the carry is recorded, its
`highest_sequence`). `plan_batch` plans a carry as it plans a batch, with the carry's `highest_sequence` as the final
mark and `in_order` (section 2a) as an input.

**Statements per batch** (by reading; budgeted as an equality in T041-6, whatever the number of records): `BEGIN`, the
lock timeout, T1, T2, the session, one statement that takes the six policy fences in order, one that reads the three
values, T4, the tenant allowance, the binding's patterns, the membership, T8, T9 and `COMMIT`: 14 round trips.

**One lock order** for everything that writes step logs (ADR-0095 decision 7):

- L0. what a report already takes before its carry: the run rows, then the authority locks of ADR-0050 decision 7 (the
  binding lock, the namespace secrets lock shared, the membership row). Only a report takes them; an ingest, the purge,
  the delete route and the script marker take none;
- L1. the policy fences of the namespace's step-log resources (shared, tenant then namespace);
- L2. the stream row;
- L3. at commit, the policy fences of the classes the transaction charges, in ADR-0027's order (content, log, then
  step-log; an ingest charges the step-log class only, a report with output and a carry both content and step-log);
- L4. the tenant's `tenant_retained_usage` row.

The finalizer takes L3 and L4 in that order for every writer; a change of a namespace step-log resource takes its own
L1 fence exclusively and nothing else of this order. The purge and the delete route take stream rows (L2) in
`stream_id` order, and nothing earlier. The script marker (section 7) takes L1, then a stream row no other transaction
can see, then L3 and L4. A report that carries nothing takes none of L1 and L2; a carrying report takes L0, then L1 and
L2 for its own attempt's stream only, then L3 and L4. Putting L1 before L2 is the reading of ADR-0084 decision 11 the
owner accepted (O041-025, OD-2026-11-16-11). No cycle can form: no holder of L1, L2, L3 or L4 then waits for a run row or
an authority lock. The lock-order test, built on `tests/support/lock_order.rs` with new slots for L1 and L2 placed after
its `Bookkeeping` slot and before `Usage` (a carry runs right before the report's commit), grows with the tasks that add
a writer: T041-6 runs an ingest, a report that carries nothing and a change of a step-log resource; T041-10 adds the
purge; T041-36 adds a carrying report beside an ingest of the same attempt, the purge and a queued change of a step-log
resource; T041-13 adds the script marker and T041-30 the delete route.

**What a report can wait for** (the bound). A report of tenant T takes T's usage row (L4) in its own commit when it
stores output or keeps a carried record. It waits there only for the commits queued ahead of it on that row: the other
reports of T that store output or keep carried records, as completions with output already queue today; at most
tickets × R ingests of T, each holding the row from its first deferred flush call to the end of its commit; the purge's
delete transactions, each holding it for at most 500 rows; and script markers, each holding it for the flush of one
row. Before its commit, a carrying report can also wait on L1 (a change of a step-log resource holding or queued for its
fence exclusively) and on L2, its own attempt's stream row, which an ingest of the attempt holds from T4 to its commit
(only after the SDK's 2-second wait for its batch in flight expired, section 6) and which the purge's mark, the purge's
delete and the delete route hold while they work on an expired or deleted stream. Each of those waits is bounded at
200 ms by the carry's lock timeout (section 2a, CA); past it the carry fails and the report is re-run once without it. A
report that carries nothing takes neither L1 nor L2. Before L1, the ingest takes none of the locks a report, a lease or a
heartbeat takes. The effect of these waits on time is what SC-041-007 bounds (the completion p99 within 10% of its value
with capture off), measured by T041-35 before the schema and T041-9 at the end. A per-tenant step-log usage row of its
own (O041-026) would move the step-log part of the queue to a row of its own; it would not remove it, since every report
that keeps a carried record would take that row.

**Capacity.** `AppendStepLogs` now serves only long steps: each sends about one batch for each second it logs, plus
the batches its last records need when they do not fit one carry. One tenant's batch rate on a replica, in batches per
second, is therefore about the number of its long logging attempts in flight there. By Little's law the ingest holds
rate × S tickets on average, S being the service time of one batch, here taken at its p99 to be conservative. T041-31
measured S p99 at 4 to 8 ms at 1 replica and 16 workers and at 35 to 89 ms at 8 replicas and 128 workers, where the
database served 128 workers; and its ticket gate failed in every round (11 to 16 tickets needed against 10). The defaults
of section 1 (a cap of a quarter of the pool, 5 of 20, and 2 tickets) stay placeholders until T041-35 measures S under
workload (g), in which every step sends one batch and a carry, at both sizes: n tickets at 70% use serve 0.7 × n / S
batches a second (at S = 8 ms, 175 at 2 tickets and 875 at 10). T041-35 then sets the tenant tickets from (g)'s p99 of S
and its offered batch rate (tickets ≥ rate × S / 0.7, the cap at least the tickets and at least a quarter of the pool),
and if that needs more than half the pool, the design changes and is reviewed again. Carrying reports hold their own
pool connections a little longer (section 2a); the pool's budget at the chosen cap, leases, reports and ingests together,
is recorded with the figures.

### 2a. The carry on a report

The last records of an attempt travel on its report and are written by the report's own transaction (OD-2026-11-19-1,
ADR-0095). Nothing here changes what the report itself checks, decides or answers.

**What a carry may hold.** At most 64 records and 16 KiB encoded (the encoded `StepLogCarry` message), and
`highest_sequence`. The bound is set by the time a carry runs under the run lock, not by the message size: T041-31
measured a 500-record write at 61 ms p50 and about 200 ms p99, some 25 times a ten-record one, because each row fires a
trigger and a deferred call (validation-report.md). 16 KiB also keeps a report with a 256 KiB output well under the
decoding cap (section 1). An attempt whose remaining records do not fit one carry sends the rest as batches before its
report (section 6).

**Before the transaction**, in the application, so that no CPU work of the carry runs under the run lock:

- The gRPC handlers of `CompleteStep` and `FailStep` (`crates/api-grpc/src/service.rs:931, 1244`) pass the request's
  `step_logs`, when present, and its encoded size to the carrying entry points of `WorkerService` (T041-36); without the
  field they call the present methods, and the report costs what it costs today.
- C1. `WorkerSettings.step_logs_enabled` false (the deployment switch `SAGAT_STEP_LOGS_ENABLED`, which the execution
  engine passes to `WorkerService`), or `WorkerStore::supports_step_log_protocol()` false: the carry is ignored (`off`), no
  statement.
- C2. `validate_carry`: at most 64 records and 16 KiB encoded, sequences strictly increasing in 1 to 2^62 − 1, a known
  level, `highest_sequence` at least the last record's sequence and below 2^62. A carry that fails but whose
  `highest_sequence` alone is valid becomes a *mark-only* carry, its records counted as dropped by the server (`invalid`);
  one whose `highest_sequence` is not valid either is ignored (`invalid`), and the report is recorded without it. A carry
  never makes a report fail (FR-041-005).
- C3. P5 of section 2: normalise, scrub with the server's shapes and sensitive names, cut each message and exception in
  bytes at a character boundary so that no row can reach the size CHECK of data-model.md §2, build the canonical fields
  and size each record. It runs before the lease is checked, on at most 16 KiB.

**In the transaction**, from what the report has already loaded under the run lock, with no statement:

- C0. The carry is ignored (`refused`) when the caller's token role is runner or database-runner, or when the step is a
  managed step (`managed_kind` is a database, lake or probe kind): those steps are run by the platform's runners, whose
  output is kept as the managed paths keep it and never as step logs (FR-041-016), and the ingest refuses the same roles
  (P1, T7).

**Which transaction writes it.** The carry is an explicit argument, `CarryPlan` (`Full`, `MarkOnly` or `None`), of
every function that records an attempt and commits, so that the compiler makes each site say what it writes:

| Transaction that records the attempt | What it writes |
|---|---|
| `finish` (`crates/application/src/worker.rs:2861`) after a root completion, failure, reconciliation or compensation | the carry |
| `finish_path_report` (`crates/application/src/worker/scoped_reports.rs:419`) after a scoped completion or failure | the carry |
| `finish` after `reject_secret_output`, and `refuse_scoped_output` (`worker/scoped_output_refusal.rs:25`), which record a refused output (a declared secret, or the run's data limit) | a mark-only carry: no line of an attempt whose output held a secret is stored |
| the content-quota fallbacks `fail_success_for_storage_quota`, `fail_report_for_storage_quota` (`worker.rs:2474`) and `refuse_scoped_content` | a mark-only carry (it charges no content, so the fallback's own reason is unchanged) |
| the step-log re-runs below | a mark-only carry after a step-log quota refusal; none after a failed carry statement |

It runs only when the report is recorded now: a resent report whose idempotency key is already recorded is answered as
a replay (`replayed`, or `tx.replay` on the scoped path) before its carry is read, so a carry is stored at most once,
and its lines are keyed (attempt, generation, sequence) besides. The carry is not part of the report's idempotency
fingerprint (`completion_idempotency`), so a resend with another carry is answered as the replay it is.

The stream's identity (tenant, namespace, run, step, node, instance path, attempt, generation) is the attempt the
report's fence verified, captured before the engine applies the report, never read from state after it (a retryable
failure has by then moved the step to its next attempt) and never from the request.

**The statements**, right before the report transaction's commit, after its engine change and journal writes. They are
the five methods of a port, `StepLogCarryTx`, a supertrait of the report transactions (`PgRunTx` and
`PgPathExecutionTx`, implemented in `crates/infra-postgres/src/step_logs.rs` through a `pub(crate)` connection accessor
of `execution.rs`), each one statement and no rule; the rules are `plan_batch` and `validate_carry` of the domain. The
statements that write begin with `INSERT` or `UPDATE`, never with `WITH`, so that the budget's measure counts their rows.
data-model.md §8a lists the SQL.

- CA. One `SELECT`: the present `lock_timeout`, kept to be restored; `set_config('lock_timeout', '200ms', true)`; then
  the six shared policy fences of the namespace's three step-log resources (L1, tenant then namespace). From here every
  lock wait of the carry is bounded at 200 ms.
- CB. One `SELECT`, whose snapshot begins after CA's locks are held: the three resources' rows (the read
  `effective_policy` makes, batched as T041-6's variant of `admission_policy`) and `sagat_quota_step_log_allowance(tenant)`
  read without a lock. Resolved in the application: `step_log_attempt_records = 0` is capture off in the namespace; then
  CR, one statement that restores `lock_timeout`, and nothing more (`off`).
- CC. The stream row, created or locked to commit (L2), as T4 creates it, returning its counters, `removed_at`,
  `expires_at`, `final_at` and whether its `last_stored_xid` is below this transaction's id (`in_order`); when the row is
  removed, expired or already final, its `RETURNING` also restores `lock_timeout`, and nothing more is written
  (`removed`, or `final_recorded`, which only a hand-written client that sends a second report reaches).
- The plan: `plan_batch` with the carry as the final batch and `in_order` as an input: duplicates dropped, gaps
  counted, the attempt allowance, the ceiling marker, the tenant allowance from CB's unlocked read (the kept bytes plus 128
  a row, the marker included). When `in_order` is false, no record above `last_sequence` is kept and those records are
  counted as dropped by the server (`out_of_order`). A mark-only carry keeps no record and plans no marker.
- CD. One insert of the kept lines and the marker, at most once each (attempt, generation, sequence), returning the text
  size of each row. Not run when nothing is kept.
- CE. The stream's counters from CD's rows, never from the carry; `last_sequence`, both drop counts,
  `ceiling_sequence`, `ceiling_reason`, `first_at`, `last_at`, `expires_at`, `final_at` (which only a carry sets), and
  `last_stored_xid = GREATEST(last_stored_xid, this transaction's id)` only when CD inserted a row; its `RETURNING`
  restores `lock_timeout`.

**At the commit** the finalizer of ADR-0027 charges the kept rows to `tenant_step_log_bytes` (L3, L4), as for a batch.

**When the carry makes the report's transaction fail**, the report is recorded anyway by one *re-run*: the report's
method runs again from its start, as the worker's own resend of the same report would, with the carry downgraded
(ADR-0095 decision 5). A re-run is never re-run.

- Any error the finalizer's step-log branch raises at the commit (`tenant_step_log_quota_exceeded`, or a guard of that
  branch, which T041-5 gives a constraint name of its own): the store maps it to a typed refusal, and the re-run carries
  a mark-only carry, with the records counted as `tenant_quota` (or `failed` for another error). A mark-only carry charges
  nothing, so the step-log branch does not run again (`quota_rerun`).
- A carry statement fails (a lock wait past 200 ms, a lost connection, a bug): the store maps the error of CA to CE to
  `StoreError::StepLogCarry`, a new variant that keeps only the SQLSTATE (through `store_error_without_text`) and becomes
  `AppError::Unavailable` if it ever escapes; the re-run carries none (`failed`), and the stream keeps no final mark.
- A content-quota raise in a re-run goes to the content fallback of the table above, which keeps the downgraded carry
  (mark-only after a quota refusal, none after a failed statement).
- The re-run is fenced as any report: if the lease expired between the two transactions, it is refused as a late
  report would be (G041-021). This is the one case where capture changes a report's answer, and only after its first
  transaction failed because of the carry.

**What a report pays** (the bound that replaces "capture adds no statement to a lease or a completion", ADR-0084
decision 2, superseded by ADR-0095 decision 2 when accepted):

| Report | Statements | Transactions | Rows (the budget's measure) |
|---|---|---|---|
| no carry; or a carry ignored by C0 to C2 | as today | as today | as today |
| capture off in the namespace | + 3 | + 0 | + 0 |
| the stream removed or already final | + 3 | + 0 | + 1 |
| a mark-only carry, or a carry with no record kept | + 4 | + 0 | + 2 |
| a carry with *k* rows kept (a ceiling marker counts as one) | + 5 | + 0 | + 2 + *k* |
| a carry whose transaction fails (above) | the failed transaction, then the report once more with a mark-only carry or none | + 1 | as the re-run's row |

So `work_budget.rs` keeps its three constants for leases and for reports without a carry, and gains six equalities, each
counted as its constants are (T041-36): a completion with a next step that carries ten records (55 statements, 1
transaction, 23 rows), a mark-only carry (54, 1, 13) and capture off in the namespace (53, 1, 11); and the completion that
ends the run in the same three cases (52, 1, 21; 51, 1, 11; 50, 1, 9). Inside its statements a carry with *k* kept rows
also fires *k* row-trigger calls (each an upsert of the transaction's `quota_transaction_deltas` row) and, at the commit,
*k* deferred flush calls, the first of which takes L3 and L4; the budget counts top-level statements only, so T041-35
measures their time. Each statement is a round trip while the run row is held: at a round trip of *r* milliseconds to the
database, a carry holds the run row about 5*r* longer (G041-022; T041-35's database is on the same machine). A report that
keeps a carried record takes the tenant's usage row at its commit whatever its output. A lease and a heartbeat pay
nothing. A short step pays no transaction of its own; a long step pays about one batch transaction for each second it
logs, and the batches that what does not fit its carry needs.

**What does not change.** The ceilings, the 128 bytes a row and the separate tenant quota (OD-2026-11-15-5, -6), the
configurable attempt ceilings (OD-2026-11-16-9), the absence of a run ceiling (OD-2026-11-16-10), the SDK's redaction with
the process's secret set and the server's scrubbing (OD-2026-11-15-7), the reads and the PII reveal grant on every read
(OD-2026-11-16-8): a carry's lines are rows of the same tables, read by the same routes. A compensation or reconciliation
of a stopping run carries its lines whenever its report is accepted. On the root path a report and a batch share the stop
rule `report_fence_allows` (OD-2026-11-16-11); on the scoped path an ordinary report is fenced by `verify_path_lease` and
not by the stop rule, so a scoped forward step of a stopping run can store its carry while its batches are answered
`LEASE` (D041-095). The carry needs no worker session: a report is fenced by its lease and by the binding and membership
read under their locks, and C0 refuses the runner roles and managed steps, which together cover what the ingest's T3 and
T7 check but the session's minor; a report of a client that negotiated below 1.7 can carry records, and they are stored,
a case of G041-001 (D041-092).

### 3. Storage

Two tables, in one migration of T041-5 (data-model.md §2 gives the DDL): `step_log_streams` (one row per stream, with
its counters and expiry) and `step_log_lines` (one row per stored line or marker), with a run-scoped and a
stream-scoped read index.

**No foreign key to `runs` or `steps`.** An insert that references `runs` takes `KEY SHARE` on the run row, which
conflicts with the `FOR UPDATE` the engine holds while it leases or completes. The run's existence in the tenant is proven
by T2 of the ingest, or by the report's own lease for a carry, in the same transaction, and nothing deletes a run today. Feature 020 must delete a run's streams,
lines and stream rows when it deletes the run (G041-004, registered in feature 020's research as G1618). `step_log_lines`
references `step_log_streams` (same tenant), a row the engine never locks. A trigger on `step_log_streams` refuses an
update that changes its identity or retention or clears a removal, final or delete time (data-model.md §2), so no
writer can undo a delete.

**Accounting** (ADR-0027 Amendment of 2026-11-15, accepted, OD-2026-11-15-16). `step_log_lines.message`,
`.exception` and `.fields` are of the new class `step_log` in `crates/infra-postgres/storage-classification.json`, and
`.level` carries the fixed charge of 128 bytes (`fixed_octets`, OD-2026-11-15-6); every other variable-size column of
the two tables is `protected_system`, so a marker costs its 128 bytes and nothing more. They count against
`tenant_retained_usage.step_log_bytes` and `tenant_step_log_bytes`, not against `tenant_log_bytes`, so a step that logs
a lot never takes the allowance of script logs, dispatch errors and connector errors (OD-2026-11-15-5). The usage column
comes with the resource in T041-4, so that its usage view has a column to read; T041-5 adds the rest of the Amendment's
accounting (data-model.md §3) and creates the triggers the manifest generates on `step_log_lines` only, the one table
with accounted columns.

**Grants** are those of data-model.md §4: no `UPDATE` on lines, no grant on `namespaces`, no `TRUNCATE`, and no
`SECURITY DEFINER` function beyond the accessor `sagat_quota_step_log_allowance` of ADR-0027's Amendment, revoked from
`PUBLIC`; the functions the migration replaces keep their `SECURITY DEFINER`, pinned `search_path` and grants, tested
in T041-5. The runtime
privilege audit covers the new statements. There is no row-level security in this database; the tenant boundary is the
`tenant_id` in every key and predicate, tested by the cross-tenant battery
(`tests/contract/security-multitenancy/rest/battery.rs`). The down migration refuses while a line exists.

**No classified-namespace record.** Every read needs the PII reveal grant (FR-041-021, OD-2026-11-16-8), so the
design keeps no record of which namespaces carry annotations; the partial index of D041-067 is not built.

### 4. Retention, purge and deletion

A stream takes `step_log_retention_days` from its namespace when it is created and keeps it (`retention_days`); its
`expires_at` is the server's time of its last recorded batch or carry plus that retention, written by T9 of every batch
and CE of every carry. This
is ADR-0027's retention rule ("lowering policy does not rewrite stored expiry … or authorize retroactive deletion",
[ADR-0027](../adr/ADR-0027-durable-content-rate-and-retention-quotas.md), line 93): a lowered retention applies to the streams created after it, and the delete route is the
way to remove lines early.

A housekeeping pass (`crates/scheduler/src/passes.rs`, beside `purge_script_records`, through a new method of
`MaintenanceStore`, `crates/application/src/maintenance.rs:117`) removes lines in two steps:

1. **Mark.** Streams with `removed_at` null and `expires_at` past (`step_log_streams_expiry_idx`) are locked
   `FOR UPDATE SKIP LOCKED`, a page at a time, and get `removed_at`. From then on readers show the stream as expired and
   return none of its lines, and T4 of the ingest answers `REMOVED` (and CC of a carry stores nothing) under the same
   row lock, so no batch or carry can add a line to a stream whose removal has begun. The delete route sets `deleted_at` and `removed_at` together.
2. **Delete.** For streams with `removed_at` set and `purged_at` null (`step_log_streams_removal_idx`), at most 500 lines
   per transaction are deleted; when none is left, `purged_at` is set. Deleting releases the quota through the triggers,
   and 500 rows per transaction bound how long the purge holds the tenant usage row.

A stream whose attempt still writes has a recent `expires_at` and is not marked; one that stopped writing for longer
than its retention is marked even if its lease lives, and its next batch is answered `STOP_ATTEMPT` (`REMOVED`).
Stream rows stay until feature 020 deletes the run (G041-004); T041-9 measures their size per row. The purge's backlog,
`step_log_purge_backlog`, is the number of streams whose `expires_at` has passed or whose removal has begun and whose
`purged_at` is null, counted by the purge pass at the start of each run.

`DELETE /runs/{runId}/logs[?stream=]` (FR-041-023) locks the run's streams (or the one named) `ORDER BY stream_id FOR
UPDATE`, sets `deleted_at` and `removed_at`, writes `run.logs_deleted` in the same transaction, every time, and answers
`204` with no body, so the answer says nothing about how many streams the run has; readers skip deleted streams at once. It is admitted as a new `ApiOperation::RunLogsDelete` (`crates/application/src/api_admission.rs:32`) that maps to `&[RunRead, RunLogsDelete]`.

### 5. Reading

A second port, `StepLogReader`, used by new handlers in a file of their own, `crates/api-http/src/routes/step_logs.rs`,
so that the test selector can match them by path (T041-32). Each read calls `admit` with a new
`ApiOperation::RunLogsRead` that maps to `&[RunRead, RunLogsRead]`; the application authorizes `run:logs:read` again
and, in every namespace (FR-041-021), checks
`has_effective_payload_reveal_grant` (`crates/infra-postgres/src/reveal_grants.rs:40`) in the read transaction, which
holds the grant-set lock shared, so that a revocation cannot commit between the check and the audit. This check is one
application function, `step_logs::may_read(principal, tx, run)`, built in T041-11; it takes the read transaction, and
every route that discloses step-log text or a script execution's log calls it inside its own read transaction and writes
the throttled `run.logs_read` record in that transaction before it answers (T041-11, T041-13, T041-29). The permissions
`run:logs:read` and `run:logs:delete` join `Permission::ALL` and the role matrix test. The refusals are the error
table of contracts.md §2. No switch turns the read routes off (D041-069): rate admission and a binary rollback already
shed read load.

**A stream's state** (contracts.md §2) is worked out by the read from the stream row and, for a worker stream, the
`steps` row of its `step_id`, read without a lock: `deleted` when `deleted_at` is set; `expired` when `removed_at` is
set otherwise; `ended` when `final_at` is set; `ended_without_final` when none of those is set and the `steps` row no
longer holds this attempt's lease (its status is not `LEASED` or `RUNNING`, its attempt or generation differs, or its
`lease_expires_at` has passed); `open` otherwise. A stream whose `expires_at` has passed is `expired` and returns no line
even before the purge marks it. FR-041-008 forbids the lease and heartbeat from touching the log tables and limits a
report to its own carry; it does not forbid the read routes from reading `steps`. A script stream is `deleted`, `expired` or `ended`; the `source = script` item of
the streams route is not the stream but the execution, whose state is `expired` once the execution's own expiry has
passed and `ended` before, whether or not its marker was written or deleted.

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
live took its id before it read the lease (T1), and that read did not see the run finished, so its id is below
`end`. A carry is written by its report's transaction, which holds the run row; the run is finished only by a
transaction that holds that row, so a carry commits before any reader can see the run finished, or in the very
transaction that finishes it, and its id is below `end` too. A read whose cursor holds `end` answers `complete: true`, and the tail sends `event: end`, only when the run is
finished, `pg_snapshot_xmin(pg_current_snapshot()) > end` in the reading statement, and the request's scan ended because
no row below the horizon was left in the index range it reads (neither the page limit nor the 5,000-row bound stopped
it), whether or not the filters matched any row. That predicate is one pure function of (run finished, horizon, `end`,
why the scan stopped), unit-tested on each case. An `end` in a
cursor for a run that is not finished is ignored, so a forged cursor cannot end a running run's tail. The comparison is
strict, as in D1958 (`crates/infra-postgres/src/batch.rs:453-459, 654-656`), where the old horizon `xmax` let a
transaction open with that id pass `>=` (a held transaction whose id equalled the horizon was missed). T041-11 builds the
rule for the list route's `complete`, with tests of a finished run of 12,000 lines read with a filter that matches only
lines 11,000 to 11,010 and of a finished run of 300 lines read unfiltered at `limit=50` to its end; T041-12 uses the
same function for the tail and tests a tail over 450 lines, an ingest held after T2 across the end and a first read of a
new audit window that sees the run finished.

**Filters.** The run index (`tenant_id, run_id, stored_xid, sequence`) serves a read without `stream`; the stream index
(`tenant_id, stream_id, stored_xid, sequence`) serves a read with it, so a step's view reads only its attempt log.
`node`, `attempt`, `minLevel`, `from` and `to` are evaluated on the scanned rows, and `q` is
`position(lower($q) in lower(message)) > 0` as a bound parameter. A request scans at most 5,000 rows of the run from its
cursor, in index order, and `nextCursor` is the position after the last row it scanned, whether or not it matched, as
the tail's `progress` event is; so a filter that matches nothing moves on and never rescans. A client that gets an
empty page with `complete` false reads again at once only while `nextCursor` moved; when it did not move, the client
has caught up and polls at the tail's interval (section 8). `may_read` runs right after the run is found and before
`stream` is resolved, so a reader without the grant gets the same `403` whether or not the stream exists. `stream` is
matched by public id together with the run and the tenant. A marker has the level `info`, and `minLevel` never hides a marker.

**Audit.** `run.logs_read` is a new `AuditAction` in `crates/domain/src/audit.rs`, added to `collapses_repeats()` and
to `repeat_window()` with 300 seconds (`audit.rs:259, 287`). It is written by `insert_audit`
(`crates/infra-postgres/src/identity.rs:239`), which takes an advisory lock on (tenant, actor, action, resource) and
writes only when no identical record exists in the window, so two first reads on two replicas write one record. The
resource is the run and the record holds no filter, no line and not the value of `q`, so that every read of the window is
the same event. It is written in the read transaction before the page is returned ("no audit, no lines"). A tail
remembers when its window began and writes the record again, before the first line of the next window, when 300 seconds
have passed.

**Tail.** `GET /runs/{runId}/logs/stream` (contracts.md §2) follows the loop of `follow` in `crates/api-http/src/routes/execution.rs:1340`, refactored
to take the poll function and the event encoder as parameters so that the graph stream and the tail share one loop:
`event: log` with up to 200 lines and the cursor as `id`; `event: progress` with the cursor when the scanned position
moved and no line matched the filters, so that a filtered tail does not rescan on reconnect; a `: ping` every 15 seconds
otherwise; `event: end` by the rule above; `event: reconnect` at token expiry or after 15 minutes; resume by
`Last-Event-ID`; polling every 0.5 s and every 2 s once idle for 30 s; each poll's statement also checks, without a
lock, that the reader's PII reveal grant still exists, and the reader's membership and the
grant under its lock are checked again every `StreamLimits::recheck` (30 s, `crates/api-http/src/streams.rs:34-50`); the
caps of the graph stream (20 per subject, 200 per tenant), shared with it. The *graph stream* is
`GET /runs/{runId}/stream`, the run's existing server-sent event route.

**Contract.** The four operations are tagged `x-planned: '041'` until their route task serves them, and the battery's
`body_of` (`battery.rs:29`) gains them when they are served. The digest test
`operations_outside_the_approved_additions_and_event_extensions_are_byte_stable`
(`tests/contract/payload-classification/openapi.rs:343-388`) gains `x-planned == "041"` and `x-implemented == "041"` in
its list of approved additions. The `ScriptExecution` change of FR-041-022 is additive (`log` stays required; an optional
`logWithheld`); the digest hashes path operations only, so it moves only for the descriptions of `getScriptExecution`
and `listScriptExecutions`, which T041-29 changes and records in the digest's comment. No check today fails when a
component schema used by an existing operation changes; T041-29 says so in its pull request.
`recordScriptExecution` does not change: the script marker is keyed on the execution the report creates (section 7).
The `Quota` description states one more case in which `used` is absent: `tenant_step_log_bytes` shows no `used` to a
caller without `run:logs:read`, so that a viewer, who holds `quota:read`, cannot watch how much a tenant's runs log (ADR-0085 decision 6;
T041-4).

### 6. The six SDKs

`sdks/SPEC.md` §6.3 gains the logger's semantics and a new section defines capture; §6.7, which today has an SDK wipe a
secret value when the handler returns, gains the redaction set's own rule (kept for the process, never logged, printed
by `Debug` or reachable from activity code, wiped at `stop()`). Its §9 already holds the register row of 1.7; T041-7
amends §9's text for the minor and §7.1 for managed database operations. The specification has two sections numbered
10 and one numbered 11, in the order 10, 11, 10; T041-14 renumbers the second 10 to 12, and the one on failure isolation keeps its rule that a
logger that throws never kills a heartbeat or a task, and adds that capture never does either.

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

How each SDK maps levels, exceptions, context and options:

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
with an 8 KiB message and a secret set of 1,024 values, written with the language's monotonic clock in the SDK's test
tree (10,000 calls after 1,000 warm-up calls, the p99 printed), run by hand on the machine of the validation report and
not in CI; its p99 must be under 1 ms, and the figure is recorded in `specs/041-step-logs/validation-report.md`. An SDK
that does not meet it replaces its plain search by a multi-pattern matcher (Aho–Corasick) written in the SDK, with no
new dependency, in the same task.

Common rules, each a line of `sdks/SPEC.md` and a conformance assertion:

- **The process's secret set** (OD-2026-11-15-7): every value `ResolveSecrets` returns in the
  process is added and kept for the life of the process, up to 1,024 values or 256 KiB; when the set is full, capture
  stops for the process, one local line says so, and the records not captured count as `secret_set_full`; each attempt that assigned a sequence still sends its carry
  with `highest_sequence` and no record, so its drops are counted. The SDKs add the value where they resolve it (Java
  `WorkerHost.java:977-992`, TypeScript `host.ts:896-913`, Go `task.go:456-460`, Python `host.py:1111-1150`, C#
  `WorkerHost.cs:1299-1351`, Rust `capabilities.rs:111-113`). Every captured line is redacted with the whole set
  (FR-041-007 (a)).
- A logger call at or above the capture level takes the next sequence first. If the record will be dropped anyway (the
  attempt's buffer or the process bound is full, the attempt was answered `FINAL_ONLY`, or capture stopped), it is
  dropped then, before any formatting or redaction, so that a flood past a ceiling costs no redaction. Otherwise it
  formats, redacts, cuts to the line ceilings and appends to the attempt's buffer, without waiting. A dropped record
  leaves a gap that is the server's evidence, and the SDK counts the reason locally (data-model.md §7).
- A sender per attempt flushes through `AppendStepLogs` one second after the attempt's first captured record and every
  second after, or as soon as 256 records are buffered, one batch in flight. A short step (see Terms) makes no
  `AppendStepLogs` call. The answers are those of contracts.md §4: two retries of the same bytes for
  `UNAVAILABLE` and `DEADLINE_EXCEEDED` (after 200 ms and 1 s, ±20%), then the batch is dropped; `RETRY_LATER` and
  `RESOURCE_EXHAUSTED` wait the given delay and do not count, up to 10 s of a batch's age; after `FINAL_ONLY` no record
  is sent, in a batch or in the carry.
- **The carry** (OD-2026-11-19-1). When the activity returns or fails, an attempt that assigned at least one sequence
  puts its last records (the newest, at most one carry: 64 records and 16 KiB encoded) and `highest_sequence` in the
  `step_logs` field of its `CompleteStep` or `FailStep`. A short step does nothing else. A long step first waits for its
  batch in flight, if any, to be answered, and sends as batches, oldest first, the records that do not fit one carry,
  all within 2 seconds of the activity's end. When the 2 seconds end, the SDK stops resending the batch in flight (its
  answer, if it comes, is ignored), drops what is still buffered beyond one carry (counted locally as `report_deadline`),
  and sends the report with the newest records that fit one carry; the carry's `highest_sequence` counts every drop. An
  attempt that assigned no sequence sends no carry (no stream, "no log"); one that was answered `STOP_ATTEMPT` or
  `STOP_CAPTURE`, or whose capture is off for the session, sends none either; one answered `FINAL_ONLY` sends a carry
  with no record. A resent report (a lost answer, `UNAVAILABLE`) carries the same bytes, and the server stores the carry
  at most once. Nothing about the carry is in the report's answer. The SDK never prints a report that holds a carry
  (FR-041-007 (d)).
- After `STOP_CAPTURE` (`CAPTURE_OFF`, or `SESSION` when the token sent is the one held) capture stays off for the
  session until the next successful registration, which brings a new policy.
- The buffer is in memory only and dropped with the attempt. Capture uses no thread or task that can keep the process
  alive after shutdown; worker shutdown sends each attempt's buffered records once, as a batch, within the shutdown
  deadline, and an attempt that still reports before the process exits carries the rest.
- The SDK never logs a batch, and nothing about capture is written at a level the host logs by default except one line
  per process when the server stops capture or the secret set is full.

**Conformance** (T041-14 extends `tests/contract/sdk-conformance`): activity behaviours `logs` (a list of level, message,
fields, exception and a repeat count) and `logsSecret` (log the resolved secret in the six positions of SC-041-004), a
step op `stepLogs` that reads `GET /runs/{runId}/logs` and `/logs/streams` and compares them, normalised (times, ids and
cursors removed), with the golden; a fault action `verdict` on the gRPC channel that answers `AppendStepLogs` with a
given verdict, implemented by each SDK's fault plan (`sdks/go/conformance/faults.go`, `sdks/python/conformance/faults.py`,
`sdks/typescript/conformance/faults.ts`, `FaultPlan.java` under `sdks/java/conformance/`,
`sdks/dotnet/conformance/FaultPlan.cs`, `sdks/rust/conformance/src/faults.rs` with its gRPC proxy
`sdks/rust/conformance/src/faults/grpc.rs`); and an evidence key `mustNotStore`, searched in the stored lines, the
routes' bodies and the server's captured output (`mustNotLeak` searches only errors and `mustNotAppear` not the worker
log, `tests/contract/sdk-conformance/harness.rs:942-1040`); a harness self-test shows `mustNotStore` failing on a
planted canary. The `stepLogs` op polls the list route until `heldBack` is false, within a bound, so that a transaction
elsewhere in the cluster does not make it flaky. Each scenario file names its *pending languages*: an SDK whose task has
not landed must fail it on the `stepLogs` op, which every step-log scenario uses, with that runner's existing refusal of
an unknown op, and passes only that way. The harness matches the six refusals as they are today: `unknown step op` in
TypeScript (`sdks/typescript/conformance/runner.ts:861`), Go (`sdks/go/conformance/run.go:926`), Python
(`sdks/python/conformance/runner.py:1764`), C# (`sdks/dotnet/conformance/RunMode.Operations.cs:430`) and Java
(`Operations.java:766` under `sdks/java/conformance/`), and `scenario operation stepLogs is not implemented yet` in Rust
(`sdks/rust/conformance/src/runtime.rs:434-436`). Each SDK task removes its language from the five files. So CI, which plays every scenario in every SDK (`.github/workflows/ci.yml:398-406`),
stays green between T041-14 and T041-20. Five scenarios, one file and one golden each:

1. `step-logs-capture`: the five levels and the capture level, fields of each type, a non-scalar field, NUL and
   non-finite doubles, an exception, order by sequence, two attempts of one step, an iteration of a root collection,
   language context captured as fields, a field map over 32 keys and over 2 KiB, a host logger set above the capture
   level.
2. `step-logs-redaction`: two attempts in flight at once with different secrets, and a third that starts after the
   first ended; each logs its own and the others' secrets whole, inside a message, as a field key, as a field value, as
   a number equal to a numeric secret (12345678 and 1e15) and inside an exception; `mustNotStore` for all.
3. `step-logs-limits`: a message over 8 KiB is cut and flagged; a flood meets the buffer and the attempt ceiling; the
   step completes with its normal output; stored + dropped by the worker + dropped by the server = the highest
   sequence.
4. `step-logs-transport-faults`: a lost response (the same batch twice), `INVALID_ARGUMENT`, `RETRY_LATER`,
   `RESOURCE_EXHAUSTED`, `UNIMPLEMENTED`, `FINAL_ONLY`, a re-registration in the middle of an attempt, the server down for
   the whole attempt; a lost answer to a carrying `CompleteStep` and to a carrying `FailStep` (the report sent twice); a
   short step (one carry, no batch) and a long one (batches, then a carry); a batch's answer held by the fault proxy,
   after which the report is sent only once the answer is released or 2 seconds have passed (ADR-0095 decision 8); no
   duplicate; the step completes.
5. `step-logs-negotiation`: a worker pinned to 1.5 sends nothing; a 1.7 worker told `enabled = false` sends nothing;
   both log locally; a 1.7 ordinary worker calling `ResolveDatabase` is refused, with a worker token and with an admin
   token.

Before an SDK's task lands, each scenario fails in that SDK only because its runner refuses the new behaviours and the
`stepLogs` op as unknown, which the pending list expects; after it, a failing scenario names the assertion of its golden
that did not hold.

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

**When it is written** (OD-2026-11-16-7). The decision runs in the adapter's one method, which calls the domain's
`script_marker_decision` between its reads and its writes. `record_execution` (`crates/infra-postgres/src/scripts.rs:1775`) is unchanged
and commits first. When it reports the execution inserted (not a repeat) for a run, the application's script service
(`crates/application/src/scripts.rs`) calls `StepLogStore::record_script_marker(execution)`, a short transaction of its
own in `crates/infra-postgres/src/step_logs.rs`. The decision is a pure function of the domain,
`script_marker_decision`, beside `plan_batch` in `crates/domain/src/step_logs.rs`, over facts that transaction reads: the
control plane's `SAGAT_STEP_LOGS_ENABLED` (passed to the script service by the control plane's configuration), the
namespace's `step_log_attempt_records` above 0, and the run's `finished_at` null. A run already finished gets no marker,
so that a marker can never commit after a tail has sent `end`. Each outcome is counted (`step_log_script_markers_total`,
data-model.md §7), which is where a skipped marker is seen. A crash between the two commits loses that one marker; the
streams route still lists the execution.

**The marker's transaction**, opened by `begin_safely` with a lock timeout that is a parameter of the store
(`StepLogStore::with_marker_lock_timeout`, 200 ms by default; the T041-13 tests that hold a lock the marker waits on
pass 60 s, as the ingest's held transactions have), in this order, which keeps the one lock order of section 2:

1. `SELECT pg_current_xact_id()` first, as the ingest does, so that a marker whose transaction saw the run unfinished has
   an id below any end horizon (section 5).
2. The facts of the decision: the run's `finished_at`, read without a lock, and the namespace's capture and retention
   resources, read under their policy fences taken shared (L1), as the ingest's T3 reads them.
3. The marker stream: `INSERT … ON CONFLICT (tenant_id, public_id) DO NOTHING` into `step_log_streams` with
   `source = script` and `public_id` the execution's id, so a repeated call stores nothing; `last_at` now and
   `expires_at` the earlier of now plus the namespace's `step_log_retention_days` and the stored
   `script_executions.expires_at` of that (tenant, id), read in this statement (the store keeps the retention pinned on the lease,
   `crates/infra-postgres/src/scripts.rs:1812-1826`, which can be shorter than the current one), so that the marker
   never outlives the execution it opens.
4. The tenant allowance, `sagat_quota_step_log_allowance(tenant)`, without a lock: if the marker's 128 bytes do not
   fit, nothing more is written and the metric counts `skipped_quota`.
5. One line of kind `script_marker`, level `info`, with the execution id, at the stream's `next_marker_sequence`.
6. Commit: the finalizer charges the marker to `tenant_step_log_bytes` (L3, L4). If it raises
   `tenant_step_log_quota_exceeded`, the transaction has rolled back, no marker is stored and the metric counts
   `skipped_quota`. The script record, already committed, never depends on step-log quota (ADR-0027 Amendment, item 1).

Any other error of the marker's transaction (a lock timeout, a lost connection, a deadlock) is caught by the script
service, counted as `failed`, logged without text, and never changes `record_execution`'s answer, so the runner never
retries a report that was kept; the marker of that execution is then lost, as after a crash.

`take_log_prefix` and its callers do not change, and the Amendment's locking accessor
`sagat_quota_lock_step_log_usage` is not built (a Note in ADR-0027, OD-2026-11-16-7). `listRunLogStreams` with
`source = worker` does not list marker streams; with `source = script` its items are the executions, whose ids are the
marker streams' public ids. The purge and the delete route remove marker lines as any other; the execution and its log
are untouched.

**Tests (T041-13).** One marker per inserted execution, with the execution's node and attempt; with two iterations of
one attempt in a collection, one execution and one marker are kept and the second report answers as a repeat with no
marker; a repeat report writes none; a second call for the same execution stores nothing; a test run, a finished run,
capture off in the namespace and the control plane's switch off write none; at the step-log limit, by the unlocked read
and by the finalizer's raise, the execution is recorded and no line is stored; a marker transaction made to fail (a
`BEFORE INSERT` trigger the test installs on `step_log_streams`) leaves the report answered as kept and counts `failed`,
also while the tenant usage row is held; the marker stream expires no later than the execution, with a retention
pinned on the lease shorter than the namespace's; filtering `listRunLogs` by the execution id returns its marker; as the runtime role
(`SAGAT_TEST_AS_RUNTIME_ROLE=1`); at barriers, a marker, an ingest of the same run, a delete of the run's logs and a
change of `tenant_step_log_bytes` all commit with no deadlock; a marker transaction held after step 2 (the test holds the
namespace's step-log policy fence exclusively, so the marker waits with its id assigned and the run read unfinished)
while the run is ended by a cancel is delivered by the tail before `end`. `script_marker_decision` has unit tests of each outcome.

### 8. Studio

In `frontend/apps/studio/src/features/runs/`:

- A fourth tab, **Logs**, in `RunDetailPage.tsx` beside Graph, Table and Timeline, and a **Logs** section in
  `NodePanel.tsx` for the selected step with an attempt selector fed by the streams route (paged, both sources); the
  section reads the chosen attempt log with the `stream` filter, so it scans only that stream.
- Paging: a list page that is empty while `complete` is false is not "no lines": while `nextCursor` moves, `LogView`
  reads again at once, showing "searching…"; once it stops moving, `LogView` has caught up and polls every 0.5 s, every
  2 s once idle for 30 s, as the tail does, until `complete` is true.
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
  flush: the last lines may be missing"; "some lines are delayed" while `heldBack`; markers as rows with text.
- A script marker row expands in place into `ScriptLogPanel.tsx`, which calls `getScriptExecution` (through
  `src/api/endpoints.ts`) and shows the execution's log as text, or "log withheld" when `logWithheld` is true.
- Keyboard: the tab follows the existing `Tabs` pattern; the list is one tab stop with arrow, Page, Home and End movement
  and Enter to expand a row; focus is not moved by arriving lines.
- Types come from `npm run api`; `npm run check:api` passes.

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
span, `worker.append_step_logs`, with counts and the verdict and no text; a carry adds counts and its outcome as
attributes of the report's existing span, and no text. The runbook `docs/runbooks/step-logs.md`
(created by T041-10, extended by T041-30 and completed by T041-25) covers the four resources and how to size
`tenant_step_log_bytes` against the retention and the log rate, the deployment switch and its use during a rolling upgrade (section 10), the purge, what each state and marker means, that a change of retention applies to new streams and a change of capture or a
ceiling to new batches, that capture turned back on reaches a worker at its next registration, the PII grant every
reader needs, the PostgreSQL settings of section 5 (with `log_error_verbosity = terse` and a bounded
`log_parameter_max_length` for the runtime role, so a database error cannot copy line text into PostgreSQL's own log),
the removal of step-log quota rows before a binary rollback, and
the incident path (CUJ 7: delete the lines, rotate the secret; a script execution's log is removed only by its own
retention, G041-013). `docs/architecture/overview.md` and the storage documentation gain the new tables.

### 10. Rollout and rollback

The migrations are additive. `SAGAT_STEP_LOGS_ENABLED` defaults to true from T041-7, which serves the RPC; the purge
lands earlier, in T041-10; the carry is stored from T041-36, which lands after T041-7. When it is false, the policy
answers `enabled = false`, the RPC answers `STOP_CAPTURE` without touching the database, a report ignores its carry
before any statement, and the control plane writes no script marker. Old and new binaries run together: an old API
replica answers `UNIMPLEMENTED`, which an SDK treats as a stop of batches for that attempt, and ignores a carry as an
unknown field, recording the report without it (G041-020). So a rollout keeps `SAGAT_STEP_LOGS_ENABLED=false` on the
new replicas until every API replica runs the release that serves carries, then turns it on; workers learn it at their
next registration. Between T041-7 and T041-36 the server offers 1.7 and ignores a carry; no release is cut in that
window, and no SDK sends a carry before T041-36 (the SDK tasks land after it). An old replica cannot parse a
step-log resource name (`crates/infra-postgres/src/quotas.rs:1302-1308`), so once a step-log `quota_limits` or
`quota_ceilings` row exists every quota listing and change of that tenant fails on it; the runbook says to finish the
upgrade before setting a step-log quota, and to delete step-log quota rows before rolling a binary back. Rolling back
the binary leaves the tables unread; rolling back the migration needs the lines purged first.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| A step's code logs a secret the platform cannot recognise (transformed, short, obtained elsewhere) | G041-001, accepted (O041-011). Value redaction with the process's secret set, two shape scrubbers, a permission of its own, the PII grant on every read, the namespace switch, the delete route and its runbook |
| Capture is on by default and sends text a customer wrote for their own machines | O041-001 chosen by the owner; the release notes say it first; the namespace switch and the worker option; the runbook |
| The six SDKs' redaction drifts apart | One rule (`redact`) and one canonical number form in `sdks/SPEC.md`; `step-logs-redaction` with the same goldens in six languages |
| A hand-written 1.7 client skips redaction | ADR-0085 rule 12 states it; the server's shapes and sensitive names still apply; the permission and the grant bound who reads |
| Log batches make completions wait | The tenant row only at commit; no lock a report, lease or heartbeat takes before L1; per-tenant tickets; the queue bound of section 2, shown by the barrier tests of T041-6; the time bound of SC-041-007, measured by T041-35 before the schema and T041-9 at the end |
| Each logging step adds transactions | A step that reports before its first flush adds none: its lines ride on its report (OD-2026-11-19-1); a longer step adds about one batch a second while it runs; per-tenant tickets and shedding; a step that logs nothing sends nothing |
| The carry adds work under the run lock to every report that carries records | At most five statements, an equality of the budget (section 2a); at most 64 rows, so *k* ≤ 65 trigger calls at insert and as many deferred calls at commit; the CPU work (validation, scrubbing, sizing) is done before the transaction; T041-35 measures a ten-record and a 64-record carry |
| A carrying report takes the tenant's usage row at its commit even when its output stores nothing, so a tenant's logging reports queue on it | Measured by T041-35 with an output that stores nothing (workload (f)), against the rate criterion; O041-026 if more than half a failing p99 excess is that wait |
| A carry makes its report fail or loop | Everything that can be checked is checked before the transaction and an invalid carry is ignored; a finalizer raise or a failed carry statement re-runs the report once, without the carry's lines (section 2a); tested by T041-36 |
| A report waits on a step-log lock while it holds the run row (an ingest of its own attempt holding the stream row from T4 to its commit, the purge or the delete route on an expired or deleted stream, a change of a step-log resource on the policy fences) | The carry sets a lock timeout of 200 ms for its own statements, so each wait is bounded and a longer one fails the carry into the report's re-run; the SDK reports only after its batch in flight is answered or 2 seconds passed; barrier tests in T041-36 |
| Lines of one attempt shown out of sequence order (a carry whose transaction id is below that of a batch stored after it) | The stream records `last_stored_xid`; such a carry keeps no record and counts them (`out_of_order`, section 2a); tested by T041-36 |
| A carry sent to an API replica that predates minor 1.7 is lost | The replica ignores the unknown field and records the report; the stream, if any, shows "ended without its final flush" (G041-020); the runbook says to keep `SAGAT_STEP_LOGS_ENABLED=false` on the new replicas until every API replica runs the release that serves carries, so that no worker is told capture is on during a mixed rollout |
| A runner, a database runner or a managed step puts lines on its report | C0 ignores the carry for those roles and for managed steps, as the ingest refuses them (section 2a); tested by T041-36 |
| The re-run of a report whose carry failed finds the lease expired | Refused as a late report would be; the only case where capture changes a report's answer, and only after a failed transaction (G041-021) |
| T041-35 measures on a database on the same machine and on the root report path only | Its result says so; T041-9 measures the built carry on both paths; G041-019 and G041-022 |
| A holder of `run:logs:read` without a namespace's PII grant watches `tenant_step_log_bytes` usage move | Open as G041-018; for the owner when the revision is approved |
| The SDK's wait before a report delays the completion of a step that streamed | Only a step that flushed waits, for its batch in flight, at most 2 seconds; a short step waits for nothing; measured in T041-9 |
| A revoked or re-registered worker | Session (namespace, expiry, token) and lease checked as reports do; binding and membership read without a lock, so a batch committed just after a revocation is stored (accepted, OD-2026-11-16-7); a re-registered worker resends with its new token; T041-8 tests each |
| A long or prepared transaction holds the read horizon | Lines arrive late, never out of order or missing; `heldBack`; the oldest-transaction gauge and alert; the runbook's settings |
| The tail ends before a late batch commits | The end rule of D1958 with a strict comparison and the scan reaching the run's last row; tested on PostgreSQL 16 and 18, each on a server of its own |
| A flood fills the table before the ceilings act | Ceilings from counters before the insert, in the same transaction; drop counts derived from sequences |
| Rows without log bytes escape the tenant quota | Every row is charged 128 bytes on top of its text (OD-2026-11-15-6, D041-054) |
| Step logs fill a quota other writers need | Step logs have their own quota, `tenant_step_log_bytes`; `tenant_log_bytes` keeps script, dispatch and connector logs (OD-2026-11-15-5, D041-053) |
| The script marker makes the script report fail or deadlock | Written after the report commits, in a transaction of its own that takes no lock the report takes; skipped at the step-log quota; a barrier test with an ingest, a delete and a policy change (section 7) |
| Each line upserts the transaction's `quota_transaction_deltas` row, which bloats while the horizon is held | T041-35 runs one workload with a held transaction and records the table's size and the commit time of other writers |
| Stream rows grow with every logging attempt until feature 020 deletes runs | One small row per attempt, of the order of the `steps` rows that are never purged either; the size per row is measured in T041-9 and stated in the runbook; G041-004 |
| An ordinary 1.7 SDK carries `ResolveDatabase` on the wire | The server refuses it without managed-executor ownership; `step-logs-negotiation` asserts it for a worker and an admin token |
| Readers without the PII grant lose the script log they read today, and every reader needs the grant for step logs | OD-2026-11-13-18 and OD-2026-11-16-8; the release notes; a viewer test and a test without the grant in T041-29 |
| The missing foreign key leaves log rows of a deleted run | Nothing deletes a run today; G041-004 is registered in feature 020 (G1618); T041-26 asserts that every stream's run exists after its scenarios |
| The Studio renders log text as markup, links or reordered text | Text only, no links, escape sequences removed, bidirectional and zero-width characters made visible; tested |
| A read with a filter that matches nothing scans a whole run | Each request scans at most 5,000 rows and its cursor moves past them; a step's view scans only its stream (section 5); the route's statement timeout |
| Ingest overload makes every step that streams wait at completion | The wait before a report is bounded at 2 seconds and no call waits for a ticket; a short step does not use the ingest; `SagatStepLogIngestShedding`; the runbook names the deployment and namespace switches as the levers; T041-35 runs a saturated-ingest workload and records worker throughput |
| The tenant step-log quota fills in hours at a high log rate and drops the newest lines until old streams expire | `ceiling = tenant_quota` shown on each stream; the runbook's sizing of `tenant_step_log_bytes` against the retention and the log rate; T041-35 and T041-9 set the quota explicitly |
| A worker's secret set fills (1,024 values) and capture stops for the process | One local line and the SDK's drop reason `secret_set_full`; the runbook says to restart the worker; each attempt still sends its carry with its final mark |
| A caller who can steer logged text learns, by seeing `[REDACTED]`, that a guessed value is a secret the process resolved | G041-017, accepted by the owner (OD-2026-11-16-7); the permission and the grant bound who reads; the runbook says so |
| Purge falls behind ingest | 500 lines per transaction, purge rate against ingest rate measured in T041-9; `SagatStepLogPurgeBehind` |
| The cursor shows the database's transaction counter (G041-008) | Accepted by the owner as stated (OD-2026-11-15-11); no row identity and no tenant's data are in it |
| A viewer watches the step-log quota to learn how much a tenant's runs log | `used` of `tenant_step_log_bytes` is shown only to a caller with `run:logs:read` (D041-066; G041-015 closed) |
| A long transaction of another role holds the read horizon unseen by the alert | `heldBack` on every page; the runbook's `pg_read_all_stats` grant for the gauge (G041-016) |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | A log is not execution state: no transition reads it; lease and heartbeat run no statement on it, and a report runs at most five, only to store its own carry, after the engine has decided and never read by it (FR-041-008, SC-041-001, section 2a); a carry that cannot be stored never changes the report (one re-run without it). The lease and fencing token, its expiry, the session, the binding and the membership decide whether a batch is accepted; a stale attempt is refused, never merged (section 2, T2, T3 and T7). The journal and its sequence are unchanged |
| II. Modular domain, ports and adapters | Pass | Domain types and the pure `plan_batch` and `script_marker_decision` in `crates/domain/src/step_logs.rs` with no wire or SQL import; ports `StepLogTx`, `StepLogCarryTx` (a supertrait of the report transactions, section 2a) and `StepLogReader`; scrubbing and sizing in the application; adapters in `infra-postgres`, `api-grpc`, `api-http`. `scripts/check-architecture.py` checks the layering, run by the author before each pull request and by CI at release (the plan's "Gates"), and T041-5 adds two architecture tests (no foreign key to `runs` or `steps`; the hot-path modules never name the log tables, and T041-36 restates the second so that they reach them only through `StepLogCarryTx`) |
| III. External effects are at-least-once; sagas compensate | Pass | Delivery of a batch or of a report's carry is at-least-once and made idempotent by (attempt, generation, sequence) and by the report's idempotency key; nothing here is an external effect |
| IV. Compatibility is versioned and pinned | Pass with conditions | Additive minor 1.7 (ADR-0015; held by 041 in the register of `sdks/SPEC.md` §9, OD-2026-11-15-8), changed in place before any release (D041-090: a field on `CompleteStep` and `FailStep`, two fields of `AppendStepLogsRequest` reserved); older workers and servers behave as today, and an older API replica ignores a carry; four new operations; the six SDKs pass the same five scenarios; the `ScriptExecution` schema change is additive. The condition: viewers get `getScriptExecution` without its `log` (OD-2026-11-13-18), a documented change of behaviour |
| V. Code, tenant boundary and secrets are isolated | Pass with conditions | Tenant in every key and predicate, 404 and audit for another tenant's run, a permission of its own, the PII reveal grant on every read (OD-2026-11-16-8), reads audited, an audited delete. Values the process resolved, written verbatim and at least 6 bytes long, are removed by an official SDK before a line leaves it. Conditions, each accepted by the owner: G041-001 (O041-011, ADR-0085); G041-008, the cursor shows the transaction counter (OD-2026-11-15-11); and G041-017, the `[REDACTED]` oracle over the process's secret set (OD-2026-11-16-7). The rows without text are charged 128 bytes each (OD-2026-11-15-6), so no row escapes the step-log quota; the step-log quota's usage is shown only to readers of step logs (D041-066), so G041-015 is closed |
| VI. Observability and audit are part of the feature | Pass | FR-041-018: metrics with closed labels, a span without text, typed errors, alerts on shedding, purge and old transactions, `run.logs_read` and `run.logs_deleted` |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | No broker, inbox or outbox is involved: a batch is a synchronous call whose answer the worker waits for, and it is safe to deliver twice. Backpressure is designed (bounded buffers, admission and tickets, `RETRY_LATER`, counted drops). A dropped line is counted, not parked, so the dead-letter rule has nothing to apply to (ADR-0084 decision 7) |
| VIII. Failure is tested and contracts are verified, honestly | Pass | The Verification Plan names the crash, duplicate, lost-batch, stale and expired lease, revocation, re-registration, flood, old-worker, lock-order, tail-end and restart tests, a compensation that logs while its run is stopping and after its forward step failed, and a lease that expires while its batch is in flight, a carry refused at commit by the step-log quota, a carry statement that fails, a lost answer to a carrying report and a carry behind an ingest of its own attempt, with barriers and held locks; each is written first and seen failing on its own assertion before the code it tests exists. What is not proven is listed as gaps G041-001 to G041-022 (G041-015 closed; G041-009, G041-010 and G041-011 moot) |
| Delivery Constraints | Pass | Rust, tonic, Axum, SQLx, PostgreSQL as the only store; no new component; SC-041-007 is a criterion on a stated machine |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| Condition on principle V: free text written by customer code is stored and may hold a transformed secret | The feature is the storage of that text; the server never holds the secret values | Re-resolving the attempt's secrets on the server for every batch: a disclosure per batch, each to audit |
| Condition on principle IV: viewers get the script execution without its log | The owner closed the script log to callers without the step-log right (OD-2026-11-13-18) | Leaving it readable under `script:read`, which contradicts ADR-0085 rule 6 |
| Condition on principle V: the `[REDACTED]` oracle over the process's secret set (G041-017) | The secret set lives for the process (OD-2026-11-15-7), so a line is redacted with every value the process resolved; accepted by the owner (OD-2026-11-16-7) | A set per attempt, rejected by the owner (O041-017) |
| Condition on principle V: the read cursor shows the database's transaction counter (G041-008) | The read order is the storing transactions' ids; the owner accepted the plain cursor (OD-2026-11-15-11) | A cursor encrypted with a server key, which no other cursor of the API needs (D041-035) |
| Tables with no foreign key to `runs` | A foreign key makes each insert take a lock that conflicts with the engine's run lock | Keeping the convention |
| A new RPC in protocol minor 1.7, carrying the session token, for a long step's batches | A long step must be readable while it runs, and its batches must not share the fate of the heartbeat; the session's namespace, minor and subject must be checkable | A field on `Heartbeat` (statements under the run lock once a second per long step); an RPC with no session |
| The last records on the report (at most 64), five statements in its transaction under a 200 ms lock timeout, a re-run when the carry fails it, and an order guard (ADR-0095) | T041-31 showed that a transaction per batch and a final batch lowered the rate by 13% to 21% (medians); the owner chose to remove the per-step transaction (OD-2026-11-19-1) | Every record on the report with nothing streamed (a long step shows nothing until it ends); a second transaction after the report in the same call (the extra commit the owner chose to remove); a savepoint around the carry (two statements more, and it cannot catch the commit's raise) |
| A read horizon on transaction ids, and an end horizon in the cursor | A tail over concurrent writers otherwise skips a late commit or ends before it | A per-run sequence taken under the run lock |
| A second server-sent event route | The graph stream has one resume point and a wider audience | Log events on `/runs/{runId}/stream` |
| A stream row per attempt beside the lines | Ceilings and drop counts from counters; the "expired", "deleted" and "no final flush" states | Deriving them from the lines on every batch and read |
| Two permissions and the PII reveal grant on every read | Logs can hold personal data, and an unannotated payload field is `pii` (feature 016); viewers hold `run:read`; deletion is an incident act | Reusing `run:read`; the grant only in a namespace whose schemas carry an annotation (OD-2026-11-13-18, changed by OD-2026-11-16-8) |
| A tenant quota of its own for step logs: a storage class, a usage column, a delta, a policy fence, a measurement kind, one `SECURITY DEFINER` accessor and a fifth argument of `sagat_quota_add_delta` (ADR-0027 Amendment) | The owner chose a separate quota and a charge per row (OD-2026-11-15-5, -6) | Sharing `tenant_log_bytes` (O041-005), or stopping at 90% of it |
| A script-output marker written after the script report commits, in a transaction of its own | The owner asked for the marker in the whole-run tab (OD-2026-11-15-9) | Listing script executions only (D041-030); writing it in `record_execution`'s transaction, which tied the report to step-log locks (OD-2026-11-16-7) |
| An admission class `Class::Logs` and per-tenant tickets | A flood of log calls must not take the connections that lease calls need, nor one tenant's share of them | Sharing the existing classes, which moves the discovery threshold |
| A CI rule that runs the step-log suites for step-log code (constitution 4.5.0) | The constitution's CI gate names the suites a pull request runs (OD-2026-11-15-10) | No rule, the suites running only at release |
| Capture code and new logger calls in six SDKs | Only the worker process holds the secret values and the logger | Capturing the worker's stdout from outside |

## Alternatives Considered

The options the ADRs weighed, with their reasons, are in ADR-0084 ("Considered options", 1 to 10) and ADR-0085 (1 to
11); the owner's rejected options are in "Owner decisions needed". The ones this design weighed on its own:

- **Every record in batches of their own, and a final batch before the report** (the approved transport of
  2026-11-16). Replaced (OD-2026-11-19-1): T041-31 measured it at 79% and 87% of the rate with capture off on a quiet
  machine, against 90%; the cost was the extra transactions. The carry of section 2a replaces the final batch, and a short
  step sends no batch.
- **A field on `Heartbeat`, or a client stream,** for a running attempt's records. Rejected: section 1, "Why the report
  carries the last records, and a unary RPC the rest".
- **A `DROPPED` record kind** that names the range an SDK lost. Rejected (D041-028): the server already sees every gap
  in the sequences, so the kind only added a record type, a fold step and a "lying range" case in six SDKs.
- **Tuning numbers in the policy** (buffer, flush, batch). Rejected (D041-033): each would be a permanent 1.7 field
  that six SDKs must honour, for a choice no deployment has asked to make per server.
- **A locking path near the tenant limit** (`sagat_quota_lock_log_usage`, keep what fits). Rejected (D041-029): it holds
  the row a completion needs; a lossy log can drop one counted batch at the limit.
- **A grouped commit on the server** (several requests' batches in one transaction). Rejected again (ADR-0095 option
  4): it lengthens each hold of the tenant row while cutting their number, and a short step would still pay a round trip
  and a wait for its group.
- **The carry written by a second transaction after the report commits, in the same call**, or **inside a savepoint**.
  Rejected (ADR-0095 options 5 and 6): the first is the extra commit the owner chose to remove; the second adds two
  statements to every carrying report and cannot catch the finalizer's raise at commit.
- **Lines on the report only when the attempt never flushed**, and a final batch otherwise. Rejected: two final paths
  in six SDKs and on the server; the carry is the one final mark for every attempt that logged.
- **A sealed cursor.** Rejected (D041-035): key management no other cursor of the API needs; the run id in a plain
  cursor is compared instead.
- **Script output through the streams list only, with no marker row.** Proposed by the design (D041-030) and not chosen
  by the owner (OD-2026-11-15-9): the whole-run tab would not show when a script step produced output.
- **Step logs inside `tenant_log_bytes`, stopping at 90% of it.** Proposed by the design and not chosen by the owner
  (OD-2026-11-15-5): a separate quota is exact and leaves the other log writers their whole allowance.
- **Recording the classified namespace in the binary, with a backfill gated by the instance registry.** Rejected
  (D041-037): the registry cannot see binaries older than itself and a requirement can be removed, so a version an old
  binary publishes could stay unrecorded.
- **A record of classified namespaces** (a table kept by a trigger, D041-037; then a partial index, D041-067), and the
  PII grant only where it says so. Replaced (OD-2026-11-16-8): an unannotated payload field is `pii`, so the grant is
  needed on every read and no record is kept.
- **A run ceiling**, exact (`run_log_usage` locked by every batch of a run) or approximate (an unlocked sum). Removed
  (OD-2026-11-16-10): the attempt ceilings and the tenant quota bound what a run stores, and the exact form made a wide
  fan-out wait on one row.
- **The ingest holding the binding lock, the namespace secrets lock and the membership row to commit**, as a report
  does. Replaced (OD-2026-11-16-7): it delayed managed-database admission and every lease, heartbeat and completion
  queued behind a `lock_secrets`; a lossy record reads them without a lock.
- **A batch (then the final batch) that waits for a ticket.** Replaced: it held a class slot while waiting, so one tenant could fill the
  Logs class; every batch is answered `RETRY_LATER` at once.
- **A per-tenant step-log usage row of its own**, apart from `tenant_retained_usage`. Not chosen now: with the carry it
  would move the step-log part of the queue on the tenant row to a row of its own, not remove it, since every report
  that keeps a carried record would take it, and a report that also stores output would then take two hot rows (a new
  slot after L4 in the lock order). It changes the ADR-0027 Amendment the owner accepted as written (OD-2026-11-15-16);
  it is O041-026, asked if T041-35 finds the completion p99 more than 10% above its value with capture off and more than
  half of that excess spent waiting on `tenant_retained_usage`.
- **Striped or per-transaction step-log accounting** (several usage rows per tenant, or one charge per transaction
  computed from the sizes C3 already knows). Not chosen: either changes the ADR-0027 Amendment's triggers, which keep
  one accounting rule for every class; it is the next lever if O041-026's row of its own is not enough.
- **A switch that turns the read routes off** (`SAGAT_STEP_LOGS_READ_ENABLED`, the 2026-11-15 text). Rejected (D041-069):
  no owner decision asks for it, and rate admission and a binary rollback already shed read load.
- **Expiry computed at purge time from the retention in force.** Rejected (D041-049): ADR-0027 forbids retroactive
  deletion by a lowered policy; the delete route is the early removal.
- **Re-authorizing the tail on every poll.** Rejected (D041-041): a membership lock twice a second per tail, where the
  graph stream re-checks every 30 seconds.
- **Capture through a handler on the language's shared logger.** Rejected (ADR-0085 option 5): it sees every attempt's
  lines and redacts them with the wrong secrets.
- **A script marker that carries the runner's iteration**, reported by the runner and verified against a `step.leased`
  event, with a resend without the fields for an older control plane. Rejected (D041-060): `script_executions` keeps one
  record per (run, node, attempt), so the iteration could name only the first report of an attempt, and the runner
  fields, the fallback and the verification added three mechanisms for that one fact; the marker is keyed on the
  execution.
- **A second transaction after the finalizer's raise**, re-planned with a tenant allowance of 0. Rejected (D041-061):
  it only attributes the drops to `tenant_quota`; `RETRY_LATER` and the resend's own plan do the same with one
  transaction fewer, and a batch that is never stored is still counted as a gap.
- **Sending logs to the OpenTelemetry Collector.** Rejected: no tenant boundary, ceiling or retention the platform can
  enforce. Export is a later feature.

## Non-Functional Requirements

- **NFR-041-PERF (performance)** SC-041-001 by test on every pull request that touches the hot-path files (the selector
  rule of design 038 picks `integration_claim_transition_efficiency`, where the lock tests live); SC-041-007 by T041-35
  before the schema and T041-9 at the end, with the machine, its load and the PostgreSQL configuration stated, in
  `specs/041-step-logs/validation-report.md`. The ingest's statement count and the carry's cost on a report (section 2a)
  are budgeted as equalities.
- **NFR-041-SEC (security)** No grant beyond those of data-model.md §4, no new `SECURITY DEFINER` function beyond the
  accessor `sagat_quota_step_log_allowance` of ADR-0027's Amendment, no role membership. The
  cross-tenant battery covers the RPC and the four routes. The canary (SC-041-004) runs in the conformance suite and in
  the server's integration suite. Line text never reaches a metric, span, audit record, error or the platform's log; T041-8
  searches the server's captured output for the canary.
- **NFR-041-OPS (operations)** Additive migrations; the purge lands before capture can be served; the deployment switch; old
  and new replicas and workers run together; the runbook and the alerts ship with the code.
- **NFR-041-A11Y (accessibility)** WCAG 2.2 AA as the Studio's axe run checks it, reflow at 320 px, keyboard-only journey,
  level by text and shape, no focus theft, announcements throttled (SC-041-006).
- **NFR-041-COST (cost)** The stored size of a line is its message, exception and fields plus a fixed overhead that T041-9
  measures and the runbook states; each row is charged its text plus 128 bytes (OD-2026-11-15-6).

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Six slices. Each task is one pull request; T041-2 and T041-33, which record owner answers, are each a pull request of
documentation only. Each slice leaves `develop` releasable.

- **Order.** Tasks are listed in the order they land. The one exception: the SDK tasks T041-15 to T041-20 land in any
  order after T041-14.
- **Owner gates.** A task that builds what an owner answer decided carries an `Owner-gate:` line citing its row of
  `docs/owner-decisions.md`; a task that waits for an owner question still open says "Waits for", right after its first
  line, and gains its `Owner-gate:` line when the answer is recorded. Task numbers are never reused; T041-28 to T041-30
  were added on 2026-11-13, T041-31 and T041-32 on 2026-11-15, T041-33 on 2026-11-16, T041-34 on 2026-11-18, and
  T041-35 and T041-36 on 2026-11-19 (the revision that OD-2026-11-19-1 decided), each in the slice it belongs to.
  Withdrawn: T041-28 (the tests of the classified-namespace record) on 2026-11-16, since no record is kept
  (OD-2026-11-16-8); T041-34 on 2026-11-19, since T041-6 carries its gate.
- **Tests first.** Each test is written in the pull request of the task that builds what it tests and is seen failing
  on its own assertion (not on a compile error or a missing fixture) before that code exists; the pull request names,
  for each new test, the assertion that failed first. A task that only adds tests for code already merged (T041-8,
  T041-24, T041-26), and a guard test that passes from the start (an architecture test, `work_budget.rs` unmodified),
  shows each test failing against a violation planted by hand in a scratch commit, and names it.
- **Gates.** CI runs the unit tests of the changed packages and the suites `scripts/select-test-targets.py` picks, in the
  `rust` job's step "integration suites that touch the change" (`.github/workflows/ci.yml:179-187`); it does not run the
  lints, the checkers or the runtime-role runs on a pull request (`docs/constitution.md`, CI gates). So before each pull
  request the author runs `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, "the
  checkers" (`scripts/check-architecture.py`, `scripts/check-agent-config.py`, `scripts/check-design-docs.py`) and,
  where a task says "also as the runtime role", its test suites with `SAGAT_TEST_AS_RUNTIME_ROLE=1`, and names each
  command and its result in the pull request.
- **Held transactions** in tests set their lock timeout when they begin (60 s while held) and are released within
  10 s; an assertion on stored rows or on a read polls with `eventually_within(what, bound, check)`, which T041-6 adds to
  `tests/support/mod.rs` beside `eventually` (whose bound is 5 s, `:383-399`), with a bound of 30 s.
- **Paths** are relative to the repository root; `openapi.yaml` is `specs/001-platform-foundation/contracts/openapi.yaml`.

Slice A, decisions, contracts and the spike:

- [x] T041-1 Review this design with `egm-review` (its five stages, in order) and the three repository reviewers
  (`architecture-reviewer`, `security-reviewer`, `correctness-reviewer` of `.claude/agents/`), answer every finding in
  the design and in `resolution.md` of the review folder, and repeat on the committed text until no Blocker and no Major
  of the last complete run is open, by the approval rule of `docs/designs/constitution-4.0.0-egm.md` section 3 (a
  finding is not open once it is fixed and re-read on the owner's approval path, or accepted by the owner in writing).
  Done when the four reports (the `egm-review` summary and the three reviewers') are cited by the approval block (Status
  `Approved`) and the checkers pass.
  Refs: FR-041-001, FR-041-013
  Evidence (2026-11-16): three complete runs of the five egm-review stages, in order, and of the three repository
    reviewers, on 882e56fc, fefcfafb and 6dceb548 (docs/designs/041-step-logs.reviews/2026-11-15/*-approval*.md); every
    finding answered in resolution.md sections 9 to 12. On the owner's approval path "Corrigir e aceitar o resto"
    (OD-2026-11-16-7) the third run's Blocker and defects were fixed and the three repository reviewers re-read the
    fixes on 46c42bbb, fac00787 and f9d4a65f (docs/designs/041-step-logs.reviews/2026-11-16/*-reread*.md), the new
    Majors of the first two re-reads fixed; the last re-read reports Blocker 0, Major 0 from each reviewer. The 63
    Blocker and Major findings of the egm-review summary egm-review-final.md are settled by OD-2026-11-16-200 to
    OD-2026-11-16-262, one row each; the owner accepted the design for 0.3.0 (OD-2026-11-15-17). The approval block
    cites the four reports. python3 scripts/check-design-docs.py: design docs ok; python3
    scripts/check-architecture.py: architecture check passed; python3 scripts/check-agent-config.py: agent
    configuration ok.
- [x] T041-2 Put O041-014 to O041-024, except O041-020 and O041-021 (which T041-31 owns), to the owner and record each
  answer as a row of `docs/owner-decisions.md` and in research.md; correct the ADRs' Revision sections and this design
  for the answers. Done when each of those questions has a row with Status `Decided`, and every task that builds one of
  the answers cites its row on an `Owner-gate:` line.
  Refs: FR-041-007, FR-041-008
  Owner-gate: OD-2026-11-15-17
  Evidence (2026-11-16): docs/owner-decisions.md rows OD-2026-11-15-4 to OD-2026-11-15-11 (O041-014 to O041-019,
    O041-022, O041-023) and OD-2026-11-15-16 (O041-024), each owner and Decided, recorded in research.md's owner
    questions; the ADR Revision sections corrected (ADR-0084 and ADR-0085 Revision of 2026-11-15, ADR-0027 Amendment);
    every task building one of those answers cites its row on an Owner-gate line (T041-3 to T041-7, T041-10 to T041-14,
    T041-29, T041-30, T041-32 and the SDK tasks); python3 scripts/check-design-docs.py: design docs ok.
- [ ] T041-33 Put the restated O041-025 to the owner and record the answer as a row of `docs/owner-decisions.md` and in
  research.md; for (a), add the reading as a Note of ADR-0084's Revision; for (b), change this design (T2 and the lock
  order) and have it reviewed again. Done when the row is `Decided` and T041-6 cites it on an `Owner-gate:` line.
  Refs: FR-041-005, FR-041-020
  Evidence (2026-11-18): the owner answered (a), "Sim, como os reports", received by the coordinator on 2026-11-16 and
    recorded as docs/owner-decisions.md row OD-2026-11-16-11 (owner, Decided); research.md's O041-025 row says it is
    answered; ADR-0084's Revision has the Note of 2026-11-18 with the reading. The approved body of T041-6 cannot gain a
    line, so its Owner-gate is carried by the follow-up task T041-34; python3 scripts/check-design-docs.py: design docs ok.
  Evidence continued (2026-11-19): unticked while the design is Draft for its revision of 2026-11-19, because R9 of
    scripts/check-design-docs.py lets a task ticked since develop stand only in an Approved doc; the evidence above is
    unchanged, and the approval pull request of this revision ticks it again. T041-6 now carries OD-2026-11-16-11 on its
    own Owner-gate line, so T041-34 is withdrawn.
- [ ] T041-31 Spike, before the schema is fixed. A log driver in `tests/performance/scalability-ha/main.rs` works in the
  benchmark's own disposable database; nothing it creates reaches a migration.
  - **Switches.** `SAGAT_PERF_STEP_LOG_SPIKE=1` selects the spike's workload (the engine completes steps with a 1 KiB
    output, where it completes with `{}` today, `main.rs:1437-1447`; the fan-out workflow below is published; the
    tenant's `tenant_step_log_bytes` is set to 1 TiB, so no batch takes the over-quota path);
    `SAGAT_PERF_STEP_LOG_DRIVER=on|off` turns the driver on, or off for the baseline of the same workload. Without the
    first, the benchmark runs as today.
  - **Scratch schema** `spike_041`: the DDL of data-model.md §2 for `step_log_streams` and `step_log_lines`. The line
    table gets the two triggers ADR-0027 generates, charging its text columns to the existing `log` class
    (`sagat_quota_row_delta('log', 'octet_length_utf8', 'message', …)` and the deferred `sagat_quota_flush_deltas`,
    created as `crates/infra-postgres/migrations/20261024000000_durable_quotas.up.sql:444-558` creates them), so that
    every commit takes a policy fence and the tenant usage row as the `step_log` class will.
  - **Prototype.** The SQL of data-model.md §8 (T1 to T10) against those tables, with the lease and session reads
    against the real tables; the policy fences are taken by their keys and the resources' defaults used, since the
    resources do not exist before T041-4. Admission is a semaphore of `SAGAT_STEP_LOGS_TENANT_CALLS` tickets per tenant
    per replica (half of `SAGAT_GRPC_MAX_LOG_CALLS`, rounded down, at least 1) that answers `RETRY_LATER` at once when
    none is free.
  - **Workloads** on the benchmark's replicas (R is the number of in-process API replicas, 1 and 8, as in its rows):
    (a) ten 200-byte `info` lines per step in one batch, then a final batch before the completion; (b) full batches of
    500 records from tickets × R concurrent attempts of one tenant; (c) a fan-out: a workflow with a `forEach` over 200
    items at concurrency 64 whose body is one logging step, published by the driver as the benchmark publishes its chain
    (`main.rs:1284-1293`); (d) workload (a) again while one connection holds `BEGIN; SELECT pg_current_xact_id();` open
    for a minute, holding no lock; (e) workload (a) with one ticket per tenant and a cap of 1, so that ingest sheds.
  - **Recorded.** SC-041-007's two figures against the baseline; the batches per second of workload (a) and the
    service time S of one batch (p50 and p99); each completion's wait on the tenant row, sampled every 10 ms from
    `pg_stat_activity.wait_event`; the share of batches and of final batches answered `RETRY_LATER`, ticket refusals and
    lock timeouts counted apart; what the final flush adds to a completion; under (d), the size of
    `quota_transaction_deltas` and the commit time of other writers; under (e), worker throughput and the completion p99.
    The figures other than SC-041-007's and the fan-out's are recorded with no threshold.
  - **Then.** The tenant tickets are set from the p99 of S and the measured rate of workload (a) (section 2,
    "Capacity"), with the cap, and recorded in data-model.md §6 by this task's pull request. Done when the per-line
    rows meet SC-041-007's two figures and fewer than 1% of the fan-out's batches are answered `RETRY_LATER` for a lock
    timeout. Then O041-021 and O041-026 are recorded in research.md as not needed, with the figures. If a figure fails,
    stop: the design changes and is reviewed again before T041-5 starts. A failing completion p99 whose excess is more
    than half wait on the tenant row is put to the owner as O041-026; a failing throughput figure is measured again with
    chunk rows, and if those pass it is put to the owner as O041-021. The figures, the machine, its load and the
    PostgreSQL configuration go to `specs/041-step-logs/validation-report.md`.
  Refs: NFR-041-PERF, SC-041-001, SC-041-007
  Files: modify `tests/performance/scalability-ha/main.rs`, `specs/041-step-logs/data-model.md` (§6, the defaults),
  `specs/041-step-logs/research.md`; create `specs/041-step-logs/validation-report.md`.
  Evidence (2026-11-19): not done. The spike ran (scenario step_log_ingest_spike of tests/performance/scalability-ha,
    driver step_log_spike.rs; figures, machine, load and configuration in specs/041-step-logs/validation-report.md):
    workload (a) fails SC-041-007 at both rows in three runs (rate 56% to 75% at 1 x 16 and 82% to 87% at 8 x 128;
    completion p99 up to +76%); chunk rows fail at 1 x 16 (67%); one calibration needed 17 tickets against half the pool
    (10). By this task's rule and the Open Questions the design changes and is reviewed again, and nothing after T041-31
    starts until then: asked as O041-036 (research.md, D041-087). O041-021 is not put (chunk rows fail too); O041-026 is
    not settled by this run. T041-3 and T041-4, built on the branch nonetheless, are held unticked (O041-037).
  Evidence continued (2026-11-19): the quiet rerun (validation-report.md, three rounds under a load of 3) failed again
    (rate medians 79.2% and 87.0%). The owner answered O041-036 with (d), "Redesenhar (CompleteStep)" (OD-2026-11-19-1):
    this task is not run again and stays unticked; its gate passes to T041-35, the spike of the carry.
- [ ] T041-35 Spike of the carry (the new spike of OD-2026-11-19-1, called T041-31′ in its question), before the schema
  is fixed. It replaces T041-31 as the gate: T041-3 and T041-4, already built, stay unticked until it passes, and no
  other task after it starts until it passes and this revision is approved. It extends the driver of T041-31
  (`tests/performance/scalability-ha/step_log_spike.rs`) in the benchmark's own disposable database; nothing it creates
  reaches a migration.
  - **Switches.** `SAGAT_PERF_STEP_LOG_SPIKE=1` with `SAGAT_PERF_STEP_LOG_CARRY=1` selects this spike; without the
    second, T041-31's spike runs as it did; without either, the benchmark runs as today.
  - **Scratch schema** `spike_041` as T041-31 builds it, plus `step_log_streams.last_stored_xid` (data-model.md §2).
  - **The report hook.** A report hook runs the carry's statements CA to CE of data-model.md §8a inside the real report
    transaction of a root attempt, just before its `COMMIT`. `PgRunStore::with_report_hook(Arc<dyn ReportHook>)`
    installs it; `PgRunTx` keeps the reported attempt (`effects.reported`, with the step id, attempt and generation of
    the locked state) when `persist` sees one, and `PgRunTx::commit` calls the hook with its connection and that attempt
    only then, so that leases, heartbeats and the managed path's intermediate commits never fire it; a hook error
    panics the benchmark, which also asserts that the hook fired once per measured completion. The hook and
    `with_report_hook` compile only under a `perf-spike` feature of `sagat-infra-postgres`, enabled only through an
    opt-in feature of `sagat-tests` of the same name that the spike's command passes
    (`cargo test -p sagat-tests --features perf-spike …`), so that `cargo test --workspace`, clippy and the release
    images never build it; `cargo tree -p sagat-execution-engine -e features` shows no `perf-spike`. The hook sets its
    backend's `application_name` for the transaction so that the commit-wait sampler can tell a carrying report apart.
    It plans the two cases the workloads meet with the domain's `plan_batch` (built on this branch by T041-4): a new
    stream (workloads (a), (f), (d), (h)) and a stream holding one batch (workload (g)); the resources' defaults stand
    in for their values and `tenant_retained_usage.log_bytes` against `tenant_log_bytes` for the step-log allowance, as
    in T041-31. The ingest of workload (g) is T041-31's SQL with no final batch and with `last_stored_xid` written. The
    hook, its feature and the driver's carry mode are removed by T041-36, which ports the mode to the built carry.
  - **Workloads**, letters continuing T041-31's ((b), (c) and (e) are not run). Two sizes, 1 × 16 (1 replica, 16
    workers) and 8 × 128 (8 replicas, 128 workers), 3,000 runs of three chained activities per measurement: (a) every
    step carries ten 200-byte `info` lines on its completion and completes with a 1 KiB output that differs per task,
    as T041-31's (a); (f) the same with the output `{}`, which stores nothing new, so that the carry's own take of the
    tenant usage row is measured; (g) every step sends one `AppendStepLogs` batch of ten lines, awaits its answer, then
    carries ten more, with a Logs class cap of 10 and 10 tenant tickets (half the pool of 20 of `API_POOL`), a model of a
    long step that flushes once before it ends; (h) at 1 × 16, every step carries 64 records of 200 bytes, the largest
    carry; (d) at 1 × 16, workload (a) while one connection holds `BEGIN; SELECT pg_current_xact_id();` for the whole
    measurement.
  - **One round**, in this order, each size in turn: capture off with the 1 KiB output (off-1K), (a), capture off with
    `{}` (off-{}), (f), (g), off-1K again, off-{} again; then (h) and (d) at 1 × 16. A round is one invocation of the
    benchmark on a fresh database, begun only when `uptime`'s 1-minute load average is under 3 (the runner waits at most
    10 minutes and otherwise reports the round as not run).
  - **The acceptance rule, fixed before the first round.** Gates: (1) the rate of (a) is at least 90% of off-1K, at
    each size; (2) the completion p99 of (a) is at most off-1K's + 10%, at each size; (3) the rate of (f) is at least 90%
    of off-{}, at each size; (4) the tickets (g) needs, ceil(offered batch rate × p99 of S / 0.7), are at most 10 at
    each size, with fewer than 1% of (g)'s batches refused for a ticket. The off figures are the first of the round; the
    offered batch rate of (g) is its transitions per second per replica (one batch a step); S is every ingest transaction that ran
    to its end, lock timeouts included (as `step_log_spike.rs` counts it); 0.7 is the target occupancy of section 2,
    "Capacity". A round whose two off figures of one size differ by more than 10% is void and run again. Gates (1) to (3)
    pass when the median of three valid rounds meets the criterion; gate (4) passes when every valid round meets it.
    T041-35 passes when all four pass with three valid rounds; with fewer, it is not decided and runs again, and never
    counts as passed.
  - **Recorded with no threshold:** the carry's statement time per completion (CA to CE, p50 and p99); (g)'s and (h)'s
    rate against off-1K and their completion p99; the commit waits sampled every 10 ms as T041-31 sampled them, and how
    many were behind a carrying report or an ingest; under (d), the size of `quota_transaction_deltas` and the completion
    p99; the second off figures; the machine, its load before and after each round and the PostgreSQL configuration.
    Not measured, and said so with the figures: the scoped report path (G041-019), a database on another host
    (G041-022), the CPU of C2 and C3 and the larger gRPC message (the benchmark calls the application in-process), and
    a purge running beside the workloads (T041-9 measures it).
  - **Then.** If it passes: the tenant tickets are set to max(1, ceil(offered rate × p99 of S / 0.7)) from the larger of
    (g)'s two sizes and recorded in data-model.md §6 as a share of `SAGAT_DB_MAX_CONNECTIONS`, the cap to the larger of
    the tickets and a quarter of the pool; O041-026 is recorded in research.md as not needed, with the figures; O041-038
    (ADR-0095) is put to the owner; the revision goes to its approval review. If a gate fails: stop; the design changes
    and is reviewed again before T041-5 starts. If gate (2) fails and the sampled commit waits account for more than half
    of the excess, O041-026 is put to the owner. The figures go to `specs/041-step-logs/validation-report.md`.
  Refs: NFR-041-PERF, SC-041-001, SC-041-007
  Owner-gate: OD-2026-11-19-1
  Files: modify `tests/performance/scalability-ha/{main.rs,step_log_spike.rs}`, `tests/Cargo.toml` (the opt-in feature),
  `crates/infra-postgres/{Cargo.toml,src/execution.rs,src/lib.rs}` (the report hook, behind the feature),
  `specs/041-step-logs/{validation-report.md,data-model.md,research.md}`.
- [ ] T041-3 Contracts: contracts.md §1, as revised on 2026-11-19 (D041-090), and §2.
  Waits for O041-038 (the owner's acceptance of ADR-0095, asked once T041-35 passes).
  Built on this branch before the revision (D041-091); what changes from that build: the proto's field 13 on both
  reports and `StepLogCarry`, fields 11 and 12 of `AppendStepLogsRequest` reserved (the baseline loses `final_batch` and
  `highest_sequence`, a removal that is expected because 1.7 is unreleased), `StepLogCarry` in the three `skip_debug`
  lists and their `Debug` impls, which no longer print the two removed fields, and
  `crates/api-grpc/tests/step_log_debug.rs`. Until T041-36 the server reads no field 13, so a carrying report is recorded
  as if it carried none. In full: copy contracts.md §1 (`StepLogCarry` on `CompleteStep` and `FailStep`, `final_batch`
  and `highest_sequence` of `AppendStepLogsRequest` reserved) into `worker.proto` and its six SDK copies and update
  `worker.baseline.json`
  (`SAGAT_UPDATE_BASELINE=1 cargo test -p sagat-api-grpc --test compatibility`) after reading the diff; answer
  `UNIMPLEMENTED` in the seven implementations of section 1; regenerate the Go stubs (`sdks/go/scripts/generate.sh`,
  protoc v7.36.2, protoc-gen-go v1.36.11, protoc-gen-go-grpc v1.6.1) and the Python stubs
  (`sdks/python/scripts/sync_shared.py`, protobuf 7.35.1, grpcio 1.84.0), the versions the committed stubs record; add
  the four messages and the `oneof` of section 1 to `skip_debug` in `crates/api-grpc/build.rs`, and in
  `sdks/rust/build.rs` and `sdks/rust/conformance/build.rs`, which first switch to `compile_fds_with_config` as
  `crates/api-grpc/build.rs:16-34` does, with `Debug` impls that print counts beside each generated module and a unit
  test beside each that `format!("{:?}")` of an `AppendStepLogsRequest`, and of a `CompleteStepRequest` and a
  `FailStepRequest` that hold a carry, holds no message text. Copy contracts.md §2 into `openapi.yaml`:
  the operations tagged `x-planned: '041'`, the `ScriptExecution` and `Quota` changes, the four resources in
  `Quota.resource` and `QuotaResourceName`. Add the four operations to the battery's `not_built` list
  (`tests/contract/security-multitenancy/rest/battery.rs:584-591`); add both `x-planned == "041"` and
  `x-implemented == "041"` to the digest test's approved additions
  (`tests/contract/payload-classification/openapi.rs:349-358`); regenerate `docs/reference/api.md`
  (`SAGAT_DOCS_BLESS=1 cargo test -p sagat-tests --test contract_developer_experience`) and the Studio types
  (`npm run api` in `frontend/apps/studio`, writing `src/api/schema.d.ts`), each after reading the diff. Done when
  `cargo test -p sagat-api-grpc --test compatibility`, `cargo test -p sagat-api-grpc`, `cargo test` in `sdks/rust` and in
  `sdks/rust/conformance`, `cargo test -p sagat-tests --test contract_payload_classification`, the battery of
  `contract_rest_security_multitenancy` and `npm run check:api` pass, and regenerating the Go and Python stubs a second
  time leaves `git diff --exit-code` clean.
  Refs: FR-041-004, FR-041-010, FR-041-011, FR-041-012, FR-041-014, FR-041-022, FR-041-023
  Owner-gate: OD-2026-11-15-4, OD-2026-11-15-8, OD-2026-11-15-9, OD-2026-11-15-16, OD-2026-11-19-1
  Files: modify `specs/001-platform-foundation/contracts/{worker.proto,worker.baseline.json,openapi.yaml}`, the six SDK
  proto copies, `crates/api-grpc/build.rs`, `crates/api-grpc/src/{lib.rs,service.rs}`, `sdks/rust/build.rs`,
  `sdks/rust/src/lib.rs`, `sdks/rust/src/worker_tests.rs`, `sdks/rust/conformance/build.rs`,
  `sdks/rust/conformance/src/faults/grpc.rs`, `tests/contract/durable-execution/grpc/limits.rs`,
  `tests/contract/security-multitenancy/grpc/stub.rs`, `tests/integration/database-runner/proxy.rs`,
  `apps/database-runner/src/testing.rs`, `sdks/go/internal/workerpb/*.pb.go`,
  `sdks/python/sagat_octopus_orchestrator/_proto/{worker_pb2.py,worker_pb2_grpc.py,worker_pb2.pyi}`,
  `tests/contract/payload-classification/openapi.rs`, `tests/contract/security-multitenancy/rest/battery.rs`,
  `docs/reference/api.md`, `frontend/apps/studio/src/api/schema.d.ts`, `crates/api-grpc/tests/step_log_debug.rs`.

Slice B, the server stores:

- [ ] T041-4 Domain and quota resources. `crates/domain/src/step_logs.rs`: the types, `validate_batch` (which takes no
  final mark, D041-090), `validate_carry` (section 2a, C2, 64 records and 16 KiB) and `plan_batch`, which plans a carry as
  it plans a batch, with the carry's `highest_sequence` as the final mark and `in_order` as an input, with property tests
  of the counting rule of section 2 (lines stored, markers excluded, + dropped by the worker + dropped by the server =
  `last_sequence`, over any sequence of batches, carries, `in_order` values and refusals). Built on this branch before
  the revision (D041-091): the change is the removal of `final_batch` and `highest_sequence` from `validate_batch` and
  its property test, `validate_carry`, and `in_order`. The four quota resources of
  data-model.md §6, named `step_log_attempt_records`, `step_log_attempt_bytes`, `step_log_retention_days` and
  `tenant_step_log_bytes` (their `as_str` and `limit_name`): `QuotaResource` variants, a `minimum()` per resource checked
  by `validate` and `validate_ceiling`, and the one named exception that lets `step_log_attempt_records` take 0 (the
  capture switch), with tests. A migration named `<timestamp>_step_log_quotas`, following `.claude/skills/migration`,
  that restates `quota_limits_resource_valid`, `quota_ceilings_resource_valid`, `quota_limits_tenant_scoped`,
  `quota_limits_value_range` and `quota_ceilings_value_range` (with the ranges of data-model.md §6, every CHECK added
  `NOT VALID` and validated by a second migration, `<timestamp>_step_log_quotas_validate`, D041-088, with a test of the
  1 GiB ceiling of `tenant_step_log_bytes`) and adds
  `tenant_retained_usage.step_log_bytes` (`bigint NOT NULL DEFAULT 0`); its down refuses while a `quota_limits` or
  `quota_ceilings` row names a new resource, as `crates/infra-postgres/migrations/20270108000000_pending_resets.down.sql:5-15`
  does, and otherwise restores the three constraints and drops the column; its up and down test is
  `tests/integration/security-multitenancy/step_log_quotas_migration.rs`, on the pattern of `undo_to`
  (`tests/integration/schedules-bulk-operations/migration.rs:123`); the upgrade-boundary oracle
  (`assert_eq!(after, <timestamp>)`) moves to it in the three files `scripts/check-upgrade-boundary.py` reads. The
  exhaustive matches gain the new resources: the usage of `crates/infra-postgres/src/quotas.rs:1211-1291`
  (`tenant_step_log_bytes` reads the new column; the three namespace resources join the arm with no scalar usage) and the
  cases of `tests/integration/security-multitenancy/quotas.rs:507-598`. The quota view of
  `crates/application/src/quotas.rs` leaves `used` of `tenant_step_log_bytes` empty for a principal without
  `run:logs:read` (D041-066). The permissions `run:logs:read` and `run:logs:delete` with the role matrix test;
  `AuditAction::RunLogsRead` and `RunLogsDeleted`, the first in `collapses_repeats` and `repeat_window`. Done when the
  domain's tests, the migration test, `python3 scripts/check-upgrade-boundary.py`, the quota cases of
  `integration_security_multitenancy` and the role matrix test pass, the suites also as the runtime role.
  Refs: FR-041-002, FR-041-003, FR-041-006, FR-041-013, FR-041-020, FR-041-023
  Owner-gate: OD-2026-11-13-14, OD-2026-11-15-4, OD-2026-11-15-5, OD-2026-11-15-16, OD-2026-11-16-9, OD-2026-11-16-10, OD-2026-11-19-1
  Files: create (built on the branch; modified by the revision) `crates/domain/src/step_logs.rs`, the migration (`.up.sql`, `.down.sql`) under
  `crates/infra-postgres/migrations/` and `tests/integration/security-multitenancy/step_log_quotas_migration.rs`; modify
  `crates/domain/src/{lib.rs,quota.rs,identity.rs,audit.rs}`, `crates/domain/tests/identity.rs`,
  `crates/application/src/quotas.rs`, `crates/infra-postgres/src/quotas.rs`,
  `tests/integration/security-multitenancy/{main.rs,quotas.rs}`,
  `tests/integration/workflow-primitives/{release_upgrade.rs,release_upgrade_inflight.rs,release_upgrade_callback_cutover.rs}`.
- [ ] T041-5 The migration of data-model.md §2 to §4, named `<timestamp>_step_logs`: the two tables (with
  `step_log_streams.last_stored_xid`, which the guard trigger also keeps from being lowered or cleared, section 2a; and
  a constraint name of their own for every raise of the finalizer's step-log branch, so that the store maps them to one
  typed refusal), their indexes (the
  run index and the stream index), the no-update trigger on lines and the guard trigger on streams; the rest of the
  ADR-0027 Amendment's accounting (data-model.md §3) but its locking accessor, with the manifest test taught the class
  and the kind; the quota triggers on `step_log_lines`; the storage-classification entries with the migration as owner
  path; grants, with the accessor revoked from `PUBLIC`; a down that refuses while a line exists and otherwise restores
  what it replaced. It follows `.claude/skills/migration`; the upgrade-boundary oracle moves to it in the three files
  of T041-4. Tests, in `tests/integration/claim-transition-efficiency/step_log_isolation.rs`: the migration's up and
  down, on the pattern of `undo_to`; the finalizer's new branch raises at the limit and charges 128 bytes for an empty
  line; a script log, a dispatch error and a connector error are stored in full under `tenant_log_bytes` while the
  tenant is at its step-log limit; every function the migration creates or replaces (`sagat_quota_step_log_allowance`,
  `sagat_quota_add_delta` with its new signature, `sagat_quota_row_delta`, `sagat_quota_flush_deltas`,
  `sagat_quota_measure`) keeps or has its `SECURITY DEFINER` and pinned `proconfig` as before, and none is executable by
  `PUBLIC` or, but the accessor, by `sagat_app`; the guard trigger refuses each change it guards, as the runtime role;
  and two architecture tests, each with its row in `docs/engineering/architecture-principles.md`: no foreign key leads
  from the log tables to `runs` or `steps`; and a source-text test that the production code (outside `#[cfg(test)]`) of
  `crates/application/src/worker.rs`, `crates/application/src/worker/`, `crates/application/src/execution*`,
  `crates/infra-postgres/src/execution.rs` and `crates/infra-postgres/src/execution/` never names `step_log_lines`,
  `step_log_streams` or `StepLogTx` (ADR-0084 decisions 1 and 2). Done when these tests, the manifest test of
  `tests/integration/security-multitenancy/durable_quotas.rs`, the privilege audit of
  `tests/integration/platform-foundation/runtime_privileges.rs` and `python3 scripts/check-upgrade-boundary.py` pass, the
  suites also as the runtime role.
  Refs: FR-041-008
  Owner-gate: OD-2026-11-13-14, OD-2026-11-13-18, OD-2026-11-15-4, OD-2026-11-15-5, OD-2026-11-15-6, OD-2026-11-15-9, OD-2026-11-15-16, OD-2026-11-16-10, OD-2026-11-19-1
  Files: create the migration (`.up.sql`, `.down.sql`), `tests/integration/claim-transition-efficiency/step_log_isolation.rs`;
  modify `crates/infra-postgres/src/identity.rs` (the typed refusal of `tenant_step_log_quota_exceeded` in `classify`,
  `:787-791`), `crates/infra-postgres/src/content_usage.rs` (`retained_usage`, `:146-173`), `crates/application/src/quotas.rs`
  (`RetainedUsage`), `crates/infra-postgres/storage-classification.json`,
  `tests/integration/security-multitenancy/durable_quotas.rs`, `tests/integration/platform-foundation/runtime_privileges.rs`,
  `tests/integration/claim-transition-efficiency/main.rs`, the three oracle files of T041-4,
  `docs/engineering/architecture-principles.md`.
- [ ] T041-6 The ingest of section 2.
  Builds: `sagat_application::step_logs` (`StepLogIngest`, the pre-checks P1, P2, P4 and P5, the control flow);
  `report_fence_allows` in `crates/domain/src/execution/state.rs`, called by `attempt_authority_fence`
  (`crates/application/src/worker.rs:2813-2838`) and by T2; `StepLogStore`, `StepLogTx` and the adapter (data-model.md
  §8), whose T3 reads the policies through `content_usage::admission_policy`, made `pub(crate)` with a batched variant
  for the three resources; T7's unlocked reads of the binding's patterns and of the membership;
  `the_shared_secrets_lock_has_one_caller_and_the_scoped_path_never_upgrades` (`crates/infra-postgres/src/quotas.rs:1323-1344`)
  extended to assert that `step_logs.rs` calls none of `share_secrets_lock(`, `lock_secrets(`, `hold_worker_binding(`,
  `bound_patterns(` and `sagat_lock_memberships` (data-model.md §8, T7); the two public scrubber
  entry points; `WorkerStore::supports_step_log_protocol` and its implementation
  (`crates/infra-postgres/src/workers.rs:199-206`); `store_error_without_text` on every statement that binds line text;
  the metrics the ingest emits (`step_log_batches_total`, `step_log_lines_stored_total`, `step_log_bytes_stored_total`,
  `step_log_lines_dropped_total`, `step_log_lines_truncated_total`, `step_log_redactions_total`,
  `step_log_ingest_seconds`, `step_log_lock_timeouts_total`, `step_log_quota_retries_total`); the service's wiring into
  `WorkerGrpc` (`apps/execution-engine/src/lib.rs:775`, `tests/support/api.rs:616`; the RPC still answers
  `UNIMPLEMENTED` until T041-7) and an `Options` field `step_logs_enabled` (`tests/support/api.rs:226`); the
  `integration_step_logs` target (`tests/Cargo.toml`); `eventually_within` in `tests/support/mod.rs`; the lock-order slots
  L1 and L2 in `tests/support/lock_order.rs`; and `tests/support/step_logs.rs`: a session fixture (`worker_sessions` rows
  inserted at protocol minor 7) and a `StepLogTx` wrapper with six hooks: fail before the commit; commit and then fail as
  a lost answer would; hold after T2 (no lock held); hold after T3 (the policy fences held); hold after T7 (the stream
  row held); hold after the quota flush (`SET CONSTRAINTS ALL IMMEDIATE`; the usage row held).
  Tests, in `tests/integration/claim-transition-efficiency/step_log_isolation.rs`, calling `StepLogIngest` directly: the
  lock-free test of SC-041-001 (a batch commits while the run row is held) and its barrier tests, as SC-041-001 states
  them, every one of the K ingests held after its flush; `work_budget.rs` unmodified; the lock-order test (section 2).
  Tests, in `integration_step_logs`, calling `StepLogIngest` directly and asserting stored rows by SQL (the read routes
  come in T041-11): the session cases (expired, other namespace, other subject, below 1.7, replaced token); the
  authority cases (binding removed, membership revoked, a membership whose role is runner or database-runner), each
  answered `AUTHORITY` even where another answer was due, with the denial audited; a runner token refused by P1 with the
  application's `Forbidden`; a lease sent with another run's id; a forward step of a stopping run refused `LEASE` and a
  compensation step of it stored (OD-2026-11-16-11); duplicates; gaps; a late batch after a newer one; the tenant limit by
  the unlocked read (`FINAL_ONLY`, counted, the planned marker's 128 bytes included) and by the finalizer's raise
  (`RETRY_LATER`, nothing counted, then the resend's `FINAL_ONLY`), with a gap in the batch and a later batch after it,
  asserting the counts; capture off in the namespace (no row written); a change of `step_log_attempt_records` to 0 that
  waits for a batch held after T3 and commits after it; a scrubber case for each shape of both scrubbers in the message,
  the exception, a key and a string value, and for each sensitive field name with a value of each type;
  `stored_through_sequence` 0 with `AUTHORITY` and `SESSION`; T9 raising `last_stored_xid` only when it inserted a row,
  and the guard trigger refusing a lower value; the statement budget of section 2 as an equality, counted
  with `pg_stat_statements` as `work_budget.rs` counts, in `tests/integration/step-logs/budget.rs`. Done when these pass,
  also as the runtime role.
  Refs: FR-041-002, FR-041-004, FR-041-006, FR-041-007, FR-041-008, SC-041-001
  Owner-gate: OD-2026-11-13-14, OD-2026-11-15-4, OD-2026-11-15-5, OD-2026-11-15-6, OD-2026-11-15-16, OD-2026-11-16-7, OD-2026-11-16-10, OD-2026-11-16-11, OD-2026-11-19-1
  Files: create `crates/application/src/step_logs.rs`, `crates/infra-postgres/src/step_logs.rs`,
  `tests/integration/step-logs/main.rs` and its modules, `tests/support/step_logs.rs`; modify
  `crates/domain/src/execution/state.rs`, `crates/application/src/{lib.rs,worker.rs}`,
  `crates/infra-postgres/src/{lib.rs,workers.rs,quotas.rs,content_usage.rs}`, `crates/api-grpc/src/service.rs`,
  `apps/execution-engine/src/lib.rs`, `crates/telemetry/src/{redact.rs,metrics.rs}`, `crates/domain/src/scripts.rs`,
  `crates/infra-postgres/storage-classification.json` (owner path), `tests/Cargo.toml`,
  `tests/support/{mod.rs,api.rs,lock_order.rs}`, `tests/integration/claim-transition-efficiency/step_log_isolation.rs`.
- [ ] T041-32 The CI gate of constitution 4.5.0 (OD-2026-11-15-10). Three entries of `HARNESSES` in
  `scripts/select-test-targets.py` (a `HARNESSES` match selects its suites whether or not the path is also a shared
  input):
  - `integration_step_logs` and `integration_claim_transition_efficiency` for the server's step-log code: the regex
    `crates/[a-z-]+/src/(.*/)?step_logs(\.rs$|/)|crates/infra-postgres/migrations/[0-9]+_step_log[a-z_]*\.(up|down)\.sql$|crates/api-grpc/src/(limits|service)\.rs$|crates/api-http/src/routes/(execution|scripts)\.rs$|crates/application/src/scripts\.rs$|crates/infra-postgres/src/maintenance\.rs$|crates/scheduler/src/passes\.rs$`;
  - `integration_step_logs` for the report path that stores a carry (section 2a): the regex
    `crates/application/src/worker(\.rs$|/)|crates/infra-postgres/src/execution(\.rs$|/)`, which today selects only
    `integration_claim_transition_efficiency` (`scripts/select-test-targets.py:71`);
  - `integration_step_logs` and `sdk_conformance` for the worker protocol and each SDK's capture code: the regex
    `specs/001-platform-foundation/contracts/worker\.proto$`, and these files: `sdks/typescript/src/worker/{step-logs.ts,activity.ts,host.ts,grpc.ts}`,
    `sdks/go/worker/{steplogs.go,activity.go,task.go,host.go,rpc.go}`, `sdks/go/internal/safelog/safelog.go`,
    `sdks/python/sagat_octopus_orchestrator/worker/{_step_logs.py,host.py,activity.py,_grpc.py}`,
    `sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/Workers/{StepLogCapture.cs,WorkerHost.cs,WorkerRpc.cs,ActivityExecutionContext.cs}`,
    `sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/Internal/Safe.cs`,
    `sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/worker/{StepLogCapture.java,ActivityLogger.java,WorkerHost.java,WorkerRpc.java}`,
    `sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/Hooks.java`,
    `sdks/rust/src/{step_logs.rs,activity.rs,worker.rs,worker_rpc.rs,capabilities.rs}`.
  Each path is a case of `scripts/tests/test_select_test_targets.py`; `docs/test-strategy.md` and `tests/AGENTS.md` gain
  the rule's paragraph and `scripts/README.md` its line. Done when `python3 -m unittest discover -s scripts/tests` passes.
  Refs: NFR-041-PERF, SC-041-001
  Owner-gate: OD-2026-11-15-10, OD-2026-11-19-1
  Files: modify `scripts/select-test-targets.py`, `scripts/tests/test_select_test_targets.py`, `docs/test-strategy.md`,
  `tests/AGENTS.md`, `scripts/README.md`.
- [ ] T041-10 Retention and deletion storage: the two steps of section 4 (mark the streams whose `expires_at` has passed,
  500 streams a transaction; then delete 500 lines a transaction, `purged_at` kept), through two new methods of
  `MaintenanceStore`, `mark_expired_step_log_streams(limit)` and `purge_step_log_lines(limit)`; the purge metrics
  (`step_log_purged_lines_total`, and `step_log_purge_backlog`, counted by the pass as section 4 defines it). Tests, at
  the store, asserting rows by SQL: a purge releases the quota bytes; a lowered retention leaves existing streams' expiry
  as it was and applies to new streams; an idle stream whose lease lives is marked and its next batch is `REMOVED`; a
  stream row held by an ingest (the hold after T7) is skipped by the mark and marked by the next pass; a stream being
  deleted over several transactions keeps `removed_at` from the first of them; the purge joins the lock-order test. The
  runbook `docs/runbooks/step-logs.md` is created with the purge, the retention rule and the deployment switch that
  T041-7 builds. Done when these pass, also as the runtime role.
  Refs: FR-041-009, FR-041-018, FR-041-020
  Owner-gate: OD-2026-11-13-14, OD-2026-11-15-4, OD-2026-11-16-10
  Files: modify `crates/scheduler/src/passes.rs`, `crates/application/src/maintenance.rs`,
  `crates/infra-postgres/src/{maintenance.rs,step_logs.rs}`, `crates/telemetry/src/metrics.rs`,
  `tests/integration/claim-transition-efficiency/step_log_isolation.rs`; create `docs/runbooks/step-logs.md` and
  `tests/integration/step-logs/retention.rs`.
- [ ] T041-7 The gRPC handler, in `crates/api-grpc/src/step_logs.rs`, called from `WorkerGrpc`, taking the tenant ticket
  (P3) before it calls the service and auditing an `AUTHORITY` answer with `spawn_audit_access_denied`; `Class::Logs` and
  the per-tenant tickets with their configuration keys and the defaults T041-35 set, no call waiting for a ticket (the
  server still reads no field 13 of a report: T041-36 does; no release is cut between the two); tests
  with admission counters that a full Logs class leaves discovery admission unchanged and that a second tenant's batch
  is admitted while the first tenant's tickets are all taken; protocol 1.7 in `negotiate_protocol_details` and
  `protocol_ceiling` (1.6 kept for the database runner) with the ceiling tests; the policy in `RegisterWorkerResponse`;
  `SAGAT_STEP_LOGS_ENABLED` (default true) in the execution engine; the span; the Rust SDK's refusal of an accepted minor
  above 6 (`sdks/rust/src/worker.rs:468`) raised to 7, so that the Rust runner accepts the server's new answer;
  `sdks/SPEC.md` §7.1 and §9's text for 1.7 (O041-018); the golden
  `tests/contract/sdk-conformance/expected/protocol-negotiation.json` and the three tests that assert the server's
  minor (`tests/e2e/sdk-multilanguage/us04.rs:29`, `tests/e2e/developer-experience/doctor.rs:323` with
  `crates/api-http/src/routes/identity.rs:68`, and `tests/contract/durable-execution/grpc/protocol_compatibility.rs:131, 138`)
  updated after reading each diff. Done when the ceiling, admission and configuration tests,
  `SAGAT_SDK_SCENARIO=protocol-negotiation cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored`,
  `contract_grpc_durable_execution`, and the two changed cases of `e2e_sdk_multilanguage` and
  `e2e_developer_experience` pass.
  Refs: FR-041-001, FR-041-004, FR-041-005, FR-041-014, FR-041-018
  Owner-gate: OD-2026-11-13-14, OD-2026-11-15-4, OD-2026-11-15-8, OD-2026-11-16-7
  Files: create `crates/api-grpc/src/step_logs.rs`; modify `crates/api-grpc/src/{lib.rs,service.rs,limits.rs}`,
  `crates/application/src/worker.rs`, `apps/execution-engine/src/{lib.rs,cli.rs,settings.rs}`,
  `apps/execution-engine/tests/config.rs`, `crates/telemetry/src/metrics.rs`, `crates/api-http/src/routes/identity.rs`,
  `sdks/rust/src/worker.rs`, `sdks/SPEC.md`, `tests/contract/sdk-conformance/expected/protocol-negotiation.json`,
  `tests/e2e/sdk-multilanguage/us04.rs`, `tests/e2e/developer-experience/doctor.rs`,
  `tests/contract/durable-execution/grpc/protocol_compatibility.rs`.
- [ ] T041-8 Server fault and security tests with barriers and held locks. Crash points use the `StepLogTx` wrapper of
  T041-6. A lost answer through the RPC uses the gRPC proxy of `tests/integration/database-runner/proxy.rs`, which this
  task moves to `tests/support/worker_proxy.rs`, declared by the two targets that use it through `#[path]` (not by
  `tests/support/mod.rs`), and teaches a `lose_append_ack` fault that forwards `AppendStepLogs` and drops its answer,
  and a `lose_report_ack` fault that forwards a `CompleteStep` or `FailStep` and drops its answer (used by T041-36).
  Cases: crash before the commit; crash after the commit and before the answer; the same batch twice at once
  (`RETRY_LATER`, then a resend that stores nothing); a stale lease; an expired lease not yet reclaimed; a batch after
  its attempt's report was recorded (`LEASE`); a batch for another tenant's run (audited, the audit kept after the rollback); two attempts of one run
  at their attempt ceilings and two runs of one tenant at its `tenant_step_log_bytes` limit, at once; a value that
  redaction lengthens at the tenant limit; a client that ignores every verdict (stored bytes stay at the ceilings);
  two tenants that flood and a third whose batches are still admitted (admission counters); a restart of the API in the
  middle of an attempt (the attempt's next batch is stored, no duplicate); a task delivered again under a new generation
  writes a new stream; rebuilding the projections of a run changes no log row; a database error that quotes the bound
  text, forced by a `BEFORE INSERT` trigger the test installs on `step_log_lines` that raises with `NEW.message` in its
  message, after which the canary is in neither the answer nor the captured `tracing` output; and the server canary (a
  secret value the test's attempt resolved and logged) searched in the captured `tracing` output, the stored rows,
  `run_events` and `audit_records`. Done when these pass, also as the runtime role.
  Refs: NFR-041-SEC, SC-041-002, SC-041-003
  Owner-gate: OD-2026-11-19-1
  Files: create modules under `tests/integration/step-logs/` and `tests/support/worker_proxy.rs` (moved from
  `tests/integration/database-runner/proxy.rs`); modify `tests/integration/database-runner/main.rs`,
  `tests/integration/step-logs/main.rs`.
- [ ] T041-36 The carry on a report (section 2a).
  Waits for O041-038 (the owner's acceptance of ADR-0095).
  Builds:
  - the domain type `sagat_domain::step_logs::Carry` (records, `highest_sequence`, encoded size) and `CarryPlan`
    (`Full`, `MarkOnly`, `None`);
  - the carrying entry points `WorkerService::complete_carrying`, `complete_with_blob_carrying` and `fail_carrying`,
    which take a `Carry`; `complete`, `complete_with_blob` and `fail` keep their signatures and delegate with none, so
    their callers do not change; the gRPC handlers of `CompleteStep` and `FailStep` call the carrying ones when field 13
    is present;
  - C0 to C3; `WorkerSettings.step_logs_enabled`, set by the execution engine from `SAGAT_STEP_LOGS_ENABLED` (as T041-7
    reads it) and by `tests/support/api.rs`;
  - `CarryPlan` as an explicit argument of `finish`, `finish_path_report`, `refuse_scoped_output`,
    `fail_success_for_storage_quota`, `fail_report_for_storage_quota` and `refuse_scoped_content`, with what each passes
    by the table of section 2a;
  - the port `StepLogCarryTx` (CA to CE, data-model.md §8a) as a supertrait of the run and path transactions,
    implemented for `PgRunTx` and `PgPathExecutionTx` in `crates/infra-postgres/src/step_logs.rs` through a `pub(crate)`
    connection accessor in `execution.rs`; `StoreError::StepLogCarry` in `crates/application/src/error.rs` (the SQLSTATE
    only, `AppError::Unavailable` if it escapes), produced from `store_error_without_text` for every carry statement;
  - the re-run of section 2a around `complete_inner`, the root body of `fail` and `report_path` (reusing its `loop`),
    downgrading `Full` to `MarkOnly` after the typed step-log refusal and to `None` after `StoreError::StepLogCarry`, and
    never re-running a re-run; the typed refusal for every raise of the finalizer's step-log branch;
  - CE's `last_stored_xid` and the order guard (T9's is T041-6's); the metric `step_log_carries_total{outcome}`, and the
    carry's lines counted in the ingest's line, byte, truncation, redaction and drop metrics (`out_of_order` included);
  - the removal of T041-35's report hook, its features and the driver's carry mode, which is ported to the carrying entry
    points for T041-9.
  Tests, in `tests/integration/claim-transition-efficiency/`: the six equalities of section 2a in `work_budget.rs`, each
  counted as the existing three are and beside them, through a carrying `World::complete_carrying` in
  `tests/support/engine.rs`, the existing three passing with their constants unmodified; the source-text test of T041-5
  restated so that the worker and execution modules reach the log tables only through `StepLogCarryTx`; the lock-order
  test with a carrying report beside an ingest of the same attempt held after T7, the purge, and a change of a step-log
  resource queued for its fence, at barriers, all committing; a carrying report blocked on its stream row by an ingest
  held after T7 that fails its carry at 200 ms and is recorded by its re-run; a change of `step_log_attempt_records` to
  0 holding its fence exclusively while a carrying report waits at CA, after whose commit the carry takes the `off` path
  (a wait under 200 ms) or fails into the re-run (longer). Tests, in `integration_step_logs` (which T041-32 selects for
  these paths), each asserting the stored rows, the stream's `final_at` and the outcome counter: a carry stored with a
  root completion, a root failure, a scoped completion and a scoped failure; a resent report stores nothing more, also
  through `lose_report_ack` of T041-8, and a resend with another carry is answered as a replay; a batch held after T2
  while its attempt's carrying report commits gets `FINAL_RECORDED`; two runs of one tenant whose carries cross its
  `tenant_step_log_bytes` limit, held at the commit by a `BEFORE INSERT` trigger the test installs on `step_log_lines`
  that waits on an advisory lock the test holds: one report re-run, its outcome and output as without a carry, the
  records counted `tenant_quota`, the final mark recorded, for a completion and for a failure; a failing carry statement
  (a trigger that raises) re-runs the report once with the same outcome and output, counts `failed` and leaves no final
  mark; a carry on a report whose output exceeds `tenant_content_bytes` (the content fallback, a mark-only carry) and on a
  report whose output holds a declared secret (`reject_secret_output`, a mark-only carry; no line stored); C0 for a
  runner token on an ordinary step and a database-runner on a managed SQL and a lake step (nothing stored, the report
  unchanged); an invalid carry with a valid `highest_sequence` (mark-only) and without one (ignored), capture off in the
  namespace, the deployment switch off, the store proof missing, and a removed stream, each leaving the report's outcome
  and output as without a carry at the cost of section 2a; the order guard (an ingest with a higher transaction id than
  the report's commits first: its lines kept, the carry's counted `out_of_order`, the final mark recorded) and a batch
  with a lower id that stores only duplicates after a higher one, which leaves `last_stored_xid` unchanged; a carry on a
  compensation of a stopping run stored (OD-2026-11-16-11); the counting rule over batches and a carry; the server
  canary of T041-8 carried on a report, searched as T041-8 searches it. Done when these pass, also as the runtime role.
  Refs: FR-041-004, FR-041-005, FR-041-008, SC-041-001, SC-041-002, SC-041-003, SC-041-004
  Owner-gate: OD-2026-11-15-5, OD-2026-11-15-6, OD-2026-11-16-9, OD-2026-11-16-10, OD-2026-11-16-11, OD-2026-11-19-1
  Files: create `crates/application/src/step_logs/carry.rs`, `tests/integration/step-logs/carry.rs`; modify
  `crates/domain/src/step_logs.rs`, `crates/application/src/{worker.rs,worker/output.rs,worker/scoped_reports.rs}`,
  `crates/application/src/{worker/scoped_output_refusal.rs,execution.rs,step_logs.rs,error.rs}`,
  `crates/infra-postgres/src/{step_logs.rs,execution.rs,execution/path.rs,identity.rs,lib.rs}`,
  `crates/api-grpc/src/service.rs`, `crates/telemetry/src/metrics.rs`, `apps/execution-engine/src/lib.rs`,
  `tests/support/{api.rs,engine.rs}`,
  `tests/integration/claim-transition-efficiency/{work_budget.rs,step_log_isolation.rs}`,
  `tests/integration/step-logs/main.rs`, `tests/performance/scalability-ha/{main.rs,step_log_spike.rs}`,
  `crates/infra-postgres/Cargo.toml` and `tests/Cargo.toml` (the hook's features removed),
  `specs/041-step-logs/data-model.md` (§7, the metric owners).

Slice C, the server serves:

- [ ] T041-11 `StepLogReader`, `listRunLogs` and `listRunLogStreams` for `source = worker` (until T041-13,
  `source = script` answers an empty page), in `crates/api-http/src/routes/step_logs.rs`, switched to
  `x-implemented: '041'`, removed from the battery's `not_built` list and added to its `body_of`.
  `StepLogReader::begin()` opens a `StepLogReadTx`, the transaction `step_logs::may_read(principal, &mut StepLogReadTx,
  run)` takes. It builds: the horizon, `heldBack`, the stream states of section 5, the cursor, the scan bound of 5,000
  rows with `nextCursor` at the scanned position, the stream index for a read with `stream`, the end rule of D1958 for
  `complete`, the filters, the `[RunRead, RunLogsRead]` admission, `may_read` (the PII grant in every namespace), the
  throttled audit, the gauge `database_oldest_transaction_age_seconds` (section 9). Tests: each of those; a reader
  without the grant refused in a namespace whose schemas carry no annotation, with the same `403` for an existing and a
  missing `stream`; a stream whose `expires_at` has passed returns no line before it is marked, and a stream being
  purged is shown expired; the completion predicate as a pure function, one unit test per case; a race of two first reads that
  writes one record; a failure to write the audit (a test-installed trigger on `audit_records` that raises) that
  returns no line; a filtered read that matches nothing and still moves on; the canary of T041-8 absent from every body;
  the cross-tenant battery; and, named `horizon_*` and run on PostgreSQL 16 and 18, the held-transaction tests of the
  horizon, a finished run of 12,000 lines over two attempts (the attempt ceiling is 10,000) read with a filter that
  matches only lines 11,000 to 11,010, which returns them before `complete`, and a finished run of 300 lines read
  unfiltered at `limit=50`, which returns all 300 before `complete`.
  Those run on a server of their own: `start_server` of `tests/support/mod.rs:136-176` takes the image tag as a
  parameter, and the support gains `isolated_server(tag)`, which starts a container no other test of the process shares
  and returns a guard holding it, and `fresh_db_on(&guard)`; the tag is `16-alpine` or `SAGAT_TEST_POSTGRES_TAG`; under
  `SAGAT_TEST_DATABASE_URL` these tests fail with a message, since a shared cluster cannot isolate the horizon. The
  `rust` job of `.github/workflows/ci.yml` gains a step, under the condition
  `contains(format(' {0} ', needs.changes.outputs.targets), ' integration_step_logs ')` (the form of `ci.yml:95`), that
  runs `cargo test -p sagat-tests --test integration_step_logs --locked horizon_` with `SAGAT_TEST_POSTGRES_TAG=18-alpine`.
  `docs/reference/api.md` is regenerated after reading the diff. Done when every test of this task passes, also as the
  runtime role, and the `horizon_` tests pass on 16 and 18.
  Refs: FR-041-002, FR-041-010, FR-041-011, FR-041-013
  Owner-gate: OD-2026-11-13-14, OD-2026-11-13-18, OD-2026-11-15-4, OD-2026-11-15-11, OD-2026-11-16-8
  Files: create `crates/api-http/src/routes/step_logs.rs`; modify `crates/api-http/src/routes/mod.rs`,
  `crates/application/src/{api_admission.rs,step_logs.rs,operations.rs}`,
  `crates/infra-postgres/src/{step_logs.rs,operations.rs}`, `crates/scheduler/src/monitor.rs`,
  `crates/telemetry/src/metrics.rs`, `openapi.yaml`, `tests/contract/security-multitenancy/rest/battery.rs`,
  `tests/support/mod.rs`, `.github/workflows/ci.yml`, `docs/reference/api.md`; tests in `tests/integration/step-logs/`.
- [ ] T041-12 `streamRunLogs`, switched to `x-implemented: '041'` and removed from `not_built`: the `follow` loop of
  `crates/api-http/src/routes/execution.rs:1340` shared with the graph stream, events, resume, `progress`, reconnect,
  caps shared with the graph stream, the per-poll grant check, the audit at each window, the re-check every 30 s, the
  end rule built in T041-11, and the metrics `step_log_tails_open` and `step_log_tails_refused_total`. Tests: a reconnect
  neither skips nor repeats; a revoked PII grant stops the lines at the next poll; the graph stream emits only `graph`,
  `reconnect` and `end` (ADR-0084 decision 10); and, named `horizon_*` and run on `isolated_server` on 16 and 18, a
  filtered tail of the 12,000-line run of T041-11 that delivers lines 11,000 to 11,010 before `end`, an unfiltered tail
  of a finished run of 450 lines that delivers all 450 before `end`, an ingest held after T2
  across the run's terminal transition is delivered before `end`, a first read of a new audit window that sees the run
  finished does not end early, and a forged `end` for a running run is ignored. `docs/reference/api.md` regenerated.
  Done when these pass, also as the runtime role, the `horizon_` tests on 16 and 18.
  Refs: FR-041-012
  Owner-gate: OD-2026-11-13-14, OD-2026-11-13-18, OD-2026-11-15-4, OD-2026-11-16-8
  Files: modify `crates/api-http/src/{routes/step_logs.rs,routes/execution.rs,streams.rs}`, `crates/telemetry/src/metrics.rs`,
  `openapi.yaml`, `tests/contract/security-multitenancy/rest/battery.rs`, `docs/reference/api.md`; tests in
  `tests/integration/step-logs/`.
- [ ] T041-13 Script steps (section 7): the `source = script` items of `listRunLogStreams`, with their state from the
  execution, through `may_read`, with a test that a viewer is refused `403 FORBIDDEN` and a reader without the grant
  `403 FORBIDDEN` with `PII_REVEAL_NOT_GRANTED`; the marker
  written by `StepLogStore::record_script_marker` after `record_execution` commits, called by the script service;
  `script_marker_decision`; `SAGAT_STEP_LOGS_ENABLED` read by the control plane and passed to the script service, with
  its test, and the test platform's `Options` field `step_logs_enabled` of T041-6 applied to it too; the metric
  `step_log_script_markers_total`; the marker joins the lock-order test; the tests of section 7, which run the script
  runner through `lab_with` (`tests/support/scripts.rs:38`), the terminal-transition case holding the namespace's
  step-log policy fence exclusively while a cancel ends the run, and it and the barrier test run the store with the
  long marker lock timeout of section 7. Done when these pass, also as the runtime role.
  Refs: FR-041-016
  Owner-gate: OD-2026-11-13-14, OD-2026-11-15-9, OD-2026-11-15-16, OD-2026-11-16-7, OD-2026-11-16-8
  Files: modify `crates/domain/src/step_logs.rs`, `crates/infra-postgres/src/step_logs.rs`,
  `crates/application/src/{step_logs.rs,scripts.rs}`, `crates/api-http/src/routes/step_logs.rs`,
  `crates/telemetry/src/metrics.rs`, `apps/control-plane/src/{config.rs,cli.rs,lib.rs}`, `apps/control-plane/tests/config.rs`,
  `tests/support/api.rs`, `tests/integration/claim-transition-efficiency/step_log_isolation.rs`; create
  `tests/integration/step-logs/script_marker.rs`.
- [ ] T041-29 `getScriptExecution` and `listScriptExecutions` withhold the log of FR-041-022 (`log` empty,
  `logTruncated` false, `logWithheld: true`): for an execution of a run, a function of `crates/application/src/scripts.rs`
  opens a `StepLogReadTx` of T041-11, reads the execution through it and asks `may_read` in it; when a log is returned,
  the throttled `run.logs_read` record is written in it first; an execution with no run is answered as today. The
  descriptions of the two operations change, and with them the digest value and its comment in
  `tests/contract/payload-classification/openapi.rs:385`, and `docs/reference/api.md`, each after reading the diff; the
  release-note line that viewers, and readers without the PII grant, no longer read script logs. Tests: a viewer gets
  no log; a developer without the grant gets no log, and `listScriptExecutions` shows them `logTruncated` false; a returned log is audited; the canary of T041-8 written to a script
  log is absent from a withheld body. Done when these pass, also as the runtime role.
  Refs: FR-041-022
  Owner-gate: OD-2026-11-13-18, OD-2026-11-15-4, OD-2026-11-16-8
  Files: modify `crates/api-http/src/{routes/scripts.rs,dto_scripts.rs}`, `crates/application/src/{scripts.rs,step_logs.rs}`,
  `crates/infra-postgres/src/{scripts.rs,step_logs.rs}`, `openapi.yaml`, `tests/contract/payload-classification/openapi.rs`,
  `docs/reference/api.md`, `docs/releases/unreleased.md`; tests in `tests/integration/step-logs/`.
- [ ] T041-30 `deleteRunLogs`, switched to `x-implemented: '041'` and removed from `not_built`: `run:logs:delete`
  (`ApiOperation::RunLogsDelete`), `run.logs_deleted` in the same transaction for every delete, streams locked in
  `stream_id` order, `204` with no body, readers skipping deleted streams at once, `REMOVED` for a deleted stream, a
  `stream` of another run answered 404; the delete joins the lock-order test; the incident path in the runbook;
  `docs/reference/api.md` regenerated after reading the diff. Tests: each of those behaviours. Done when they pass, also
  as the runtime role.
  Refs: FR-041-023
  Owner-gate: OD-2026-11-15-4
  Files: modify `crates/api-http/src/routes/step_logs.rs`, `crates/application/src/{api_admission.rs,step_logs.rs}`,
  `crates/infra-postgres/src/step_logs.rs`, `openapi.yaml`, `tests/contract/security-multitenancy/rest/battery.rs`,
  `tests/integration/claim-transition-efficiency/step_log_isolation.rs`, `docs/reference/api.md`,
  `docs/runbooks/step-logs.md`; tests in `tests/integration/step-logs/`.

Slice D, the SDKs. Each SDK task builds the rows of section 6 for its language, its buffer, sender and carry, its
protocol constant at 1.7, the `AppendStepLogs` call and the carry on `CompleteStep` and `FailStep` in its RPC wrapper, and, in its conformance runner, the behaviours
`logs` and `logsSecret`, the `stepLogs` op and the `verdict` fault; it adds the micro-benchmark of section 6 and removes
its language from the five scenarios' pending lists. Each is done when the five scenarios (`step-logs-capture`,
`step-logs-redaction`, `step-logs-limits`, `step-logs-transport-faults`, `step-logs-negotiation`) pass in that SDK
(`SAGAT_SDK_LANGS=<language> SAGAT_SDK_SCENARIO=<scenario> cargo test -p sagat-tests --test sdk_conformance --locked --
--ignored`), the SDK's own checks of `sdks/AGENTS.md` pass, and the micro-benchmark's p99, recorded in the validation
report, is under 1 ms.

- [ ] T041-14 `sdks/SPEC.md`: the logger's semantics, the two tables of section 6, the capture section (per-attempt
  object, the secret set and §6.7's rule for it, keys, numbers, exceptions, language context, the answers of
  contracts.md §4, the carry and the wait before a report of section 6), its second section numbered 10 (after §11,
  `sdks/SPEC.md:688`) renumbered 12; the harness additions of section 6 (the scenario
  schema of the behaviours, of the `stepLogs` op, which each SDK runner executes with a plain HTTP request and polls
  until `heldBack` is false, and of the pending languages, matched against the six unknown-op refusals of section 6;
  `mustNotStore` and its self-test with a planted canary); the five scenarios and their goldens, written by hand from
  the scenario, each listing all six languages as pending. Done when the harness's own tests pass and
  `cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored` passes with every SDK pending.
  Refs: FR-041-007, FR-041-015, SC-041-005
  Owner-gate: OD-2026-11-15-4, OD-2026-11-15-7, OD-2026-11-19-1
  Files: modify `sdks/SPEC.md`, `tests/contract/sdk-conformance/{harness.rs,normalize.rs,README.md}`; create the five
  scenario files and their goldens under `tests/contract/sdk-conformance/`.
- [ ] T041-15 TypeScript: the protocol constant to 1.7 (`sdks/typescript/src/config.ts:48`).
  Refs: FR-041-005, FR-041-007, FR-041-015, SC-041-004
  Owner-gate: OD-2026-11-15-7, OD-2026-11-19-1
  Files: modify `sdks/typescript/src/worker/{activity.ts,host.ts,grpc.ts}`, `sdks/typescript/src/config.ts`, `sdks/typescript/conformance/{faults.ts,runner.ts}`, `sdks/typescript/test/worker.test.ts`, the five scenario files and, if a golden needs correcting, its golden; create `sdks/typescript/src/worker/step-logs.ts`, its tests and its micro-benchmark.
- [ ] T041-16 Go: `ProtocolVersion` to 1.7 (`sdks/go/version.go:9`).
  Refs: FR-041-005, FR-041-007, FR-041-015, SC-041-004
  Owner-gate: OD-2026-11-15-7, OD-2026-11-19-1
  Files: modify `sdks/go/worker/{activity.go,task.go,host.go,rpc.go,fake_test.go}`, `sdks/go/internal/safelog/safelog.go`, `sdks/go/version.go`, `sdks/go/conformance/{faults.go,run.go,worker.go}`, the five scenario files; create `sdks/go/worker/steplogs.go`, its tests and its micro-benchmark.
- [ ] T041-17 Python: `PROTOCOL_VERSION` to 1.7 (`sdks/python/sagat_octopus_orchestrator/config.py:25`).
  Refs: FR-041-005, FR-041-007, FR-041-015, SC-041-004
  Owner-gate: OD-2026-11-15-7, OD-2026-11-19-1
  Files: modify `sdks/python/sagat_octopus_orchestrator/worker/{host.py,activity.py,_grpc.py}`, `sdks/python/sagat_octopus_orchestrator/config.py`, `sdks/python/tests/fakes.py`, `sdks/python/conformance/{faults.py,runner.py}`, the five scenario files; create `sdks/python/sagat_octopus_orchestrator/worker/_step_logs.py`, its tests and its micro-benchmark.
- [ ] T041-18 C#: `SdkInfo.ProtocolVersion` to 1.7 (`sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/SdkInfo.cs:12`).
  Refs: FR-041-005, FR-041-007, FR-041-015, SC-041-004
  Owner-gate: OD-2026-11-15-7, OD-2026-11-19-1
  Files: modify `sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/{Workers/WorkerHost.cs,Workers/WorkerRpc.cs,Workers/ActivityExecutionContext.cs,Internal/Safe.cs,SagatOptions.cs,SdkInfo.cs}`, `sdks/dotnet/tests/SagatOctopusOrchestrator.Sdk.Tests/Support/FakeWorkerServer.cs`, `sdks/dotnet/conformance/{FaultPlan.cs,RunMode.cs,RunMode.Operations.cs,Behaviors.cs}`, the five scenario files; create `sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/Workers/StepLogCapture.cs`, its tests and its micro-benchmark.
- [ ] T041-19 Java: `SagatSdk.PROTOCOL_VERSION` to 1.7 (`sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/SagatSdk.java:12`).
  Refs: FR-041-005, FR-041-007, FR-041-015, SC-041-004
  Owner-gate: OD-2026-11-15-7, OD-2026-11-19-1
  Files: modify `sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/worker/{ActivityLogger.java,WorkerHost.java,WorkerConfig.java,WorkerRpc.java}`, `sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/{Hooks.java,SagatSdk.java}`, `sdks/java/sdk/src/test/java/dev/sagatoctopusorchestrator/sdk/worker/FakeWorkerServer.java`, the conformance `FaultPlan.java`, `Runner.java`, `Operations.java` and `Behaviors.java` under `sdks/java/conformance/`, the five scenario files; create `sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/worker/StepLogCapture.java`, its tests and its micro-benchmark.
- [ ] T041-20 Rust: `ActivityContext::log()`, the default `protocol_version` to 1.7 (`sdks/rust/src/worker.rs:58`);
  `Cargo.lock`, `sdks/rust/Cargo.lock`, `sdks/rust/conformance/Cargo.lock` and `sdks/rust/consumer/Cargo.lock` updated in
  the same pull request if the crate's version changes (no dependency is added).
  Refs: FR-041-005, FR-041-007, FR-041-015, SC-041-004
  Owner-gate: OD-2026-11-15-7, OD-2026-11-19-1
  Files: modify `sdks/rust/src/{lib.rs,activity.rs,worker.rs,worker_rpc.rs,worker_tests.rs,capabilities.rs}`, `sdks/rust/conformance/src/{faults.rs,faults/grpc.rs,runtime.rs}`, the four lock files, the five scenario files; create `sdks/rust/src/step_logs.rs`, its tests and its micro-benchmark.
- [ ] T041-21 Run the five scenarios in the six SDKs on the merged tree (`cargo test -p sagat-tests --test
  sdk_conformance --locked -- --ignored`, no language pending) and record the matrix in the validation report, with the
  counting rule checked in `step-logs-limits` in each. Done when every scenario passes in every SDK and the matrix is
  recorded.
  Refs: SC-041-002, SC-041-004, SC-041-005
  Owner-gate: OD-2026-11-13-14
  Files: modify `specs/041-step-logs/validation-report.md`.

Slice E, the Studio:

- [ ] T041-22 `LogView`, the Logs tab and the step section over the list and streams routes: filters, search, the states of
  FR-041-017 (the missing permission from the session, the missing grant from the refusal), markers, the paging through
  empty pages of section 8, windowing, text-only rendering without links and with bidirectional and zero-width
  characters made visible, and `ScriptLogPanel` for a script marker row; unit tests. Done when `npm test` and
  `npm run check:api` pass in `frontend/apps/studio`.
  Refs: FR-041-017, FR-041-019, FR-041-022
  Owner-gate: OD-2026-11-13-18, OD-2026-11-15-9, OD-2026-11-16-8
  Files: create `frontend/apps/studio/src/features/runs/{LogView.tsx,ScriptLogPanel.tsx}` and their tests; modify `frontend/apps/studio/src/features/runs/{RunDetailPage.tsx,NodePanel.tsx}`, `frontend/apps/studio/src/api/endpoints.ts`.
- [ ] T041-23 The tail (`followLogs`, pause and resume, announcements, the polling fallback that stops on `complete`),
  with a test that a dropped connection neither skips nor repeats. Done when `npm test` passes in
  `frontend/apps/studio`.
  Refs: FR-041-012, FR-041-017
  Files: modify `frontend/apps/studio/src/api/{follow.ts,follow.test.ts}`, `frontend/apps/studio/src/features/runs/LogView.tsx`.
- [ ] T041-24 Accessibility and journeys: the axe run and the reflow at 320 px of the Logs tab in
  `frontend/e2e/tests/a11y.spec.ts`; a Playwright spec `frontend/e2e/tests/step-logs.spec.ts` with the keyboard-only
  journey and the journeys of CUJ 1, CUJ 2 and CUJ 5, played by `the_studio_journeys_pass_in_a_real_browser`
  (`tests/e2e/workflow-studio/studio_journeys.rs:95-227`, run as `cargo test -p sagat-tests --test e2e_workflow_studio --
  --ignored journeys`), whose workers use the TypeScript SDK at 1.7 (T041-15). Done when the axe run, the reflow check
  and the journeys pass.
  Refs: SC-041-006, NFR-041-A11Y
  Files: modify `frontend/e2e/tests/a11y.spec.ts`, `tests/e2e/workflow-studio/studio_journeys.rs`; create
  `frontend/e2e/tests/step-logs.spec.ts`.

Slice F, measure, operate and close:

- [ ] T041-9 Measure the built ingest and carry on the benchmark of SC-041-007, the scoped report path included. The
  driver (`tests/performance/scalability-ha/{main.rs,step_log_spike.rs}`) gains a mode that calls the application's
  ingest (`sagat_application::step_logs`) and the carrying report entry points of T041-36 on each in-process replica, as
  the benchmark calls the application's completion, with the ticket semaphore of T041-31 set to T041-7's defaults (the
  gRPC tickets themselves are covered by T041-7's tests); the baseline is the same workload with capture off, as
  SC-041-007 states, and the tenant quota is set as in T041-31. Workloads, named as in T041-35: (a) ten lines carried on
  every completion, with a 1 KiB output, and (f) the same with `{}`, at both sizes; (g) a batch, its answer awaited by the
  driver (the SDK's wait before a report), then a carry; (c) T041-31's fan-out, each body step carrying ten lines (the
  scoped path); (h) 64-record carries. It records SC-041-007's two figures for (a) and (f) at both sizes and for (c);
  the p99 of lease calls while the modelled Logs class is full; the trigger cost and commit latency of a 500-record batch
  (T041-31's workload (b)); the purge rate against the ingest rate and the completion p99 during a purge; what (g)'s
  awaited batch adds to a completion; the stored size per line and per stream row and the stream table's dead tuples after
  the run, in the dated section of the validation report. Done when the figures are recorded; a failure of SC-041-007
  stops the release of the feature for a design change.
  Refs: NFR-041-COST, NFR-041-PERF, SC-041-007
  Owner-gate: OD-2026-11-19-1
  Files: modify `tests/performance/scalability-ha/{main.rs,step_log_spike.rs}`, `specs/041-step-logs/validation-report.md`.
- [ ] T041-25 Alerts with their runbook entries and a promtool rule test `infra/observability/tests/step_logs_test.yaml`;
  the step "promtool test rules" of the `observability` job of `.github/workflows/ci.yml` (`:617-622`, which runs only
  `batch_stalled_test.yaml` today) changed to run every `*_test.yaml` of `infra/observability/tests/`; the runbook
  `docs/runbooks/step-logs.md` completed; the reference documentation (generated files regenerated after reading the
  diff), `docs/architecture/overview.md`, `docs/traceability.md`, a quickstart `docs/quickstarts/step-logs.md` with the
  happy and the failure scenario, and the release notes, whose first line on this feature says that capture is on by
  default (O041-001). Done when the promtool step passes with the new test, and `contract_observability_operations`
  (every alert names an emitted metric and a runbook section) and `contract_developer_experience` pass.
  Refs: FR-041-018, FR-041-022, NFR-041-OPS
  Owner-gate: OD-2026-11-13-14, OD-2026-11-13-18
  Files: modify `infra/observability/alerts.yaml`, `.github/workflows/ci.yml`, `docs/runbooks/alerts.md`, `docs/runbooks/step-logs.md`, `docs/architecture/overview.md`, `docs/traceability.md`, `docs/reference/` (generated), `docs/releases/unreleased.md`, `docs/quickstarts/README.md`; create `docs/quickstarts/step-logs.md` and `infra/observability/tests/step_logs_test.yaml`.
- [ ] T041-26 End-to-end lineage, the switch and mixed versions.
  - (a) A new target `e2e_step_logs` (`tests/e2e/step-logs/main.rs`) on the platform of `tests/support/worker.rs` and,
    for the script marker, the lab of `tests/support/scripts.rs:38`, with a `Worker::append_step_logs` this task adds
    to `tests/support/worker.rs`, expecting: a retry leaves one attempt log per attempt; a reset, a rerun and a
    continue-as-new each start a new run with no log while the source keeps its own; a worker that registers at 1.5
    beside one that registers at 1.7 (`Worker::register_version`, `tests/support/worker.rs:374`): the first is accepted
    at 1.5 and stores nothing, the second stores its lines; with the `Options` field `step_logs_enabled = false`,
    registration answers `enabled = false`, a batch `STOP_CAPTURE` (`CAPTURE_OFF`), and no script marker is written; at
    the end every stream's run exists.
  - (b) An older API replica: a scenario `release-step-logs` of `scripts/test-release-upgrade-profile.py`, with the case
    `release_upgrade_step_logs::a_released_engine_answers_unimplemented_to_a_step_log_batch` and the transcript
    `step-logs.txt`. The current phase migrates the schema; the released phase (the v0.2.0 tree, `RELEASE`) starts its
    execution engine and sends its gRPC address over the scenario's control stream (`control::send`, as the
    `inflight-worker` scenario hands its state over); the current phase registers a session at 1.7 on a current replica,
    sends a batch to the released address and gets `UNIMPLEMENTED`; and, once a step-log quota row exists, the released
    replica's quota listing is refused (section 10).
  Done when both pass.
  Refs: FR-041-014, FR-041-019, SC-041-003
  Files: create `tests/e2e/step-logs/main.rs` and its `[[test]]` entry in `tests/Cargo.toml`,
  `tests/integration/workflow-primitives/release_upgrade_step_logs.rs`, `tests/fixtures/release-upgrade/step_logs.rs`;
  modify `tests/support/worker.rs`, `scripts/test-release-upgrade-profile.py` (`SCENARIOS` and `OVERLAYS`),
  `tests/fixtures/release-upgrade/phase.rs`, `tests/integration/workflow-primitives/main.rs`, `.github/workflows/ci.yml`
  (the scenario list of `released-upgrade`).
- [ ] T041-27 Review of the merged slices B to E by the `security-reviewer` and `correctness-reviewer` agents, each of
  their findings proven by a failing test before it is fixed, and the drift report of `egm-drift-report` against this
  design, each drift answered in the design or the code, written to `docs/designs/041-step-logs.reviews/YYYY-MM-DD/`.
  Done when every finding and every drift has an answer, and every fixed finding its test.
  Refs: NFR-041-SEC, SC-041-004
  Files: create `docs/designs/041-step-logs.reviews/YYYY-MM-DD/{security,correctness,drift}.md`.

#### Follow-up tasks

T041-34 (the Owner-gate of T041-6 on OD-2026-11-16-11) was withdrawn on 2026-11-19: the revision made the design a
Draft, so T041-6 carries the gate itself.

### Verification Plan

What each pull request runs, in CI and before it, is the "Gates" rule of the plan. The feature's integration suite is a
new target of `sagat-tests`, `integration_step_logs`, with real PostgreSQL, picked by the selector rule of T041-32
(constitution 4.5.0); the tests that guard the hot path live in `tests/integration/claim-transition-efficiency/`, which
`scripts/select-test-targets.py:71` selects for every change under `crates/infra-postgres/` and the worker and execution
modules of the application. Races use barriers or held locks, never sleeps; an assertion on what a read returns polls
until `heldBack` is false, within a bound.

| What is proven | How |
|---|---|
| The hot path is unchanged for a report without a carry (SC-041-001) | `work_budget.rs` with its constants unmodified while ingest runs between its windows; a batch commits under `lock_timeout = 50ms` while another transaction holds the run row `FOR UPDATE` and has updated the leased step row; a source-text test that the worker and execution modules never name the log tables and reach them only through `StepLogCarryTx` (T041-5, T041-36) |
| What a carry adds to a report (SC-041-001, section 2a) | The six equalities of section 2a in `work_budget.rs` (T041-36); the time bound by T041-35 before the schema and T041-9 at the end |
| A carry never changes a report | A finalizer raise at the tenant's step-log limit and a failing carry statement each re-run the report once with the same outcome and output; an invalid carry, C0's roles and managed steps, capture off, the deployment switch off and a removed stream leave the report's outcome and output as without a carry; a carry waits at most 200 ms on a step-log lock (T041-36) |
| What a report waits for (SC-041-001) | The barrier tests of SC-041-001: every one of K ingests held after its quota flush, the completion blocked only by them; and an ingest held after T7 that blocks neither a `lock_secrets` nor a completion (T041-6); a carrying report behind an ingest of its own attempt held after T7, failing its carry at 200 ms into its re-run (T041-36). The time bound is SC-041-007's |
| Crash between accept and store | The `StepLogTx` wrapper fails before the commit: nothing stored, the resend stores all, once |
| Crash between store and answer | The wrapper commits and then fails (T041-6, T041-8); separately, the proxy of `tests/support/worker_proxy.rs` drops the answer of `AppendStepLogs` (T041-8): the resend stores nothing and gets `CONTINUE`; the same proxy drops the answer of a carrying report (T041-8, T041-36): the resent report is answered as a replay and stores nothing more |
| Duplicate batch | The same batch twice at a barrier: the second waits on the stream row and gets `RETRY_LATER` at the lock timeout; its resend stores nothing; counters rise once |
| Gaps and late batches | A batch the SDK gave up on is counted as dropped by the worker once; a late batch after a newer one stores nothing and does not lower `last_sequence`; the carry's `highest_sequence` counts the drops at the end; a carry whose transaction id is below that of a batch stored after it stores none of its lines and counts them `out_of_order`; a batch with a lower transaction id never lowers `last_stored_xid` (T041-36) |
| A worker that floods (SC-041-002) | `step-logs-limits` for the SDKs; a client that ignores verdicts for the server: stored bytes stay at the ceilings, tickets hold, another tenant's completion latency is recorded; lines stored + dropped = highest sequence |
| Ceilings under concurrency | Two attempts of one run at their attempt ceilings, and two runs of one tenant at its `tenant_step_log_bytes` limit, at a barrier: the ceilings hold exactly; the finalizer's raise answers `RETRY_LATER` and stores and counts nothing, and the resend, with a gap in the batch and a carry after it, keeps lines stored + dropped = `last_sequence` (T041-6, T041-8); a value that redaction lengthens is charged at its stored size |
| Revocation, session and fence | Binding removed, membership revoked, lease expired but not reclaimed, a membership of the runner or database-runner role, a runner token, an expired session, a session of another namespace or subject or below 1.7, a replaced token, a valid lease sent with another run's id: each stores nothing and gets its verdict; a revoked caller gets `AUTHORITY` whatever else was due, and is audited; a forward step of a stopping run gets `LEASE` and a compensation of it is stored (OD-2026-11-16-11) (T041-6, T041-8) |
| An old worker, an old server | `step-logs-negotiation`; a released (v0.2.0) engine beside a current one (T041-26 (b)) |
| Redelivery does not duplicate lines | A task delivered again under a new generation writes a new stream; the same generation's resend inserts nothing; rebuilding the projections of a run changes no log row (T041-8) |
| Reset, rerun, continue-as-new, retry | `e2e_step_logs` (T041-26): each new run starts with no log and the source keeps its own |
| Read horizon and end | Transaction A stays open, B stores a line and commits: the line waits until A ends, then both arrive in cursor order and `heldBack` was true meanwhile; a filtered read of a 12,000-line run returns its last matching lines before `complete`; an ingest held across the terminal transition is delivered before `end`; a first read of a new audit window that sees the run finished does not end early; a forged `end` for a running run is ignored; a reconnect at any point neither skips nor repeats; the held-transaction cases on PostgreSQL 16 and 18, each on a server of its own (T041-11, T041-12) |
| Lock order | An ingest, a report of the same namespace and a change of a step-log resource (T041-6), then a purge (T041-10), a carrying report beside an ingest of its attempt, the purge and a queued change of a step-log resource (T041-36), a script marker (T041-13) and a delete (T041-30), at barriers, all commit; no deadlock is reported |
| Redaction (SC-041-004) | `step-logs-redaction` in six SDKs; server tests for each shape of both scrubbers in each part of a line and each sensitive field name, of any value type (T041-6); the canary searched in the stored rows, `run_events`, `audit_records` and the captured process output (T041-8) and in the routes' bodies (T041-11, T041-29); the canary carried on a report (T041-36) |
| Tenant boundary | The cross-tenant battery on the four routes; a batch for another tenant's run (T041-8): 404 or `LEASE`, `access.denied.cross_tenant`, no count or size disclosed; a cursor or `stream` of another run is refused |
| Permission, grant and audit | A viewer is refused on the read routes and gets the script execution without its log; a developer without the PII grant is refused in a namespace whose schemas carry no annotation and allowed with it; two first reads in a window write one `run.logs_read` before returning, and a failure to write it returns no line (T041-11); a returned script log is audited (T041-29); a revoked grant stops a tail at its next poll (T041-12); only admin may delete, and every delete is audited (T041-30) |
| Retention | A stream past its expiry returns no line, is marked, then purged 500 lines at a time and shown as expired from the first of them; quota bytes return; the stream row stays with `purged_at`; a lowered retention leaves existing streams' expiry as it was; a stream row held by an ingest is skipped by the mark and marked by the next pass (T041-10, T041-11) |
| Script marker (OD-2026-11-15-9) | The tests of section 7 (T041-13): one marker per inserted execution, written after the report commits, with its node and attempt; one per (run, node, attempt) with two iterations; none for a repeat, a second call, a test run, a finished run, capture off or the switch off; the record kept at the step-log limit; the marker found by the execution id; no deadlock with an ingest, a delete or a policy change; delivered before `end` |
| Separate step-log quota (OD-2026-11-15-5, -6) | A tenant at its `tenant_step_log_bytes` limit stores no step-log line while a script log, a dispatch error and a connector error are stored in full under `tenant_log_bytes`; an empty line is charged 128 bytes; the finalizer's new branch raises at the limit (T041-5); the ingest at the limit (T041-6) |
| Runtime privileges and storage classification | The privilege audit and manifest tests pass with the new statements and columns; every function the migration creates or replaces keeps its `SECURITY DEFINER`, pinned `search_path` and grants (T041-5) |
| Studio (SC-041-006) | Unit tests of the states and markers; the axe run; the keyboard journey; a line holding markup, a URL, escape sequences and bidirectional controls is shown as inert text |

What this plan does not prove is registered as gaps in research.md, all open unless said otherwise: G041-001 (a
transformed or foreign secret; accepted by the owner), G041-002 (the tail of an attempt that lost its lease or its
network), G041-003 (script lines are not live), G041-004 (feature 020 must delete streams with their run; G1618 in
feature 020's research), G041-005 (a deployment whose missing store proof holds its protocol ceiling below 1.7 captures
nothing), G041-006 (SC-041-007 is measured on one machine), G041-007 (a script marker carries no iteration), G041-008
(the cursor shows the transaction counter; accepted by the owner), G041-012 (any open transaction of the cluster holds
the read horizon), G041-013 (no route removes a script execution's log early), G041-014 (an attempt whose capture
stopped before its first batch has no stream), G041-016 (the oldest-transaction gauge sees other roles' transactions only
with `pg_read_all_stats`) and G041-017 (redaction against the process's secret set tells a caller who steers logged text
that a guessed value is a secret; accepted by the owner). G041-009, G041-010 and G041-011 are moot and G041-015 is
closed.

### Estimate

An estimate by reading, not a measurement. Slice A (T041-1 to T041-3, T041-31, T041-33, T041-35): the reviews, two
documentation pull requests of owner answers, the two spikes (about 800 lines of Rust in the benchmark, and about 400 more
for the carry and its seam) and the contracts. Server (T041-4 to T041-8, T041-36, T041-10 to T041-13, T041-29, T041-30):
about 7,000 to 8,000 lines of Rust, most of them tests, in 12 pull requests, and T041-32, a Python selector rule. SDKs: the specification and harness (T041-14), six SDK tasks of 600 to 900
lines each with their tests (T041-15 to T041-20), and the matrix run (T041-21). Studio (T041-22 to T041-24): about 1,300
to 1,600 lines with tests, in 3 pull requests. Measure, operate and close (T041-9, T041-25 to T041-27): 4 pull requests.
That is 35 tasks (T041-28 withdrawn; T041-31 replaced by T041-35) and 34 pull requests. A pull request's CI takes up to two hours, and they land one
at a time where they touch research.md or the contracts, so the calendar is set by the queue: about four weeks for one
agent session working alone, less with the six SDK tasks in parallel after T041-14. The places most likely to move it are
T041-35 (a failure stops for a design change) and the read horizon on two PostgreSQL versions.
