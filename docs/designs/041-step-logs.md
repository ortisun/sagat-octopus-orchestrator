# Step logs per run

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-13

--------------------------------------------------------------------------------

## Overview

The lines a step's code writes to its logger are sent by the worker to the platform, stored per attempt with
ceilings and a retention period, and read through a REST route, a live stream and a Studio screen. The owner
chose this on 2026-11-13 ("Logs de passo por run na 0.3.0 — Feature nova: os workers enviam as linhas de log
de cada passo, a plataforma guarda-as com retenção e limites, e há rota e ecrã no Studio", to be recorded as
OD-2026-11-13-9).

The design treats a step log as a bounded diagnostic record kept **beside** the journal. It is never execution
state: no transition reads it, a lost line never changes a step's outcome, and the transactions that lease and
complete a step do not touch it. What it must never be is a second place where a secret or another tenant's
data can be read.

Eleven choices in this design had more than one defensible answer and were put to the owner as O041-001 to
O041-011, each with its options and a recommendation. The owner answered all of them on 2026-11-13 ("Aceitar
todas", OD-2026-11-13-14): every recommendation is the decision. The section "Owner decisions needed" records
each answer and what was not chosen; the rest of the text states the decided design.

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
`GET /namespaces/{namespaceId}/script-executions/{executionId}`, not from the run.

Three facts of the code shape what can be built:

1. **Workers hold secret values.** `ResolveSecrets` returns them once per attempt, and the server does not keep
   them afterwards. The server therefore cannot recognise a secret value inside a line it receives. The script
   sandbox can (`sagat_domain::scripts::redact` is given the values it handed to the script); for worker logs
   only the worker process can.
2. **The hot path has a budget that fails a pull request.** `tests/integration/claim-transition-efficiency/
   work_budget.rs` holds a lease to 38 statements, 2 transactions and 5 rows and a completion to 50/1/11 or
   47/1/9, as equalities (constitution 4.4.0, CI gates). The engine locks the run row `FOR UPDATE`. A table
   with a foreign key to `runs` would make every insert take a `KEY SHARE` lock on that row and wait for, or
   delay, a lease or a completion.
3. **Nothing deletes a run.** `run_events` and `payloads` are append-only by trigger and feature 020
   (retention and archive, ADR-0054) is a draft. "Purged with the run" is not available; logs need a retention
   of their own.

## Requirements

### Functional

- **FR-041-001 Capture.** When the worker session was accepted at protocol minor 1.7 or later and the policy
  the server returned says capture is on, every line an activity writes through the SDK's activity logger at or
  above the capture level MUST be sent to the platform. The logger MUST keep writing to the host's own logging
  as it does today; capture is added, it replaces nothing.
- **FR-041-002 Identity and order.** A stored line MUST be identified by tenant, run, instance path (the
  scope and iteration of a collection or loop body, as in `ExecutionInstanceIdentity`), node, attempt,
  generation and a sequence that starts at 1 and increases by one per record the worker sends for that
  attempt (the markers the server adds take sequences from a range of their own). It MUST carry the worker's
  time for the line and the server's time of storage. Within an attempt the worker's records are in sequence
  order. Across attempts of a run the order is the order in which the platform stored the lines; the
  worker's time is shown and never used to order.
- **FR-041-003 Levels and fields.** A line has one of five levels (`trace`, `debug`, `info`, `warn`, `error`),
  a text message and an optional flat map of fields whose values are strings, numbers or booleans. Each SDK
  maps its language's levels onto the five; a level above `error` maps to `error`. The capture level is `info`
  unless the worker's configuration says otherwise.
- **FR-041-004 Transport.** Lines travel in batches through a new unary RPC, `AppendStepLogs`, added in
  protocol minor 1.7. A batch names one attempt with the same identity and fence a heartbeat carries (task,
  run, step, attempt, generation, lease token, instance) and holds at most 500 records and 192 KiB. The server
  MUST accept a batch only while that attempt holds its lease, MUST answer only after the lines are committed,
  and MUST store a record at most once however many times its batch arrives (identity of a record: attempt,
  generation, sequence).
- **FR-041-005 Logging never decides the step.** A call to the logger MUST NOT block on the network or on a
  full buffer, and MUST NOT throw into the activity. No failure of capture (refused batch, timeout, server
  down, ceiling reached, invalid line, bug in the capture path) may change the outcome, the output, the retry
  or the timing of a step beyond the bounded final flush of FR-041-015.
- **FR-041-006 Ceilings and markers.** These ceilings hold, each enforced by the server whatever the worker
  does: 8 KiB per message and 2 KiB and 32 keys per field map; 1 MiB and 10,000 records per attempt; 16 MiB and
  100,000 records per run; the tenant's `tenant_log_bytes`. A message over its ceiling is cut at a character
  boundary and flagged `truncated`. Records past an attempt, run or tenant ceiling are not stored; the first
  time a ceiling is met the server stores one marker record that names the ceiling, and the stream counts
  what was dropped. A step MUST NOT fail, wait or retry because a ceiling was met.
- **FR-041-007 Redaction.** Before a line leaves the worker process the SDK MUST replace, in the message and
  in every string field value, each secret value the attempt received through `ResolveSecrets` with
  `[REDACTED]`, by the rule of `sagat_domain::scripts::redact` (values of at least 6 bytes, longest first). The
  server MUST then apply the existing shape rules (`redact_credentials`: PEM private keys, `AKIA`/`ASIA` keys,
  `Bearer` tokens) and MUST replace the value of a field whose key the existing sensitive-name list matches
  (`sagat_telemetry::redact::is_sensitive_name`). The content of a line MUST NOT appear in the journal, a
  projection, telemetry, an audit record, a metric, a span, an error body or the platform's own logs.
- **FR-041-008 Storage beside the journal.** Lines are stored in PostgreSQL in tables of their own. Storing a
  batch MUST NOT lock, or wait for a lock on, a row of `runs`, `steps` or `run_events`, and leasing, heartbeat
  and completion MUST NOT read or write the log tables. The bytes are counted as `log` in the storage
  classification and released when the rows are deleted.
- **FR-041-009 Retention.** Lines are deleted when their attempt's log is older than
  `step_log_retention_days` (a new quota resource, 1 to 365, default 14). The small record that says an attempt
  had a log, with its counters and the time its lines expired, is kept, so that a reader is told "expired" and
  not shown an empty list.
- **FR-041-010 Read.** `GET /runs/{runId}/logs` returns lines in stored order with `cursor` and `limit`
  (default 50, at most 200, clamped, as every list). Filters: `stream` (one attempt's log), `node`, `attempt`,
  `minLevel`, `from` and `to` (server time) and `q` (a literal, case-insensitive substring of the message, at
  most 128 characters). Every item carries the identity of FR-041-002, both times, level, message, fields, the
  `truncated` flag and, for a marker, its kind and counts.
- **FR-041-011 Streams.** `GET /runs/{runId}/logs/streams` lists the attempts of the run that have a log:
  node, instance path, attempt, source (`worker` or `script`), records and bytes stored, records dropped by the
  SDK and by the server with the reason, whether the attempt ended with its final flush, and the expiry time
  or the time the lines were purged.
- **FR-041-012 Live tail.** `GET /runs/{runId}/logs/stream` is a server-sent event stream of new lines with
  the filters of FR-041-010, resumable with `Last-Event-ID`, ended by `end` when the run is terminal and no
  line has arrived for the grace period, with the `reconnect` and ping behaviour and the per-subject and
  per-tenant stream caps of `/runs/{runId}/stream`. It never skips or repeats a line across a reconnect.
- **FR-041-013 Who may read.** The three routes need a new permission, `run:logs:read`, held by the admin,
  developer and operator roles and not by viewer, worker or runner. Another tenant's run is answered 404 and
  recorded as `access.denied.cross_tenant`, as every run route. A read is audited as `run.logs_read`, at most
  once per subject and run in 300 seconds, committed before the first page of that window is returned.
- **FR-041-014 Mixed versions.** A worker accepted below 1.7 sends nothing and behaves exactly as today. A
  1.7 SDK against a server that accepts a lower minor, or whose policy says capture is off, sends nothing and
  logs locally. No published workflow, run in progress or existing RPC changes meaning.
- **FR-041-015 The six SDKs.** Each SDK keeps its present logger type and adds capture behind it: a bounded
  buffer per attempt (1,000 records or 256 KiB, whichever is met first), a flush every second or at 256
  records, one batch in flight per attempt, at most two retries of the same unchanged batch, and a final flush
  before `CompleteStep` or `FailStep` that waits at most 2 seconds. The Rust SDK gains the logger its
  `ActivityContext` lacks. A record the SDK drops is counted, and the next batch carries a marker with the
  count and the reason.
- **FR-041-016 Steps the platform runs.** A script step's log stays where feature 005 stores it. When the
  script runner's report is recorded, the platform stores one marker record in the run's log that names the
  script execution, so the run's log shows that the step produced output and where to read it. The database
  runner and the lake runner capture no log (they return typed results and write no user text).
- **FR-041-017 Studio.** The run screen gains a Logs tab for the whole run and a Logs section for the selected
  step and attempt, with filters, search, a live tail that pauses when the reader scrolls up, markers shown as
  rows, the states "not allowed", "no log", "expired" and "ended without its final flush", and full keyboard
  operation.
- **FR-041-018 Observability.** The platform counts records accepted, dropped (by where and why), truncated
  and redacted, batches by outcome, ingest duration, purged records and open tail streams, with labels that are
  closed enumerations only, and alerts when the server drops lines for a sustained period or the purge falls
  behind.
- **FR-041-019 Lineage.** A log belongs to the run and attempt that wrote it. A retry is a new attempt and a
  new log. A reset, a rerun and a continue-as-new open a new run; the new run starts with no log and the
  earlier run keeps its own, reachable through the links the Studio already shows between those runs. Lines are
  never copied.
- **FR-041-020 Policy.** Capture on or off, the attempt and run ceilings and the retention are quota resources
  read at registration and at storage; changing one is audited by the quota catalogue as every quota change is.

### Success criteria

- **SC-041-001** With the migration applied and a worker that logs on every step, `work_budget.rs` passes with
  its three constants unmodified, and a test that holds the run row `FOR UPDATE` in one transaction sees a
  batch for that run commit without waiting.
- **SC-041-002** An activity that writes 1,000,000 lines as fast as it can completes with the same outcome and
  output as one that writes none, the store holds no more than the attempt ceiling plus its markers, and the
  stream's counters account for every line not stored.
- **SC-041-003** In the fault tests of the Verification Plan (crash between accept and commit, crash between
  commit and answer, duplicate batch, lost batch, stale lease, server unavailable) no record is stored twice
  and every record that is missing is covered by a marker or by the "ended without its final flush" state.
- **SC-041-004** A canary secret resolved by the attempt and written to the logger whole, inside a longer
  message and as a field value appears nowhere in the stored lines, the three routes, the journal, telemetry,
  audit records or the platform's logs, in all six SDKs.
- **SC-041-005** The five conformance scenarios of the Technical Plan pass in TypeScript, Go, Python, C#, Java
  and Rust from the same scenario files and goldens.
- **SC-041-006** The Logs tab and section pass the axe run at WCAG 2.2 AA used by `frontend/e2e/tests/
  a11y.spec.ts`, reflow at 320 px, and a journey done with the keyboard only.
- **SC-041-007** On one machine, the same day, the transitions scenario of `tests/performance/scalability-ha`
  with every step writing ten `info` lines reaches at least 90% of its rate with capture off, in the rows
  "1 API replica, 16 workers" and "8 replicas, 128 workers". The machine, its load and the PostgreSQL
  configuration are stated; the figure is a criterion to verify, not a promise.

### User stories

- **US01** A developer opens a failed run in the Studio, selects the failed step and reads what its code logged
  in the attempt that failed and in the ones before it.
- **US02** An operator watches a long step of a running workflow and sees its lines arrive.
- **US03** A tenant administrator bounds what logs cost (bytes, days) and turns capture off for a namespace
  whose code logs personal data.
- **US04** An SDK user changes nothing in their activity code and gets capture after upgrading the SDK.

## Goals

- What a step's code logged is readable from the run, by the people allowed to, during the run and for a
  stated number of days after.
- Capture costs the lease and completion transactions nothing, and costs the step at most one bounded flush.
- Every line that is not there is accounted for by a count and a reason the reader can see.
- A secret the platform handed to an attempt is not stored, in any of the six SDKs.

## Non-Goals

- **Not the platform's own logs.** The stdout JSON of the processes, their spans and the Collector pipeline
  belong to features 008 and 015. This feature does not export step logs to OpenTelemetry; that is a later
  feature and would need its own redaction review.
- **Not stdout or stderr of the worker process.** Only lines written through the SDK's activity logger are
  captured. A `print` in an activity is not, and lines written by a thread that outlives its activity are not.
- **Not a log search product.** There is no index on the text, no query across runs, no aggregation, no
  alerting on content. `q` is a substring match inside one run.
- **Not execution state.** A workflow cannot read a step's log, an expression cannot test it, and it is not an
  input to a compensation or a review.
- **Not an archive.** Legal hold, export to a customer's storage and retention beyond 365 days belong to
  feature 020.
- **No change to script logs.** `script_executions`, its route, its ceiling and its retention stay as feature
  005 built them. Live lines from a running script are a follow-up (G041-003).
- **No classification of log content by field.** The `x-sagat-classification` annotations of feature 016
  describe payload schemas; a log line has no schema. `docs/runbooks/payload-classification.md` already says
  the annotation does not classify logs.
- **No download or export button** in the first delivery. Reading is through the routes.

## Open Questions

The owner's questions are answered (next section). These are technical and are closed by a task, not by a
decision:

- **Does the tenant usage row become a point of contention?** ADR-0027 counts retained bytes in
  `tenant_retained_usage`, one row per tenant, updated by a deferred trigger at commit. A completion that
  stores an output updates the same row. Every log batch of a tenant will too. The update is short, but it is
  one row. T041-9 measures it (SC-041-007); if the criterion fails, the fallback is a commit that groups the
  batches of several requests (Alternatives Considered), and a separate usage row is the step after that and
  needs an amendment of ADR-0027.
- **Does the stored-order cursor hold on PostgreSQL 16 and 18?** The order is (transaction id, row id), read
  below the snapshot's `xmin` horizon (Technical Plan, Reading). CI runs 16 and the local machine 18.6; T041-11
  proves it with a held transaction on both.
- **Which scope do the new quota resources take?** The catalogue has tenant-scoped storage and retention
  resources; whether the namespace scope is available for the capture switch and the ceilings is settled in
  T041-4 against `crates/domain/src/quota.rs`. If it is not, the switch O041-001 decided "per namespace" cannot
  be built as decided: T041-4 stops and the difference goes back to the owner, it is not reinterpreted.
- **Is 1.7 reachable where 1.6 is not?** `protocol_ceiling` (`crates/application/src/worker.rs`) offers 1.5
  only when the store proves scoped support and 1.6 only when it also proves database support. Minors are a
  line, so 1.7 is offered when both hold and the log tables exist. A deployment held at 1.5 by a missing proof
  captures nothing (G041-005).

## Owner decisions needed

All answered. The owner accepted every recommendation on 2026-11-13 ("Aceitar todas"), recorded as
OD-2026-11-13-14 in `docs/owner-decisions.md`; the tasks that waited on an answer carry an `Owner-gate:` line
citing that row. Each item below states the decision, then the options that were not chosen and why. They are
registered as O items in `specs/041-step-logs/research.md`. Two decisions remain the owner's and are separate
from this row: accepting ADR-0084 and ADR-0085, which are proposed.

- **O041-001 Capture is on by default.** *Decided (OD-2026-11-13-14): on for every namespace at level `info`,
  with a switch to turn it off per namespace and a worker option to turn it off per process.* Not chosen: off
  until a namespace turns it on; on for new namespaces only. The cost of the decision is that upgrading an SDK
  starts sending text the customer wrote for their own machines, so the release notes say it in their first
  paragraph (T041-25). Governs FR-041-001 and FR-041-020.
- **O041-002 Who may read step logs.** *Decided (OD-2026-11-13-14): a new permission `run:logs:read` held by
  admin, developer and operator.* Not chosen: giving it to viewer too (a log is free text that can hold
  personal data, and viewers hold `run:read`); granting it per namespace as `payload:reveal-pii` is (the
  feature would be unusable until someone grants it). Governs FR-041-013.
- **O041-003 Reads are audited.** *Decided (OD-2026-11-13-14): one `run.logs_read` record per subject and run
  per 300 seconds, as `payload.inspected` is.* Not chosen: one per request (an audit row for every page a tail
  delivers); none. Governs FR-041-013.
- **O041-004 Where lines are stored.** *Decided (OD-2026-11-13-14): PostgreSQL, one row per line, in an
  ordinary table, for 0.3.0.* It gives duplicate removal by primary key, filters and paging in SQL, and exact
  quota accounting with the triggers ADR-0027 already generates. Not chosen: one row per batch holding
  compressed lines (about a tenth of the rows; it is the next step if SC-041-007 fails on storage rather than
  on transactions, and filters and search would then decode chunks in the application); objects in the blob
  store with an index row per chunk (cheapest per byte, but committed blobs are never purged today, G69, and a
  tail would read objects); a partitioned table (Alternatives Considered). Governs the Technical Plan and
  ADR-0084.
- **O041-005 The ceilings.** *Decided (OD-2026-11-13-14): 8 KiB per message; 1 MiB and 10,000 records per
  attempt (the figure a script log already has); 16 MiB and 100,000 records per run; the tenant's existing
  `tenant_log_bytes`, shared with script logs.* Not chosen: a separate `tenant_step_log_bytes`; one number says
  how much log a tenant keeps. Governs FR-041-006.
- **O041-006 Retention.** *Decided (OD-2026-11-13-14): 14 days by default, 1 to 365, as
  `step_log_retention_days`.* Not chosen: 7 days, 30 days, reusing `script_retention_days`. When feature 020 is
  built, a run's log never outlives the run. Governs FR-041-009.
- **O041-007 What is kept when an attempt reaches its ceiling.** *Decided (OD-2026-11-13-14): the first lines,
  then a marker and a count.* Not chosen: the last lines, by deleting the oldest. That keeps what precedes a
  failure, which is often what is wanted, but it needs deletes on the write path and makes a cursor unstable;
  the step's error is in the journal either way. Governs FR-041-006.
- **O041-008 The live tail.** *Decided (OD-2026-11-13-14): a new server-sent event route,
  `/runs/{runId}/logs/stream`.* Not chosen: the Studio polling the list route with its cursor (it remains the
  fallback when a stream cannot be held). Log events on the existing `/runs/{runId}/stream` were never an
  option: that stream resumes by one `Last-Event-ID`, which is the projection sequence, and it is granted by
  `run:read`. Governs FR-041-012.
- **O041-009 Script steps in the first delivery.** *Decided (OD-2026-11-13-14): a marker in the run's log that
  points to the script execution, whose log is read where it is today.* Not chosen: the script runner
  streaming its lines through 1.7 during the run (it needs redaction over a moving window in
  `crates/infra-sandbox` and either stores the log twice or changes feature 005's contract; it stays as
  G041-003); leaving script steps out. Governs FR-041-016.
- **O041-010 What "in 0.3.0" requires.** *Decided (OD-2026-11-13-14): server, six SDKs, routes and Studio
  together.* Not chosen: the SDKs following one by one. Principle IV holds the six SDKs to one conformance
  suite; a release that captures from two languages would publish a difference between them. The estimate at
  the end of this document says what the decision costs.
- **O041-011 The limit of redaction.** *Decided (OD-2026-11-13-14): accepted as the stated limit.* The
  platform cannot recognise a secret that the step's code transforms before logging it (encoded, split,
  shorter than 6 bytes) or one the code obtained elsewhere, and a hand-written client that speaks 1.7 without
  the SDK's redaction can store what it was given (G041-001). The limit is written into ADR-0085, the runbook
  and the release notes; the namespace switch of O041-001 is the control for code that cannot be trusted with
  it.
- **Still to decide: acceptance of the ADRs.** [ADR-0084](../adr/ADR-0084-step-logs-are-a-bounded-lossy-per-attempt-record-beside-the-journal.md)
  and [ADR-0085](../adr/ADR-0085-step-log-text-is-read-under-its-own-permission-and-redacted-by-value-in-the-worker.md)
  are Proposed. Accepting an ADR is the owner's, separately from OD-2026-11-13-14.

## Critical User Journeys (CUJs)

### CUJ 1: Why did this step fail?

*   **Given**: a run whose step `charge` failed on its third attempt; the activity logs at `info` and `error`
    through the SDK's logger; the reader holds `run:logs:read`.
*   **When**: they open the run in the Studio, select `charge` and open Logs.
*   **Then**: they see the lines of attempt 3, can switch to attempts 1 and 2, each line with its level as text
    and shape, its worker time and its fields, and the secret the activity used shows as `[REDACTED]`.

### CUJ 2: Watching a long step

*   **Given**: a running step that logs a line every few seconds.
*   **When**: the reader opens the Logs tab with the tail on.
*   **Then**: lines appear within about a second of the worker's flush; scrolling up pauses the tail and a
    "Jump to latest" control resumes it; when the connection drops and returns, no line is missing or doubled;
    when the run ends, the stream ends.

### CUJ 3: A step that floods

*   **Given**: an activity with a loop that logs two million lines.
*   **When**: it runs.
*   **Then**: the step completes as it would without logging; the log holds the first lines up to the attempt
    ceiling and a marker "attempt ceiling reached"; the streams list says how many were dropped by the SDK's
    buffer and how many by the ceiling; `step_log_lines_dropped_total` rises and, if it lasts, the alert fires.

### CUJ 4: A worker that was not upgraded

*   **Given**: a worker on an SDK that offers protocol 1.5.
*   **When**: it runs steps on a server that supports 1.7.
*   **Then**: nothing changes for it; its steps show "no log" in the Studio, with the note that the worker did
    not send any.

### CUJ 5: A reader without the permission

*   **Given**: a viewer, who holds `run:read` and not `run:logs:read`.
*   **When**: they open the run.
*   **Then**: the Logs tab says which permission is missing and makes no request; a direct call to the route is
    refused with the typed error, and nothing about the log's existence or size is returned.

### CUJ 6: A collection's iterations

*   **Given**: a `forEach` over 200 items whose body step logs.
*   **When**: the reader opens the whole-run Logs tab and filters by that node, then picks iteration 17.
*   **Then**: the tab shows lines of all iterations in stored order, each labelled with its iteration; the
    filter narrows to the one attempt log of iteration 17.

## Technical Plan (Architecture)

```text
activity code
   | logger.info(...)                      (unchanged API; never blocks, never throws)
   v
SDK capture: level floor -> redact by value -> cut -> bounded buffer per attempt -> markers for drops
   | AppendStepLogs (unary, 1.7): attempt identity + fence, <=500 records, <=192 KiB; same batch on retry
   v
api-grpc -> application::step_logs::ingest
   | one transaction, separate from lease/heartbeat/complete:
   |   read the lease (no lock) -> upsert stream -> charge run -> shape scrub -> insert (conflict: nothing)
   v
PostgreSQL: step_log_streams, step_log_lines, run_log_usage     (no foreign key to runs or steps)
   ^                                   ^
   | purge (housekeeping pass)         | GET /runs/{id}/logs, /logs/streams, /logs/stream (run:logs:read)
                                       v
                                    Studio: Logs tab and step section
```

### 1. Wire: protocol minor 1.7

`specs/001-platform-foundation/contracts/worker.proto` (the canonical copy; the six SDK copies are
byte-identical and move with it, and `worker.baseline.json` records the addition) gains, additively:

- `rpc AppendStepLogs(AppendStepLogsRequest) returns (AppendStepLogsResponse)`.
- `AppendStepLogsRequest`: the fields `HeartbeatRequest` carries to name and fence an attempt (`task_id`,
  `worker_id`, `generation`, `lease_token`, `run_id`, `step_id`, `attempt`, `instance`), `repeated
  StepLogRecord records`, and `bool final` (the SDK's last batch for the attempt).
- `StepLogRecord`: `uint64 sequence`; `int64 worker_time_epoch_ms`; `level`; `kind` (`LINE`, `DROPPED`);
  `string message`; `repeated StepLogField fields` (key and one of string, double, int64, bool); `bool
  truncated`; for `DROPPED`, `uint64 dropped_count` and a reason (`BUFFER_FULL`, `SEND_FAILED`,
  `STOPPED_BY_SERVER`).
- `AppendStepLogsResponse`: `uint64 stored_through_sequence` and a verdict: `CONTINUE`; `STOP_ATTEMPT` (a
  ceiling was met: send nothing more for this attempt except the final marker); `STOP_CAPTURE` (capture is off,
  or the lease is no longer this attempt's); `RETRY_LATER` with a delay (the server is shedding ingest).
- `RegisterWorkerResponse` gains `StepLogPolicy step_logs`: `enabled`, the line, field, batch, buffer and
  attempt ceilings and the flush interval. An SDK captures only when the accepted minor is at least 1.7 and
  `enabled` is true, and uses the server's numbers, so a ceiling can change without an SDK release.

Negotiation is unchanged (ADR-0015): `negotiate_protocol_details` answers the lower of the two minors;
`PROTOCOL_MINOR` becomes 7 with a `STEP_LOG_PROTOCOL_MINOR = 7` constant beside the others, and the handler
refuses the RPC below it as `StreamTasks` is refused below 1.4. The new SDK versions offer `1.7`; the
conformance scenarios pin the minor they test, as `sdks/AGENTS.md` requires. The sandbox runner keeps offering
`1` and the database runner `1.6`.

The call is authenticated as every worker RPC (bearer token, role `worker`, tenant from the principal, the
session's namespace). It needs no new worker permission: it reports on a lease the caller holds.

**Why a separate unary RPC.** A field on `Heartbeat` would tie the size and the failure of a log batch to the
call that keeps the lease alive. A client stream would need acknowledgement and resend rules in six languages
for a gain that the batch already gives. Riding on `CompleteStep` would add statements to the completion,
which SC-041-001 forbids.

### 2. Ingest

A new application module, `sagat_application::step_logs`, with a port `StepLogStore` and its adapter in
`crates/infra-postgres/src/step_logs.rs`. Domain types (level, record kind, ceilings, the marker reasons, the
validation of a record) live in `crates/domain/src/step_logs.rs` and import nothing from the wire.

One batch is one transaction, opened with the project's `begin_safely` rule (ADR-0014):

1. **Read the lease without a lock.** One `SELECT` on `steps` by its primary key (tenant, run, instance path,
   node) compares attempt, generation and the lease-token hash and requires the leased state. No `FOR SHARE`,
   no `FOR UPDATE`. A mismatch answers `STOP_CAPTURE` and stores nothing. The read can race a completion: a
   batch that was valid when read and commits just after the completion is still that attempt's lines, so the
   race is harmless. A stale holder's token never matches, so it cannot write into a newer attempt's log.
2. **Find or create the stream.** `INSERT … ON CONFLICT DO UPDATE … RETURNING` on `step_log_streams`, keyed by
   (tenant, run, instance path, node, attempt, generation). The row is locked for the rest of the transaction;
   only batches of the same attempt wait on it, and an SDK sends one at a time.
3. **Apply the attempt ceiling from the stream's counters**, then **the run ceiling** by updating
   `run_log_usage` (one row per run; parallel attempts of one run queue here briefly, the engine never touches
   it), then **the tenant ceiling** with the existing log allowance (`sagat_quota_lock_log_usage`, as
   `content_usage::take_log_prefix` does for script logs): the records that fit are kept, the rest are counted.
4. **Scrub and cut** each kept record (FR-041-007's server rules, the message and field ceilings).
5. **Insert** the kept records in one statement with `ON CONFLICT (tenant_id, stream_id, sequence) DO NOTHING`
   and update the stream's counters by the rows actually inserted. A repeated batch inserts nothing and gets
   the same answer. When a ceiling is first met, the marker record is stored in the same statement under the
   next sequence the server reserves for markers (sequences at and above 2^62 belong to the server, so a
   marker never collides with a worker's record).
6. **Commit, then answer.** A crash before the commit leaves nothing and the SDK sends the batch again; a
   crash after it leaves the lines, and the resend inserts nothing.

The number of top-level statements per batch does not depend on the number of records. It is recorded as a
budget in the feature's own suite when T041-6 is built, as an equality, in the manner of `work_budget.rs`.

**Shedding.** Ingest takes its connection through a semaphore sized well below the pool (a setting, default a
quarter of it). When no permit is free within 50 ms the answer is `RETRY_LATER`; the SDK keeps the batch, and
its buffer drops and counts if the delay outlasts it. The lease and completion paths never wait behind logs
for a connection.

**What a batch may contain.** Sequences must increase within a batch; a record with a sequence at or below
the stream's `last_sequence` is a duplicate and is ignored; a batch that is malformed (an empty message with
no fields, a sequence in the server's range, an unknown level) is refused whole as `INVALID_ARGUMENT`, counted,
and the SDK drops it. Worker time is stored as sent when it is within a sane range (2000-01-01 to one day
ahead of the server) and as null otherwise.

### 3. Storage

Three tables, in one migration (the next free timestamp after `20270112000000_lake_blob_pin_release`, checked
against the other open branches before it is taken):

- `step_log_streams`: `tenant_id`, `stream_id` (identity), `namespace_id`, `run_id`, `instance_path_nodes
  text[]`, `instance_path_indices integer[]`, `node`, `step_id`, `attempt`, `generation`, `source` (`worker`,
  `script`), `records`, `bytes`, `last_sequence`, `dropped_by_sdk`, `dropped_by_server`, `ceiling_reason`,
  `truncated_records`, `final_at`, `first_at`, `last_at`, `expires_at`, `purged_at`. Unique on the attempt
  identity; indexed by (tenant, run) and by `expires_at` where `purged_at` is null.
- `step_log_lines`: `tenant_id`, `run_id`, `stream_id`, `sequence`, `id` (identity), `stored_xid` (the
  transaction that stored it, as the quota tables already record `pg_current_xact_id()`), `stored_at`,
  `worker_time`, `level`, `kind`, `truncated`, `message text`, `fields jsonb`. Primary key (tenant, stream,
  sequence); index (tenant, run, stored_xid, id) for reading. A trigger refuses `UPDATE`; `DELETE` is allowed
  for the purge.
- `run_log_usage`: `tenant_id`, `run_id`, `records`, `bytes`.

**No foreign key to `runs` or `steps`.** This departs from the `*_same_tenant_fk` convention and is the
reason FR-041-008 can hold: an insert that references `runs` takes `KEY SHARE` on the run row, which conflicts
with the `FOR UPDATE` the engine holds while it leases or completes. The run's existence in the tenant is
proven by step 1 of the ingest in the same transaction, and nothing deletes a run today. Feature 020 must
delete a run's streams when it deletes the run; that is written into G041-004 so it is not forgotten.
`step_log_lines` does reference `step_log_streams` (same tenant), a row the engine never locks.

**Accounting.** `step_log_lines.message` and `.fields` are classified `log` in
`crates/infra-postgres/storage-classification.json`; the other variable-size columns (`node`, the path arrays,
`ceiling_reason`) are classified with the class the same columns of `steps` have. The generated triggers of
ADR-0027 then charge `tenant_retained_usage.log_bytes` at commit and release it on delete, with no new
mechanism, and `manifest_covers_every_variable_size_column_…` keeps it honest.

**Grants.** In the migration's `DO $grants$` block: `sagat_app` gets `SELECT, INSERT, DELETE` on
`step_log_lines`, `SELECT, INSERT, UPDATE` on `step_log_streams` and `run_log_usage`, and usage of the
identity sequences. No `UPDATE` on lines, no `TRUNCATE`, no function with `SECURITY DEFINER`. The runtime
privilege audit (`every_statement_the_stores_issue_is_within_the_runtime_roles_privileges`) covers the new
statements. There is no row-level security in this database; the tenant boundary is the `tenant_id` in every
key and predicate, tested by the cross-tenant battery. The down migration refuses to run while a line exists.

**Not partitioned.** No table in this database is partitioned. Dropping a partition would not fire the
triggers that release quota bytes, and retention differs per tenant, so a partition by time would still be
purged by `DELETE`. The table gets its own autovacuum settings instead.

### 4. Retention and purge

`expires_at` on the stream is set at creation and moved forward by each batch to "now plus
`step_log_retention_days`". A housekeeping pass (`crates/scheduler/src/passes.rs`, beside
`purge_script_records`, through a new method of `MaintenanceStore`) takes streams past `expires_at` with
`FOR UPDATE SKIP LOCKED`, deletes their lines in batches of `PURGE_BATCH`, and sets `purged_at` when none is
left. Deleting releases the quota through the triggers. A stream of a step that is still leased is skipped
until the lease ends. `run_log_usage` is not reduced by a purge: the run ceiling bounds what a run may ever
write, not what is retained.

### 5. Reading

A second port, `StepLogReader`, used by three handlers in `crates/api-http/src/routes/execution.rs`. Each
calls `admit` with a new `ApiOperation::RunLogsRead` that maps to `Permission::RunLogsRead` (`run:logs:read`),
and the application layer authorizes again, as `RunRead` is checked twice today. The permission is added to
`Permission::ALL` (53 becomes 54), to the admin, developer and operator lists, and to the role matrix test.

**Order and cursor.** Lines are read in (`stored_xid`, `id`) order, and only those whose `stored_xid` is
below the oldest transaction still in progress (`pg_snapshot_xmin(pg_current_snapshot())`). Every transaction
with a smaller id has ended, so no line can later appear before the cursor: a page is final and a tail never
skips. The cursor is that pair, opaque to the client. The cost is that a long write transaction anywhere in
the database holds the tail back until it ends; the lines are not lost, they arrive late, and the response says
how far behind the horizon is. With `stream` set, the worker's records come out in sequence order, because
one attempt's batches commit one after another, and a server marker sits where it was stored.

**Filters** are predicates on indexed columns except `q`, which is `position(lower($q) in lower(message)) >
0` over the lines of one run (at most 16 MiB by the run ceiling), under the route's statement timeout. The
value is a bound parameter, never part of the statement text.

**Audit.** `run.logs_read` is a new action in `crates/domain/src/audit.rs`. The first read by a subject of a
run in a 300-second window writes the record in the read transaction before any line is returned ("no audit,
no lines"); later reads in the window do not write. The record holds the run, the subject and the filters'
names, never a line or the value of `q`.

**Tail.** `GET /runs/{runId}/logs/stream` reuses the loop of `follow` in `execution.rs`: server-sent events,
`event: log` with up to 200 lines and the cursor as `id`, a `: ping` comment every 15 seconds, `event: end`
when the run is terminal and no line has been stored for 5 seconds, `event: reconnect` at token expiry or the
15-minute limit, resume by `Last-Event-ID`, polling every 0.5 s and every 2 s once idle for 30 s, the caller's
role checked again on each poll, and the same counters and caps (20 per subject, 200 per tenant) as the graph
stream, shared with it.

**Contract.** Three operations in `specs/001-platform-foundation/contracts/openapi.yaml`, tagged
`x-implemented: '041'`: `listRunLogs`, `listRunLogStreams`, `streamRunLogs`, with typed errors. The digest
test `operations_outside_the_approved_additions_and_event_extensions_are_byte_stable` gains `041` in its list
of approved additions; no existing operation changes, so the digest itself does not. `/attempts` is left as it
is for that reason: the counters live in the streams route.

### 6. The six SDKs

`sdks/SPEC.md` §6.3 gains the logger's semantics and a new section defines capture; §9 lists 1.7; §10 keeps
its rule that a logger that throws never kills a heartbeat or a task, and adds that capture never does either.

| SDK | Logger today (kept) | Capture is added as |
|---|---|---|
| TypeScript | `ActivityContext.log: ActivityLogger` | a second sink behind the same four methods |
| Go | `Logger *slog.Logger` | a `slog.Handler` that wraps the host's handler |
| Python | `ActivityContext.log` (`logging.LoggerAdapter`) | a `logging.Handler` attached to the adapter's logger for the attempt |
| C# | `ILogger Logger` | an `ILogger` decorator |
| Java | `ActivityLogger logger()` | a decorator of `ActivityLogger` |
| Rust | none | a new `ActivityContext::log()` with the five levels, which also emits `tracing` events |

Common rules, each a line of the specification and a conformance assertion:

- The logger call formats, redacts by value (the secrets the attempt resolved; the set is empty when the
  attempt resolved none), cuts to the line ceiling and appends to the attempt's buffer, without waiting. A full
  buffer drops the new record and counts it.
- A sender per attempt flushes every second, or at 256 records, one batch in flight. A failed send is retried
  twice with the same bytes, then the batch is dropped and counted; the next batch starts with a `DROPPED`
  marker.
- `STOP_ATTEMPT` and `STOP_CAPTURE` stop sending and keep counting. `RETRY_LATER` waits the given delay.
- Before `CompleteStep` or `FailStep`, the SDK flushes what remains, with `final = true` and the last
  counts, waiting at most 2 seconds; then it reports the step whatever the flush did.
- The buffer is in memory only and dropped with the attempt. Capture uses no thread or task that can keep the
  process alive after shutdown, and worker shutdown flushes each attempt once within the shutdown deadline.
- Nothing about capture is written at a level the host logs by default except one line per process when the
  server stops capture.

Conformance scenarios, one file and one golden each, run by all six through `tests/contract/sdk-conformance`.
The harness gains a step that reads `GET /runs/{id}/logs` and compares it with the golden, and a server fault
hook for the RPC:

1. `step-logs-capture`: the five levels, the floor, fields of each type, order by sequence, two attempts of
   one step, and an iteration of a root collection (instance path).
2. `step-logs-redaction`: a resolved secret logged whole, inside a message and as a field value; `mustNotLeak`.
3. `step-logs-limits`: a message over the ceiling is cut and flagged; a flood meets the buffer and the attempt
   ceiling; the step completes with its normal output; the counts add up.
4. `step-logs-transport-faults`: a lost response (same batch twice), a refused batch, `RETRY_LATER`, the server
   down for the whole attempt; no duplicate; the step completes.
5. `step-logs-negotiation`: a worker pinned to 1.5 sends nothing; a 1.7 worker told `enabled = false` sends
   nothing; both log locally.

### 7. Steps the platform runs

`record_execution` (`crates/application/src/scripts.rs`), which stores the script runner's report, also stores
one record in the run's log for that attempt in the same transaction: source `script`, kind `script_output`,
with the execution id, the log's size and its `log_truncated` flag, and no text. The streams route lists the
attempt with source `script`. The Studio shows the marker as a row that opens the script execution's log
through the route feature 005 already has, under that route's permission. The script's text is stored once,
counted once, redacted by the sandbox as today.

### 8. Studio

In `frontend/apps/studio/src/features/runs/`:

- A fourth tab, **Logs**, in `RunDetailPage.tsx` beside Graph, Table and Timeline, and a **Logs** section in
  `NodePanel.tsx` for the selected step with an attempt selector fed by the streams route.
- `LogView.tsx`: rows windowed with `useWindowedRows`; each row shows time (worker time, with the stored time
  on demand and a note when the two differ by more than five minutes), level as text and shape as well as
  colour, the step and iteration in the whole-run view, the message as text (never as HTML; terminal escape
  sequences are shown stripped), and fields in an expandable part.
- Filters: level, step, attempt, time range; a search field that sends `q`; all reflected in the URL except
  the search text, which `frontend/AGENTS.md` keeps out of URLs when it could hold a value.
- Tail: `followLogs()` beside `followRun()` in `src/api/follow.ts`, same parser (`readSse`), same fallback to
  polling the list route. The tail pauses when the reader leaves the end; "Jump to latest" resumes. New lines
  are announced through `Announcer` as a count at most every ten seconds, not line by line.
- States: `NotAllowed permission="run:logs:read"` with no request made; "no log" with the reason when known
  (worker below 1.7, capture off); "expired on …"; "ended without its final flush: the last lines may be
  missing"; markers for drops, ceilings and script output as rows with text.
- Keyboard: the tab follows the existing `Tabs` pattern; the list is one tab stop with arrow, Page, Home and
  End movement and Enter to expand a row; every control is reachable and named; focus is not moved by arriving
  lines.
- Types come from `npm run api`; `npm run check:api` stays green.

### 9. Observability

Metrics (bare names as the code uses; labels are closed enumerations, never tenant, namespace, run, node or
worker):

- `step_log_batches_total{outcome}`: `stored`, `duplicate`, `stopped_attempt`, `stopped_capture`,
  `retry_later`, `invalid`.
- `step_log_lines_accepted_total{level}`, `step_log_bytes_accepted_total`.
- `step_log_lines_dropped_total{where, reason}`: `where` is `sdk` (reported by markers) or `server`; reasons
  `buffer_full`, `send_failed`, `stopped_by_server`, `attempt_ceiling`, `run_ceiling`, `tenant_quota`.
- `step_log_lines_truncated_total`, `step_log_redactions_total{rule}` (`shape`, `field_name`).
- `step_log_ingest_seconds`, `step_log_purged_lines_total`, `step_log_purge_backlog` (streams past expiry),
  `run_log_streams_open`, `run_log_streams_refused_total`.

Alerts in `infra/observability/alerts.yaml`, with entries in `docs/runbooks/alerts.md`: `SagatStepLogsDropping`
(the server-side share of dropped records above 1% for ten minutes), `SagatStepLogIngestShedding`
(`retry_later` sustained), `SagatStepLogPurgeBehind`. The ingest has a span, `worker.append_step_logs`, with
the counts and the verdict as attributes and no text. Typed errors follow the catalogue (`code`, `retryable`,
`traceId`). A new runbook, `docs/runbooks/step-logs.md`, covers the ceilings, the switch, the purge, what each
marker means and what to do when a customer reports a secret in a log (delete the stream's lines, rotate the
secret).

### 10. Rollout and rollback

The migration is additive. A server setting, `SAGAT_STEP_LOGS_ENABLED` (default true), makes the policy answer
`enabled = false` and the RPC answer `STOP_CAPTURE` without touching the database; it is the switch for an
incident. Old and new binaries run together: an old API replica refuses the unknown RPC as unimplemented, which
an SDK treats as `STOP_CAPTURE` for the session. Rolling back the binary leaves the tables unread; rolling back
the migration needs the lines purged first.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| A step's code logs a secret the platform cannot recognise (transformed, short, obtained elsewhere) | Stated as G041-001 and accepted by the owner as the limit (O041-011, OD-2026-11-13-14). Value redaction in the SDK, shape rules on the server, a permission of its own, the namespace switch, a runbook step to delete a stream's lines |
| Log batches contend with completions on `tenant_retained_usage` | Measured by T041-9 against SC-041-007 before the SDKs are built; the fallback is the grouped commit, then a separate usage row with an amendment of ADR-0027 |
| Each logging step adds one transaction to the database beside its lease and completion | The ingest semaphore bounds it and sheds; a step that logs nothing sends nothing; the same measurement decides whether the grouped commit is needed |
| The final flush delays the completion of every step that logs | One call, bounded at 2 seconds, skipped when the buffer is empty; its added time is measured in T041-9 and stated |
| A long write transaction holds the tail back | Lines arrive late, never out of order or missing; the response states the delay; tested with a held transaction |
| A flood fills the table before the ceilings act | The ceilings are checked before the insert, in the same transaction, from counters, not by counting rows |
| The missing foreign key leaves log rows of a deleted run | Nothing deletes a run today; G041-004 binds feature 020; a test asserts that every stream's run exists |
| A reader takes the worker's time as the order of events | The order is the stored order; worker time is labelled as the worker's and flagged when it differs by more than five minutes |
| Text search scans a run's lines | Bounded by the run ceiling and the statement timeout; no index is promised |
| Six SDKs drift in what they capture | One scenario set and goldens; the rules are in `sdks/SPEC.md`, not in each SDK's notes |
| The Studio renders log text as markup | Rendered as text; a test with markup and escape sequences in a line |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | A log is not execution state: no transition reads it and the lease, heartbeat and completion transactions do not touch it (FR-041-008, SC-041-001). The lease and fencing token decide whether a batch is accepted; a stale attempt is refused, never merged (Ingest, step 1). The journal and its sequence are unchanged |
| II. Modular domain, ports and adapters | Pass | Domain types in `crates/domain/src/step_logs.rs` with no wire or SQL import; ports `StepLogStore` and `StepLogReader` in the application crate; adapters in `infra-postgres`, `api-grpc`, `api-http`. No DTO without semantics: the wire record maps to the domain record at the gRPC boundary only. `scripts/check-architecture.py` runs on every slice |
| III. External effects are at-least-once; sagas compensate | Pass | Delivery of a batch is at-least-once and made idempotent by (attempt, generation, sequence); nothing here is an external effect or changes how one is retried, and a compensation step logs as any step does |
| IV. Compatibility is versioned and pinned | Pass | Additive minor 1.7 negotiated as ADR-0015 says; older workers and servers behave as today (FR-041-014); three new OpenAPI operations, no changed one; no IR change; the six SDKs pass the same five scenarios (SC-041-005) |
| V. Code, tenant boundary and secrets are isolated | Pass with conditions | Tenant in every key and predicate, 404 and audit for another tenant's run, a permission of its own, reads audited. Secrets: the platform's own paths never write a secret value to a log line, and values handed to an attempt are removed before the line leaves the worker. The condition is G041-001: a value the step's code transforms cannot be recognised; the owner accepted that limit (O041-011, OD-2026-11-13-14). See Complexity Tracking and ADR-0085 |
| VI. Observability and audit are part of the feature | Pass | FR-041-018: metrics with closed labels, a span without text, typed errors, alerts, `run.logs_read`; no tenant or run identifier in a metric label |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | No broker is involved: a batch goes worker to API to PostgreSQL and is safe to deliver twice. Backpressure is designed (bounded buffer, `RETRY_LATER`, counted drops). There is no dead-letter path because a dropped line is counted and marked, not parked; that is a stated property of a lossy diagnostic record (ADR-0084), not an omission |
| VIII. Failure is tested and contracts are verified, honestly | Pass | The Verification Plan names the crash, duplicate, lost-batch, stale-lease, flood, old-worker and restart tests, with barriers and held locks, each seen failing first. What is not proven is listed as gaps G041-001 to G041-006 |
| Delivery Constraints | Pass | Rust, tonic, Axum, SQLx, PostgreSQL as the only store; no new component, broker or region; SC-041-007 is a criterion on a stated machine, not a promise |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| Condition on principle V: free text written by customer code is stored and may hold a transformed secret | The feature is the storage of that text; the server never holds the secret values and so cannot match them | Re-resolving the attempt's secrets on the server for every batch to redact there: a secret read per batch, each a disclosure that principle V requires to be audited, on a path that must stay cheap |
| Tables with no foreign key to `runs` | A foreign key makes each insert take a lock that conflicts with the engine's run lock | Keeping the convention: it would put log ingest in the lock queue of leases and completions |
| A new RPC and protocol minor | The log batch must not share the fate of the heartbeat or the cost of the completion | A field on `Heartbeat` or `CompleteStep` |
| A cursor made of transaction id and row id, read below the snapshot horizon | A tail over rows written by concurrent transactions otherwise skips a row that commits late | A per-run sequence taken under the run lock: it would put ingest on the hot path. A cursor per attempt: its size grows with the number of iterations |
| A second server-sent event route | The graph stream has one resume point and a wider audience | Putting log events on `/runs/{runId}/stream` |
| A stream row per attempt beside the lines | Ceilings from counters and not from counting rows; the "expired" and "no final flush" states; the attempt list for the Studio | Deriving all of it from the lines on every batch and every read |
| A permission (the 54th) | A log can hold personal data and viewers hold `run:read` | Reusing `run:read` |
| Capture code in six SDKs | Only the worker process holds the secret values and the logger | Capturing the worker's stdout from outside: there is no agent beside a customer's worker, and no redaction by value |

## Alternatives Considered

- **Chunk rows in PostgreSQL** (one row per batch, compressed). About a tenth of the rows, index entries and
  trigger calls. Filters, search and the per-line identity move into the application, and duplicate removal
  depends on a resend being byte-identical. Not chosen for 0.3.0 (O041-004); the next step if the measurement
  asks for it.
- **The blob store** (ADR-0023) with an index in PostgreSQL. Cheapest per byte and the natural home for very
  large logs. Today committed blobs are never purged (G69), so retention would have to be built there first,
  and the tail and the filters would read objects. Not for 0.3.0.
- **A partitioned table dropped by time.** Purge would be a metadata operation. It does not release quota
  bytes (the triggers of ADR-0027 fire on row deletes), it does not fit a retention that differs per tenant,
  and it would be the first partitioned table of the database, with its own maintenance job, grants and
  privilege audit.
- **Log lines as journal events.** The journal is the authority for state, append-only, never purged, written
  under the run lock and read by every projection. Log volume there would slow replay and the hot path and
  could never be deleted.
- **Sending logs to the OpenTelemetry Collector** and reading them from a log backend. The platform would
  depend on a store it does not own for a route it serves, with no tenant boundary, ceiling or retention it
  can enforce. Export is a later, separate feature.
- **A grouped commit on the server** (several requests' batches in one transaction, each answered after it).
  Fewer transactions and one update of the tenant usage row per group. It adds a queue and a failure that
  touches several requests at once. Held back until T041-9 shows it is needed; it changes no contract.
- **Keeping the last lines at the ceiling** (a ring). Not chosen: O041-007.
- **Capture off unless a namespace asks.** Not chosen: O041-001.
- **Accepting batches after the lease has ended**, with a log capability that outlives the lease. It would
  save the tail of an attempt whose lease expired. It needs a second token with its own lifetime and
  revocation; the loss it avoids is at most one flush interval and is shown as "ended without its final
  flush" (G041-002).

## Non-Functional Requirements

- **NFR-041-PERF** SC-041-001 by test on every pull request that touches the hot-path files (the selector rule
  of design 038 already picks the budget suite); SC-041-007 by the benchmark, with the machine, its load and
  the PostgreSQL configuration stated, appended as a dated section to
  `specs/041-step-logs/validation-report.md`. The ingest's own statement count is budgeted as an equality.
- **NFR-041-SEC** No grant beyond those of the Technical Plan, no `SECURITY DEFINER`, no role membership. The
  cross-tenant battery covers the RPC and the three routes. A canary test (SC-041-004) runs in the conformance
  suite and in the server's integration suite. Line text never reaches a metric, span, audit record, error or
  the platform's log; a test greps the captured output of the suites for the canary.
- **NFR-041-OPS** Additive migration; `SAGAT_STEP_LOGS_ENABLED` as the incident switch; old and new replicas
  and workers run together; the runbook and the alerts ship with the code.
- **NFR-041-A11Y** WCAG 2.2 AA as the Studio's existing axe run checks it, reflow at 320 px, keyboard-only
  journey, level by text and shape, no focus theft, announcements throttled (SC-041-006).
- **NFR-041-COST** The stored size of a line is its message and fields plus a fixed overhead that T041-9
  measures and the runbook states, so an operator can turn `tenant_log_bytes` into a disk figure.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Six slices; each task is one reviewable pull request unless it says otherwise, and each slice leaves `develop`
releasable. A task that builds what an owner answer decided carries an `Owner-gate:` line citing
OD-2026-11-13-14.

Slice A, decisions and contracts (nothing runs yet):

- [ ] T041-1 Take this design, with the owner's answers of OD-2026-11-13-14 written in, through `egm-review`
  and the three repository reviewers, and commit their reports.
  Refs: FR-041-001, FR-041-013
- [ ] T041-2 Bring ADR-0084 (step logs are a bounded, lossy, per-attempt record beside the journal) and
  ADR-0085 (step-log text is read under its own permission, redacted by value in the worker and by shape on
  the server), written as Proposed with this design, to the owner's acceptance, and correct them for what the
  reviews find.
  Refs: FR-041-007, FR-041-008
  Owner-gate: OD-2026-11-13-14
- [ ] T041-3 Add `AppendStepLogs`, its messages and `StepLogPolicy` to `worker.proto` and its six copies,
  update `worker.baseline.json`, and add the three operations to `openapi.yaml` tagged `x-implemented: '041'`
  with the digest test's list extended; regenerate the Studio types. Contract checks pass; nothing is served.
  Refs: FR-041-004, FR-041-010, FR-041-011, FR-041-012, FR-041-014

Slice B, the server stores:

- [ ] T041-4 Domain types and validation (`crates/domain/src/step_logs.rs`), the quota resources
  (`step_log_retention_days`, the attempt and run ceilings, the capture switch) at the scope the catalogue
  allows, and the `run:logs:read` permission with the role matrix test.
  Refs: FR-041-003, FR-041-006, FR-041-013, FR-041-020
  Owner-gate: OD-2026-11-13-14
- [ ] T041-5 The migration (three tables, indexes, the no-update trigger, grants, storage classification, a
  down that refuses data loss), following `.claude/skills/migration`; the privilege audit and the manifest test
  pass; a test asserts no foreign key from the log tables to `runs` or `steps`.
  Refs: FR-041-008
  Owner-gate: OD-2026-11-13-14
- [ ] T041-6 `StepLogStore` and its PostgreSQL adapter: the ingest transaction with the lock-free lease read,
  stream upsert, the three ceilings, markers, duplicate removal and the shape scrub; its statement budget as an
  equality; the held-run-lock test of SC-041-001; `work_budget.rs` unmodified.
  Refs: FR-041-002, FR-041-004, FR-041-006, FR-041-007, FR-041-008, SC-041-001
  Owner-gate: OD-2026-11-13-14
- [ ] T041-7 The gRPC handler, minor 1.7 in `negotiate_protocol_details` and `protocol_ceiling`, the policy in
  `RegisterWorkerResponse`, the ingest semaphore and `RETRY_LATER`, `SAGAT_STEP_LOGS_ENABLED`, the metrics and
  the span.
  Refs: FR-041-001, FR-041-004, FR-041-005, FR-041-014, FR-041-018
  Owner-gate: OD-2026-11-13-14
- [ ] T041-8 Server fault tests with barriers and held locks: crash before commit, crash after commit and
  before the answer, the same batch twice at once, a stale lease, a batch for another tenant's run, a flood
  from a hand-written client that ignores every verdict, restart of the API in the middle of an attempt.
  Refs: SC-041-002, SC-041-003, NFR-041-SEC
- [ ] T041-9 Measure: the transitions scenario with and without ten lines per step, the added time of the
  final flush, the stored overhead per line; write the dated section of the validation report. If SC-041-007
  fails, stop and add the grouped commit as a follow-up task before slice D.
  Refs: SC-041-007, NFR-041-PERF, NFR-041-COST

Slice C, the server serves:

- [ ] T041-10 Retention: `expires_at`, the housekeeping pass, the kept stream row with `purged_at`, the purge
  metrics; a test that a purge releases the quota bytes and skips a leased attempt.
  Refs: FR-041-009, FR-041-018
  Owner-gate: OD-2026-11-13-14
- [ ] T041-11 `StepLogReader`, `GET /runs/{runId}/logs` and `/logs/streams`: order and cursor below the
  snapshot horizon (with the held-transaction test on PostgreSQL 16 and 18), filters, `q`, the throttled
  `run.logs_read` audit, the cross-tenant battery, the runtime-role run.
  Refs: FR-041-010, FR-041-011, FR-041-013, FR-041-002
  Owner-gate: OD-2026-11-13-14
- [ ] T041-12 `GET /runs/{runId}/logs/stream`: events, resume, end, reconnect, caps shared with the graph
  stream, role re-check; a test that a reconnect neither skips nor repeats.
  Refs: FR-041-012
  Owner-gate: OD-2026-11-13-14
- [ ] T041-13 The script-output marker in `record_execution` and the `script` source in the streams route.
  Refs: FR-041-016
  Owner-gate: OD-2026-11-13-14

Slice D, the SDKs (one pull request per SDK after the specification; they can be built in parallel):

- [ ] T041-14 `sdks/SPEC.md`: the logger's semantics, the capture section, 1.7 in §9; the five scenarios and
  their goldens; the harness step that reads the logs route and the fault hook for the RPC. Scenarios fail in
  all six for the right reason.
  Refs: FR-041-015, SC-041-005
- [ ] T041-15 TypeScript: capture behind `ActivityLogger`, redaction by value, buffer, sender, final flush.
  Refs: FR-041-005, FR-041-007, FR-041-015, SC-041-004
- [ ] T041-16 Go: capture as a `slog.Handler`.
  Refs: FR-041-005, FR-041-007, FR-041-015, SC-041-004
- [ ] T041-17 Python: capture as a `logging.Handler`.
  Refs: FR-041-005, FR-041-007, FR-041-015, SC-041-004
- [ ] T041-18 C#: capture as an `ILogger` decorator.
  Refs: FR-041-005, FR-041-007, FR-041-015, SC-041-004
- [ ] T041-19 Java: capture as a decorator of `ActivityLogger`.
  Refs: FR-041-005, FR-041-007, FR-041-015, SC-041-004
- [ ] T041-20 Rust: add `ActivityContext::log()` and capture; the four lock files move together.
  Refs: FR-041-005, FR-041-007, FR-041-015, SC-041-004
- [ ] T041-21 Run the five scenarios in the six SDKs on the merged tree and record the matrix; the flood
  scenario's counts add up in each.
  Refs: SC-041-002, SC-041-004, SC-041-005
  Owner-gate: OD-2026-11-13-14

Slice E, the Studio:

- [ ] T041-22 `LogView`, the Logs tab and the step section over the list and streams routes: filters, search,
  states, markers, windowing, text-only rendering; unit tests.
  Refs: FR-041-017, FR-041-019
- [ ] T041-23 The tail (`followLogs`, pause and resume, announcements, polling fallback), with a test that a
  dropped connection neither skips nor repeats.
  Refs: FR-041-017, FR-041-012
- [ ] T041-24 Accessibility and journey: the axe run and reflow in `a11y.spec.ts`, a keyboard-only journey, and
  the Rust-driven browser journey for CUJ 1, CUJ 2 and CUJ 5.
  Refs: SC-041-006, NFR-041-A11Y

Slice F, operations and close:

- [ ] T041-25 Alerts with their runbook entries, `docs/runbooks/step-logs.md`, the reference documentation
  (generated files regenerated after reading the diff), `docs/traceability.md`, the quickstart demo of the
  happy and the failure scenario, and the release notes that state O041-001's default first.
  Refs: FR-041-018, NFR-041-OPS
  Owner-gate: OD-2026-11-13-14
- [ ] T041-26 End-to-end lineage and mixed versions on the Compose stack: retry, reset, rerun and
  continue-as-new each leave logs where FR-041-019 says; a 1.5 worker beside a 1.7 worker; an old API replica
  beside a new one; the incident switch.
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
`integration_step_logs`, with real PostgreSQL. Each test below is written first and seen failing for the
stated reason; races use barriers or held locks, never sleeps.

| What is proven | How |
|---|---|
| The hot path is unchanged (SC-041-001) | `work_budget.rs` passes with its constants unmodified while the worker logs; a transaction holds the run row `FOR UPDATE` and a batch for that run commits; `pg_locks` shows no lock of the ingest on `runs`, `steps` or `run_events` |
| Crash between accept and store | A fault point before the commit: nothing stored, the resend stores all, once |
| Crash between store and answer | A fault point after the commit: the resend stores nothing and gets the same answer |
| Duplicate batch | The same batch sent twice at a barrier: the second waits on the stream row and inserts nothing; counters rise once |
| Lost batch | The SDK gives up on a batch: the next batch's marker carries the count; the reader sees the marker between the neighbours |
| A worker that floods (SC-041-002) | Conformance `step-logs-limits` for the SDKs; a hand-written client that ignores verdicts for the server: stored bytes stay at the ceilings, the semaphore sheds, lease and completion latency of another tenant's run is measured during the flood and stated |
| An old worker, an old server | `step-logs-negotiation`; an API replica without the RPC beside one with it |
| Redelivery does not duplicate lines | A step's task delivered again under a new generation writes a new stream; the same generation's resend inserts nothing; rebuilding the projections of a run changes no log row |
| A stale attempt cannot write | A batch with the token of a superseded lease is answered `STOP_CAPTURE` and stores nothing, also while the new attempt is logging |
| Reset, rerun, continue-as-new, retry | T041-26: the new run's log is empty at start, the old run's is intact, a retry has its own stream |
| Tail order | Transaction A stays open, transaction B stores a line and commits: the line is not delivered until A ends, then both arrive in cursor order; a reconnect at any point neither skips nor repeats |
| Redaction (SC-041-004) | `step-logs-redaction` in six SDKs; server tests for each shape rule and sensitive field name; the canary is searched for in the stored rows, the routes' bodies, `run_events`, telemetry, `audit_records` and the captured process output |
| Tenant boundary | The cross-tenant battery on the RPC (a lease of tenant A naming a run of tenant B) and on the three routes: 404, `access.denied.cross_tenant`, no count or size disclosed |
| Permission and audit | A viewer is refused on the three routes; the first read in a window writes `run.logs_read` before returning and a failure to write it returns no line |
| Retention | A stream past expiry is purged in batches, the quota bytes return to the tenant, the stream row stays with `purged_at`, a leased attempt is skipped |
| Runtime privileges and storage classification | The existing privilege audit and manifest tests, unmodified, pass with the new statements and columns |
| Studio (SC-041-006) | Unit tests of the states and markers; the axe run; the keyboard journey; a line holding markup and escape sequences is shown as text |

What this plan does not prove is registered as gaps in `specs/041-step-logs/research.md`: G041-001 (a
transformed secret), G041-002 (the tail of an attempt that lost its lease or its network), G041-003 (script
lines are not live and not interleaved), G041-004 (feature 020 must delete streams with their run), G041-005
(a deployment held below protocol 1.6 captures nothing), G041-006 (SC-041-007 is measured on one machine).

### Estimate

This is an estimate by reading, not a measurement. Server (domain, application, adapter, gRPC, HTTP, purge,
tests): about 4,000 to 5,000 lines of Rust, most of them tests, in nine pull requests. SDKs: 400 to 700 lines
each with their unit tests, plus the specification, five scenarios and the harness change, in seven pull
requests. Studio: about 1,200 to 1,500 lines with tests, in three. Contracts, ADRs, runbook and documents: four.
That is 27 tasks and roughly 23 to 25 pull requests. A pull request's CI takes up to two hours and they land
one at a time where they touch `research.md` or the contracts, so the calendar is set by the queue more than by
the writing: about three working weeks for one session working alone, less with the six SDK tasks run in
parallel after T041-14. The two places most likely to move the estimate are T041-9 (if the grouped commit is
needed, add a week) and T041-11's cursor on two PostgreSQL versions.
