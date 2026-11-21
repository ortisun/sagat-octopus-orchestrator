# Step logs per run

*   **Author:** Claude Code, for the owner
*   **Status:** Approved
*   **Date:** 2026-11-21
*   **Reviewed-commit:** 543417134ff7ebda70a96690329bb95ddde55c8c
*   **EGM-hash:** 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
*   **Claude-Code:** 2.1.329
*   **Reports:** docs/designs/041-step-logs.reviews/2026-11-20-approval/final/egm-review.md sha256:3d374d4b722d069716796572818b5c11a29d48b1b9e16a470ad631cb84f562c3; docs/designs/041-step-logs.reviews/2026-11-20-approval/final/architecture.md sha256:d936baee5e664b7430a7744e3b7724bd49e417fd8e88c74803fa26f3c6d3450a; docs/designs/041-step-logs.reviews/2026-11-20-approval/final/security.md sha256:d6c213dfe43eb74eb0e9180f44a802233b2e04025b18d560651e0308f5a2389e; docs/designs/041-step-logs.reviews/2026-11-20-approval/final/correctness.md sha256:ab4528184ddef0f1142e1613529416c66b9f34d6c332ed1a4a78e379b3a88b00
*   **Open findings:** Blocker 0, Major 0
*   **Accepted:** 6; OD-2026-11-21-200, OD-2026-11-21-201, OD-2026-11-21-202, OD-2026-11-21-203, OD-2026-11-21-204, OD-2026-11-21-205
*   **Owner-acceptance:** OD-2026-11-15-17 (the design, for 0.3.0); OD-2026-11-16-7 (the approval path); OD-2026-11-19-1 (the direction of the redesign); OD-2026-11-20-1 (the carry in one round trip and the p99 allowance); OD-2026-11-20-13 to OD-2026-11-20-16 (the fourth carry spike's cost, the busy loss, ADR-0095 and ADR-0097, the amendment of ADR-0027); OD-2026-11-20-17 (the approval path of this revision); OD-2026-11-20-18 (the top-up after the commit); OD-2026-11-20-19 (the step-log usage hidden without the PII grant)

--------------------------------------------------------------------------------

## Overview

The lines a step's code writes to its logger are sent by the worker to the platform, stored per attempt with
ceilings and a retention period, and read through a REST route, a live tail and a Studio screen. The owner chose this
on 2026-11-13 (OD-2026-11-13-9: "step logs per run in 0.3.0 — a new feature: workers send the log lines of each step,
the platform keeps them with retention and limits, and there is a route and a Studio screen").

The design treats a step log as a bounded diagnostic record kept **beside** the journal. It is never execution state:
no transition reads it, a lost line never changes a step's outcome, and the transaction that leases a step runs no
statement on it. The last records of an attempt ride on its report (`CompleteStep` or `FailStep`) and are written by the
report's own transaction, which they add one statement to (one call of a database function, OD-2026-11-20-1) and no
transaction; when the carry makes that transaction fail, the report is recorded by a downgrade re-run (section 2a). A long step also streams its earlier records
through an RPC of their own, `AppendStepLogs`. A step log MUST NOT be a second place where another tenant's data can be
read, or where a secret can be read beyond the limit of G041-001 (a value the code transformed, a short one, or one
obtained outside the platform). Log writing can delay a report of the same tenant only by the waits listed in section 2,
"What a report can wait for". A carry takes its step-log locks only when they are free and otherwise skips itself; its
rows are charged to the step-log *quota lease* of the server backend that writes them (its *writer*), never to the
tenant's usage row (ADR-0097); and only when that quota lease is short does the carry wait, under the run lock, for the
usage row, in a *fallback refill* bounded per lock acquisition by 200 ms and by the wait slots of the tenant and of the
replica (section 2b).
After a carrying report commits, the store tops up that quota lease on the same connection when the carry left it under
64 KiB, off the run lock and without delaying the report's answer (the *top-up*, OD-2026-11-20-18).

**Revision of 2026-11-21, after the second and third rounds of the approval run (Draft).** The second round's short
re-reads (at `4d6fe4cc`) led to decisions D041-134 (wait slots across tenants; a top-up that never waits) and D041-135
(SC-041-007 judged on the median of all valid rounds). The third round's (at `c52b5d37`: 1 Blocker, 17 Majors) led to
the owner's answers OD-2026-11-21-1 (the 8 × 128 gate (2b) on a floor derived as at 1 × 16), OD-2026-11-21-2 (ADR-0095's
re-run tier kept) and OD-2026-11-21-3 (the two fault-injection regimes excluded from the busy rule), and to decisions
D041-136 (at least seven valid rounds and the false-stop rates) and D041-137 (the owned connection, the wait slots passed
per transaction, the pooler's sizing rule, the top-up's answers); `resolution.md` of the approval folder answers each
finding. Changed: this Overview, SC-041-007, the lock order (L4), the batch's T1, the script marker's transaction,
sections 2a and 2b, the Risks, tasks T041-5, T041-9, T041-36 and T041-37, and the Verification Plan.

**Revision of 2026-11-20, after the first round of the approval run (Draft).** The first round
(`041-step-logs.reviews/2026-11-20-approval/`, OD-2026-11-20-17) found 3 Blockers, 68 Majors and 86 Nits. The owner
answered two (OD-2026-11-20-18: the ensure before the report is replaced by a top-up after the commit, so that the store
needs no backend identity; OD-2026-11-20-19: the step-log usage is hidden from a caller without the PII grant), and the
coordinating session decided the others under OD-2026-11-20-6 (decisions D041-124 to D041-133); `resolution.md` of
that folder answers each finding. Changed: the header, this Overview, FR-041-005, -006, -008, -011 and -017, SC-041-001
and SC-041-007 (a criterion per size), the Goals, the Open Questions, the owner decisions, the Terms, the diagram,
sections 1, 2, 2a, 2b, 3, 5, 8 and 9, the Risks, the Constitution Check, Complexity Tracking, the Alternatives, the NFRs,
the plan (T041-1, T041-3 to T041-10, T041-14, T041-22, T041-25, T041-31 withdrawn, T041-32, T041-35 and T041-36; T041-37
new), the Verification Plan and the Estimate.

**Revision of 2026-11-20, after the owner's answers to the fourth run (Draft).** The owner settled SC-041-007 by the
measured cost of task T041-35's fourth run (OD-2026-11-20-13), accepted the busy loss past the budget as built
(OD-2026-11-20-14), accepted ADR-0095 and ADR-0097 (OD-2026-11-20-15) and amended ADR-0027 (OD-2026-11-20-16); decision
D041-123 records the consequences. Changed: the owner-acceptance line, this Overview's references, the Open Questions,
the owner decisions, SC-041-007, the waits of tasks T041-3, T041-5, T041-35 and T041-36, and, in section 2b, the
citations of those decisions (the acceptance of ADR-0097 and the busy loss as built).

**Revision of 2026-11-20, after task T041-35's third run (Draft): writer leases.** The third run was not decided:
its carries were answered mark-only while the tenant had room (decision D041-116). Two diagnoses split them by cause:
every one was a refill that met the busy usage row, and the shared pool of leases collided (validation-report.md,
"split by cause"). Decisions D041-117 to D041-120 give each server backend its own lease, make its room before the
report in a transaction of its own, bound the fallback refill inside a report by a 200 ms budget past which the carry
answers mark-only with a reason the reader sees, and declare task T041-35's fourth run (section 2b; ADR-0097; the
reviews in `041-step-logs.reviews/2026-11-20-writer-leases/`).

**Revision of 2026-11-20, after T041-35's second run (Draft).** The second run failed at 1 × 16 (D041-105). Two
diagnoses on the hosted runners followed: the commits did wait on the tenant's usage row, but that queue was not what
failed the gates (D041-106); the carry's cost lies in its stream row and, less, its line rows (D041-107). The revision
charges the step-log class to exact per-tenant leases
([ADR-0097](../adr/ADR-0097-the-step-log-quota-is-charged-to-exact-per-tenant-leases-never-to-the-usage-row-at-commit.md),
then Proposed, accepted on 2026-11-20; D041-108; section 2b), writes a short step's new stream once and makes a later stream update HOT (D041-111;
sections 2a and 3), and keeps T041-35's environment, gates and rule unchanged (D041-109). Changed: this Overview, FR-041-008,
SC-041-001, sections 2, 2a, 2b (new), 3 and 4, the Risks, the Alternatives and T041-5, T041-35 and T041-36. The reviews of
this revision are in `041-step-logs.reviews/2026-11-20-hot-row/`.

**Revision of 2026-11-19 (Draft).** The design was approved on 2026-11-16 (commit `f9d4a65f`; the four reports of
`041-step-logs.reviews/2026-11-16/`). Its spike before the schema, T041-31, failed SC-041-007: with an `AppendStepLogs`
transaction per batch and a final batch before every report, the median rate with capture on was 79.2% (1 replica, 16
workers) and 87.0% (8 replicas, 128 workers) of the rate with capture off on a quiet machine, against 90%
(`specs/041-step-logs/validation-report.md`). The owner chose to remove the per-step transaction ("Redesenhar
(CompleteStep)", "redesign (CompleteStep)", OD-2026-11-19-1, O041-036 option (d)): the last records travel on the report,
so a short step needs no transaction of its own. The change is
[ADR-0095](../adr/ADR-0095-the-last-step-log-records-of-an-attempt-ride-on-its-report.md) (then Proposed, accepted on
2026-11-20; O041-038). The
revision changes the body of an approved doc, so by the approval rule of `docs/designs/constitution-4.0.0-egm.md`
(section 4, "Amending an approved doc") the design is `Draft` until it is reviewed again. Changed: the Overview,
FR-041-004/005/008/009/014/015/018/020, SC-041-001/003, the Goals, the Open Questions, the owner decisions, the Terms,
the diagram, sections 1, 2, 2a (new), 3, 4, 5, 6, 9, 10, the Risks, the Constitution Check, Complexity Tracking, the
Alternatives, the NFRs, the plan (T041-3 to T041-9, T041-14, T041-32, T041-33; T041-35 and T041-36 new; T041-34
withdrawn), the Verification Plan and the Estimate. The reviews of the revision, dated by their folder, are in
`041-step-logs.reviews/2026-11-20-redesign/`. A new spike, T041-35, with its acceptance rule set in its task before its
first round, replaces T041-31 as the gate before the schema.

The owner's acceptance of the design for release 0.3.0 (OD-2026-11-15-17) decides its scope, in every revision. The
review findings of the first approval, of 2026-11-16, were either fixed and re-read by the three repository reviewers or
accepted by the owner, one row each, on the approval path the owner chose then ("Corrigir e aceitar o resto",
OD-2026-11-16-7); the revisions of 2026-11-19 and 2026-11-20 are approved on the path of OD-2026-11-20-17.

The decisions, gaps and owner questions are registered in [specs/041-step-logs/research.md](../../specs/041-step-logs/research.md);
the contract this design freezes is [specs/041-step-logs/contracts.md](../../specs/041-step-logs/contracts.md) and the
schema detail is [specs/041-step-logs/data-model.md](../../specs/041-step-logs/data-model.md). The architecture decisions
are [ADR-0084](../adr/ADR-0084-step-logs-are-a-bounded-lossy-per-attempt-record-beside-the-journal.md),
[ADR-0085](../adr/ADR-0085-step-log-text-is-read-under-its-own-permission-and-redacted-by-value-in-the-worker.md),
[ADR-0095](../adr/ADR-0095-the-last-step-log-records-of-an-attempt-ride-on-its-report.md) and
[ADR-0097](../adr/ADR-0097-the-step-log-quota-is-charged-to-exact-per-tenant-leases-never-to-the-usage-row-at-commit.md),
both accepted on 2026-11-20 (OD-2026-11-20-15; decision 3b of ADR-0097 amended by OD-2026-11-20-18), and the
amendments of [ADR-0027](../adr/ADR-0027-durable-content-rate-and-retention-quotas.md) of 2026-11-15 and of 2026-11-20
(its "Amendment of the Amendment", OD-2026-11-20-16). How
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
  store nothing and be answered `STOP_ATTEMPT`: `LEASE` when it read the task lease after the report committed, and
  `FINAL_RECORDED` when it read the task lease before.
- **FR-041-005 Logging never decides the step.** A call to the logger MUST NOT block on the network or on a full buffer,
  and MUST NOT throw into the activity. No failure of capture (refused batch, timeout, server down, ceiling reached,
  invalid line, a carry the server cannot store, bug in the capture path) may change the outcome, the output or the retry
  of a step. Nor may capture change a step's timing beyond the waits that section 2, "What a report can wait for", lists:
  the bounded wait of FR-041-015 before the report, the carry's own call, its fallback refill when its writer's quota
  lease is short, and, when the carry makes the report's transaction fail, its downgrade re-run (section 2a). The
  fallback refill runs after the report's fence checks, under its run lock, so it can delay the report's commit but never
  makes the report late (refused because its task lease expired before its checks ran: the checks have already passed,
  and an expired task lease cannot be reclaimed while the report holds the run row); the top-up runs after the commit
  (OD-2026-11-20-18).
  The residual (G041-021): the re-run is fenced as any report, so it is refused when, after the first transaction
  failed, the task lease expired or was reclaimed, a cancel was requested, the run began to stop (for a forward step) or
  finished, or the worker's binding or membership was revoked.
- **FR-041-006 Ceilings and drop counts.** The server enforces these from counters, regardless of what the worker sends:
  - per line: 8 KiB per message and per exception text, 2 KiB and 32 keys per field map, measured after redaction; a
    message or exception over its ceiling is cut at a character boundary and the line is flagged `truncated`;
  - per attempt: the attempt ceilings of FR-041-020 (defaults 10,000 records and 1 MiB of stored text);
  - per tenant: the step-log quota `tenant_step_log_bytes`, a resource of its own (OD-2026-11-15-5).
  There is no run ceiling: the attempt ceilings and the tenant quota bound what a run stores (OD-2026-11-16-10).
  Records past a ceiling are not stored and are counted on the attempt's stream. When the attempt ceiling is first met
  the server stores one marker line, which holds no text; the tenant quota is recorded only on the stream row. Two more
  drops of a carry's records are permitted, each counted and shown on its stream: when the tenant's last room is held by
  the quota leases of open writers (G041-023), recorded as the tenant quota; and when the quota cannot be checked within
  its budget, or with no wait slot free, recorded as not stored because the log service was busy, a reason of its own
  that is not a ceiling (G041-026, accepted by the owner, OD-2026-11-20-14; decision D041-129). Every
  sequence the worker numbered and the server never stored is counted once, as dropped by the worker (a gap in the
  sequences it sent) or by the server (a ceiling, the busy service or a failed carry, including the records the worker
  stopped sending after a ceiling). A step MUST NOT fail, wait or retry because a ceiling was met or a record dropped.
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
  (one method, one call of the function `sagat_step_log_carry`, section 2a), at the cost section 2a states as an
  equality. A batch or a carry charges its rows to its writer's own step-log quota lease (ADR-0097) and takes the
  tenant's `tenant_retained_usage` row only to refill that quota lease (section 2b); after a carrying report commits, the
  store may top that quota lease up in a statement of its own (section 2b, OD-2026-11-20-18).
  The attempt ceilings count text bytes (the UTF-8 octets of message, exception and fields); the bytes charged to the
  tenant, each row's text plus a fixed 128 bytes (OD-2026-11-15-6), are counted in a storage class of their own,
  `step_log` (ADR-0027's Amendment of 2026-11-15, measured once per insert statement by its Amendment of the Amendment of
  2026-11-20), and released when the rows are deleted.
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
  server, the ceiling met, the reason records were not stored when it is not a ceiling (`notStoredReason`, FR-041-006),
  first and last time, expiry or removal time) and, with `source = script`, the run's script
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
  final flush" (the attempt's task lease is gone and no final mark was stored), "some lines are delayed" (`heldBack`),
  the ceiling met, "not stored: the log service was busy (not your quota)" (FR-041-006) and "log withheld" for a script
  execution, with full keyboard operation.
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
    "The budget's measure changes"), each written as statements, transactions and top-level rows, with the step-log rows
    counted by the second measure section 2a adds to the test:

    | Report | Completion with a next step (statements, transactions, rows) | Completion that ends the run (statements, transactions, rows) | Step-log rows |
    |---|---|---|---|
    | carries ten records, all kept | 51, 1, 11 | 48, 1, 9 | 1 stream, 10 lines |
    | holds a mark-only carry | 51, 1, 11 | 48, 1, 9 | 1 stream, 0 lines |
    | carries records, capture off in the namespace | 51, 1, 11 | 48, 1, 9 | none |
  - A batch for a run commits, under `lock_timeout = 50ms`, while another transaction holds that run's row `FOR UPDATE`
    and has updated the leased `steps` row.
  - One ingest of tenant T is held after T8 (its charge holds its writer's quota lease): three more ingests of T, each on
    a backend of its own, commit while it is held, each charged to its own quota lease, none taking T's usage row
    (`pg_stat_activity` shows none of them waiting; ADR-0097; task T041-6), and so does a carrying report of T (task
    T041-36). One transaction that holds T's usage row (a refill held open) delays only a charge whose writer's quota
    lease is short, and a completion of T with output at its commit, as today.
  - The quota leases keep the quota exact: under concurrent charges, refills, quota reclaims, releases and a lowered
    limit, T's usage never exceeds the limit in force when each charge was admitted and equals what the stored rows
    measure; a charge is refused only when the room left is in quota leases that open transactions hold (G041-023) or
    not there at all (ADR-0097's tests, T041-5).
  - One ingest of the namespace is held after T7: a `lock_secrets` of the namespace and a completion of the namespace
    both commit while it is held, since the ingest takes no secrets lock and no membership lock (OD-2026-11-16-7).
  - A carrying report whose carry's charge is refused at the tenant's step-log limit, or whose fallback refill passes its
    budget or finds no wait slot free, is answered in the same call by a mark-only final mark that says why, and the
    report commits in its first transaction with the same outcome and output as without a carry; only a carry call that
    fails (an error the function's block does not catch) leads to a downgrade re-run (section 2a).
  - A carrying report never waits for a step-log lock: with a change of a step-log resource holding or queued for its
    fence, or an ingest of the same attempt held after T3, the carry writes nothing more (`contended`) and the report
    commits in its first transaction.
  - The effect of these waits on time is what SC-041-007 bounds, per size.
- **SC-041-002 A flood.** An activity that writes 1,000,000 lines as fast as it can completes with the same outcome and
  output as one that writes none; the store holds at most the attempt ceiling plus its marker; and for the stream,
  lines stored (the marker excluded) + dropped by the worker + dropped by the server = the highest sequence the logger
  assigned (the drop counts saturate at 2^53, so this holds for highest sequences below it).
- **SC-041-003 Faults.** In the fault tests of the Verification Plan no record is stored twice, and every record of an
  attempt that has a stream and is missing is covered by a counter or by the "ended without its final flush" state. A
  stream whose removal began while its lease lives answers its next batch `STOP_ATTEMPT` (`REMOVED`) and stores
  nothing, and the reader sees it as expired or deleted. An attempt with no stored batch and no stored carry has no
  stream (G041-014): its capture stopped before its first batch, or its carry was lost (an API replica that predates
  1.7, a failed carry statement, a carry ignored by C1 or skipped as `contended`).
- **SC-041-004 Redaction.** A canary secret resolved by an attempt and written to the logger whole, inside a longer
  message, as a field key, as a field value, as a number and inside an exception, by that attempt and by another attempt
  of the same worker process (at the same time and after the first ended), appears nowhere in the stored lines, the
  routes' bodies, the journal, telemetry, audit records or the platform's captured output, in all six SDKs.
- **SC-041-005 Conformance.** The five scenarios of section 6 pass in TypeScript, Go, Python, C#, Java and Rust from
  the same scenario files and goldens.
- **SC-041-006 Accessibility.** The Logs tab and section pass the axe run at WCAG 2.2 AA of
  `frontend/e2e/tests/a11y.spec.ts`, reflow at 320 px, and a journey done with the keyboard only.
- **SC-041-007 Throughput, per size** (decision D041-126). The transitions scenario of
  `tests/performance/scalability-ha` runs, in the rows "1 API replica, 16 workers" (1 × 16) and "8 replicas, 128
  workers" (8 × 128), these workloads, each against its capture-off row:
  - (a) every step writes ten `info` lines and returns a 1 KiB output, its lines carried on its report; off-1K is the
    same with capture off;
  - (f) as (a) with the output `{}`, against off-{};
  - (g) every step sends one `AppendStepLogs` batch of ten lines, waits for its answer, then carries ten more.

  The gates: (1) the rate of (a) over off-1K; (2b) the completion p99 of (a) minus off-1K's, minus the *allowance*, the
  larger of 10% of off-1K's p99 and 2 ms (OD-2026-11-20-1); (3) the rate of (f) over off-{}; (4) the tenant tickets that
  (g) needs on a replica, ⌈rate × S p99 / 0.7⌉ with S the batch's service time, at most 10, with ticket refusals under
  1%, judged at the cap and tickets of 10 that the fourth run measured, since it sizes the need (the shipped defaults,
  7 tickets, are measured by task T041-37). The runs use the validity rule and the hosted runners of task T041-35's
  fourth run (decision D041-102: GitHub-hosted runners of 4 logical cores, saturated by the scenario), and the runners'
  CPUs, their load and the PostgreSQL configuration are stated. Task T041-9 runs twelve rounds, as the fourth run did
  (four runners, three rounds each), and judges each gate on the median of all its valid rounds, at least seven
  (decisions D041-135 and D041-136); the median of an even count is the mean of its two middle values. A run with
  fewer than seven valid rounds is not judged: its twelve rounds are run once more as a new run (validity does not
  depend on the gates, so this is not a re-run of a miss), and a second run with fewer than seven goes to the owner as
  the environment question (decision D041-109).
  - **At 8 × 128:** (1) at least 90%, (3) at least 90% and (4) as above, the original criterion; (2b) at most
    +112.9 ms, a floor derived as at 1 × 16 (the owner's answer "Derived floor, like 1×16", OD-2026-11-21-1, which
    relaxes the allowance of OD-2026-11-20-1 at this size): the median of the three rounds used is −17.0 ms, the worst
    valid round +112.86 ms (round 6), so the tolerance is 129.9 ms and the floor that worst round, rounded in the
    build's favour. Under "at most 0" the eight valid rounds' median, (−2.19 + 6.30) / 2 = +2.06 ms, would fail the
    build the owner accepted, and four of its eight valid rounds are above 0.
  - **At 1 × 16, a floor at the measured cost** the owner accepted ("Accept the measured cost", OD-2026-11-20-13): the
    fourth run's figures minus a tolerance derived from the spread of its eight valid rounds (1, 2, 3, 4, 6, 7, 9 and
    11). The tolerance of each gate is the distance from the median of the three rounds used (1, 2 and 3) to the worst
    valid round, so the floor is that worst round, rounded in the build's favour:
    - (1) at least 80.4%: median 82.0%, worst valid round 80.46% (round 2), tolerance 1.6 points;
    - (2b) at most +29.1 ms: median +8.6 ms, worst valid round +29.03 ms (round 11), tolerance 20.5 ms;
    - (3) at least 83.5%: median 87.1%, worst valid round 83.52% (round 11), tolerance 3.6 points;
    - (4) as above (7 tickets in the rounds used, 10 in the worst valid round).

    The build the fourth run measured meets each floor, at both sizes, in every one of its valid rounds. An unchanged
    build can still fail one by chance. With exchangeable rounds, the median of *m* new valid rounds is beyond the
    worst of the eight old ones only when at least *r* = ⌈*m* / 2⌉ new rounds are, with probability
    C(*m*, *r*) / C(8 + *m*, *r*): exact for odd *m* and an upper bound for even *m* (the mean of the two middle
    values can stay inside). Per gate: 2.56% at *m* = 7 (35/1,365), at most 3.85% at 8 (70/1,820), 2.04% at 9
    (126/6,188), at most 2.94% at 10 (252/8,568), 1.70% at 11 (462/27,132) and at most 2.38% at 12 (924/38,760); so
    at most 3.85% whatever the number of valid rounds. The same bound holds for gate (4) at 1 × 16, whose bar of 10
    tickets is its worst valid round. Gates (1) and (3) at 8 × 128 keep the fixed 90%, which two of their eight valid
    rounds missed ((1) 89.58% and 88.63%, (3) 83.39% and 89.36%); for them no bound holds without a model, and
    resampling the eight valid rounds estimates 7.1% each at *m* = 7, 7.4% and 7.5% at 8 and 4.1% each at 12. Per
    release, the union over the gates is at most 5 × 3.85% + 7.4% + 7.5% = 34.1% at *m* = 8 and
    5 × 2.38% + 4.1% + 4.1% = 20.1% at *m* = 12 (the gates of one round move together, so the true rate is lower);
    gate (4) at 8 × 128, at twice its worst round, adds nothing the resampling can see. A miss stops the release and
    goes to the owner with the record, and is never re-run. The (2b) floors guard against gross regressions only: at
    1 × 16 the floor admits a completion p99 of (a) of about 34.6 + 3.46 + 29.1 = 67.2 ms, about +94% over off-1K's
    34.6 ms, against the +31.9% to +36.9% of the rounds the owner accepted; at 8 × 128 about
    481.5 + 48.15 + 112.9 = 642.6 ms, about +33.4% over off-1K's 481.5 ms, against +4.6% to +9.5%. So task T041-9 also
    records, at each size, the median over the valid rounds of that p99 in percent, and reports it to the owner when
    it exceeds the largest of the rounds used (+36.9% at 1 × 16, +9.5% at 8 × 128); the report does not stop the
    release, and the runbook's and the release notes' figures are restated from the record.
  - **The fourth run's result** (run 44834024933, decision D041-122): at 1 × 16 the rate of (a) is 82.0% of off-1K, gate
    (2b) +8.6 ms (the completion p99 of (a) +31.9%, +36.9% and +35.2% over off-1K's in the three rounds used, 34.6 ms
    against 46.2 ms in their medians), the rate of (f) 87.1%, 7 tickets: capture costs about 18% of the rate there; at
    8 × 128 every gate passes (93.1%, −17.0 ms, 91.5%, 5 tickets).
  - **The accepted deviation:** the 1 × 16 cost, documented in `docs/runbooks/step-logs.md` and the release notes
    (OD-2026-11-20-13, decision D041-123).

  Task T041-9 judges the built carry by this criterion, both report paths included, and a failure stops the release;
  the figures are criteria to verify, fixed here before any later round.

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
- Capture adds no statement to a lease or a heartbeat, no transaction to a short step's report, and one statement to
  any report that carries records (one call of a database function, whatever the step's length; a downgrade re-run only
  when the carry call fails); after a carrying report commits, the store's top-up of its quota lease runs about once
  per 300 ten-line carries of a backend, off the run lock. A short step reports without waiting for a batch, and a long
  one waits at most 2 seconds for its batches before it reports (FR-041-015). The waits a report can meet are those of
  section 2, "What a report can wait for"; their effect must meet SC-041-007's criterion at each size, which task T041-9
  judges on the built carry.
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

- **What the built carry costs on the paths the spike did not reach (SC-041-007).** Settled for the root report path:
  task T041-35's fourth run failed at 1 × 16 and passed at 8 × 128 (decision D041-122), and the owner accepted the
  measured cost (OD-2026-11-20-13), so SC-041-007 is a criterion per size (decision D041-126). Left to task T041-9, on
  the built carry: the scoped report path (G041-019), a database on another host (G041-022) and the purge beside the
  workloads. A scoped report runs the same carry call in a transaction that also holds the run row, so its cost per
  report is the same; in a fan-out the run row is the bottleneck, so each millisecond a carry adds to its hold lowers
  the fan-out's rate in proportion, and if T041-9 finds the fan-out under SC-041-007's criterion, the server-side remedy
  is to carry nothing on the scoped path (the iteration's records go as one batch before its report, as a long step's
  do), a change of the server and the SDK rule only. A carry is one round trip (section 2a), so a remote database adds
  one round trip a carrying report (G041-022), which T041-9 records with the deployment's; the purge's hold of the
  tenant usage row (500 rows a transaction) is unchanged by the carry. A failure of SC-041-007 there stops the release.
  The quota leases' own regimes are task T041-37's.
- **Does the read horizon and its end rule hold on PostgreSQL 16 and 18?** T041-11 and T041-12, with held transactions
  on both, each in a disposable container of its own (the horizon is cluster-wide).

## Owner decisions needed

### Open

No question is open as of 2026-11-20 (the last answers are OD-2026-11-20-18 and OD-2026-11-20-19).

### Answered

Recorded in `docs/owner-decisions.md` and in research.md, newest first:

- **O041-043 and O041-044**, the two questions of the approval run's first round, were answered on 2026-11-20:
  - O041-043 (Blocker A-1: the store cannot learn which server backend a pooled connection is without a statement, so
    the ensure before the report could not work as written), "Top up after the commit" (OD-2026-11-20-18): after a
    carrying report commits, the same connection tops up its quota lease from the room that carry returned, off the run
    lock; no backend identity is needed in Rust and no transaction runs before a report (decision D041-124; ADR-0097
    decision 3b amended).
  - O041-044 (Major A-2, gap G041-018: a reader with the logs permission but without a namespace's PII grant could watch
    the tenant's step-log usage move), "Hide it without the grant" (OD-2026-11-20-19): the usage of
    `tenant_step_log_bytes` is shown only to a caller who holds `run:logs:read` and the PII reveal grant in every
    namespace of the tenant; others see that the quota exists and its limit, not its usage (decision D041-133).
- **O041-026 A step-log usage row of its own** was decided, not put to the owner, on 2026-11-20 (D041-108, delegate
  OD-2026-11-20-6): its condition was not met (D041-106: the queue on the usage row was not what failed T041-35), and the
  exact quota leases of ADR-0097 remove the step-log part of that queue, where a row of its own would only have moved it.
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
  - O041-031 to O041-035, accepted as recommended on the approval path "Corrigir e aceitar o resto" ("fix and accept
    the rest", OD-2026-11-16-7): the script marker in its own transaction after the report commits; the ingest reads the
    binding and the membership without the namespace secrets lock; O041-025 restated to the code's stop rule; G041-017
    accepted in writing; a Note on ADR-0085.
- **O041-025**, "Sim, como os reports" ("yes, as the reports"), option (a), answered 2026-11-16 (OD-2026-11-16-11).
- **O041-021** (chunk rows), not asked on 2026-11-19: T041-31 measured chunk rows and they failed SC-041-007 too
  (OD-2026-11-19-1); one row per line stays (O041-004).
- **O041-039**, "Cortar custo, depois limite absoluto" ("cut the cost, then an absolute limit"), answered 2026-11-20
  (OD-2026-11-20-1): (a) the carry in one round trip and the step-log charge once per insert statement, then T041-35
  again; (b) if the p99 still misses +10%, the p99 criterion for a completion that carries records is the capture-off p99
  plus 2 ms, fixed in T041-35 before its rounds; T041-35 fixed it, before its rounds, as the larger of 10% of the
  capture-off p99 and 2 ms (the allowance of SC-041-007). Rejected: (c) a second transaction after the report; (d)
  deferring.
- **O041-038, O041-040, O041-041 and O041-042** were answered on 2026-11-20 (OD-2026-11-20-14 to -16, decision
  D041-123), after task T041-35's fourth run:
  - O041-041, "Accept the visible loss", option (a) as built (D041-119): a carry whose fallback refill waits longer than
    its 200 ms budget drops its records, visibly (since decision D041-129, as `service_busy`, a reason of its own), and
    the report commits; the question goes back to the owner above 0.1% of carries answered busy in any regime that task
    T041-37 (split from T041-9 by decision D041-131) measures, except the two that hold the usage row past *b* on
    purpose, whose busy answers are the fault they measure (OD-2026-11-20-14; that reading is reported to the owner).
    Rejected: (b) failing the report for a resend (a step's outcome could change, ADR-0084 decision 7); (c) writing the
    records after the commit (a durable spool or the worker's resend, and an exception to ADR-0027).
  - O041-042 and O041-038, "Accept both": ADR-0097 and ADR-0095 are Accepted as written (OD-2026-11-20-15). Rejected:
    acceptance with changes; rejection.
  - O041-040, "Amend ADR-0027", option (a): the statement-level measurement, charged to the writer's lease as ADR-0097
    states it, and the per-row release, as a section of ADR-0027; the deferred flush on insert replaced by ADR-0097
    (OD-2026-11-20-16). Rejected: (b) the same moved into ADR-0097; (c) acceptance with changes.
- **The environment question of decision D041-109** (the 1 × 16 cost on a saturated 4-core runner) was answered on
  2026-11-20, "Accept the measured cost" (OD-2026-11-20-13): SC-041-007 records the measured figures per size and the
  cost is documented in the runbook and the release notes.
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
- *Usage row*: the tenant's row of `tenant_retained_usage`, which the quota finalizer locks at commit for the content
  and log classes, and a step-log refill or release locks (ADR-0097).
- *Task lease* and *quota lease*: a *task lease* is a step's lease (a `steps` row `LEASED` or `RUNNING` until
  `lease_expires_at`, with its fencing token); a *quota lease* is one of a tenant's rows of `step_log_quota_leases`,
  holding step-log quota *room* (bytes granted from the usage row and not yet charged) for one *writer*, the server
  backend (`pg_backend_pid()`) whose transactions charge it (section 2b). A *lease reclaim* takes an expired task lease
  from its worker; a *quota reclaim* folds a quota lease's room back into the tenant's free room.
- *Refill*, *fallback refill*, *reservation*, *top-up* and *b* (section 2b): a *refill* locks the usage row and grants a
  writer's quota lease new room; a *fallback refill* is a refill inside a report, when the carry's writer's quota lease is
  short; a carry's *reservation* locks its own quota lease with room for its planned charge before it writes; the *top-up*
  is a refill without a charge that the store runs after a carrying report commits, on the same connection, when the
  carry left its quota lease under 64 KiB (OD-2026-11-20-18); *b* = 200 ms is the budget of each lock acquisition of a
  refill on the usage row, and a *wait slot* is one of the advisory locks, a tenant's (*P* on a replica) and the
  replica's (*Q*), without both of which no refill waits for that row (decisions D041-125 and D041-134).
- *Step-log lock*: one of the locks L1, L1b, L2, L3 and L3b of section 2's lock order, which a carry only tries; the
  usage row (L4) is not one.
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
- *Tickets*: the per-tenant share of the Logs class on one replica (section 1); *R* is the number of API replicas
  (section 2b writes the leased room in words, and S there is the folded usage, not the service time of section 2).
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
   v                                                 api-grpc -> WorkerService::complete / fail
api-grpc: Class::Logs admission -> per-tenant         | the report transaction (run row, task lease, binding,
   ticket -> application::step_logs                  |   membership, engine) and, before its commit, one call of
   (validate, scrub, cut, size, plan)                 |   sagat_step_log_carry: step-log locks tried, never waited
   | one transaction of its own, lock_timeout 200 ms: |   for: fences + stream lock -> values -> stream -> lines,
   |   xid -> task lease read (no lock) -> session,   |   charged to the writer's quota lease; when it is short, a
   |   policy fences and values, stream lock (waited) |   fallback refill waits for the usage row (<= 200 ms a lock,
   |   -> stream row -> plan -> tenant usage read     |   with a wait slot); a quota refusal or a busy row: a
   |   (no lock) -> binding and membership read (no   |   mark-only final mark; a failed call: a re-run, the carry
   |   lock) -> insert -> counters -> commit (lines   |   downgraded (section 2a) -> commit
   |   charged to the writer's quota lease)           |   then, carry room under 64 KiB: the top-up, off the run
   |                                                  |   lock, on the same connection (section 2b)
   v                                                  v
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
checks under their locks, and commits once; a carry adds to it one call of a database function and one statement-trigger
call that charges a quota lease (section 2b), and no transaction or round trip of the worker (section 2a). A long step must still be readable
while it runs (CUJ 2), so its earlier records stream through `AppendStepLogs`, about one batch for each second it logs. A field on `Heartbeat` was
weighed again and rejected: a heartbeat renews the lease under the run row, so records there would add statements under
the run lock once a second per long step, where `AppendStepLogs` takes no run row, and a batch's size and failure would
ride on the call that keeps the lease alive. A client stream would need acknowledgement and resend rules in six
languages for a gain the batch already gives.

**Admission.** `AppendStepLogs` gets a fourth class, `Class::Logs`, in `crates/api-grpc/src/limits.rs:489-508`, with its
own counter outside `Shared.in_flight`, so that a flood of log calls never moves the threshold at which discovery is
shed (`limits.rs:548-563, 595`). Its cap is `SAGAT_GRPC_MAX_LOG_CALLS` (default half of `SAGAT_DB_MAX_CONNECTIONS` rounded down,
at least 1: 10 of a pool of 20, the Helm chart's value; 5 of the binary's default of 10, which the execution engine
reads through `sagat_control_plane::config`, `apps/control-plane/src/config.rs:234`, whatever its `cli.rs:55` help
says), per replica; a call
over it is shed `RESOURCE_EXHAUSTED` with `retry-after-ms`, as shedding is today (`limits.rs:675-678`). The cap is what
keeps ingest from holding the connections that lease calls need: at most that many of the pool's connections are in a
log transaction at once. The tenant is known only after authentication, so the per-tenant share is taken in the handler: a
ticket set built as `TaskStreams` is (`crates/api-grpc/src/limits.rs:47-129`), `SAGAT_STEP_LOGS_TENANT_CALLS` per tenant
per replica. A batch that finds no ticket free
is answered `RETRY_LATER` with 200 ms at once: no call waits for a ticket, so a call holds its class slot only while it
runs and one tenant cannot fill the Logs class with waiting calls. The SDK resends a batch until the batch is 10 seconds
old or, once the activity has ended, until its 2-second wait before the report ends, whichever comes first. A carry is
not admitted in this class: it is part of its report, whose class is the reserved one of calls that keep leased work
alive; its cost there is bounded by its 64 records, by never waiting for a step-log lock and by the wait slots, which
bound how many of a replica's connections waits on usage rows hold: at most P = 3 for one tenant and Q = 5 for all
tenants together at a pool of 20 (section 2b, decisions D041-125 and D041-134).
The defaults (decision D041-127, from task T041-35's fourth run; data-model.md §6): the cap is half of
`SAGAT_DB_MAX_CONNECTIONS` rounded down, at least 1 (10 of a pool of 20, the chart's value and the configuration that run measured),
and the tenant tickets 7, at most the cap (section 2, "Capacity", gives the derivation), so a second tenant keeps 3 of
the cap while the first holds its 7. The class test `only_the_calls_that_keep_leased_work_alive_are_reserved_capacity` (`limits.rs:706`) lists
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
  it); the same select list sets the replica's wait slots *Q* for this transaction,
  `set_config('sagat.step_log_wait_slots', $q, true)` (section 2b, decision D041-137), so no statement is added.
- T2. The lease, read without a lock: the `steps` row of the request's step, attempt, generation, instance and
  lease-token hash, `LEASED` or `RUNNING`, unexpired, and the run, both passed to `report_fence_allows` (the step not
  cancel-requested; the run not finished; the run not stopping unless the node is a compensation or reconciliation
  node; OD-2026-11-16-11). Every later step takes the run, namespace, node, path and activity from this row, never from
  the request. No row, or a refusal: `LEASE`, and a run of another tenant is audited as `access.denied.cross_tenant`
  outside the transaction.
- T3. The session (tenant, id, the lease's namespace, the caller's subject, token, unexpired, accepted at 1.7 or later),
  else `SESSION`; and the three step-log resources of the namespace, read under their policy fences taken shared,
  tenant then namespace, as `content_usage::admission_policy` reads a policy (`crates/infra-postgres/src/content_usage.rs:95-106`),
  so that a change of a resource waits for the batches that read the old value; the same statement then takes the
  attempt's stream lock (L1b, section 2), waiting under the 200 ms lock timeout, so that no carry of the attempt can run
  between this batch's T4 and its commit. `step_log_attempt_records = 0` is `CAPTURE_OFF`.
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

At T8 the statement trigger charges the rows to a lease (section 2b). If the charge raises
`tenant_step_log_quota_exceeded` (batches of other attempts crossed the limit at the same time, or the room left is in
leases that open transactions hold, G041-023), the transaction rolls back, nothing was stored or counted, and the answer is `RETRY_LATER` with 200 ms (D041-061). The
SDK sends the same batch again: when the tenant is over its limit, its T6 reads it so and counts the records as
`tenant_quota`; when the room was only in leases that open transactions held (`tenant_step_log_quota_held`, counted
`step_log_quota_refusals_total{cause="held"}`), its T6 reads the room and the resend is charged once those transactions
end, or answered `RETRY_LATER` again. A batch the SDK gives up on after its 10 seconds is counted later as a gap, dropped by the worker; a stream whose final
mark is never stored shows "ended without its final flush" (G041-002). ADR-0084 decision 3 holds: the batch stores no
line, and its records are counted on its stream. A crash before the commit leaves nothing and the SDK sends the batch
again; a crash after it leaves the lines, and the resend stores nothing and gets `CONTINUE`, or `STOP_ATTEMPT` once the
report is recorded (FR-041-004). The counting rule, held by a property test of `plan_batch` over any sequence of batches, carries
and refusals: for a stream, lines stored (markers excluded) + dropped by the worker + dropped by the server =
`last_sequence`, the highest worker sequence the server has recorded (once the carry is recorded, its
`highest_sequence`). `plan_batch` plans a carry as it plans a batch, with the carry's `highest_sequence` as the final
mark and `in_order` (section 2a) as an input.

**Statements per batch** (by reading; budgeted as an equality in T041-6, whatever the number of records): `BEGIN`, the
lock timeout, T1, T2, the session, one statement that takes the six policy fences in order and the stream lock, one that reads the three
values, T4, the tenant allowance, the binding's patterns, the membership, T8, T9 and `COMMIT`: 14 round trips.

**One lock order** for everything that writes step logs (ADR-0095 decision 7):

- L0. what a report already takes before its carry: the run rows, then the authority locks of ADR-0050 decision 7 (the
  binding lock, the namespace secrets lock shared, the membership row). Only a report takes them; an ingest, the purge,
  the delete route and the script marker take none;
- L1. the policy fences of the namespace's step-log resources (shared, tenant then namespace);
- L1b. the attempt's stream lock, an advisory lock on (tenant, run, step, attempt, generation): an ingest takes it in
  its T3, after L1, and a carry tries it in its CA; the purge, the delete route and the script marker never take it;
- L2. the stream row;
- L3. the policy fences of the quota classes, shared, in ADR-0027's order (content, log, then step-log): a step-log
  charge takes all three at its insert (ADR-0097, section 2b); the finalizer takes those of the classes it applies at
  commit;
- L3b. the writer's own lease of the tenant's step-log quota (`step_log_quota_leases`, keyed by the backend, D041-117):
  a charge only tries it (`SKIP LOCKED`); no transaction ever waits for a lease it does not own (a release does not take
  one, ADR-0097 decision 5; a reclaim takes the others `SKIP LOCKED`, or `NOWAIT` under the fence held exclusively);
- L4. the tenant's `tenant_retained_usage` row: taken by a refill (a batch's, the script marker's or a carry's fallback
  refill, each first tried without waiting, `SKIP LOCKED`, then waited for at most *b* = 200 ms a lock acquisition and
  only while holding one of the tenant's wait slots and one of the replica's, section 2b), by the top-up after a
  report (tried without waiting only: it takes no wait slot and never waits), by a release, by an
  idle quota reclaim, by a change of the tenant's step-log limit, by `sagat_quota_lock_log_usage`'s holders, and by the
  content and log finalizer at commit. The wait slots are advisory locks only tried, so they are exempt from the order.

No transaction waits for a quota lease it does not own, so no cycle can pass through L3b; a refill holds L4 and then
takes only its own quota lease `NOWAIT`, and after L4 no writer takes a lock it waits for. A change of a namespace
step-log resource takes its own L1 fence exclusively and nothing else of this order; a change of
`tenant_step_log_bytes` takes its L3 fence exclusively, then the tenant's quota leases `NOWAIT` (exempt from the order,
since it never waits for them), then L4 (ADR-0097 decision 6). The purge and the delete route take stream rows (L2) in `stream_id` order,
and nothing earlier, then at commit L3 and L4 (a release takes no lease, ADR-0097 decision 5). The script marker (section 7) takes L1, then a stream row no other
transaction can see, then at its insert L3, a lease and, on a refill, L4. A report that carries nothing takes none of
L1, L1b, L2 and L3b; a carrying report holds L0 and then only tries L1, L1b and L2 (for its own attempt's stream), never
waiting for them, tries L3 in CA (so that CD's charge holds it already), L3b by its reservation before CC (or at CD
without one) and, on a fallback refill, waits for L4 with each lock acquisition at most *b* (D041-119), and
takes L3 and L4 at its commit only when it stores output. Putting L1 before L2 is the reading of ADR-0084 decision 11 the owner accepted (O041-025,
OD-2026-11-16-11). No cycle can form: no holder of L1, L1b, L2, L3, L3b or L4 then waits for a run row or an authority
lock, a carrying report waits for none of L1, L1b, L2 and L3b before its commit and for L4 only on a refill, whose
holders wait for nothing it holds, and nobody waits for a lease it does not own. The
lock-order test, built on `tests/support/lock_order.rs` with new slots for L1, L1b, L2, L3 and L3b placed after its
`Bookkeeping` slot and before `Usage` (a carry runs right before the report's commit), models a lock only tried (`SKIP
LOCKED`, `NOWAIT`, `pg_try_advisory_xact_lock`) as exempt from the order and proves with `pg_stat_activity` that it never
waits, models the fallback refill's wait for L4 under L0 as bounded by *b*, and checks that after L4 (a refill held) no
writer waits for a lock (the refill's own lease is taken `NOWAIT`); it grows with the tasks that
add a writer: T041-6 runs an
ingest, a report that carries nothing and a change of a step-log resource; T041-10 adds the purge; T041-36 adds a
carrying report beside an ingest of the same attempt, the purge and a queued change of a step-log resource; T041-13 adds
the script marker and T041-30 the delete route.

**What a report can wait for** (the bound; FR-041-005 and the Overview point here). A report of tenant T that capture
touches can wait for these, and for nothing else of step logs:

1. **Before the report, in the SDK:** a long step waits for its batch in flight and its overflow batches, at most
   2 seconds in all (FR-041-015); a short step waits for nothing. No transaction runs before a report.
2. **In the report's transaction, its one carry call:** every step-log lock (L1, L1b, L2, L3, L3b) is tried, never
   waited for; when L1, L1b, L2 or L3 is held (a change of a step-log resource or a tenant class limit holding or queued
   for its fence; an ingest of the attempt between its T3 and its commit, which happens only after the SDK's 2-second
   wait for its batch in flight expired, section 6; the purge or the delete route on an expired or deleted stream), the
   carry writes nothing more (`contended`, section 2a).
3. **In the same call, the fallback refill**, only when the carry's writer's quota lease is held (only by a quota
   reclaim) or short: under the run lock, it tries T's usage row (L4) without waiting (`SKIP LOCKED`); when the row is
   held it takes one of T's wait slots on the replica and then one of the replica's (3 and 5 at a pool of 20, decision
   D041-134), both
   tried, never waited for, and waits for the row at most *b* = 200 ms a lock acquisition, the
   statement timeout (5 s, 3 s for some transactions) the hard bound. Past *b*, or with no slot free, the carry records
   a mark-only final mark whose records are not stored because the service was busy (`service_busy`) and the report
   commits. Measured far from the limit with direct connections, a refill inside a report for 0.31% of ten-line carries
   and 2.0% of 64-record carries, the longest refill-written carry 726.7 ms (task T041-35's fourth run, which had no
   top-up). While it waits its run row is held, so the run's other reports, heartbeats, cancels and signals queue behind
   it (a risk row; task T041-37 measures a fan-out run during fallback waits it causes on purpose). It runs after the
   report's fence checks, so it delays the commit and never makes the report late.
4. **At the commit, the usage row for the content class**, only when the report stores output, as today (G041-024): it
   waits for the commits queued ahead of it there (other reports of T that store output; refills of T's quota leases,
   by batches, markers, carries' fallbacks and top-ups; the purge's delete transactions, each holding it for at most 500
   rows; a change of T's step-log limit). No ingest and no carry holds T's usage row for a charge into a quota lease, so
   a tenant's logging does not queue behind its own commits (D041-106: before the quota leases each such commit held the
   row p50 1.1 ms and p99 4.0 to 4.5 ms, to the end of its WAL flush).

After the commit the store's top-up (section 2b) waits for nothing the report holds and delays no answer. A report
that carries nothing takes none of these. Before L1, the ingest takes none of the locks a report, a task lease or a
heartbeat takes. The effect of these waits on time is what SC-041-007 bounds, per size. A per-tenant step-log usage row
of its own (O041-026) is not needed: the quota leases remove the step-log part of the queue rather than moving it
(ADR-0097).

**Capacity.** `AppendStepLogs` now serves only long steps: each sends about one batch for each second it logs, plus
the batches its last records need when they do not fit one carry. One tenant's batch rate on a replica, in batches per
second, is therefore about the number of its long logging attempts in flight there. By Little's law the ingest holds
rate × S tickets on average, S being the service time of one batch, here taken at its p99 to be conservative; n tickets
at 70% use serve 0.7 × n / S batches a second. The first spike (task T041-31, withdrawn) measured S p99 at 4 to 8 ms at
1 replica and 16 workers and at 35 to 89 ms at 8 replicas and 128 workers, and its ticket gate failed in every round (11
to 16 tickets needed against 10). Task T041-35's fourth run measured workload (g), in which every step sends one batch
and a carry, at both sizes, with a cap and tickets of 10 (decision D041-122). The defaults (decision D041-127) apply
tickets ≥ rate × S p99 / 0.7 to its medians: at 1 × 16, 275 batches a second × 16.4 ms / 0.7 = 6.44, so 7; at 8 × 128,
131 / 8 = 16.375 batches a second a replica × 180.4 ms / 0.7 = 4.22, so 5. The larger, 7, is the tenant tickets'
default; the 0.7 is 1 / 0.7 − 1 = 43% of headroom over the mean need at the p99 service time, and two of the eight
valid rounds needed 10 at 1 × 16 (rounds 4 and 6), so one tenant at that benchmark's saturation can meet `RETRY_LATER`
(resent; lossy only after 10 seconds) and the runbook names the setting. The cap is half the database pool
(`SAGAT_DB_MAX_CONNECTIONS`, of which the Logs cap `SAGAT_GRPC_MAX_LOG_CALLS` is a share), the most this design allows:
10 of 20. A carrying report holds its pool connection one round trip longer (section 2a), and a top-up holds it for one
statement after the report; the pool's budget at the chosen cap, task leases, reports and ingests together, is recorded
by task T041-9.

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

**Which transaction writes what.** The carry is an explicit argument, `CarryPlan` (`Full`, `MarkOnly` or `None`), of
every function that records an attempt and commits, so that the compiler makes each site say what it writes:

| Transaction that records the attempt | What it writes |
|---|---|
| `finish` (`crates/application/src/worker.rs:2861`) after a root completion, failure, reconciliation or compensation whose output and failure code hold no declared secret | the carry |
| `finish_path_report` (`crates/application/src/worker/scoped_reports.rs:419`) after a scoped completion or failure whose output and failure code hold no declared secret | the carry |
| a report whose output or failure code holds a declared secret: `finish` after `reject_secret_output` (root output), `finish` after the root `fail` with `leaked` set (`worker.rs:2412-2418`), `finish_path_report` after `apply_path_report` with `leaked` set (`scoped_reports.rs:274-287`, output or failure code), and `refuse_scoped_output` (`worker/scoped_output_refusal.rs:25`, a refused output, the run's data limit included) | a mark-only carry: no line of an attempt whose report held a secret is stored |
| the content-quota fallbacks `fail_success_for_storage_quota`, `fail_report_for_storage_quota` (`worker.rs:2474`) and `refuse_scoped_content` | a mark-only carry, or none if the carry was already downgraded to none |
| the downgrade re-runs below | a mark-only carry (reason `failed`) after a guard's raise; none after a failed carry call |

It runs only when the report is recorded now: a resent report whose idempotency key is already recorded is answered as
a replay (`replayed`, or `tx.replay` on the scoped path) before its carry is read, so a carry is stored at most once,
and its lines are keyed (attempt, generation, sequence) besides. The carry is not part of the report's idempotency
fingerprint (`completion_idempotency`), so a resend with another carry is answered as the replay it is.

The stream's identity (tenant, namespace, run, step, node, instance path, attempt, generation) is the attempt the
report's fence verified, captured before the engine applies the report, never read from state after it (a retryable
failure has by then moved the step to its next attempt) and never from the request.

**The statement** (OD-2026-11-20-1, "Cortar custo": the carry's statements in one round trip). Right before the report
transaction's commit, after its engine change and journal writes, the report transaction makes one call, `SELECT * FROM
sagat_step_log_carry(…)`: one top-level statement, one round trip, through the one method of a port, `StepLogCarryTx`,
a supertrait of the report transactions (`PgRunTx` and `PgPathExecutionTx`, implemented in
`crates/infra-postgres/src/step_logs.rs` through a `pub(crate)` connection accessor of `execution.rs`). The function is
plpgsql, `VOLATILE`, `SECURITY INVOKER` (it runs with the runtime role's grants on the step-log tables and is not a
`SECURITY DEFINER` function), with a pinned `search_path`; it is created by T041-36's migration
`<timestamp>_step_log_carry`, in the pull request that holds its tests, `EXECUTE` granted to `sagat_app` only. Its
arguments are the attempt's identity (tenant, namespace, run, step, node, instance path, attempt, generation), the
records as arrays (sequence, level, worker time, message, exception, fields, `truncated`, the text size C3 computed),
`highest_sequence` and the reason of a mark-only carry the application planned, one of `invalid`, `secret`,
`content_fallback` and `failed` (the function refuses any other at its top, with a fixed text). It builds its policy-fence and
stream-lock keys itself from that identity, through the SQL helpers the ingest's T3 also calls, so the two can never
take different keys. It returns the outcome, the records kept, the drops by worker and by server keyed by their reason
(the domain's `DropReason`, decision D041-128), the stream's `ceiling_reason` and `not_stored_reason`, and, for the
store, `path` (`fast`: its reservation held; `refill`: it took its block and its fallback refill ran; `none`: it
charged nothing or never reached a refill), `room` (the room left in its writer's quota lease after its charge,
measured by the database; null exactly when `path` is `none`) and `top_up` (true when `room` is under 65,536 bytes;
section 2b). data-model.md §8a gives its body. Because the function is `VOLATILE`, each statement inside it takes a snapshot of its
own under READ COMMITTED, so its policy read sees what committed while it took its fences.

The plan of a carry runs inside that function, because its inputs (the policy values and the stream's counters) are read
there and a second round trip is what the owner chose to remove. The domain's `plan_batch` stays the rule, and the
function implements exactly its T5 and T6 for a carry: records above `last_sequence` (and, when `in_order` is false,
none above it, counted `out_of_order`) kept in sequence order until the first that does not fit the attempt ceilings,
the rest dropped by the server and one ceiling marker planned when a ceiling is first met; then, if the kept bytes plus
128 a row, the marker included, do not fit the tenant allowance, nothing is stored and the records are counted
`tenant_quota` (all or nothing, as T6); the gap from `last_sequence` to `highest_sequence` dropped by the worker below
`ceiling_sequence` and by the server above it. A mark-only carry the application planned carries its reason
(`invalid`, `secret` for a report whose output or failure code held a declared secret, `content_fallback`, or `failed`
for the re-run after a guard's raise), which the function records as the drop reason of its records. The function's own block can also make a carry mark-only: a
refusal of its charge records `tenant_quota` as the drop reason and as `ceiling_reason`, and a fallback refill past *b*
or with no wait slot free records `service_busy` as the drop reason and as `not_stored_reason`, leaving
`ceiling_reason` as it was. No answer of any other transaction changes what the function plans: it plans by its own
allowance read (decision D041-124 removed the ensure before the report). The function returns the kept count and the
drops by reason. A property test of T041-36 runs the function and `plan_batch` on the same generated carries (new and
existing streams, each ceiling met, the tenant allowance exceeded, `in_order` false, duplicates, gaps, each planned
mark-only reason, and each answer of the block) and requires the same kept sequences, marker, counters and drops by
reason (D041-100, which refines D041-039 for the carry).

**A carry never waits for a step-log lock; it waits for the usage row only in the fallback, each lock acquisition at
most 200 ms and only with a wait slot.** Every step-log lock the function takes is tried, never waited for (its fences,
its stream lock and row, its own quota lease `SKIP LOCKED`). A fence, the stream lock or the stream row held by another
transaction makes it write nothing (`contended`), and the report commits as it would without the carry; its own quota
lease held (only by a quota reclaim) or short never makes it `contended`: it takes the fallback refill below. Before any write the function reserves room in
its writer's quota lease for its planned charge (`sagat_quota_step_log_reserve`, D041-113): with it (the fast path),
CC to CE run as they are and the charge in CD cannot be refused, with no subtransaction. Without it (the first carry of
a tenant on a backend; a quota lease a batch on the same backend left short; a quota reclaim that took the room near
the limit, by the idle reclaim or by a limit change; behind a transaction pooler, a backend whose quota lease no top-up
reached), CC to CE run in one `BEGIN … EXCEPTION` block (one subtransaction, no round trip; D041-112) whose fallback
refill tries the usage row without waiting (`SKIP LOCKED`), and when it is held takes a wait slot and waits at most *b* = 200 ms for each lock
acquisition; the block catches the quota's two refusals and the `55P03` of a wait past *b* or of no free slot, and
answers each with a mark-only final mark saying why (`tenant_quota`, or `service_busy`). So neither contention, a quota
refusal nor a busy usage row fails, blocks or retries a report (D041-117 had a `NOWAIT` refill that answered 0.08% to
21% of carries mark-only while the tenant had room; decisions D041-119 and D041-124 make that answer bounded and visible,
and the fourth run measured it at 1 carry in 864,000). Inside the function, in order:

- CA. `pg_try_advisory_xact_lock_shared` on the six policy fences of the namespace's three step-log resources (L1,
  tenant then namespace), then `pg_try_advisory_xact_lock` on the attempt's *stream lock*, an advisory key of (tenant,
  run, step, attempt, generation) that every ingest of the attempt also takes, waiting, in its T3 (L1b, section 2). The
  key is the 64-bit `hashtextextended` of that text, so two attempts can share a key by collision; that only makes one
  of them `contended` or one ingest wait for the other, never a wrong write, since the stream row is what identifies. Any
  refused: `contended`. (A change of a step-log resource holding or queued for its fence, or an ingest of the attempt
  between T3 and its commit, is what refuses it.) Then the tenant's three quota class fences (L3: content, log,
  step-log), tried shared in that order, so that CD's charge, which takes them again in the same transaction, never waits
  for them (ADR-0097); a change of a tenant class limit holding or queued for its fence makes the carry `contended`.
- CB. The three resources' rows (the read `effective_policy` makes) and `sagat_quota_step_log_allowance(tenant)`, in a
  statement whose snapshot begins after CA's locks are held; `step_log_attempt_records = 0` is capture off: `off`.
- The plan of a new stream (from zero counters, `in_order` true), before any write (D041-111).
- CC. `INSERT … SELECT … WHERE NOT EXISTS (<the attempt's stream>) ON CONFLICT DO NOTHING RETURNING` the new stream row,
  written once with what that plan stores: its counters, `last_sequence`, the drops, `ceiling_sequence` and
  `ceiling_reason`, `first_at`, `last_at`, `expires_at`, `final_at` and `last_stored_xid` (the transaction's id when a row
  is kept). An existing row, even one the purge or the delete route is updating, is seen by the statement's snapshot, so
  no row is offered and no unique check waits; with the stream lock held no other transaction is inserting a new one. A
  short step's stream is new; for it CD follows and CE does not run.
- CC′. Only when CC inserted nothing: `SELECT … FOR UPDATE SKIP LOCKED` of the existing stream row (L2). No row (the
  purge or the delete route holds it): `contended`. Removed, expired or already final: `removed` or `final_recorded`.
- The plan of the existing stream, from its counters. A mark-only carry keeps no record and plans no marker.
- CD. One insert of the kept lines and the marker, at most once each (attempt, generation, sequence). For a new stream
  no line of it exists, so the insert keeps every planned row and the counters CC wrote are exact.
- CE. Only for an existing stream: its counters from CD's rows; `final_at`; `last_stored_xid` raised only when CD
  inserted a row. It keeps `expires_day` on most days, so it is a HOT update (section 3).

**The charge** (ADR-0097, section 2b). CD's statement-level trigger charges the kept rows to `tenant_step_log_bytes`
once per insert statement: their text bytes plus 128 a row, measured by the database, added to its writer's own lease,
taken `SKIP LOCKED`. The rows are not charged at the commit and the usage row is not taken for them, so a
carrying report whose output stores nothing does not take the usage row (it takes the class fences shared and a lease),
and one that stores output takes the usage row at its commit for the content class only, as every report with output
does today. A charge refused (the tenant's limit, or room held only by open transactions' leases, G041-023) raises in
CD only on the slow path and is caught by the function's block (by constraint name; any other error is re-raised): the
subtransaction rolls back what CC and CD wrote, and the function writes the stream's final mark as a mark-only carry with
reason `tenant_quota` (nothing charged; a new stream inserted, an existing one taken again `SKIP LOCKED`, its row released by
the rollback) and returns `mark_only` with the cause. A fallback refill that finds the usage row held takes a wait slot
and waits for it at most *b*; past *b*, or with no slot free, its `55P03` is caught by the same block, which writes the
mark-only final mark with `not_stored_reason = service_busy` and returns `mark_only` with the cause `busy`, counted and
alerted (section 2b). In these cases the function never answers `contended`: only CA, and CC′ when the existing
stream's row is held, answer it. The report commits in its first transaction.

**When the carry makes the report's transaction fail**, the report is recorded anyway by a *downgrade re-run*: the
report's method runs again from its start, as the worker's own resend of the same report would, with the carry
downgraded (ADR-0095 decision 5). The quota's own refusals and a busy usage row never reach this path (the block above
answers them in the same call). The carry only goes down, `Full` → `MarkOnly` → `None`, and each step-log downgrade
can meet the content-quota fallback once (which keeps the downgraded carry), so a report runs at most four
transactions (`Full`; a guard's raise, then `MarkOnly`; a content raise, then its fallback with `MarkOnly`; a failed
call, then `None`); a transaction whose carry is `None` cannot fail because of it.

- A raise of a guard of the quota-lease functions that the block does not catch, or that the fast path meets with no
  block (an underflow, a quota lease `CHECK`; each with a constraint name of its own, T041-5): the store maps it to
  `StoreError::StepLogRefused`, and the re-run carries a mark-only carry the application plans with the reason
  `failed`, so the carry's records are counted dropped as `failed` and the stream keeps its final mark. A mark-only
  carry charges nothing, so the charge does not run again.
- The call fails otherwise (a lost connection, a deadlock the database breaks, a statement timeout, a bug in the
  function): the store maps its error to `StoreError::StepLogCarry`, which keeps only the SQLSTATE (through
  `store_error_without_text`); the re-run carries none, the carry's records are counted dropped as `failed`, and the
  stream keeps no final mark. A mark-only carry that fails so in a re-run or a content fallback is run once more with
  none.
- The two store errors cross into the application as two internal variants, `AppError::StepLogRefused` and
  `AppError::StepLogCarry`, which only the re-run wrapper consumes, so that an ordinary `AppError::Unavailable` (a
  database outage, the secret-version and refusal paths of the scoped report) is never re-run and a carry failure is
  never mistaken for one. If either ever reached the API edge, it would answer `UNAVAILABLE`.
- A content-quota raise in a re-run goes to the content fallback of the table above, which keeps the downgraded carry.
- The re-run is fenced as any report, so it can be refused when, between the first transaction and the re-run, the
  task lease expired or was reclaimed, a cancel was requested, the run began to stop (for a forward step), the run
  finished, or the worker's binding or membership was revoked (G041-021). Those are the only cases where capture changes
  a report's answer, and only after a carry call failed.

**What a report pays** (the bound that replaces "capture adds no statement to a lease or a completion", ADR-0084
decision 2, superseded by ADR-0095 decision 2):

| Report | Statements | Transactions | Rows by the top-level measure | Step-log rows written (the second measure) |
|---|---|---|---|---|
| no carry; or a carry ignored by C1, C2 or C0 | as today | as today | as today | none |
| any carry the function receives, whatever it does | + 1 | + 0 | + 0 | `contended`, `off`, `removed`: none; a short step: the stream inserted once and *k* lines; a long step: the stream updated (HOT when the day of its expiry is kept) and *k* lines (*k* = 0 for a mark-only carry); when *k* > 0, one quota lease row besides (ADR-0097); with `path = refill`, one write of the usage row, one quota lease row inserted or folded, and, near the limit, one write of each quota lease it reclaims |
| a carry whose transaction fails (above) | the failed transaction, then the report once to three times more | + 1 to + 3 | as the last transaction's | as the last transaction's |
| after the commit, the top-up (section 2b), when the carry returned `top_up` true | + 1 (one statement in its own implicit transaction, off the run lock, not part of the report) | + 1 | + 0 | with `made`: one quota lease row and one write of the usage row (it reclaims no other quota lease, decision D041-134); otherwise none |

**The budget's measure changes, on purpose.** `work_budget.rs` counts rows as the top-level `INSERT`, `UPDATE` and
`DELETE` statements report them; the carry's writes are inside a function called by a `SELECT`, so that measure does not
see them. T041-36 therefore changes the test deliberately: the three existing constants keep their meaning and values
for leases and reports without a carry, and every carrying case adds a second measure, the step-log rows the report
transaction wrote, counted from the step-log tables (lines whose `stored_xid` is the report's transaction, and the
stream row's change), asserted as an equality beside the first. The six carry equalities are then: a completion with a
next step whose short step carries ten records (51 statements, 1 transaction, 11 rows; step-log rows: 1 stream, 10
lines), a mark-only carry (51, 1, 11; 1 stream, 0 lines) and capture off in the namespace (51, 1, 11; none); and the
completion that ends the run in the same three cases (48, 1, 9 each, with the same step-log rows). The case of ten
records is pinned three ways, the report's own counts the same in each:

- *warm*: the carry's reservation holds and it returns `top_up` false: nothing runs on the connection after the report;
- *warm and low*: the reservation holds and the carry leaves its quota lease under 65,536 bytes: exactly one top-up
  statement runs after the report, on the same backend (`pg_backend_pid()` equal), counted apart from the report;
- *cold*: a fresh backend, so the carry takes its block and its fallback refill (`path = refill`): the second measure
  shows the usage row written once and the quota lease inserted, and no top-up follows (the refill's grant leaves room
  of about its share less the charge, at least 65,536 bytes far from the limit).

A third measure keeps the function's own cost from growing unseen. The test support starts its PostgreSQL with
`pg_stat_statements.track=all` (`tests/support/mod.rs:158-165`) and the test changes no server setting:

- the carrying cases first assert `current_setting('pg_stat_statements.track') = 'all'`, and fail with a message naming
  the setting when a server given by `SAGAT_TEST_DATABASE_URL` is configured otherwise;
- they then assert, as an equality, the nested statements (`toplevel = false`) that the call ran and their rows, for
  each case;
- the figure is derived statement by statement in the test's comment, and the test is seen failing once against a
  figure one off, before it is pinned. On the prototype the fast-path figure for ten records kept by a short step was
  26 (`step_log_leases_tests.rs::a_short_steps_carry_writes_its_new_stream_once`): the function's four (the prototype's
  application name, which the built function does not set, so that the per-replica `application_name` stays; the policy
  read; the allowance read; the plan's sizing), the reservation's three, the stream insert, the lines insert, the
  stream's path check, the charge's six, and one foreign-key check a line (ten). The built function differs (it builds
  its own keys and checks `p_mark_reason`), so its figure is derived again the same way.

A source-text check of the migration that creates the function fails if its body names `tenant_retained_usage` (FR-041-008:
a carry never takes the usage row itself; its charge takes a lease and the usage row only to refill; it reads the
allowance only through
`sagat_quota_step_log_allowance`). The test's comment says why the rows and statements of a function call are counted
apart, so that a later change of the function is not silently uncounted. T041-36 also measures, at each other call site (a root `FailStep`, a scoped completion and failure,
`reject_secret_output` and each content fallback, and a long step's carry onto an existing stream), that the cost with a
carry minus the cost without it is the row of the table above.

Inside its one call a carry with *k* kept rows runs the function's statements and fires one statement-trigger call,
which charges a quota lease (L3 tried before, L3b, and L4 only on a refill); nothing runs for it at the commit; task
T041-35 measured their time. The call is one round trip while the run row is held: at a round trip of *r*
milliseconds to the database it holds the run row about *r* longer (G041-022: a completion is about 50 round trips, so
the carry adds about 2% of them). A report that keeps a carried record takes no quota lock at its commit for it; only a
report that stores output takes the tenant's usage row there, for the content class. A task lease and a heartbeat pay
nothing. A short step pays no transaction of its own; a long step pays about one batch transaction for each second it
logs, plus the batches for the records that do not fit its carry.

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

### 2b. The step-log quota leases (ADR-0097)

Decision D041-108 (revised by D041-112, D041-117 and D041-124; delegate OD-2026-11-20-6, and OD-2026-11-20-18) takes
every step-log charge off the tenant's `tenant_retained_usage` row. In task T041-35's second run the charge went to the
transaction's delta row and ADR-0027's deferred finalizer applied it at commit, holding the tenant's one usage row from
its `SELECT … FOR UPDATE` to the end of the commit's WAL flush: measured on the hosted runners (D041-106), p50 1.1 ms and
p99 4.0 to 4.5 ms per lock, busy 23% to 24% of the time with one charge a step and 68% with two, so one tenant's charging
commits passed it one at a time, about 800 a second on that machine whatever the number of replicas; at 8 × 128, 1,075
ingest commits timed out waiting for it (run 44741773762). [ADR-0097](../adr/ADR-0097-the-step-log-quota-is-charged-to-exact-per-tenant-leases-never-to-the-usage-row-at-commit.md)
(accepted 2026-11-20, OD-2026-11-20-15; its decision 3b amended by OD-2026-11-20-18) replaces that charge with exact
quota leases; data-model.md §3b gives the table, the functions and their statements.

- **Quota leases, one a writer** (D041-117). `step_log_quota_leases (tenant_id, writer, granted, used, charged_at)`, one
  per tenant and server backend that charged for it, keyed by `pg_backend_pid()`, and
  `tenant_retained_usage.step_log_leased_bytes` (G) beside `step_log_bytes` (S, the folded usage, which may be
  negative). The tenant's usage is U = S + Σ `used`; its leased room is Σ (`granted` − `used`). A backend runs one
  transaction at a time, so no writer meets another's lock on its quota lease; behind a transaction pooler the key is
  still the server backend. (A shared pool of 256 quota leases, this section's first form, collided: validation-report.md,
  2026-11-20.)
- **A charge** (the statement trigger of the line rows, once per insert statement and tenant, with what
  `sagat_quota_measure` measures from the rows stored) takes the content, log and step-log policy fences shared, reads the
  limit L and, without a lock, S and G, and, when S + G ≤ L, adds its bytes to the writer's own quota lease when it has
  room, locked by primary key `FOR UPDATE SKIP LOCKED` (only a quota reclaim, holding the usage row, can hold it). It
  never locks the usage row and never waits for a quota lease. Otherwise it calls the refill.
- **A carry's reservation.** Before its writes a carry locks its own quota lease with room for its planned charge
  (nothing written; the planned charge as data-model §3b states it, a possible ceiling marker included); with one, its
  charge cannot be refused and it opens no subtransaction (the fast path); without one it runs its writes in a block that
  answers a refusal in the same call (section 2a).
- **The fallback refill and every refill that may wait** (a batch's T8, the script marker's insert, a carry's block),
  when the writer's quota lease is short or S + G > L: it tries the usage row without waiting (`FOR UPDATE SKIP
  LOCKED`, then, with no row, an unlocked check whether it exists, so that no exception block is needed); when the row
  is held, or absent (its insert can wait for a concurrent inserter), it takes a tenant wait slot and a replica wait
  slot (below) and waits for the row, inserting it when absent; then it takes the writer's own quota lease
  `NOWAIT`, so that nothing waits after the usage row; reads L itself; reclaims the room of every other quota lease no
  open transaction holds when the free room is under max(*n*, 1 MiB); folds the writer's own quota lease whole (its used
  bytes to S, its room back to the free room; inserted when absent); and grants it g = max(*n*, min(1 MiB,
  ⌊F / (2(*A* + 1))⌋)), F = L − S − G and *A* the tenant's other quota leases holding a grant, when that fits under L,
  charging *n* there. Otherwise it raises `tenant_step_log_quota_exceeded`, or `tenant_step_log_quota_held` when the room
  missing is in quota leases open transactions hold.
- **The wait slots** (decisions D041-125, D041-134 and D041-137). A refill waits for the usage row only while it holds
  two advisory locks, both tried in order and never waited for: one of the tenant's *P* wait slots on its replica, then
  one of the replica's *Q* wait slots, which all tenants share. With N the replica's pool (`SAGAT_DB_MAX_CONNECTIONS`:
  10 in the binary, 20 in the Helm chart), *Q* = max(1, ⌊N / 4⌋) and *P* = ⌈*Q* / 2⌉: 5 and 3 at a pool of 20, 2 and 1
  at 10 and at 8. With no slot of either kind free the refill raises `55P03` at once. The keys are single 64-bit
  advisory keys, `hashtextextended(…, 0)`, built inside the function: a tenant slot from (tenant, `application_name`,
  slot), a replica slot from (`application_name`, slot). *Q* reaches SQL per transaction, never as a session setting:
  the store passes its replica's *Q* in every transaction that may wait, in a statement it already sends (the carry
  call's argument `p_wait_slots`, which the function sets with `set_config('sagat.step_log_wait_slots', …, true)`
  before its writes; the select list of a batch's T1; the first statement of the script marker's transaction), so no
  statement is added and the value travels with its transaction through a pooler in transaction mode, where a setting
  made at connect would stay on whichever server backend ran it. `sagat_quota_step_log_wait_slots()` reads the
  transaction's value, clamped to 1 to 64 (1 when unset); *P* is computed from it. PgBouncer passes
  `application_name` to the server backend of each transaction, so the slot keys hold behind it too. So:
  - one tenant's waits on its usage row hold at most *P* of a replica's connections, and all tenants' together at most
    *Q*;
  - on a replica's own pool (direct connections, or a pooler in session mode), step-log work holds at most the Logs
    cap ⌊N / 2⌋ plus *Q* of its N connections, so at least N − ⌊N / 2⌋ − max(1, ⌊N / 4⌋) are never held by step-log
    waits or Logs calls, for task leases, heartbeats and reports that do not wait: 20 − 10 − 5 = 5 at a pool of 20,
    10 − 5 − 2 = 3 at 10, 8 − 4 − 2 = 2 at 8 (a top-up, or a refill that gets the row at once, holds a connection only
    for its statement; at N of 2 or less nothing is left, which the runbook states);
  - not bounded by it: reports of one run queued on its run row while a sibling's fallback waits under that lock; they
    wait at most the longest fallback wait plus one report's time, which task T041-37's fan-out regime measures with a
    tenant of its own beside it;
  - behind a pooler in transaction mode the bound is per client pool; the server backends are bounded only when the
    pooler's pool for the database is at least Σ over the replicas of (⌊N / 2⌋ + *Q*) plus a reserve R of at least one
    backend a replica: 8 replicas of 20 need 8 × (10 + 5) + 8 = 128; a smaller server pool lets the replicas' step-log
    work take every server backend, and heartbeats then queue in the pooler (the runbook states the rule);
  - the replica cap couples tenants through loss: while other tenants' waits hold all *Q* replica slots, a carry of a
    tenant far from its limit whose fallback finds its own usage row held at that moment (by its own commit or batch)
    answers `service_busy` at once, where it would have waited up to *b*; a loss for others' load, never a wrong
    charge, counted and shown as any busy answer, and needing both a fallback (0.31% of ten-line and 2.0% of
    64-record carries far from the limit, task T041-35's fourth run) and its own row held; the price of a reserve no
    tenant's quota state can take, measured for the tenant far from its limit in task T041-37's three-tenant regime;
  - only refills that may wait take slots: a carry's fallback, a batch's T8 and the script marker's insert; the top-up
    takes none (it never waits).

  These are the one exception to "no session setting is read": the slot keys read `application_name`, which the
  server's pool sets at connect, and *Q* reads the transaction-local `sagat.step_log_wait_slots` the store sets. The
  runtime role could change either, which only changes which slots its session shares or how many it may take on its
  own replica; the bound protects the pool against load, not against the runtime role, which is trusted code. Two
  tenants whose keys collide only share slots (more busy answers, never a wrong charge); a replica whose
  `application_name` is not its own shares its slots with the replicas of the same name (fewer waits allowed than
  intended, never more for one name). Tested with a second tenant as the victim and with the replica cap reached in task
  T041-36, and measured in task T041-37.
- **The budget** of each lock acquisition of a refill on the usage row is *b* = 200 ms: `lock_timeout` set for that one
  statement from `sagat_quota_step_log_refill_wait()`, a function no caller can change (it reads no session setting), a
  caller's lower `lock_timeout` kept (0 counts as none) and the caller's own value restored after it
  (`a_refill_restores_the_callers_lock_timeout`). *b* is a latency budget, not a worst case: it is the budget an ingest
  batch already has on the same row (section 2, T8), against measured holds of the row of p50 1.1 ms and p99 4.5 ms
  (D041-106); `lock_timeout` applies to each lock acquisition, so a wait spanning several holders can exceed *b*, and
  the statement timeout (5 s, 3 s for some transactions) is the hard bound: past it the carry call fails (`57014`) and
  the report is re-run without the carry, its records counted `failed` (section 2a). Measured with every refill inside a
  report (task T041-35's fourth run, far from the limit, direct connections): the longest refill-written carry took
  726.7 ms, 3.6 × *b*, in 64-record carries at 1 × 16, its run row held throughout; task T041-37 records the longest
  refill wait and the longest run-row hold with a criterion. Past *b*, or with no slot free:
  - in a carry, its block catches the `55P03` and records a mark-only final mark whose records are dropped as
    `service_busy` and whose stream says `not_stored_reason = service_busy` (decision D041-129), which the streams route
    returns as `notStoredReason` and Studio states as "not stored: the log service was busy (not your quota)" (section
    8); counted by the store from the carry's returned cause (`step_log_quota_refusals_total{cause="busy"}`), logged as a
    structured warning naming the tenant at most once a minute per tenant and replica with the count since the last,
    and alerted on its rate (`SagatStepLogServiceBusyDrops`); the report commits
    (`a_refill_past_its_budget_answers_busy_and_the_report_lands`);
  - in a batch, it is `RETRY_LATER`, counted as a lock-timeout retry, not alerted (an ingest's ordinary backpressure);
  - for the script marker, `skipped_quota`.

  So no step-log refusal fails, blocks or retries a step (ADR-0084 decision 7, ADR-0095 decision 4); a loss for no quota
  reason is possible only past *b* or with no slot free, visible to the reader and the operator, and accepted by the
  owner (gap G041-026, question O041-041: OD-2026-11-20-14).
- **The top-up after the report** (decision D041-124, OD-2026-11-20-18; ADR-0097 decision 3b). The carry returns the
  room it left in its writer's quota lease and `top_up`, true when that room is under the top-up mark *M* = 65,536 bytes
  (both computed by the database; null room and false when it charged nothing). After a carrying report commits with
  `top_up` true, the store runs, on the same connection (so the same backend with direct connections or a session
  pooler), off the run lock, one statement in its own implicit transaction: `sagat_quota_step_log_top_up(tenant)`, with
  the principal's tenant. The store hands the connection to a task that runs the statement and then returns the
  connection to the pool, so the report's answer to the worker does not wait for it. sqlx's `Transaction::commit`
  consumes the transaction and gives the connection back to the pool, and sqlx 0.9's `PoolConnection::close_on_drop()`
  can be set but never cleared, so the run and path transactions own their connection instead (decision D041-137), one
  acquire per transaction: they hold the `PoolConnection<Postgres>` and send `BEGIN`, `COMMIT` and `ROLLBACK`
  themselves, in a state that is `idle`, `open` or `unknown` (`unknown` from the moment one of those three is sent until
  it returns `Ok`). The top-up is the last statement on that connection before it goes back to the pool. A transaction
  dropped while `open` (a contended claim, a refused or replayed report, any early return: the ports roll back by
  dropping) or while `unknown` (a cancelled `COMMIT`, a `COMMIT` that failed) moves its connection into a spawned task
  that sends `ROLLBACK` (sqlx first drains whatever statement was in flight) and then drops the connection back into
  the pool; only when that `ROLLBACK` fails, takes longer than 5 s or finds no runtime does the task mark the connection
  `close_on_drop()` before dropping it, so it is closed. No connection returns to the pool inside a transaction, and
  none is closed on an ordinary drop: closing would churn backends, and each new backend is a new writer whose next
  carry falls back and whose old quota lease waits for the purge (task T041-36). The function takes no caller amount:
  it tries the step-log fence shared (held: `contended`); answers `room` when the writer's quota lease already has at
  least *M*; tries the usage row without waiting (`FOR UPDATE SKIP LOCKED`, so it opens no subtransaction), taking no
  wait slot (held or absent: `busy` at once; absent is unreachable after a charging carry, whose charge created the
  row); never reclaims another quota lease; answers `limit` when the share, min(1 MiB, ⌊F / (2(*A* + 1))⌋), is 0 (this
  test comes before the next); grants only when the share is above the writer's current room, folding the writer's
  quota lease and granting exactly that share with no charge (so the held-room bound below holds unchanged), and
  otherwise answers `room`. Its answers are `room`, `made`, `limit`, `busy` and `contended` (decision D041-137: it has
  no `held` answer, since it reclaims nothing), and it never raises for them. Near the limit a top-up follows nearly every carry, but it never takes
  another backend's room or a wait slot (decision D041-134). Any other error is the store's `error`. The
  store counts every answer (`step_log_quota_top_ups_total{outcome}`), and neither fails nor re-runs anything because of
  it.
  - *Why M = 65,536 bytes.* A carry's reservation is at most its records' stored sizes plus 128 a row plus 128 for a
    marker. For records whose stored text is no longer than their encoding (16 KiB a carry), that is at most 16,384 +
    65 × 128 = 24,704 bytes, under *M*, so the next such carry after a top-up takes the fast path. Records whose stored
    size exceeds their encoding (number and redacted fields grow in text) can need more and fall back, as any short room
    does. Far from the limit (a share of 1 MiB) a backend tops up once per 1,048,576 − 65,536 = 983,040 bytes charged:
    for ten-line carries charging 3,280 bytes each, once per 983,040 / 3,280 = 299.7 carries.
  - *What still falls back.* The first carry of each (tenant, backend), after a deploy, a pool recycle or an idle quota
    reclaim (its refill's grant leaves room of about its share less its charge, so no top-up follows it); a carry after
    a batch on the same backend that left less room than the carry reserves (with the room spread over (0, 1 MiB], about
    3,408 / 1,048,576 = 0.33% of ten-line carries that follow a batch); a carry whose room a quota reclaim took near the
    limit or by a limit change; and, behind a pooler in transaction mode, a carry on a backend no top-up reached.
  - *Session affinity is not required* (decision D041-130, gap G041-027, accepted by the owner, OD-2026-11-20-15). Behind
    a pooler in transaction mode the top-up can run on another backend; it then tops up that backend's quota lease if it
    is short, which is still correct, and the carry backend's next carry may fall back. Capture then falls back about as
    often as task T041-35's fourth run measured with every refill inside a report (0.31% of ten-line carries and 2.0% of
    64-record carries far from the limit), a regime that passed every gate at 8 × 128. The reason
    `docs/runbooks/scaling.md` gives for a pooler stands, and the step-log runbook states this trade. ADR-0097 option 1c
    (a quota lease per connection slot) is the named alternative if task T041-37's pooler regime shows carry fallbacks
    above half of all refills, or busy carries above 0.1%.
  - The top-up runs only after a report committed, so after its authorization and C0: a forged task lease, a revoked
    worker or a runner token never causes one. A source-text check of task T041-36 fails when anything but the store's
    top-up calls `sagat_quota_step_log_top_up`.
- **Why the waits cannot deadlock.** No transaction waits for a quota lease it does not own: other writers' quota
  leases are taken `SKIP LOCKED` (a refill's or the purge's quota reclaim) or `NOWAIT` under the step-log fence held
  exclusively (a limit change), and the writer's own quota lease `NOWAIT` after the usage row (only a holder of the usage
  row, or a limit change holding the fence, could hold it). The wait slots are only tried. No holder of the usage row
  waits after taking it for anything a waiter holds: a content or log finalizer is its commit's last step; a refill then
  takes only its own quota lease `NOWAIT` and writes rows only its transaction touches; a release and a quota reclaim
  take nothing more; `sagat_quota_lock_log_usage`'s holders (`content_usage.rs`: scripts, dispatch, connectors) take the
  usage row and then write their own rows, waiting for no quota lease and no run lock of a carry; a limit change holds
  the fence exclusively, which no refill waiting for the usage row can hold at the same time. Tests with held
  transactions and `pg_stat_activity`, each checking 0 ≤ U ≤ S + G ≤ L after:
  `a_completion_holding_the_usage_row_refills_while_another_refill_waits`,
  `a_writer_holding_its_lease_waits_for_a_reclaiming_refill_without_a_deadlock` (which a quota reclaim waiting for a
  held quota lease turns into a stall, mutation 3 of the writer leases in validation-report.md),
  `an_ingest_refill_and_a_carry_refill_wait_for_each_other_in_turn`,
  `a_release_and_a_limit_change_around_a_waiting_refill_end_without_a_deadlock`,
  `a_limit_change_waits_for_an_open_idle_reclaim_instead_of_failing`.
- **Room held by open writers, bounded.** A grant's unused room is at most its share, min(1 MiB, ⌊F / (2(*A* + 1))⌋)
  at the grant; the rest of a grant of *n* past its share is the planned charge itself, charged at once by the refill
  that grants it (a top-up grants its share and no more). So open writers hold at most the sum of their grants' shares
  plus the planned charges of carries that reserved and have not charged yet. When *k* writers take fresh grants one
  after another for charges within their share, the *i*-th finds at least *i* − 1 other quota leases holding a grant, so
  F_i ≥ F_{i−1} (1 − 1/(2*i*)); the product of (2*i* − 1)/(2*i*) for *i* = 1..*k* is C(2*k*, *k*) / 4^*k*; so at least
  F₀ · C(2*k*, *k*) / 4^*k* (> F₀ / √(π(*k* + ½))) of the room F₀ free before them stays chargeable by another writer
  while they hold their quota leases (`room_held_by_open_writers_is_bounded_from_the_free_room`: *k* = 1, 2, 8, 32 from
  a quarter-used limit; mutation 10 of the writer leases grants F / (*A* + 1) and fails it). For an operator: a charge
  of *n* is refused as held only when the unleased room is under *n* and the tenant's *k*_open open charging
  transactions hold at most *k*_open × 1 MiB plus their planned charges. A held refusal of a carry records a mark-only
  final mark with `tenant_quota`, which Studio states as "the tenant's log quota is full or its last room is in use by
  other steps" (G041-023). Idle quota leases give back all their room at the next refill near the limit, so a small
  quota with many idle writers admits everything their quota leases hold
  (`a_small_quota_with_many_idle_writers_admits_everything_their_leases_hold`).
- **Idle and ended quota leases.** The purge pass (task T041-10) folds and deletes, one tenant a transaction, every
  quota lease not charged for 10 minutes and every quota lease whose backend has ended (its pid absent from
  `pg_stat_activity`), `SKIP LOCKED`: it finds those tenants through `sagat_quota_step_log_idle_tenants(limit)` (tenant
  ids only, a bounded count), checks for such a quota lease without a lock, tries the step-log fence shared (a tenant
  whose fence is held is skipped this pass and answered `skipped`, so that holding a quota lease always means holding
  that fence, which a limit change's `NOWAIT` relies on), then takes the usage row; it answers `(reclaimed, skipped)`. The interval is fixed in the function; it is the connection
  pools' idle timeout (sqlx's default, kept by the platform): a pooled backend idle longer is closed, so its quota lease
  could only strand room; a live writer charging less often refills again, by a fallback or a top-up. A tenant's quota
  lease rows are then at most the backends that charged for it within 10 minutes and are still alive; a near-limit quota
  reclaim zeroes the quota leases it folds, and only quota leases holding a grant count in *A*.
- **A release** (the purge's `DELETE`, unchanged: per-row delta and deferred flush) never waits for a quota lease: under
  the usage row it reads Σ `used`, refuses an underflow of U, and sets S −= the released bytes.
- **A change of the limit** of `tenant_step_log_bytes` or its ceiling reclaims every quota lease of the tenant in the same
  transaction, by a trigger on `quota_limits` and `quota_ceilings` that takes the step-log fence exclusively itself (so no
  quota lease is held), then the quota leases `NOWAIT`, then the usage row: a lowered limit leaves no stranded room,
  whoever wrote it.
- **Exactness.** A charge moves bytes from the leased room to U and changes neither S nor G; a grant commits only with
  S + G ≤ L; so U ≤ S + G ≤ L at every commit: the usage never exceeds the limit in force when the charge was admitted,
  ADR-0027's exactness today, with no overshoot to bound. A charge can be refused while room exists only in quota leases
  that open transactions hold, at most that room (bounded above) and only until they end (G041-023). Such a refusal is a
  counted drop (`tenant_quota` for the reader, `cause="held"` in the metric), never an overshoot.
- **Where a refusal lands.** The charge runs in the insert's statement trigger, so a refusal raises at the insert, not at
  the commit: T8 of a batch (mapped to `RETRY_LATER`, as T10's finalizer refusal was) and CD of a carry, which a reserved
  carry never meets and a carry without a reservation catches in its own block, recording a mark-only stream in the same
  call (section 2a): no report is re-run for a quota refusal. The script marker's insert raises the same way and is
  counted `skipped_quota`.
- **The regime near the limit and for small tenants.** A grant reaches 1 MiB only while F ≥ 2(*A* + 1) MiB; far from
  the limit, a writer refills once per grant (the top-up's figures above). A tenant with less volume per backend per 10
  minutes, a small quota, or one near its limit refills more often, by top-ups and by fallbacks inside reports (each with
  its block's subtransaction); near the limit the share falls under *M*, so a top-up follows nearly every carry of a
  backend (it never takes another backend's room or a wait slot, and grants only when its share is above the room the
  backend has: decision D041-134), and other refills' quota reclaims can take a backend's room before its next carry
  reserves it, so fallbacks
  are frequent there (a fresh grant is not protected from the quota reclaim: ADR-0097 option 1g). Task T041-37 measures
  these regimes with a criterion each: near the limit with a purge beside it, a small quota with many backends, many
  low-volume tenants (the transaction-id rate and `pg_stat_slru` for `subtransaction`), a cold start, behind a
  transaction pooler, a fan-out run during fallback waits it causes on purpose, and a second tenant as the victim of the
  first's waits.
- **Observability** (data-model.md §7; one producer for each metric, decision D041-128):
  `step_log_quota_fallbacks_total` (a carry that returned `path = refill`, counted by the store),
  `step_log_quota_top_ups_total{outcome="room"|"made"|"limit"|"held"|"busy"|"contended"|"error"}` (counted by the
  store), `step_log_quota_refusals_total{cause="limit"|"held"|"busy"}` (counted by the store, from the carry's returned
  cause and from a batch's T8 raise by its constraint name; a batch's `55P03` is a lock-timeout retry, never `busy`),
  `step_log_purge_failures_total{cause="underflow"}`, and, from SQL sequences that the replica holding shard 0 reads
  through a definer accessor (`sagat_quota_step_log_counters()`, task T041-10), `step_log_quota_refills_total{kind=
  "refill"|"reclaim"|"idle"|"ended"}` and the gauge `step_log_quota_lease_rows`; sequences are not transactional, so
  those count attempts, rolled-back ones included. Labels are closed sets with no tenant, and nothing inside SQL updates
  a counter row. A quota lease's backend is attributed to a replica through `pg_stat_activity`, by the per-replica
  `application_name` task T041-5 sets (`sagat-octopus-orchestrator/<instance id>`; today every replica sets the same
  one).
- **What it costs** (the budget lines, pinned on the prototype by `tests/performance/scalability-ha/step_log_leases_tests.rs`
  and by task T041-36's equalities on the built carry): an insert charged into a free quota lease writes one quota lease
  row and no usage or delta row, runs six nested statements for its charge (the transition-table read, the charge call,
  the three fences in one statement, the limit's read, the unlocked read of S and G, the update of the writer's quota
  lease) besides the one foreign-key check per stored row, and queues no deferred call for the commit; a refill adds one
  write of the usage row, the insert or fold of the writer's quota lease, and one write of each quota lease it reclaims.
  On the prototype a short step's carry of ten records on the fast path ran 26 nested statements and no subtransaction
  (section 2a gives the list; `only_a_carry_that_needs_a_refill_opens_a_subtransaction`); the built function's figure is
  derived again by task T041-36. A report that stores output still takes the usage row at its commit for the content
  class, as ADR-0027 does today (G041-024).

### 3. Storage

Two tables, in one migration of T041-5 (data-model.md §2 gives the DDL): `step_log_streams` (one row per stream, with
its counters and expiry) and `step_log_lines` (one row per stored line or marker), the lines with a run-scoped and a
stream-scoped read index.

**What a write costs, measured (D041-107, D041-111).** At 1 replica and 16 workers on the hosted runners the carry's
fences, stream lock and policy read cost nothing measurable, its stream row 5.0% of the completion rate (+2.1 ms of p50)
and its ten line rows 2.2% more (+1.3 ms); storing the lines without the secondary indexes, without the foreign key, as
one chunk row or unlogged moved the rate by less than the noise, while a chunk row wrote 33% less WAL
(validation-report.md, 2026-11-20). So the lines stay one row each, which the readers, the purge's per-row release and
the cursor rely on, and the stream row is cut instead: a carry that creates its stream inserts it once, with the
counters, final mark, expiry and order guard its plan computed, and updates nothing (section 2a); the table keeps its run
index (D041-112: dropping it saved one index entry a stream, unmeasured, against a sort of all a run's streams per page);
and its expiry index is keyed on `expires_day`, the whole days since the Unix epoch of `expires_at`, a
stored generated column, with a fill factor of 70, so that a batch or a carry on an existing stream that keeps the day
changes no indexed column and is a HOT update (no new index entry) on every update that does not cross a UTC midnight.
The fill factor costs about 43% more heap for the stream table; T041-9 measures its size per row and the HOT ratio of a
long step's updates with it and without it. Chunk rows were rejected (D041-111): they would change the read paths, the
cursor, the markers and the release for less WAL, with no measurable gain in rate (task T041-35's fourth run then
measured the carry with one row a line, decision D041-122).

**No foreign key to `runs` or `steps`.** An insert that references `runs` takes `KEY SHARE` on the run row, which
conflicts with the `FOR UPDATE` the engine holds while it leases or completes. The run's existence in the tenant is proven
by T2 of the ingest, or by the report's own lease for a carry, in the same transaction, and nothing deletes a run today. Feature 020 must delete a run's streams,
lines and stream rows when it deletes the run (G041-004, registered in feature 020's research as G1618). `step_log_lines`
references `step_log_streams` (same tenant), a row the engine never locks. A trigger on `step_log_streams` refuses an
update that changes its identity or retention or clears a removal, final or delete time (data-model.md §2), so no
writer can undo a delete.

**Accounting** (ADR-0027's Amendment of 2026-11-15, accepted, OD-2026-11-15-16, and its Amendment of the Amendment of
2026-11-20, OD-2026-11-20-16: the charge measured once per insert statement and made to the writer's quota lease, the
release per row). `step_log_lines.message`,
`.exception` and `.fields` are of the new class `step_log` in `crates/infra-postgres/storage-classification.json`, and
`.level` carries the fixed charge of 128 bytes (`fixed_octets`, OD-2026-11-15-6); every other variable-size column of
the two tables is `protected_system`, so a marker costs its 128 bytes and nothing more. They count against
`tenant_retained_usage.step_log_bytes` and `tenant_step_log_bytes`, not against `tenant_log_bytes`, so a step that logs
a lot never takes the allowance of script logs, dispatch errors and connector errors (OD-2026-11-15-5). The usage column
comes with the resource in T041-4, so that its usage view has a column to read; T041-5 adds the rest of the Amendment's
accounting (data-model.md §3) and creates the triggers the manifest generates on `step_log_lines` only, the one table
with accounted columns.

**Grants** are those of data-model.md §4, and this paragraph is the design's one list of them (NFR-041-SEC refers to
it): no `UPDATE` on lines, no grant on `namespaces`, no `TRUNCATE`, and no `SECURITY DEFINER` function beyond the
accessor `sagat_quota_step_log_allowance` of ADR-0027's Amendment, the statement-level quota trigger function
`sagat_quota_step_log_insert_trigger` of its Amendment of the Amendment of 2026-11-20 (no `EXECUTE` to `sagat_app`:
only its trigger calls it), and ADR-0097's quota-lease functions (ADR-0097 decision 1): `sagat_quota_step_log_charge`,
`sagat_quota_step_log_refill`, `sagat_quota_step_log_refill_wait` and the limit-change trigger function
`sagat_quota_step_log_reclaim` (executable by neither `PUBLIC` nor `sagat_app`, revoked explicitly), and six definer
functions `sagat_app` may execute: `sagat_quota_step_log_reserve`, `sagat_quota_step_log_top_up`,
`sagat_quota_step_log_usage`, `sagat_quota_step_log_counters`, `sagat_quota_step_log_idle_tenants` and
`sagat_quota_step_log_reclaim_idle` (`EXECUTE` to `sagat_app`, revoked from `PUBLIC`). Two helpers are `SECURITY
INVOKER`, not definer functions, and `sagat_app` may execute them: `sagat_quota_step_log_top_up_mark()` (`IMMUTABLE`,
65,536) and `sagat_quota_step_log_wait_slots()` (`STABLE`, decision D041-134), revoked from `PUBLIC`; the counter sequences revoked from `sagat_app`; no privilege on `step_log_quota_leases` for `sagat_app`,
revoked explicitly; the carry function `sagat_step_log_carry` is `SECURITY INVOKER`, `EXECUTE` to `sagat_app` only;
the accessor is revoked from `PUBLIC`; the functions the migration replaces keep their `SECURITY DEFINER`, pinned
`search_path` and grants, tested in T041-5. The runtime
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

1. **Mark.** Streams with `removed_at` null, `expires_day` at most today's and `expires_at` past
   (`step_log_streams_expiry_idx` on `expires_day`, the exact time rechecked) are locked
   `FOR UPDATE SKIP LOCKED`, a page at a time, and get `removed_at`. From then on readers show the stream as expired and
   return none of its lines, and T4 of the ingest answers `REMOVED` (and CC of a carry stores nothing) under the same
   row lock, so no batch or carry can add a line to a stream whose removal has begun. The delete route sets `deleted_at` and `removed_at` together.
2. **Delete.** For streams with `removed_at` set and `purged_at` null (`step_log_streams_removal_idx`), at most 500 lines
   per transaction are deleted, of one tenant per transaction; when none is left, `purged_at` is set. Deleting releases the
   quota through the triggers (under the usage row, never waiting for a lease, ADR-0097), and 500 rows per transaction
   bound how long the purge holds the tenant usage row. "Today" for the mark is the day number of `now()` computed by the
   same expression as `expires_day` (UTC), so no time zone delays a mark; T041-10 measures one pass with a day's worth of
   streams not yet expired.

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
The `Quota` description states one more case in which `used` is absent: `tenant_step_log_bytes` shows no `used`, and
`usageWithheld: true`, to a caller who does not hold both `run:logs:read` and the PII reveal grant in every namespace
of the tenant (fail-closed), so that neither a viewer, who holds `quota:read`, nor a reader without a namespace's grant
can watch how much the tenant's runs log; the resource and its limit are still shown (ADR-0085 decision 6;
OD-2026-11-20-19, decision D041-133; T041-4).

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
   an id below any end horizon (section 5); its select list also sets the replica's wait slots *Q* for the transaction
   (`set_config('sagat.step_log_wait_slots', $q, true)`, section 2b).
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
6. The marker's insert is charged to a lease (section 2b); if the charge raises `tenant_step_log_quota_exceeded`, the
   transaction rolls back, no marker is stored and the metric counts
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
and by the lease charge's raise, the execution is recorded and no line is stored; a marker transaction made to fail (a
`BEFORE INSERT` trigger the test installs on `step_log_streams`) leaves the report answered as kept and counts `failed`,
also while the tenant usage row is held (a marker whose charge needs a refill waits for it under the marker's lock timeout); the marker stream expires no later than the execution, with a retention
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
  flush: the last lines may be missing"; "some lines are delayed" while `heldBack`; markers as rows with text; a stream's
  `ceiling` as a line of text above its lines, as the markers are: "some lines were not stored: the attempt's line
  ceiling" (`attempt_records`) or "…byte ceiling" (`attempt_bytes`), "some lines were not stored: the tenant's log
  quota is full or its last room is in use by other steps" (`tenant_quota`); and a stream's `notStoredReason` the same
  way: "not stored: the log service was busy (not your quota)" (`service_busy`, decision D041-129); each with its drop
  count.
- The step-log quota's usage: Studio has no quota screen today; the quota route returns `usageWithheld: true` and no
  `used` for `tenant_step_log_bytes` to a caller without `run:logs:read` and the PII reveal grant in every namespace of
  the tenant (OD-2026-11-20-19), and any later screen shows "usage hidden" then, never a figure.
- A script marker row expands in place into `ScriptLogPanel.tsx`, which calls `getScriptExecution` (through
  `src/api/endpoints.ts`) and shows the execution's log as text, or "log withheld" when `logWithheld` is true.
- Keyboard: the tab follows the existing `Tabs` pattern; the list is one tab stop with arrow, Page, Home and End movement
  and Enter to expand a row; focus is not moved by arriving lines.
- Types come from `npm run api`; `npm run check:api` passes.

### 9. Observability

The metrics of data-model.md §7, with closed labels. Alerts in `infra/observability/alerts.yaml`, with entries in
`docs/runbooks/alerts.md`: `SagatStepLogIngestShedding` (`retry_later` above 5% of batches for ten minutes),
`SagatStepLogPurgeBehind` (purge backlog growing for an hour), `SagatStepLogQuotaHeldRefusals`
(`step_log_quota_refusals_total{cause="held"}` above 1% of `step_log_carries_total` for ten minutes: room stranded in
held quota leases, the runbook says to look at refills and the quota-lease-row gauge), `SagatStepLogServiceBusyDrops`
(`step_log_quota_refusals_total{cause="busy"}` above 0.1% of `step_log_carries_total` for ten minutes: carries'
records dropped as `service_busy` because the quota could not be checked within its budget or no wait slot was free;
the store writes a structured warning naming the tenant at most once a minute per tenant and replica, with the count
since the last, which the runbook points to, with the usage row's holders, the top-ups and the fallbacks to look at;
decision D041-128) and `SagatDatabaseOldTransaction` (the oldest open transaction older than five minutes). The quota
leases of ADR-0097 add `step_log_quota_fallbacks_total`, `step_log_quota_top_ups_total{outcome}`,
`step_log_quota_refusals_total{cause}`, `step_log_quota_refills_total{kind}`, `step_log_purge_failures_total{cause}`
and the gauge `step_log_quota_lease_rows` (data-model.md §3b and §7). The gauge `database_oldest_transaction_age_seconds` is a read of the operations
snapshot built as the `count_and_age` reads of `crates/infra-postgres/src/operations.rs:437-448`, over `pg_stat_activity`
(`xact_start`) and `pg_prepared_xacts` (`prepared`). The runtime role sees `xact_start` only for its own role's sessions
unless the deployment grants it `pg_read_all_stats`, so a long transaction of another role (a backup, a person in
`psql`) can hold the read horizon unseen by the alert (G041-016); the reader still sees `heldBack`, and the runbook
names the grant. Drops at a ceiling are the customer's code and are shown on the run, not alerted. The ingest has a
span, `worker.append_step_logs`, with counts and the verdict and no text; a carry adds counts and its outcome as
attributes of the report's existing span, and no text. The runbook `docs/runbooks/step-logs.md`
(written with task T041-35's record, modified by T041-10 and T041-30 and completed by T041-25) covers the four resources and how to size
`tenant_step_log_bytes` against the retention and the log rate, the deployment switch and its use during a rolling upgrade (section 10), the purge, what each state and marker means, that a change of retention applies to new streams and a change of capture or a
ceiling to new batches, that capture turned back on reaches a worker at its next registration, the PII grant every
reader needs, the PostgreSQL settings of section 5 (with `log_error_verbosity = terse`, `log_parameter_max_length = 0`
and `log_parameter_max_length_on_error = 0` for the runtime role, so neither a slow carry call nor a database error can
copy line text into PostgreSQL's own log), the admission defaults and when to raise them, the top-up and the trade
behind a transaction pooler (decision D041-130),
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
| Capture on by default costs every tenant the measured throughput at small deployments without opting in (at 1 × 16 about 18% of the rate and +31.9% to +36.9% of a carrying completion's p99) | The owner accepted the measured cost of capture as the feature ships, on by default (O041-001), with OD-2026-11-20-13; the levers are the namespace switch (`step_log_attempt_records = 0`) and the deployment switch `SAGAT_STEP_LOGS_ENABLED`, documented in the runbook and the release notes; SC-041-007's floor at 1 × 16 catches gross regressions (its (2b) floor admits about +94% of p99), and task T041-9 reports to the owner a carrying completion's p99 above the accepted +36.9% |
| The six SDKs' redaction drifts apart | One rule (`redact`) and one canonical number form in `sdks/SPEC.md`; `step-logs-redaction` with the same goldens in six languages |
| A hand-written 1.7 client skips redaction | ADR-0085 rule 12 states it; the server's shapes and sensitive names still apply; the permission and the grant bound who reads |
| Log batches make completions wait | Charges go to quota leases and the usage row only to a refill (ADR-0097); no lock a report, task lease or heartbeat takes before L1; per-tenant tickets and wait slots; the queue bound of section 2, shown by the quota-lease tests of T041-6; the effect on time that SC-041-007 bounds per size, measured by T041-35 before the schema and T041-9 on the built carry |
| Each logging step adds transactions | A short step (see Terms) adds none before or in its report: its lines ride on it (OD-2026-11-19-1); a top-up after a carrying report runs about once per 300 ten-line carries of a backend, off the run lock; a longer step adds about one batch a second while it runs; per-tenant tickets and shedding; a step that logs nothing sends nothing |
| The carry adds work under the run lock to every report that carries records | One call of a database function (one round trip), an equality of the budget (section 2a); at most 64 rows, one statement-trigger call charging the writer's lease, a subtransaction only in the fallback, no deferred call (OD-2026-11-20-1, ADR-0097, D041-112); the CPU work (validation, scrubbing, sizing) is done before the transaction; T041-35 measures a ten-record and a 64-record carry |
| A tenant's step-log charges queue on its one usage row (measured: held to the end of each commit's WAL flush, D041-106) | Exact per-tenant leases (ADR-0097, section 2b): a charge takes a lease `SKIP LOCKED` and the usage row only to refill; the invariants, held-lock tests and budget line of T041-5; a charge refused while room sits in open transactions' leases is a counted drop (G041-023); completions with output still queue for the content class (G041-024) |
| The carry's own writes cost the completion rate (measured: the stream row 5.0%, the lines 2.2%, D041-107) | A new stream written once and later updates HOT (D041-111); T041-35's fourth run measured the result: every gate passed at 8 × 128 and the cost at 1 × 16 was accepted (OD-2026-11-20-13), so SC-041-007 is a criterion per size (D041-126) |
| A carry makes its report fail or loop | Everything that can be checked is checked before the transaction and an invalid carry is ignored; a refusal of the carry's charge, or a fallback refill past its 200 ms budget or with no wait slot free, is answered inside its call by a mark-only stream that says why (one subtransaction, D041-119); a guard's raise or a failed carry call re-runs the report without the carry's lines, at most four transactions in all (section 2a); tested by T041-36 |
| A report would wait on a step-log lock while it holds the run row (an ingest of its own attempt, the purge or the delete route on its stream, a change of a step-log resource) | A carry only tries its step-log locks (the policy fences, the attempt's stream lock, the stream row) and, when one is held, writes nothing more (`contended`), so the report never waits for them and commits in its first transaction; the SDK reports only after its batch in flight is answered or 2 seconds passed, so the case is rare; tests in T041-36 |
| A fallback refill waits for the usage row under the run lock, so the run's other reports, heartbeats, cancels and signals queue behind it | Measured far from the limit with direct connections and every refill inside a report: 0.31% of ten-line carries and 2.0% of 64-record carries refilled, the longest refill-written carry 726.7 ms, 3.6 × *b* (T041-35's fourth run); the top-up after the commit moves the steady refills out of reports (D041-124); the row tried without waiting first, each lock acquisition at most 200 ms and only with a tenant and a replica wait slot (3 and 5 at a pool of 20, D041-134), the statement timeout the hard bound; past *b* or with no slot the carry answers `service_busy` and the report commits; a report waits for the usage row at most once before its commit; counted (`step_log_quota_fallbacks_total`) and alerted (`SagatStepLogServiceBusyDrops`); T041-37 measures a fan-out run during fallback waits, a cold start and the longest run-row hold, each with a criterion |
| One tenant's waits on its usage row hold the replica's shared pool connections that other tenants' reports and heartbeats need | At most *P* = ⌈*Q* / 2⌉ connections of a replica per tenant and *Q* = max(1, ⌊N / 4⌋) for all tenants together (3 and 5 at a pool of 20, 1 and 2 at the binary's 10; D041-125, D041-134), *Q* passed per transaction (D041-137), so on a replica's own pool step-log waits and Logs calls hold at most the Logs cap plus *Q* and leave at least 5 of 20 connections (3 of 10) to task leases, heartbeats and reports that do not wait; reports queued on a run row behind a fallback are outside that bound and measured with a victim tenant in T041-37's fan-out regime; behind a transaction pooler the server backends are bounded only by the sizing rule of section 2b (Σ (⌊N / 2⌋ + *Q*) plus one a replica), which the runbook states and T041-37 applies; held-lock tests with a second tenant as the victim and the replica cap reached (T041-36), and the two-tenant and three-tenant regimes of T041-37, also behind the pooler, with their criteria fixed before they run |
| Other tenants' waits fill the replica's *Q* slots, so a carry of a tenant far from its limit whose own usage row is held at that moment answers `service_busy` at once instead of waiting up to *b* (the replica cap couples tenants through loss) | Needs a fallback (0.31% of ten-line and 2.0% of 64-record carries far from the limit) and its own row held at once; counted and shown as any busy answer, never a wrong charge; the price of a reserve no tenant's quota state can take (D041-134); T041-37's three-tenant regime gives the tenant far from its limit 64-record carries with output and requires its `service_busy` carries at most 0.1% |
| Behind a transaction pooler the top-up can run on another backend than the carry, so carries fall back more often (G041-027, accepted, OD-2026-11-20-15) | Correct either way; the fallback rate is then about the fourth run's, which passed at 8 × 128; `step_log_quota_fallbacks_total` and `step_log_quota_top_ups_total` show the regime; session pooling is not required and `docs/runbooks/scaling.md` stands (D041-130); T041-37 measures it behind PgBouncer in transaction mode; ADR-0097 option 1c is the named alternative past its declared threshold |
| A carry's records are dropped while the tenant has room (a fallback past its budget or with no wait slot free, or room held by open writers' quota leases) | The reader sees why (`notStoredReason = service_busy`, `ceiling = tenant_quota`) on the stream and in Studio; the held room is bounded (section 2b); the busy case is alerted and accepted by the owner (G041-026, OD-2026-11-20-14); the fourth run counted one busy carry in 864,000 and closed G041-025 |
| Lines of one attempt shown out of sequence order (a carry whose transaction id is below that of a batch stored after it) | The stream records `last_stored_xid`; such a carry keeps no record and counts them (`out_of_order`, section 2a); tested by T041-36 |
| A carry sent to an API replica that predates minor 1.7 is lost | The replica ignores the unknown field and records the report; the stream, if any, shows "ended without its final flush" (G041-020); the runbook says to keep `SAGAT_STEP_LOGS_ENABLED=false` on the new replicas until every API replica runs the release that serves carries, so that no worker is told capture is on while binaries of both releases serve |
| A runner, a database runner or a managed step puts lines on its report | C0 ignores the carry for those roles and for managed steps, as the ingest refuses them (section 2a); tested by T041-36 |
| The re-run of a report whose carry failed is refused (the lease expired or was reclaimed, a cancel, a stopping or finished run, a revoked worker) | Refused as a late report would be; the only cases where capture changes a report's answer, and only after a transaction failed because of the carry, which no contention causes (G041-021) |
| T041-35 measures on a database on the same machine and on the root report path only | Its result says so; T041-9 measures the built carry on both paths; G041-019 and G041-022 |
| A holder of `run:logs:read` without a namespace's PII grant watches `tenant_step_log_bytes` usage move | Closed (G041-018): the usage is returned only with `run:logs:read` and the PII reveal grant in every namespace of the tenant (OD-2026-11-20-19, D041-133), tested |
| The SDK's wait before a report delays the completion of a step that streamed | Only a long step waits, for its batch in flight and its overflow batches, at most 2 seconds; a short step waits for nothing; measured in T041-9 |
| A revoked or re-registered worker | Session (namespace, expiry, token) and lease checked as reports do; binding and membership read without a lock, so a batch committed just after a revocation is stored (accepted, OD-2026-11-16-7); a re-registered worker resends with its new token; T041-8 tests each |
| A long or prepared transaction holds the read horizon | Lines arrive late, never out of order or missing; `heldBack`; the oldest-transaction gauge and alert; the runbook's settings |
| The tail ends before a late batch commits | The end rule of D1958 with a strict comparison and the scan reaching the run's last row; tested on PostgreSQL 16 and 18, each on a server of its own |
| A flood fills the table before the ceilings act | Ceilings from counters before the insert, in the same transaction; drop counts derived from sequences |
| Rows without log bytes escape the tenant quota | Every row is charged 128 bytes on top of its text (OD-2026-11-15-6, D041-054) |
| Step logs fill a quota other writers need | Step logs have their own quota, `tenant_step_log_bytes`; `tenant_log_bytes` keeps script, dispatch and connector logs (OD-2026-11-15-5, D041-053) |
| The script marker makes the script report fail or deadlock | Written after the report commits, in a transaction of its own that takes no lock the report takes; skipped at the step-log quota; a barrier test with an ingest, a delete and a policy change (section 7) |
| Every charge updates its writer's lease row (one per tenant and backend that charged within 10 minutes), which bloats while the horizon is held (no HOT pruning past it) | No index besides the primary key, so a charge is a HOT update; the lease table's fill factor of 50 and per-table autovacuum settings (data-model.md §3b); the idle reclaim; T041-35's (d) records the lease table's size and the completion p99 under a held transaction |
| A carry without a reserved quota lease opens a subtransaction (its block), which takes a second transaction id and makes readers look up `pg_subtrans` while it runs | Only carries that fall back open one (section 2b, "What still falls back": 0.31% of ten-line carries far from the limit, more near the limit, for small or low-volume tenants and behind a transaction pooler); the refill opens none (D041-119); T041-37 reports the transaction-id rate and `pg_stat_slru` for `subtransaction` near the limit, for many low-volume tenants and under a held transaction |
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
| A viewer watches the step-log quota to learn how much a tenant's runs log | `used` of `tenant_step_log_bytes` is shown only to a caller with `run:logs:read` and the PII reveal grant in every namespace of the tenant (D041-066, OD-2026-11-20-19; G041-015 and G041-018 closed) |
| A long transaction of another role holds the read horizon unseen by the alert | `heldBack` on every page; the runbook's `pg_read_all_stats` grant for the gauge (G041-016) |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | A log is not execution state: no transition reads it; lease and heartbeat run no statement on it, and a report makes one call of a database function, only to store its own carry, after the engine has decided and never read by it (FR-041-008, SC-041-001, section 2a); a carry that cannot be stored changes neither the report's outcome nor its output (a downgrade re-run, refused only where a late report would be, G041-021). The lease and fencing token, its expiry, the session, the binding and the membership decide whether a batch is accepted; a stale attempt is refused, never merged (section 2, T2, T3 and T7). The journal and its sequence are unchanged |
| II. Modular domain, ports and adapters | Pass | Domain types and the pure `plan_batch` and `script_marker_decision` in `crates/domain/src/step_logs.rs` with no wire or SQL import; ports `StepLogTx`, `StepLogCarryTx` (a supertrait of the report transactions, section 2a) and `StepLogReader`; scrubbing and sizing in the application; adapters in `infra-postgres`, `api-grpc`, `api-http`. `scripts/check-architecture.py` checks the layering, run by the author before each pull request and by CI at release (the plan's "Gates"), and T041-5 adds two architecture tests (no foreign key to `runs` or `steps`; the hot-path modules never name the log tables, and T041-36 restates the second so that they reach them only through `StepLogCarryTx`) |
| III. External effects are at-least-once; sagas compensate | Pass | Delivery of a batch or of a report's carry is at-least-once and made idempotent by (attempt, generation, sequence) and by the report's idempotency key; nothing here is an external effect |
| IV. Compatibility is versioned and pinned | Pass with conditions | Additive minor 1.7 (ADR-0015; held by 041 in the register of `sdks/SPEC.md` §9, OD-2026-11-15-8), changed in place before any release (D041-090: a field on `CompleteStep` and `FailStep`, two fields of `AppendStepLogsRequest` reserved); older workers and servers behave as today, and an older API replica ignores a carry; four new operations; the six SDKs pass the same five scenarios; the `ScriptExecution` schema change is additive. The condition: viewers get `getScriptExecution` without its `log` (OD-2026-11-13-18), a documented change of behaviour |
| V. Code, tenant boundary and secrets are isolated | Pass with conditions | Tenant in every key and predicate, 404 and audit for another tenant's run, a permission of its own, the PII reveal grant on every read (OD-2026-11-16-8), reads audited, an audited delete. Values the process resolved, written verbatim and at least 6 bytes long, are removed by an official SDK before a line leaves it. Conditions accepted by the owner: G041-001 (O041-011, ADR-0085); G041-008, the cursor shows the transaction counter (OD-2026-11-15-11); and G041-017, the `[REDACTED]` oracle over the process's secret set (OD-2026-11-16-7). G041-018 (a holder of `run:logs:read` without a namespace's PII grant could watch `tenant_step_log_bytes` usage move) is closed by the owner's answer, "Hide it without the grant" (OD-2026-11-20-19). The rows without text are charged 128 bytes each (OD-2026-11-15-6), so no row escapes the step-log quota; the step-log quota's usage is shown only to readers of step logs who hold the PII reveal grant in every namespace of the tenant (D041-066, OD-2026-11-20-19), which closes G041-015 and G041-018 |
| VI. Observability and audit are part of the feature | Pass | FR-041-018: metrics with closed labels, a span without text, typed errors, alerts on shedding, purge, held quota room (`SagatStepLogQuotaHeldRefusals`) and old transactions, the quota leases' counters (ADR-0097), `run.logs_read` and `run.logs_deleted`; the quota leases' second alert, `SagatStepLogServiceBusyDrops`, and one producer for each metric (decision D041-128) |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | No broker, inbox or outbox is involved: a batch is a synchronous call whose answer the worker waits for, and it is safe to deliver twice. Backpressure is designed (bounded buffers, admission and tickets, `RETRY_LATER`, counted drops). A dropped line is counted, not parked, so the dead-letter rule has nothing to apply to (ADR-0084 decision 7) |
| VIII. Failure is tested and contracts are verified, honestly | Pass | The Verification Plan names the crash, duplicate, lost-batch, stale and expired lease, revocation, re-registration, flood, old-worker, lock-order, tail-end and restart tests, a compensation that logs while its run is stopping and after its forward step failed, and a lease that expires while its batch is in flight, a carry refused by the step-log quota (a mark-only stream in the same call) and one whose fallback refill waits longer than its budget or finds no wait slot (a `service_busy` mark-only stream in the same call), a second tenant whose reports commit while the first tenant's waits hold its slots, the top-up's answers, quota exactness under interleaved charges, refills, quota reclaims, releases and limit changes, a lowered limit above the usage, a release beside a held quota lease, a carry call that fails, a lost answer to a carrying report and a carry behind an ingest of its own attempt, with barriers and held locks; each is written first and seen failing on its own assertion before the code it tests exists (the Verification Plan's table, with the task that tests each). What is not proven is listed, with its status, in the Verification Plan's gap list, G041-001 to G041-027 (G041-015, G041-018 and G041-025 closed; G041-009, G041-010 and G041-011 moot) |
| Delivery Constraints | Pass | Rust, tonic, Axum, SQLx, PostgreSQL as the only store; no new component; SC-041-007 is a criterion per size on stated hosted runners |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| Condition on principle V: free text written by customer code is stored and may hold a transformed secret | The feature is the storage of that text; the server never holds the secret values | Re-resolving the attempt's secrets on the server for every batch: a disclosure per batch, each to audit |
| Condition on principle IV: viewers get the script execution without its log | The owner closed the script log to callers without the step-log right (OD-2026-11-13-18) | Leaving it readable under `script:read`, which contradicts ADR-0085 rule 6 |
| Condition on principle V: the `[REDACTED]` oracle over the process's secret set (G041-017) | The secret set lives for the process (OD-2026-11-15-7), so a line is redacted with every value the process resolved; accepted by the owner (OD-2026-11-16-7) | A set per attempt, rejected by the owner (O041-017) |
| Condition on principle V: the read cursor shows the database's transaction counter (G041-008) | The read order is the storing transactions' ids; the owner accepted the plain cursor (OD-2026-11-15-11) | A cursor encrypted with a server key, which no other cursor of the API needs (D041-035) |
| Exact per-tenant step-log quota leases (ADR-0097): a table, a column, the charge, reserve, top-up, refill, quota-reclaim, idle-reclaim and usage functions, a release branch, the wait slots, the metrics of data-model.md §7 and two alerts; and the residual gaps G041-023, G041-026 and G041-027 | The tenant's one usage row capped a tenant's charging commits at about 800 a second and turned batches away at 8 × 128 (D041-106, D041-108); the quota must stay exact (OD-2026-11-20-6); the top-up after the commit takes steady refills out of reports with no state in the store (OD-2026-11-20-18); the leases do not change the content class's ceiling: every completion that stores output still passes the usage row at its commit (G041-024) | The simpler alternative rejected: one unlocked ceiling, read without a lock at each charge, with an overshoot proved to be at most the pool's connections times a 24 KiB carry (ADR-0097 option 4): admissible if proved and documented, but every quota view would carry that tolerance, against this machinery and its three gaps. Also: the usage row at commit (the measured ceiling); a step-log row of its own (moves the queue); striped counters (false refusals, or an overshoot); a release that folds every quota lease waiting (stalls the tenant on the purge's cadence, D041-112); an ensure before each report that needs room, which needed the backend's identity a pooled connection cannot give without a statement (replaced, OD-2026-11-20-18); a bounded fallback with no top-up, a measured and acceptable baseline (the fourth run's 0.31% of ten-line carries refilled inside a report passed every gate at 8 × 128), which the owner chose to improve on by the top-up after the commit (OD-2026-11-20-18; task T041-37 shows that fallbacks stay at most at that 0.31%); per-tenant wait slots alone (decision D041-125 as first written), which bounded one tenant but not the sum over tenants, and a Rust semaphore held across every carrying report, which cannot bound a wait that starts inside the function and would take permits from carries on the fast path (decision D041-134); a quota lease per (replica, connection slot), ADR-0097 option 1c (the slot token is the client's word, not the server's) |
| Tables with no foreign key to `runs` | A foreign key makes each insert take a lock that conflicts with the engine's run lock | Keeping the convention |
| A new RPC in protocol minor 1.7, carrying the session token, for a long step's batches | A long step must be readable while it runs, and its batches must not share the fate of the heartbeat; the session's namespace, minor and subject must be checkable | A field on `Heartbeat` (statements under the run lock once a second per long step); an RPC with no session |
| The last records on the report (at most 64), one database-function call in its transaction whose locks are only tried, a stream lock per attempt, a charge per insert statement, a downgrade re-run when the carry call fails, and an order guard (ADR-0095, OD-2026-11-20-1) | T041-31 showed that a transaction per batch and a final batch lowered the rate by 13% to 21% (medians); the first carry spike (T041-35, 2026-11-20) showed five round trips and a charge per row add more than +10% to a 7.5 ms p99 | Every record on the report with nothing streamed; a second transaction after the report in the same call (the extra commit the owner chose to remove); the plan kept in Rust at the cost of a second round trip (what OD-2026-11-20-1 removed) |
| The batch plan written twice: T5 and T6 in Rust (`plan_batch`, the ingest) and in plpgsql (the carry function), kept equal by the cross-language property test of T041-36 (D041-100; decision D041-132) | The carry runs in SQL only because the owner removed its second round trip (OD-2026-11-20-1); the ingest waits for its locks under its lock timeout and has T7's control flow (noted answers, skip to T7, roll back) and per-step verdicts, which belong in the domain (principle II) | One write path: the ingest calling the carry's function in a waiting lock mode, which would move the ingest's verdict rules out of the domain into plpgsql as well |
| The server's order guard (`last_stored_xid`, its trigger, `in_order`, the `out_of_order` drop reason) | FR-041-002's order within an attempt is a property of what the server stores, for every client (decision D041-132) | An SDK rule alone (a mark-only carry after any batch of unknown fate), which protects only conforming SDKs |
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
  statements to every carrying report and could not catch a raise at commit. Since ADR-0097 the charge raises at the insert,
  so the carry's function catches it in one `BEGIN … EXCEPTION` block of its own (D041-112), with no client statement.
- **An ensure before the report** (ADR-0097 decision 3b as accepted on 2026-11-20): a transaction of its own that made
  the writer's quota lease's room before each carrying report whose room the store had seen short, unknown or stale.
  Replaced by the top-up after the commit ("Top up after the commit", OD-2026-11-20-18, decision D041-124): the store
  cannot learn which server backend a pooled connection is without a statement of its own (`process_id` is private in
  sqlx 0.9), so its per-backend kept room could not be built as written, and the ensure could make a report late.
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
- **A per-tenant step-log usage row of its own**, apart from `tenant_retained_usage` (O041-026 (b)). Rejected
  (D041-108): it moves the step-log part of the queue to a row of its own instead of removing it; every report that
  keeps a carried record would still take it at its commit. The exact leases of ADR-0097 were chosen.
- **Striped step-log counters** (several usage rows per tenant). Rejected (D041-108): with per-stripe ceilings a tenant
  is refused while other stripes have room; with one ceiling read without a lock the usage can pass the limit by the
  charges in flight. **A ledger folded later** and **a charge after the report commits** were rejected as well (ADR-0097,
  Considered options).
- **Chunk rows** (one row holding a carry's or a batch's records). Rejected (D041-111): 33% less WAL a carrying commit
  but no measurable gain in rate; the readers' level filter, paging and cursor, the markers and the per-row release would
  all change. Dropping the lines' foreign key or a read index: no measurable gain (D041-107).
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
  rule of design 038 picks `integration_claim_transition_efficiency`, where the lock tests live); SC-041-007, per size,
  by T041-35 before the schema (settled, OD-2026-11-20-13) and by T041-9 on the built carry, the quota leases' regimes by
  T041-37, with the runners, their load and the PostgreSQL configuration stated, in
  `specs/041-step-logs/validation-report.md`. The ingest's statement count and the carry's cost on a report (section 2a)
  are budgeted as equalities.
- **NFR-041-SEC (security)** No grant and no `SECURITY DEFINER` function beyond the one list of section 3, "Grants"
  (data-model.md §4; ADR-0097 decision 1), and no role membership; the privilege audit checks exactly that list. The
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
  line, and gains its `Owner-gate:` line when the answer is recorded. Once the answer is recorded, the "Waits for" line
  becomes a "No longer waits:" line, in the same place, right after the task's first line: it names the question and
  the owner row that answered it, so that a reader sees why the task may start. Task numbers are never reused; T041-28
  to T041-30 were added on 2026-11-13, T041-31 and T041-32 on 2026-11-15, T041-33 on 2026-11-16, T041-34 on 2026-11-18,
  T041-35 and T041-36 on 2026-11-19 (the revision that OD-2026-11-19-1 decided), and T041-37 on 2026-11-20 (decision
  D041-131), each in the slice it belongs to. Withdrawn: T041-28 (the tests of the classified-namespace record) on
  2026-11-16, since no record is kept (OD-2026-11-16-8); T041-34 on 2026-11-19, since T041-6 carries its gate; T041-31
  (the spike of the approved transport) on 2026-11-20, since the owner replaced its transport (OD-2026-11-19-1) and
  T041-35 took its gate; its record is the first section of `specs/041-step-logs/validation-report.md` and decision
  D041-087.
- **Tests first.** Each test is written in the pull request of the task that builds what it tests and is seen failing
  on its own assertion (not on a compile error or a missing fixture) before that code exists; the pull request names,
  for each new test, the assertion that failed first. No task tests a carry before task T041-36, which builds it
  (decision D041-131). A task that only adds tests for code already merged (T041-8, T041-24, T041-26), and a guard
  test that passes from the start (an architecture test, `work_budget.rs` unmodified), shows each test failing against
  a violation planted by hand in a scratch commit, and names it.
- **Gates.** CI runs the unit tests of the changed packages and the suites `scripts/select-test-targets.py` picks, in
  the sharded `integration` job (`.github/workflows/ci.yml:130-219`); it does not run the lints or the checkers on a
  pull request (`docs/constitution.md`, CI gates). So before each pull request the author runs `cargo fmt --check`,
  `cargo clippy --workspace --all-targets --locked -- -D warnings`, "the checkers" (`scripts/check-architecture.py`,
  `scripts/check-agent-config.py`, `scripts/check-design-docs.py`) and, where a task says "also as the runtime role",
  its test suites with `SAGAT_TEST_AS_RUNTIME_ROLE=1`, and names each command and its result in the pull request.
- **Held transactions** in tests set their lock timeout when they begin (60 s while held) and are released within
  10 s; an assertion on stored rows or on a read polls with `eventually_within(what, bound, check)`, which T041-6 adds to
  `tests/support/mod.rs` beside `eventually` (whose bound is 5 s, `:413-429`), with a bound of 30 s.
- **Paths** are relative to the repository root; `openapi.yaml` is `specs/001-platform-foundation/contracts/openapi.yaml`.
- **Leases.** A *task lease* is a step's lease on its work (`steps.lease_expires_at`, renewed by heartbeats); a *quota
  lease* is a row of `step_log_quota_leases` (section 2b). The plan never writes "lease" alone for either. Likewise a
  *lease reclaim* takes back an expired task lease, and a *quota reclaim* folds quota leases (the near-limit, idle,
  ended-backend and limit-change reclaims of section 2b).

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
    The approval of this design's revision of 2026-11-19 and 2026-11-20 runs under this task, on the owner's path "Full
    review, capped" (OD-2026-11-20-17): its reports are in docs/designs/041-step-logs.reviews/2026-11-20-approval/,
    and the new approval block cites them.
  Evidence continued (2026-11-21): the revision's approval run on the owner's path "Full review, capped"
    (OD-2026-11-20-17): the first round, all five egm-review stages in order and the three repository reviewers on
    ff8033cd (docs/designs/041-step-logs.reviews/2026-11-20-approval/); short re-reads at 4d6fe4cc (round-2/) and
    c52b5d37 (round-3/); one final read of the last fixes at 54341713 (final/), Blocker 0; its six Majors and the first
    round's Z-2 accepted under the cap by OD-2026-11-21-200 to -205 (final/egm-review.md#1 to #4, architecture.md#1,
    correctness.md#1); every finding answered in resolution.md; the approval block cites the four reports of final/.
    python3 scripts/check-design-docs.py: design docs ok.
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
- [x] T041-33 Put the restated O041-025 to the owner and record the answer as a row of `docs/owner-decisions.md` and in
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
  Evidence continued (2026-11-21): ticked again by the approval of this revision (Reviewed-commit 54341713); the
    evidence of 2026-11-18 is unchanged; python3 scripts/check-design-docs.py: design docs ok.
- [x] T041-35 Spike of the carry (the new spike of OD-2026-11-19-1), before the schema was fixed: a prototype of the carry
  of section 2a and of the quota leases of section 2b in the benchmark's own disposable database (the scratch schema
  `spike_041`, a report hook behind the `perf-spike` feature, nothing reaching a migration), measured against
  SC-041-007's gates (1), (2b), (3) and (4) (defined in SC-041-007) at 1 × 16 and 8 × 128, each gate judged on the
  median of three valid rounds, with its acceptance rule fixed in this task before each run's first round. It held
  tasks T041-3, T041-4 and T041-5 and question O041-038 until it passed. Four runs, each recorded with its machine, its
  rounds and their validity in `specs/041-step-logs/validation-report.md` and decided in
  `specs/041-step-logs/research.md`:
  - first run (2026-11-20, the laptop, attempts 2 to 4 valid): gate (2) failed at both sizes, decision D041-099;
  - second run (run 44717007853, hosted runners from here on, decision D041-102): failed at 1 × 16, decision D041-105;
  - third run (run 44766210424): not decided, no valid round, decision D041-116;
  - fourth run (run 44834024933, commit `77fcb33d`): failed gates (1), (2b) and (3) at 1 × 16 and passed all four at
    8 × 128, decision D041-122.
  The fourth run's declarations (its setup additions, its expected refill shares, its reading rules) are in the
  validation report's fourth-run section and in decisions D041-120 and D041-121.
  Refs: NFR-041-PERF, SC-041-001, SC-041-007
  Owner-gate: OD-2026-11-19-1, OD-2026-11-20-1, OD-2026-11-20-13
  Files (as built on this branch): modify `tests/performance/scalability-ha/{main.rs,run.sh}`, `tests/Cargo.toml` (the
  opt-in feature), `crates/infra-postgres/{Cargo.toml,src/execution.rs,src/lib.rs}`,
  `specs/041-step-logs/{validation-report.md,data-model.md,research.md}`; create
  `crates/infra-postgres/src/report_hook.rs`,
  `tests/performance/scalability-ha/{step_log_spike.rs,step_log_diag.rs,step_log_leases_tests.rs,carry_spike_rounds.sh,carry_diag_rounds.sh,carry_spike_judge.py}`,
  `.github/workflows/perf-spike.yml`. Task T041-36 removes or ports each of them.
  Evidence (2026-11-20): run 44834024933 (commit 77fcb33d; validation-report.md, the fourth run; decision D041-122): at
    1 × 16 gate (1) 82.0%, (2b) +8.6 ms over the allowance, (3) 87.1%, (4) 7 tickets; at 8 × 128 (1) 93.1%, (2b)
    −17.0 ms, (3) 91.5%, (4) 5 tickets; eight valid rounds of twelve, rounds 1 to 3 used; gap G041-025 closed; gap
    G041-026 measured at 1 carry of 864,000. The owner answered the environment question of decision D041-109 with
    "Accept the measured cost" (OD-2026-11-20-13, decision D041-123): SC-041-007 is restated per size (decision
    D041-126) and the 1 × 16 cost is in docs/runbooks/step-logs.md and docs/releases/unreleased.md. OD-2026-11-20-13
    stands in for this task's pass for every gate it held: tasks T041-3, T041-4 and T041-5 and question O041-038
    (answered with ADR-0095's acceptance, OD-2026-11-20-15). The box stays empty while the design is Draft, because
    rule 9 of scripts/check-design-docs.py lets a task ticked since develop stand only in an Approved doc; the approval
    pull request of this revision ticks it on that row, with this run as its evidence.
  Evidence continued (2026-11-21): ticked on OD-2026-11-20-13 with run 44834024933 as its evidence, as stated above,
    now that the revision is Approved (Reviewed-commit 54341713, the approval run's reports in
    docs/designs/041-step-logs.reviews/2026-11-20-approval/final/, Accepted OD-2026-11-21-200 to -205 under
    OD-2026-11-20-17); python3 scripts/check-design-docs.py: design docs ok.
- [ ] T041-3 Contracts: contracts.md §1, as revised on 2026-11-19 (D041-090), and §2.
  No longer waits: task T041-35 is settled by OD-2026-11-20-13, which stands in for its pass, and the owner accepted
  ADR-0095 (O041-038, OD-2026-11-20-15).
  Built on this branch before the revision (D041-091); what changes from that build: the proto's field 13 on both
  reports and `StepLogCarry`, fields 11 and 12 of `AppendStepLogsRequest` reserved (the baseline loses `final_batch` and
  `highest_sequence`, a removal that is expected because 1.7 is unreleased), `StepLogCarry` in the three `skip_debug`
  lists and their `Debug` impls, which no longer print the two removed fields, and
  `crates/api-grpc/tests/step_log_debug.rs`; and, in the planned `openapi.yaml` (edited by hand on 2026-11-20, then
  regenerated), `StepLogStream.ceiling` without `tenant_quota_busy`, the new `StepLogStream.notStoredReason` (one
  value, `service_busy`, decision D041-129) and `Quota.usageWithheld` (decision D041-133). Until T041-36 the server
  reads no field 13, so a carrying report is recorded as if it carried none. In full: copy contracts.md §1
  (`StepLogCarry` on `CompleteStep` and `FailStep`, `final_batch` and `highest_sequence` of `AppendStepLogsRequest`
  reserved) into `worker.proto` and its six SDK copies; the author updates `worker.baseline.json`
  (`SAGAT_UPDATE_BASELINE=1 cargo test -p sagat-api-grpc --test compatibility`) after reading the diff; answer
  `UNIMPLEMENTED` in the seven implementations of section 1 of this design; regenerate the Go stubs
  (`sdks/go/scripts/generate.sh`, protoc v7.36.2, protoc-gen-go v1.36.11, protoc-gen-go-grpc v1.6.1) and the Python
  stubs (`sdks/python/scripts/sync_shared.py`, protobuf 7.35.1, grpcio 1.84.0), the versions the committed stubs record;
  add the four messages of contracts.md §1 (`AppendStepLogsRequest`, `StepLogCarry`, `StepLogRecord`, `StepLogField`)
  and the `oneof` `StepLogField.value` to `skip_debug` in `crates/api-grpc/build.rs`, and in `sdks/rust/build.rs` and
  `sdks/rust/conformance/build.rs`, which first switch to `compile_fds_with_config` as `crates/api-grpc/build.rs:37-40`
  does, with `Debug` impls that print counts beside each generated module and a unit test beside each that
  `format!("{:?}")` of an `AppendStepLogsRequest`, and of a `CompleteStepRequest` and a `FailStepRequest` that hold a
  carry, holds no message text. Copy contracts.md §2 into `openapi.yaml`: the operations tagged `x-planned: '041'`,
  the `ScriptExecution` and `Quota` changes, the four resources in `Quota.resource` and `QuotaResourceName`. Add the
  four operations to the battery's `not_built` list (`tests/contract/security-multitenancy/rest/battery.rs:584-591`);
  add both `x-planned == "041"` and `x-implemented == "041"` to the digest test's approved additions
  (`tests/contract/payload-classification/openapi.rs:349-361`); regenerate `docs/reference/api.md`
  (`SAGAT_DOCS_BLESS=1 cargo test -p sagat-tests --test contract_developer_experience`) and the Studio types
  (`npm run api` in `frontend/apps/studio`, writing `src/api/schema.d.ts`), each after reading the diff. Done when
  `cargo test -p sagat-api-grpc --test compatibility`, `cargo test -p sagat-api-grpc`, `cargo test` in `sdks/rust` and in
  `sdks/rust/conformance`, `cargo test -p sagat-tests --test contract_payload_classification`, the battery of
  `contract_rest_security_multitenancy` and `npm run check:api` pass, and regenerating the Go and Python stubs a second
  time leaves `git diff --exit-code` clean.
  Refs: FR-041-004, FR-041-010, FR-041-011, FR-041-012, FR-041-014, FR-041-022, FR-041-023
  Owner-gate: OD-2026-11-15-4, OD-2026-11-15-8, OD-2026-11-15-9, OD-2026-11-15-16, OD-2026-11-19-1, OD-2026-11-20-15, OD-2026-11-20-19
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

- [x] T041-4 Domain and quota resources. `crates/domain/src/step_logs.rs`: the types, `validate_batch` (which takes no
  final mark, D041-090), `validate_carry` (section 2a, C2, 64 records and 16 KiB) and `plan_batch`, which plans a carry as
  it plans a batch, with the carry's `highest_sequence` as the final mark and `in_order` as an input, with property tests
  of the counting rule of section 2 (lines stored, markers excluded, + dropped by the worker + dropped by the server =
  `last_sequence`, over any sequence of batches, carries, `in_order` values and refusals); the closed enums
  `CeilingReason` (`attempt_records`, `attempt_bytes`, `tenant_quota`), `NotStoredReason` (`service_busy`, decision
  D041-129) and `DropReason` (`worker_gap`, `attempt_records`, `attempt_bytes`, `tenant_quota`, `service_busy`,
  `out_of_order`, `invalid`, `secret`, `content_fallback`, `failed`; decision D041-128), each with `ALL` and `as_str`.
  No longer waits: task T041-35 is settled by OD-2026-11-20-13, which stands in for its pass.
  Built on this branch before the revision (D041-091). What changes from that build: it removes `final_batch` and
  `highest_sequence` from `validate_batch` and its property test; it adds `validate_carry` and `in_order`; it removes
  `CeilingReason::TenantQuotaBusy` (decision D041-129: the busy loss is not a ceiling) and adds `NotStoredReason` and
  `DropReason`; it corrects its migration's comment ("ADR-0097 (Proposed)" and D041-112) to ADR-0097 as accepted
  (OD-2026-11-20-15). Its migrations never create a `tenant_retained_usage_step_log_nonnegative` constraint
  (D041-113: under ADR-0097 the folded usage S may be negative). Fitness tests, in
  `tests/contract/platform-foundation/step_log_ceiling.rs` (target `contract_platform_foundation`): the planned
  contract's `StepLogStream.ceiling` enum equals `CeilingReason`'s set and `StepLogStream.notStoredReason` equals
  `NotStoredReason`'s (task T041-5 extends both to the migration's CHECKs).
  The four quota resources of data-model.md §6, named `step_log_attempt_records`, `step_log_attempt_bytes`,
  `step_log_retention_days` and `tenant_step_log_bytes` (their `as_str` and `limit_name`): `QuotaResource` variants, a
  `minimum()` per resource checked by `validate` and `validate_ceiling`, and the one named exception that lets
  `step_log_attempt_records` take 0 (the capture switch), with tests. A migration named `<timestamp>_step_log_quotas`,
  following `.claude/skills/migration`, that restates `quota_limits_resource_valid`, `quota_ceilings_resource_valid`,
  `quota_limits_tenant_scoped`, `quota_limits_value_range` and `quota_ceilings_value_range` (with the ranges of
  data-model.md §6, every CHECK added `NOT VALID` and validated by a second migration,
  `<timestamp>_step_log_quotas_validate`, D041-088, with a test of the 1 GiB ceiling of `tenant_step_log_bytes`) and
  adds `tenant_retained_usage.step_log_bytes` (`bigint NOT NULL DEFAULT 0`); its down refuses while a `quota_limits`
  or `quota_ceilings` row names a new resource, as
  `crates/infra-postgres/migrations/20270108000000_pending_resets.down.sql:5-15` does, and otherwise restores the five
  constraints and drops the column; its up and down test is
  `tests/integration/security-multitenancy/step_log_quotas_migration.rs`, on the pattern of `undo_to`
  (`tests/integration/schedules-bulk-operations/migration.rs:123`); the upgrade-boundary oracle
  (`assert_eq!(after, <timestamp>)`) moves to `<timestamp>_step_log_quotas_validate`, the newer of the two, in the three
  files `scripts/check-upgrade-boundary.py` reads. The exhaustive matches gain the new resources: the usage of
  `crates/infra-postgres/src/quotas.rs:1211-1291` (`tenant_step_log_bytes` reads the new column; the three namespace
  resources join the arm with no scalar usage) and the cases of `tests/integration/security-multitenancy/quotas.rs:507-598`.
  The quota view of `crates/application/src/quotas.rs` returns `used` of `tenant_step_log_bytes` only to a caller who
  holds `run:logs:read` and the PII reveal grant in every namespace of the tenant, and otherwise the resource and its
  limit with `usageWithheld: true` and no usage figure (decision D041-133, OD-2026-11-20-19, replacing D041-066's
  permission-only rule), the grants read as `has_effective_payload_reveal_grant` reads them; tests: a caller with both
  sees `used`; a viewer, and a holder of `run:logs:read` without the grant in one of the tenant's two namespaces, each
  get no usage figure and `usageWithheld: true`. The permissions `run:logs:read` and `run:logs:delete` with the role
  matrix test; `AuditAction::RunLogsRead` and `RunLogsDeleted`, the first in `collapses_repeats` and `repeat_window`.
  Done when the domain's tests, the fitness tests of `contract_platform_foundation`, the migration test,
  `python3 scripts/check-upgrade-boundary.py`, the quota cases of `integration_security_multitenancy` and the role
  matrix test pass, the suites also as the runtime role.
  Refs: FR-041-002, FR-041-003, FR-041-006, FR-041-013, FR-041-020, FR-041-023
  Owner-gate: OD-2026-11-13-14, OD-2026-11-15-4, OD-2026-11-15-5, OD-2026-11-15-16, OD-2026-11-16-9, OD-2026-11-16-10, OD-2026-11-19-1, OD-2026-11-20-19
  Files: create (built on the branch; modified by the revision) `crates/domain/src/step_logs.rs`, the migration
  (`.up.sql`, `.down.sql`) under `crates/infra-postgres/migrations/`, the migration
  `<timestamp>_step_log_quotas_validate` (`.up.sql`, `.down.sql`),
  `tests/integration/security-multitenancy/step_log_quotas_migration.rs` and
  `tests/contract/platform-foundation/step_log_ceiling.rs`; modify `crates/domain/src/{lib.rs,quota.rs,identity.rs,audit.rs}`,
  `crates/domain/tests/identity.rs`, `crates/application/src/quotas.rs`, `crates/infra-postgres/src/quotas.rs`,
  `tests/integration/security-multitenancy/{main.rs,quotas.rs}`, `tests/contract/platform-foundation/main.rs`,
  `tests/integration/workflow-primitives/{release_upgrade.rs,release_upgrade_inflight.rs,release_upgrade_callback_cutover.rs}`.
  Evidence (2026-11-21): the enums and the contract agree since e8b70cd2; the rest on this branch after 19cc381d:
    `validate_batch` takes no final mark, `validate_carry` returns `Full`, `MarkOnly` or `Ignored` (64 records,
    16 KiB, `highest_sequence` below 2^62 and at least the last record's), `plan_batch` takes an optional `CarryMark`
    (`highest_sequence`, `in_order`, a `MarkOnlyReason`) and returns its drops per `DropReason` (data-model.md §8a
    item 5: a mark-only carry counts each distinct sent sequence in (`last_sequence`, `highest_sequence`] once);
    `DropReason` and `MarkOnlyReason` with `ALL`/`PLANNED` and `as_str`; the stream's `not_stored_reason` set only by a
    `service_busy` mark-only carry. `cargo test -p sagat-domain --locked`: every target ok, the lib 104 passed (the
    step_logs module 19, the property test over batches, carries, `in_order` values, the six mark-only reasons and
    refusals included); `crates/domain/tests/identity.rs` (the role matrix) and `audit.rs` ok. Hand mutations, each
    seen failing and restored with a fresh build: `in_order` ignored (2 tests fail), a mark-only duplicate counted
    twice (1), 64 records refused (1), `service_busy` setting a ceiling (1), the gap above a ceiling counted as the
    worker's (1), `highest_sequence` 2^62 accepted (1). The quota view (D041-133, OD-2026-11-20-19):
    `QuotaView.usage_withheld` and `QuotaBody.usageWithheld`, `QuotaStore::reveals_in_every_namespace`
    (`crates/infra-postgres/src/reveal_grants.rs`, the membership role locked as `has_effective_payload_reveal_grant`
    locks it, none for a tenant with no namespace); test
    `quotas::the_step_log_usage_is_shown_only_with_the_grant_in_every_namespace` (a viewer; `run:logs:read` without a
    grant; the grant in one of two namespaces; in both; the views `set` and `clear` return), seen failing under two
    hand mutations (the permission or the grant; no grant needed) and passing restored;
    `the_tenant_view_has_no_single_usage_for_a_per_namespace_resource` now expects the admin's usage withheld. The
    migration's comment names ADR-0097 as accepted (OD-2026-11-20-15) and D041-113. `cargo test -p sagat-tests --test
    integration_security_multitenancy -- quota step_log`: 30 passed, 0 failed, as the owner role and with
    `SAGAT_TEST_AS_RUNTIME_ROLE=1` (the migration's up and down test among them); `--test contract_platform_foundation`:
    32 passed; `cargo test -p sagat-application --lib`: 192 passed; `python3 scripts/check-upgrade-boundary.py`:
    boundaries match migration 20270201000100; `cargo clippy` of the five touched packages `--all-targets -D warnings`:
    clean.
  Evidence continued (2026-11-21): the slice's review (`docs/designs/041-step-logs.reviews/2026-11-21-domain-and-quota-view/`,
    Blocker 0, Major 2, Nit 9) answered by D041-140: an empty batch refused (`BatchInvalid::Empty`, seen failing
    first), the mark-only gap after a ceiling pinned by `a_mark_only_carry_after_a_ceiling_splits_its_gap_as_t5_does`
    and data-model.md §8a item 5 corrected, the drop reasons checked by the property test, `run:read` checked, three
    more cases in the withheld-usage test. Hand mutations, each failing and restored with a fresh build: every unkept
    record `attempt_records` (2 tests), `run:logs:read` not checked (1), the membership role not checked (1).
    `cargo test -p sagat-domain --lib`: 105 passed; `--test integration_security_multitenancy -- quota step_log`:
    30 passed as the owner role and as the runtime role; `sagat-application --lib`: 192 passed; clippy of the five
    packages `-D warnings`: clean.
- [ ] T041-5 The quota leases and the step-log tables (data-model.md §2 to §4).
  No longer waits: the owner answered O041-040 (OD-2026-11-20-16), accepted ADR-0097 (OD-2026-11-20-15), settled the
  environment question of decision D041-109 (OD-2026-11-20-13, which stands in for task T041-35's pass) and chose the
  top-up after the commit (O041-043, OD-2026-11-20-18).
  - *Migration* `<timestamp>_step_logs`, following `.claude/skills/migration`:
    - the two tables: `step_log_streams` with `last_stored_xid` (which the guard trigger keeps from being lowered or
      cleared, section 2a), `ceiling_reason` (CHECK in `attempt_records`, `attempt_bytes`, `tenant_quota`) and
      `not_stored_reason` (CHECK in `service_busy`, decision D041-129); `step_log_lines`;
    - their indexes: the lines' run and stream read indexes; on streams the run index of data-model.md §2 (kept,
      D041-112) and the expiry index on the stored generated `expires_day`, with a fill factor of 70 (D041-111);
    - the no-update trigger on lines and the guard trigger on streams;
    - the rest of the ADR-0027 Amendment's accounting (data-model.md §3) except its locking accessor
      `sagat_quota_lock_step_log_usage`, which is not built;
    - the quota triggers on `step_log_lines` in the shape of ADR-0027's Amendment of the Amendment (its section of
      2026-11-20, OD-2026-11-20-16): a statement-level `AFTER INSERT` trigger over the transition table, charging each
      tenant once per statement to its exact quota leases, a per-row `AFTER DELETE` release, no deferred flush on
      insert, each `ENABLE ALWAYS`;
    - the objects of data-model.md §3b (ADR-0097): `step_log_quota_leases`; `tenant_retained_usage.step_log_leased_bytes`
      with its CHECK `tenant_retained_usage_step_log_leased_nonnegative` added `NOT VALID` and validated by a second
      migration of this task, `<timestamp>_step_logs_validate`; the functions `sagat_quota_step_log_charge`,
      `sagat_quota_step_log_refill` (with the wait-slot rule of decisions D041-125 and D041-134 for every wait on the
      usage row, a carry's fallback, a batch's T8 and the script marker alike: the usage row tried `SKIP LOCKED` first, a
      wait only while holding one of the tenant's *P* slots and one of the replica's *Q* slots on this replica, *Q* =
      `sagat_quota_step_log_wait_slots()` and *P* = ⌈*Q* / 2⌉; the refill runs in the insert's statement trigger, where
      nothing tells a batch from a carry), `sagat_quota_step_log_refill_wait`, `sagat_quota_step_log_top_up` (decision
      D041-124; it replaces the ensure, takes no caller amount, tries the usage row `SKIP LOCKED` only, takes no slot and
      reclaims no other quota lease, decision D041-134), `sagat_quota_step_log_reserve`, `sagat_quota_step_log_counters`,
      `sagat_quota_step_log_idle_tenants`, `sagat_quota_step_log_reclaim_idle` (returning `(reclaimed integer, skipped
      boolean)`, `skipped` true when the tenant's step-log fence was held), the limit-change trigger function
      `sagat_quota_step_log_reclaim` and `sagat_quota_step_log_usage`; the two invoker helpers
      `sagat_quota_step_log_top_up_mark()` (`IMMUTABLE`, 65,536) and `sagat_quota_step_log_wait_slots()` (`STABLE`,
      `sagat.step_log_wait_slots` clamped to 1 to 64, 1 when unset); the counter sequences; the release branch of the finalizer that never waits for a quota lease; and the
      quota reclaim on a change of the limit;
    - a constraint name of its own for every raise of the quota-lease functions and of the finalizer's step-log
      branch, so that the store maps each to one typed refusal;
    - grants (data-model.md §4), with `sagat_quota_step_log_allowance` revoked from `PUBLIC`; the storage-classification
      entries with the migration as owner path; a down that refuses while a line exists and otherwise restores what it
      replaced; the upgrade-boundary oracle moves to `<timestamp>_step_logs_validate` in the three files of T041-4.
  - *Rust*: the quota readers of `crates/infra-postgres/src/quotas.rs` (`:1228-1235` read S today) moved to
    `sagat_quota_step_log_usage`, with a test; the typed refusals of `tenant_step_log_quota_exceeded` and
    `tenant_step_log_quota_held` in `classify` of `crates/infra-postgres/src/identity.rs` (`:787-791`); and a
    per-replica `application_name` and the wait-slot setting, so that a quota lease's backend is attributable to a
    replica and the wait slots of decisions D041-125 and D041-134 are per replica:
    - `PoolConfig` (`crates/infra-postgres/src/pool.rs:11-24`) gains an `application_name` field (`PoolConfig::new`'s
      default is `sagat-octopus-orchestrator/unnamed`, so no pool keeps the old shared name by accident); the pool's
      connect options use it instead of the fixed `sagat-octopus-orchestrator` (`pool.rs:70`); the store's *Q* is
      computed once from the pool's maximum N, max(1, ⌊N / 4⌋), never configured apart, and passed per transaction
      (section 2b, decision D041-137; the stores of tasks T041-6 and T041-36 send it), never set at connect;
    - the execution engine (`apps/execution-engine/src/lib.rs`, whose pool, opened at `:95`, is the one that charges
      quota leases) and the control plane (`apps/control-plane/src/lib.rs`) create their instance id before they open
      their pool (today `registry::member` creates it at `apps/control-plane/src/registry.rs:71`, after the pool is
      opened at `apps/control-plane/src/lib.rs:523` and `apps/execution-engine/src/lib.rs:95`) and pass it both to
      `pool_config` / `open_pool` (a new argument `name: PoolName`, an enum of `Instance(Uuid)` and
      `Process(&'static str)`) and to `registry::member` (which takes the id instead of creating it); the name is
      `sagat-octopus-orchestrator/<instance id>` (27 + 36 = 63 bytes, PostgreSQL's limit), and the registry's own pool
      (`registry_pool_config`, `apps/control-plane/src/registry.rs:53-58`) takes the same name, since it serves the
      same instance; the one-shot processes use `sagat-octopus-orchestrator/<process kind>`: migrate, inventory,
      handover, cleanup, `seed_dev`, `seed_dev_namespace`, `provision_tenant`, `grant_quota_ceiling`,
      `set_scheduling_weight` and `reseal_secrets` (`apps/control-plane/src/lib.rs:171-417`, the pool builders of
      `apps/control-plane/src/{cleanup.rs,inventory.rs,handover.rs}`);
    - the test replicas (`tests/support/engine.rs:756-765`, `replica_with_pool`) name each pool
      `sagat-octopus-orchestrator/<a fresh UUID>`, so that in-process replicas do not share wait slots, and task
      T041-37's setup asserts that every replica's connections show a distinct name in `pg_stat_activity`;
    - the prefix is kept, so `scripts/smoke-compose.sh:81` matches `application_name LIKE 'sagat-octopus-orchestrator/%'`;
    - tests: two pools built with two instance ids show two names in `pg_stat_activity`, two test replicas show two
      names, and a store over a pool of 20 computes *Q* = 5 (and 2 over pools of 10 and 8).
  - *Tests*, in `tests/integration/claim-transition-efficiency/step_log_isolation.rs`, each with a plain `INSERT` into
    `step_log_lines` or a direct call of a §3b function (no carry: the carry's cases are task T041-36's), with held
    transactions and `pg_stat_activity`, never sleeps:
    - the migration's up and down, on the pattern of `undo_to`;
    - ADR-0097's invariants I1 to I5 under a property test of random interleavings of charges, refills, quota
      reclaims, releases and limit changes, each step checking 0 ≤ U ≤ S + G ≤ L;
    - each writer charges its own quota lease and never meets another's lock; a charge with room never takes the
      usage row; a refill alone waits for it, at most *b* inside a batch's insert, restoring the caller's
      `lock_timeout`; a caller's lower `lock_timeout` is kept, and a caller's `lock_timeout = 0` (none) counts as no
      timeout, so the refill sets *b*; a refill whose usage row is held by a holder that updated it waits and writes;
    - the wait slots, through plain `INSERT`s of lines (a batch's refill), each test transaction setting
      `set_config('sagat.step_log_wait_slots', '5', true)` first (*Q* = 5, *P* = 3; the store's own statements carry it,
      task T041-6 and T041-36) and the budget function replaced by the test's own bound: with the tenant's usage row held by the test and
      its quota leases emptied, five inserts on five backends of one `application_name`: three wait for the row, each
      holding a tenant slot and a replica slot (`pg_stat_activity` and `pg_locks`), and two raise `55P03` at once; with
      five tenants' rows held and one waiter each, a sixth tenant's insert raises `55P03` at once; an insert under
      another `application_name` takes slots of its own; the slot keys are `hashtextextended` single-bigint keys; a
      transaction that sets no value has *Q* = *P* = 1, and the setting of one transaction does not reach the next one
      on the same backend; the first insert of a tenant whose usage row does not exist takes its slots before it
      inserts the row; the refill and the charge open no subtransaction when the row is held (`pg_stat_get_backend_subxact` of the
      waiting backend reads 0 subtransactions);
    - the top-up (`sagat_quota_step_log_top_up`): it answers without raising (`room`, `made`, `limit`, `busy`,
      `contended`); with at least 65,536 bytes of room it answers `room` and takes no lock on the usage row; below it,
      it grants exactly its share, min(1 MiB, ⌊F / (2(*A* + 1))⌋), and charges nothing, answers `limit` when that share
      is 0 (F < 2(*A* + 1), the writer's room under *M*: `limit`, not `room`), and answers `room` when
      the share is not above the writer's current room; it answers `busy` at once when the usage row is held (no wait, no
      wait slot taken, `pg_locks`); it reclaims no other quota lease (idle quota leases keep their room after a top-up
      near the limit); it answers `contended` with the step-log fence held;
    - the deadlock cases of section 2b, a limit change against an open idle reclaim included; a release never waits
      for a held quota lease and refuses an underflow of U; a limit lowered above the usage still admits what fits; a
      change of the limit reclaims every quota lease; with enforcement off a refill grants past the limit and the usage
      stays exact;
    - the idle reclaim folds and deletes only quota leases idle past the interval; the quota lease of an ended backend
      is reclaimed at once; with the tenant's step-log fence held it answers `skipped` true and reclaims nothing, and
      with nothing idle `skipped` false and 0;
    - the exact remainder admitted from idle quota leases near the limit and one byte more refused; the small-quota
      test; the held-room bound (`room_held_by_open_writers_is_bounded_from_the_free_room`: *k* = 1, 2, 8 and 32 writers
      from a quarter-used limit, a top-up's grant counted at its share); a lowered limit stopping charges into quota
      leases with room;
    - the quota-lease charge raises at the limit and charges 128 bytes for an empty line; an insert of 10 lines adds one
      quota-lease charge of their text plus 1,280 bytes (10 × 128) and queues no flush, and a `DELETE` of them releases
      the same; a property test that the charge of any inserted lines equals the release of their delete (Unicode,
      JSON fields, NUL-replaced text);
    - the budget line of a charged insert (one quota-lease row, no usage or delta row, six nested statements for the
      charge besides the foreign-key checks, no deferred call);
    - a script log, a dispatch error and a connector error are stored in full under `tenant_log_bytes` while the
      tenant is at its step-log limit;
    - the statement-level trigger function is `SECURITY DEFINER` with a pinned `search_path` and executable by neither
      `PUBLIC` nor `sagat_app`; every function the migration creates or replaces (`sagat_quota_step_log_allowance`,
      `sagat_quota_add_delta` with its new signature, `sagat_quota_row_delta`, `sagat_quota_flush_deltas`,
      `sagat_quota_measure`) keeps or has its `SECURITY DEFINER` and pinned `proconfig` as before, and none is
      executable by `PUBLIC` or, except `sagat_quota_step_log_allowance`, the six definer quota-lease functions and the
      two invoker helpers ADR-0097 decision 1 lists (`sagat_quota_step_log_reserve`, `sagat_quota_step_log_top_up`,
      `sagat_quota_step_log_usage`, `sagat_quota_step_log_counters`, `sagat_quota_step_log_idle_tenants`,
      `sagat_quota_step_log_reclaim_idle`; `sagat_quota_step_log_top_up_mark`, `sagat_quota_step_log_wait_slots`), by
      `sagat_app`, the privilege audit checking exactly that list and that the counter sequences are revoked from
      `sagat_app`; the runtime-role bypass tests extended to the quota lease table and functions (none reads a session
      setting except the wait slots' `application_name`, set by the pool at connect, and the transaction-local
      `sagat.step_log_wait_slots`, set by the store, which a session that changes them can only use to share other
      slots or to take more of its own replica's); a migration test that a freshly migrated
      database's `sagat_quota_step_log_refill_wait()` returns `interval '200 milliseconds'`;
    - two source-text checks of the migrations: only the top-up, the refill, the release branch, the idle reclaim and
      the limit-change trigger lock `tenant_retained_usage` for the class `step_log`; and no Rust source and no SQL
      outside the functions of data-model.md §3b and the down migration reads `step_log_bytes` or
      `step_log_leased_bytes` except through `sagat_quota_step_log_usage`;
    - the guard trigger refuses each change it guards, as the runtime role; the fitness tests of T041-4 extended to the
      migration's `ceiling_reason` and `not_stored_reason` CHECKs;
    - two architecture tests, each with its row in `docs/engineering/architecture-principles.md`: no foreign key leads
      from the log tables to `runs` or `steps`; and a source-text test that the production code (outside
      `#[cfg(test)]`) of `crates/application/src/worker.rs`, `crates/application/src/worker/`,
      `crates/application/src/execution*`, `crates/infra-postgres/src/execution.rs` and
      `crates/infra-postgres/src/execution/` never names `step_log_lines`, `step_log_streams` or `StepLogTx` (ADR-0084
      decisions 1 and 2).
    The held-lock tests are ported from the prototype's `tests/performance/scalability-ha/step_log_leases_tests.rs`,
    and each is shown failing under its hand mutation: mutations 16, 17 and 21 of
    `specs/041-step-logs/validation-report.md` (decision D041-120) applied to the top-up and the refill (16: the
    top-up no longer catches `lock_not_available`; 17: it waits for the fence instead of trying it; 21: the refill
    always sets its budget, so a caller's lower `lock_timeout` is lost), mutation 19 to the idle reclaim (the ended-backend
    clause dropped), and a new one, the top-up charging what it grants, which fails `a_top_up_charges_nothing`;
    mutations 18 and 20 concern the carry and are task T041-36's.
  Done when these tests, the fitness tests of `contract_platform_foundation`, the manifest test of
  `tests/integration/security-multitenancy/durable_quotas.rs`, the privilege audit of
  `tests/integration/platform-foundation/runtime_privileges.rs`, the compose smoke check (`scripts/smoke-compose.sh`)
  and `python3 scripts/check-upgrade-boundary.py` pass, the suites also as the runtime role.
  Refs: FR-041-008
  Owner-gate: OD-2026-11-13-14, OD-2026-11-13-18, OD-2026-11-15-4, OD-2026-11-15-5, OD-2026-11-15-6, OD-2026-11-15-9, OD-2026-11-15-16, OD-2026-11-16-10, OD-2026-11-19-1, OD-2026-11-20-1, OD-2026-11-20-6, OD-2026-11-20-13, OD-2026-11-20-15, OD-2026-11-20-16, OD-2026-11-20-18
  Files: create the migrations `<timestamp>_step_logs` and `<timestamp>_step_logs_validate` (`.up.sql`,
  `.down.sql`), `tests/integration/claim-transition-efficiency/step_log_isolation.rs`; modify
  `crates/infra-postgres/src/{identity.rs,quotas.rs,pool.rs}` (the reclaim on a change of the limit is a trigger of
  its own, which `quotas.rs` does not call), `apps/control-plane/src/{lib.rs,registry.rs}`,
  `apps/execution-engine/src/lib.rs`, the one-shot processes' pool builders
  (`apps/control-plane/src/{cleanup.rs,inventory.rs,handover.rs}`), `tests/support/engine.rs`, `scripts/smoke-compose.sh`,
  `tests/integration/workflow-primitives/scoped_event_storage/guards.rs` (`:98`, the new column excluded from the
  snapshot as `step_log_bytes` is), `crates/infra-postgres/storage-classification.json`,
  `tests/integration/security-multitenancy/durable_quotas.rs`, `tests/integration/platform-foundation/runtime_privileges.rs`,
  `tests/integration/claim-transition-efficiency/main.rs`, `tests/contract/platform-foundation/step_log_ceiling.rs`, the
  three oracle files of T041-4, `docs/engineering/architecture-principles.md`.
- [ ] T041-6 The ingest of section 2.
  Builds: `sagat_application::step_logs` (`StepLogIngest`, the pre-checks P1, P2, P4 and P5, the control flow, the
  store's *Q* sent in T1's select list, decision D041-137);
  `report_fence_allows` in `crates/domain/src/execution/state.rs`, called by `attempt_authority_fence`
  (`crates/application/src/worker.rs:2813-2838`) and by T2; `StepLogStore`, `StepLogTx` and the adapter (data-model.md
  §8), whose T3 reads the policies through `content_usage::admission_policy`, made `pub(crate)` with a batched variant
  for the three resources; T7's unlocked reads of the binding's patterns and of the membership;
  `the_shared_secrets_lock_has_one_caller_and_the_scoped_path_never_upgrades` (`crates/infra-postgres/src/quotas.rs:1329-1357`)
  extended to assert that `step_logs.rs` calls none of `share_secrets_lock(`, `lock_secrets(`, `hold_worker_binding(`,
  `bound_patterns(` and `sagat_lock_memberships` (data-model.md §8, T7); the two public scrubber
  entry points; `WorkerStore::supports_step_log_protocol` and its implementation
  (`crates/infra-postgres/src/workers.rs:199-206`); `store_error_without_text` on every statement that binds line text;
  the metrics the ingest emits (`step_log_batches_total`, `step_log_lines_stored_total`, `step_log_bytes_stored_total`,
  `step_log_lines_dropped_total{reason}` with the labels of `DropReason`, `step_log_lines_truncated_total`,
  `step_log_redactions_total`, `step_log_ingest_seconds`, `step_log_lock_timeouts_total` (a batch's `55P03`, whatever
  lock timed out), `step_log_quota_retries_total`, and `step_log_quota_refusals_total{cause}` from a batch's T8 raise by
  its constraint name, `tenant_step_log_quota_exceeded` as `limit` and `tenant_step_log_quota_held` as `held`; decision
  D041-128); the service's wiring into `WorkerGrpc` (`apps/execution-engine/src/lib.rs:788`, `tests/support/api.rs:632`;
  the RPC still answers `UNIMPLEMENTED` until T041-7) and an `Options` field `step_logs_enabled`
  (`tests/support/api.rs:226`); the `integration_step_logs` target (`tests/Cargo.toml`) and its row in
  `scripts/integration-suite-durations.toml`; `eventually_within` in `tests/support/mod.rs`; the lock-order slots
  L1, L1b, L2, L3 and L3b in `tests/support/lock_order.rs`; and `tests/support/step_logs.rs`: a session fixture
  (`worker_sessions` rows inserted at protocol minor 7) and a `StepLogTx` wrapper with six hooks: fail before the
  commit; commit and then fail as a lost answer would; hold after T2 (no lock held); hold after T3 (the policy fences
  and the attempt's stream lock held, L1 and L1b); hold after T7 (the stream row held since T4, L2); hold after T8 (its
  own quota lease held, L3b); hold after a T8 that refilled (the usage row held, L4), which the fixture guarantees by
  emptying the writer's quota lease first: it changes the tenant's `tenant_step_log_bytes` and sets it back, and the
  change reclaims every quota lease of the tenant (ADR-0097 decision 6), so T8 finds none and refills.
  Tests, in `tests/integration/claim-transition-efficiency/step_log_isolation.rs`, calling `StepLogIngest` directly: the
  lock-free test of SC-041-001 (a batch commits while the run row is held); the quota-lease test: with one ingest of a
  tenant held after T8 (its charge holds its own quota lease), three more ingests of that tenant, each on a backend of
  its own, commit while it is held, each charged to its own quota lease, none waiting (`pg_stat_activity`); and a
  refill held (the hold after a T8 that refilled) delays only a charge that needs a refill, while a charge into a quota
  lease with room commits; `work_budget.rs` unmodified; the lock-order test (section 2). The carrying-report half of
  SC-041-001's quota-lease test is task T041-36's.
  Tests, in `integration_step_logs`, calling `StepLogIngest` directly and asserting stored rows by SQL (the read routes
  come in T041-11): the session cases (expired, other namespace, other subject, below 1.7, replaced token); the
  authority cases (binding removed, membership revoked, a membership whose role is runner or database-runner), each
  answered `AUTHORITY` even where another answer was due, with the denial audited; a runner token refused by P1 with the
  application's `Forbidden`; a task lease sent with another run's id; a forward step of a stopping run refused `LEASE`
  and a compensation step of it stored (OD-2026-11-16-11); duplicates; gaps; a late batch after a newer one; the tenant
  limit by the unlocked read (`FINAL_ONLY`, counted, the planned marker's 128 bytes included); the tenant limit by the
  quota-lease charge's raise, two cases: (i) the tenant over its limit: `RETRY_LATER`, nothing counted, then the resend reads
  the limit at T6 and gets `FINAL_ONLY`, its records counted `tenant_quota`; (ii) the last room held by another open
  transaction's quota lease (`tenant_step_log_quota_held`, counted `cause="held"`): `RETRY_LATER`; then, once that
  transaction has ended, the resend's refill reclaims the room it left and the resend is stored and charged; each with a gap in the
  batch and a later batch after it, asserting the counts; capture off in the namespace (no row written); a change of
  `step_log_attempt_records` to 0 that waits for a batch held after T3 and commits after it; a scrubber case for each
  shape of both scrubbers in the message, the exception, a key and a string value, and for each sensitive field name
  with a value of each type; `stored_through_sequence` 0 with `AUTHORITY` and `SESSION`; T3 taking the attempt's
  stream lock after the fences (an ingest of the same attempt held after T3 makes a second one wait and answer
  `RETRY_LATER` at its lock timeout); T9 raising `last_stored_xid` only when it inserted a row, and the guard trigger
  refusing a lower value; the statement budget of section 2 as an equality, counted with `pg_stat_statements` as
  `work_budget.rs` counts, in `tests/integration/step-logs/budget.rs`. Done when these pass, also as the runtime role.
  Refs: FR-041-002, FR-041-004, FR-041-006, FR-041-007, FR-041-008, SC-041-001
  Owner-gate: OD-2026-11-13-14, OD-2026-11-15-4, OD-2026-11-15-5, OD-2026-11-15-6, OD-2026-11-15-16, OD-2026-11-16-7, OD-2026-11-16-10, OD-2026-11-16-11, OD-2026-11-19-1, OD-2026-11-20-15
  Files: create `crates/application/src/step_logs.rs`, `crates/infra-postgres/src/step_logs.rs`,
  `tests/integration/step-logs/main.rs` and its modules, `tests/support/step_logs.rs`; modify
  `crates/domain/src/execution/state.rs`, `crates/application/src/{lib.rs,worker.rs}`,
  `crates/infra-postgres/src/{lib.rs,workers.rs,quotas.rs,content_usage.rs}`, `crates/api-grpc/src/service.rs`,
  `apps/execution-engine/src/lib.rs`, `crates/telemetry/src/{redact.rs,metrics.rs}`, `crates/domain/src/scripts.rs`,
  `crates/infra-postgres/storage-classification.json` (owner path), `tests/Cargo.toml`,
  `scripts/integration-suite-durations.toml`, `tests/support/{mod.rs,api.rs,lock_order.rs}`,
  `tests/integration/claim-transition-efficiency/step_log_isolation.rs`.
- [ ] T041-32 The CI gate of constitution 4.5.0 (OD-2026-11-15-10). Three entries of `HARNESSES` in
  `scripts/select-test-targets.py` (a `HARNESSES` match selects its suites whether or not the path is also a shared
  input):
  - `integration_step_logs`, `integration_claim_transition_efficiency` and `contract_platform_foundation` (the fitness
    tests of T041-4, so that a change of `crates/domain/src/step_logs.rs` runs them) for the server's step-log code: the
    regex
    `crates/[a-z-]+/src/(.*/)?step_logs(\.rs$|/)|crates/infra-postgres/migrations/[0-9]+_step_log[a-z_]*\.(up|down)\.sql$|crates/api-grpc/src/(limits|service)\.rs$|crates/api-http/src/routes/(execution|scripts)\.rs$|crates/application/src/scripts\.rs$|crates/infra-postgres/src/maintenance\.rs$|crates/scheduler/src/passes\.rs$`;
  - `integration_step_logs` for the report path that stores a carry (section 2a): the regex
    `crates/application/src/worker(\.rs$|/)|crates/infra-postgres/src/execution(\.rs$|/)`, which today selects only
    `integration_claim_transition_efficiency` (the budget rule of design 038, `scripts/select-test-targets.py:85-86`,
    which also covers `runs.rs`);
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
- [ ] T041-10 Retention and deletion storage, and the purge pass's quota work.
  - The two steps of section 4 (mark the streams whose `expires_at` has passed, 500 streams a transaction; then delete
    500 lines a transaction, `purged_at` kept), through two new methods of `MaintenanceStore`,
    `mark_expired_step_log_streams(limit)` and `purge_step_log_lines(limit)`; the purge metrics
    (`step_log_purged_lines_total`, `step_log_purge_backlog`, counted by the pass as section 4 defines it, and
    `step_log_purge_failures_total{cause="underflow"}`).
  - The idle and ended-backend quota reclaims of section 2b (ADR-0097 decision 1): a third method,
    `MaintenanceStore::reclaim_idle_step_log_leases(max_tenants: u32) -> Result<IdleLeaseReclaim, StoreError>`, which
    calls `sagat_quota_step_log_idle_tenants(max_tenants)` once and then, one tenant a transaction,
    `sagat_quota_step_log_reclaim_idle(tenant)`, which answers `(reclaimed integer, skipped boolean)`; `IdleLeaseReclaim`
    counts the tenants reclaimed and those the function answered `skipped` (their step-log fence was held; they are
    tried again at the next pass), so a skip is never mistaken for a tenant with nothing idle; the pass calls it with
    `max_tenants = 100` each run.
  - The publisher of the counters that SQL keeps (decision D041-128): on the replica that holds shard 0 (the pass
    learns it from `Keeper::shards`, `crates/scheduler/src/shards.rs:77`), each run of the
    pass reads `sagat_quota_step_log_counters()` once and adds the deltas since its last read to
    `step_log_quota_refills_total{kind="refill"|"reclaim"|"idle"|"ended"}` and sets the gauge
    `step_log_quota_lease_rows`; the sequences are not transactional, so these count attempts, rolled-back ones
    included, which the metrics' help text says; a replica that loses shard 0 stops publishing, and the next holder
    starts from its own first read (no delta is published twice) and clears its gauge, so that no stale
    `step_log_quota_lease_rows` stays exported.
  Tests, at the store, asserting rows by SQL: a purge releases the quota bytes; a lowered retention leaves existing
  streams' expiry as it was and applies to new streams; an idle stream whose task lease lives is marked and its next
  batch is `REMOVED`; a stream row held by an ingest (the hold after T7) is skipped by the mark and marked by the next
  pass; a stream being deleted over several transactions keeps `removed_at` from the first of them; the purge joins
  the lock-order test; the pass's quota reclaim, ported from the prototype
  (`the_idle_reclaim_folds_and_deletes_only_leases_idle_past_the_interval` and
  `the_lease_of_an_ended_backend_is_reclaimed_at_once`, now through `reclaim_idle_step_log_leases`), and a tenant whose
  step-log fence a test transaction holds is skipped and counted, then reclaimed by the next pass; the publisher adds
  each delta once across two passes and publishes nothing on a replica without shard 0. The runbook
  `docs/runbooks/step-logs.md` (created with task T041-35's record) gains the purge, the retention rule, the quota
  reclaims and the deployment switch that T041-7 builds. Done when these pass, also as the runtime role.
  Refs: FR-041-009, FR-041-018, FR-041-020
  Owner-gate: OD-2026-11-13-14, OD-2026-11-15-4, OD-2026-11-16-10, OD-2026-11-20-15
  Files: modify `crates/scheduler/src/passes.rs`, `crates/application/src/maintenance.rs`,
  `crates/infra-postgres/src/{maintenance.rs,step_logs.rs}`, `crates/telemetry/src/metrics.rs`,
  `tests/integration/claim-transition-efficiency/step_log_isolation.rs`, `docs/runbooks/step-logs.md`; create
  `tests/integration/step-logs/retention.rs`.
- [ ] T041-7 The gRPC handler, in `crates/api-grpc/src/step_logs.rs`, called from `WorkerGrpc`, taking the tenant ticket
  (P3) before it calls the service and auditing an `AUTHORITY` answer with `spawn_audit_access_denied`; `Class::Logs` and
  the per-tenant tickets with their configuration keys and the defaults of data-model.md §6 (decision D041-127:
  `SAGAT_GRPC_MAX_LOG_CALLS` half of `SAGAT_DB_MAX_CONNECTIONS` rounded down, at least 1, so 10 of a pool of
  20, the chart's value; `SAGAT_STEP_LOGS_TENANT_CALLS` 7, at most the cap), no call waiting for a ticket (the server still reads no field
  13 of a report: T041-36 does; no release is cut between the two); configuration tests of both defaults, and of a pool
  of 8 (cap 4, tickets 4: 7 capped at the cap); tests with admission counters that a full Logs class leaves discovery
  admission unchanged and that a second tenant's batch is admitted while the first tenant's seven tickets are all
  taken; protocol 1.7 in `negotiate_protocol_details` and `protocol_ceiling` (1.6 kept for the database runner) with the
  ceiling tests; the policy in `RegisterWorkerResponse`; `SAGAT_STEP_LOGS_ENABLED` (default true) in the execution
  engine; the span; the Rust SDK's refusal of an accepted minor above 6 (`sdks/rust/src/worker.rs:468`) raised to 7, so
  that the Rust runner accepts the server's new answer; `sdks/SPEC.md` §7.1 and §9's text for 1.7 (O041-018); the
  golden `tests/contract/sdk-conformance/expected/protocol-negotiation.json` and the three tests that assert the
  server's minor (`tests/e2e/sdk-multilanguage/us04.rs:29`, `tests/e2e/developer-experience/doctor.rs:323` with
  `crates/api-http/src/routes/identity.rs:68`, and `tests/contract/durable-execution/grpc/protocol_compatibility.rs:131, 138`)
  updated after reading each diff. The Go, Python, .NET, Java and TypeScript SDKs keep their constants at 1.5 until
  their own tasks, so the updated golden must still hold for an SDK that offers 1.5 (the server answers the lower
  minor). Done when the ceiling, admission and configuration tests,
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
  (`RETRY_LATER`, then a resend that stores nothing); a stale task lease; an expired task lease not yet reclaimed; a
  batch after its attempt's report was recorded (`LEASE`); a batch for another tenant's run (audited, the audit kept
  after the rollback); two attempts of one run at their attempt ceilings and two runs of one tenant at its
  `tenant_step_log_bytes` limit, at once; a value that redaction lengthens at the tenant limit; a client that ignores
  every verdict (stored bytes stay at the ceilings); two tenants that flood and a third whose batches are still
  admitted (admission counters); a restart of the API in the middle of an attempt (the attempt's next batch is stored,
  no duplicate); a task delivered again under a new generation writes a new stream; rebuilding the projections of a
  run changes no log row; the server canary (a secret value the test's attempt resolved and logged) searched in the
  captured `tracing` output, the stored rows, `run_events` and `audit_records`; and a database error that quotes the
  bound text, forced by a `BEFORE INSERT` trigger the test installs on `step_log_lines` that raises with `NEW.message`
  in its message, after which the server canary is in neither the answer nor the captured `tracing` output. Done when
  these pass, also as the runtime role.
  Refs: NFR-041-SEC, SC-041-002, SC-041-003
  Owner-gate: OD-2026-11-19-1
  Files: create modules under `tests/integration/step-logs/` and `tests/support/worker_proxy.rs` (moved from
  `tests/integration/database-runner/proxy.rs`); modify `tests/integration/database-runner/main.rs`,
  `tests/integration/step-logs/main.rs`.
- [ ] T041-36 The carry on a report (section 2a) and the top-up after it (section 2b).
  No longer waits: the owner accepted ADR-0095 and ADR-0097 (O041-038 and O041-042, OD-2026-11-20-15), the wording of
  the ADR-0027 amendment (O041-040, OD-2026-11-20-16) and the top-up after the commit (O041-043, OD-2026-11-20-18).
  Builds:
  - the domain type `sagat_domain::step_logs::Carry` (records, `highest_sequence`, encoded size) and `CarryPlan`
    (`Full`, `MarkOnly`, `None`);
  - the carrying entry points `WorkerService::complete_carrying`, `complete_with_blob_carrying` and `fail_carrying`,
    which take a `Carry`; `complete`, `complete_with_blob` and `fail` keep their signatures and delegate with none, so
    their callers do not change; the gRPC handlers of `CompleteStep` and `FailStep` call the carrying ones when field 13
    is present;
  - C0 to C3; `WorkerSettings.step_logs_enabled`, set by the execution engine from `SAGAT_STEP_LOGS_ENABLED` (as T041-7
    reads it), in the settings literal of `apps/execution-engine/src/settings.rs`, and by `tests/support/api.rs`, which
    copies `Options.step_logs_enabled` of T041-6 into its `worker: WorkerSettings` (`api.rs:248`), so that the two
    cannot disagree;
  - `CarryPlan` as an explicit argument of `finish`, `finish_path_report`, `refuse_scoped_output`,
    `fail_success_for_storage_quota`, `fail_report_for_storage_quota` and `refuse_scoped_content`, each site passing what
    the table of section 2a says, the leaked-secret sites (`reject_secret_output`, the root `fail` with `leaked`,
    `apply_path_report` with `leaked`, `refuse_scoped_output`) a mark-only carry;
  - the migration `<timestamp>_step_log_carry` (`.up.sql`, `.down.sql`) that creates `sagat_step_log_carry`
    (data-model.md §8a, its argument `p_wait_slots` the store's *Q*, set transaction-locally before any write; `SECURITY INVOKER`, pinned `search_path`, `EXECUTE` to `sagat_app` only, revoked from `PUBLIC`;
    its down drops it; named `step_log*` so that T041-32's selector runs the step-log suites, as must any later
    migration that replaces it), following `.claude/skills/migration`; the function refuses at its top any
    `p_mark_reason` other than `invalid`, `secret`, `content_fallback` and `failed` (with a fixed text), returns its drops keyed by
    `DropReason`, `ceiling_reason`, `not_stored_reason`, `path` (`fast`, `refill`, `none`), `room` (the room left in
    the writer's quota lease, NULL exactly when `path` is `none`) and `top_up` (true exactly when `room` is not NULL
    and under 65,536 bytes), and takes, in its fallback, the usage row `SKIP LOCKED` first and a tenant and a replica wait slot before it waits
    (decisions D041-125 and D041-134), answering `service_busy` past *b* or with no slot free; the upgrade-boundary oracle moves to
    it in the three files of T041-4 (`tests/integration/workflow-primitives/{release_upgrade.rs,release_upgrade_inflight.rs,release_upgrade_callback_cutover.rs}`);
    the privilege audit of `tests/integration/platform-foundation/runtime_privileges.rs` gains `sagat_step_log_carry`
    (`SECURITY INVOKER`, executable by `sagat_app` only);
  - the top-up after the commit (ADR-0097 decision 3b as amended by OD-2026-11-20-18, decision D041-124), and the
    connection ownership it needs (sqlx 0.9's `Transaction::commit(self)` consumes the transaction and returns its
    connection to the pool, so no connection can be kept from a `Transaction<'static, Postgres>`):
    - a type `OwnedTransaction` in `crates/infra-postgres/src/pool.rs` (decision D041-137) holds an
      `Option<PoolConnection<Postgres>>` and a state `idle`, `open` or `unknown`, derefs to `PgConnection`, and sends
      `BEGIN`, `COMMIT` and `ROLLBACK` itself (the state is `unknown` from the moment one of them is sent until it
      returns `Ok`); sqlx already sends `BEGIN` as a statement, so the budget is unchanged; it never calls
      `close_on_drop()` except on the one path below that must close, since sqlx 0.9 can set that mark but not clear it;
    - its `Drop`: `idle`, the connection goes back to the pool; `open` or `unknown`, the connection is moved into a
      spawned task that sends `ROLLBACK` (sqlx first drains a statement still in flight) within 5 s and then drops it
      back into the pool; when that `ROLLBACK` fails or times out, or when no Tokio runtime is current, the connection
      is marked `close_on_drop()` and dropped, so it is closed; so a contended claim, a refused or replayed report or
      any other early return (the ports roll back by dropping: `crates/application/src/execution.rs:1649-1650`;
      the claims' `Ok(None)` after `BEGIN` at `crates/infra-postgres/src/execution.rs:867-878`, `:912-923` and
      `:1606-1616`) keeps its backend;
    - a variant of `begin_safely` (`crates/infra-postgres/src/pool.rs:99-132`) acquires the connection and sends
      `BEGIN` inside its spawned task, as `begin_safely` does, so a cancelled caller never leaves a half-begun
      connection (the task's result, dropped with the caller gone, is rolled back by the `Drop` above);
    - `PgRunTx` and `PgPathExecutionTx` (`crates/infra-postgres/src/execution.rs:3825` and `:942`) hold an
      `OwnedTransaction` instead of a `Transaction<'static, Postgres>`, for every command they serve (reports,
      `open_run` at `execution.rs:447`, admission at `:1513`, claims); `begin_in_transaction` (`execution.rs:280-293`)
      takes an `OwnedTransaction`, and its callers in `crates/infra-postgres/src/primitives.rs:3551-3846` and
      `crates/infra-postgres/src/chain_progress.rs:2346-2376` begin one with the variant above;
    - the transaction helpers that take `&mut Transaction<'_, Postgres>` (or `<'static, …>`) and are called with the
      run or path transaction take `&mut PgConnection` instead, their other callers passing `&mut *tx`: those of
      `crates/infra-postgres/src/execution.rs`, `crates/infra-postgres/src/execution/path.rs`,
      `crates/infra-postgres/src/execution/scoped_events.rs:102`, `crates/infra-postgres/src/execution/database.rs`
      (its `impl PgRunTx`), `crates/infra-postgres/src/breaker.rs:74`, `:135` and `:161`, and
      `crates/infra-postgres/src/declared_operations.rs:1230`;
    - the transactions keep the carry's returned `top_up` and the principal's tenant, and in `commit`, once `COMMIT`
      has returned, when `top_up` is true, call a helper of `crates/infra-postgres/src/step_logs.rs` that moves the
      connection into a spawned task, runs `SELECT * FROM sagat_quota_step_log_top_up($1)`
    with that tenant, as one statement in its own implicit transaction, the last on that connection, and then drops the connection back into the
    pool; `commit` returns without waiting for it, so the report's answer is not delayed; no top-up runs after a
    transaction that did not commit, after a carry whose `path` is `none`, or for a carry C0 refused; the application
    port does not change and the adapter holds no rule (SQL decides `top_up`); the store counts
    `step_log_quota_top_ups_total{outcome}` (`error` for any other error of the top-up's statement, which is not
    retried) and nothing else of it;
  - the port `StepLogCarryTx`, one method that makes the one call of `sagat_step_log_carry`, as a supertrait of the run
    and path transactions, implemented for `PgRunTx` and `PgPathExecutionTx` in `crates/infra-postgres/src/step_logs.rs`
    through a `pub(crate)` connection accessor in `execution.rs`; a property test that runs the function and the
    domain's `plan_batch` on the same generated carries (new and existing streams, each ceiling met, the tenant allowance
    exceeded, `in_order` false, duplicates, gaps, each mark-only reason, `failed` included, and each answer of the
    function's block) and requires the same kept sequences,
    marker, counters and drops by reason (D041-100); nothing outside the function's own allowance read changes its
    plan (no answer of an earlier transaction is an input);
  - `StoreError::StepLogRefused` (a guard's raise, by its constraint name) and `StoreError::StepLogCarry` (the
    SQLSTATE only) in `crates/application/src/error.rs`, and the internal `AppError::StepLogRefused` and
    `AppError::StepLogCarry` they become, which `finish` and `finish_path_report` return unchanged and the API edge maps
    to `UNAVAILABLE` (a test shows that neither is ever answered by a report);
  - the downgrade re-run of section 2a around `complete_inner`, the root body of `fail` and `report_path` (reusing its
    `loop`), consuming only those variants: `Full` → `MarkOnly` (reason `failed`) after a guard's raise, → `None` after a
    failed carry call, a content-quota raise going once to its fallback with the downgraded carry, so at most four
    transactions (ADR-0095 decision 5);
  - CE's `last_stored_xid` and the order guard (T9's update of it is task T041-6's, as is the stream lock in T3); the
    metric `step_log_carries_total{outcome}` (`written`, `mark_only`, `off`, `invalid`, `refused`, `removed`,
    `final_recorded`, `contended`, `failed`), the carry's lines counted in the ingest's line, byte, truncation, redaction
    and drop metrics (`step_log_lines_dropped_total{reason}` with `DropReason`, `out_of_order` and `failed` included),
    `step_log_quota_fallbacks_total` (one per carry whose `path` is `refill`), and `step_log_quota_refusals_total{cause}`
    from the carry's returned cause; for each `service_busy` carry a structured warning naming the tenant, at most one
    per tenant per minute per replica, carrying the count since the last;
  - the test pause of a report, on the database side: `tests/support/step_logs.rs` gains
    `pause_report_after_journal(run)`, which installs an `AFTER INSERT` trigger on `run_events` that, for that run's
    rows only, waits on an advisory lock the test holds, so that a report is held after its journal write and before
    its carry call, its transaction id assigned, holding its run row and no lock the carry tries;
  - the fitness row of ADR-0095 in `docs/engineering/architecture-principles.md`, naming `work_budget.rs`'s carry
    equalities, the restated source-text test, the lock-order and held-lock tests and the plan property test (the
    existing step-log row's status corrected to Accepted);
  - the removal of task T041-35's prototype: delete `crates/infra-postgres/src/report_hook.rs`,
    `tests/performance/scalability-ha/{step_log_diag.rs,step_log_leases_tests.rs,carry_diag_rounds.sh}`; remove the
    `perf-spike` feature from `crates/infra-postgres/Cargo.toml` and `tests/Cargo.toml`, `HookSlot` and
    `with_report_hook` from `crates/infra-postgres/src/{execution.rs,lib.rs}`, and every `cfg(feature = "perf-spike")`;
    rename `tests/performance/scalability-ha/step_log_spike.rs` to `step_log_load.rs`, keeping the driver's workloads
    and porting its carry mode to the carrying entry points (the scratch schema `spike_041` and its functions removed);
    `run.sh` and `carry_spike_rounds.sh` build without the feature; `carry_spike_judge.py` is kept and judges the
    built mode; `.github/workflows/perf-spike.yml` builds without the feature, runs on a push to a `perf/**` branch only
    (its header says so), and is what tasks T041-37 and T041-9 run. The prototype's tests and runs stay recorded in
    `specs/041-step-logs/validation-report.md`.
  Tests, in `tests/integration/claim-transition-efficiency/`, each with one expected outcome:
  - the six equalities of section 2a in `work_budget.rs`, counted as the existing three are and beside them, through a
    carrying `World::complete_carrying` in `tests/support/engine.rs`, the existing three passing with their constants
    unmodified, each pinned warm (fast path, `top_up` false: no statement on that backend after the commit); and, for
    the completion with a next step whose short step carries ten records, pinned warm-low (fast path, `top_up` true:
    exactly one top-up statement after the commit, in its own transaction, on the same backend) and cold (no quota
    lease: `path` `refill`, its block, one write of the usage row and one quota-lease insert in the second measure,
    and no top-up after); a warm carrying report runs no statement outside its budget (after the commit, at a barrier,
    `pg_stat_activity` shows the report's backend idle with no query since the `COMMIT`, and the `calls` of the
    top-up's statement in `pg_stat_statements` are unchanged);
  - the second and third measures of section 2a asserted with each: the step-log rows a report transaction wrote,
    counted from the step-log tables; and the call's nested statements (`toplevel = false`) and their rows, an equality
    whose value the test's comment derives statement by statement for each case (the prototype's derivation of 26 for
    its fast path, `step_log_leases_tests.rs:776-783`, is the pattern; the built function sets no `application_name`,
    so its counts are derived anew), each shown failing once against a statement planted by hand in the function; the
    test asserts first that the server tracks nested statements (`current_setting('pg_stat_statements.track') =
    'all'`) and fails with a message naming the setting otherwise; the test's comment says why what a function does is
    counted apart;
  - the source-text check that the function's body never names `tenant_retained_usage`; and, at every other call site
    (a root `FailStep`, a scoped completion and a scoped failure, `reject_secret_output`, each content fallback, and a
    long step's carry onto an existing stream), the cost with a carry minus the cost without one, equal to the row of
    section 2a's table;
  - the source-text test of T041-5 restated so that the worker and execution modules reach the log tables only through
    `StepLogCarryTx`;
  - the lock-order test with a carrying report beside an ingest of the same attempt, the purge, and a change of a
    step-log resource, at barriers, all committing;
  - an ingest held after T3 (it holds the stream lock): a carrying report of its attempt commits at once with its carry
    `contended` and no line stored; a change of `step_log_attempt_records` holding its fence exclusively: a carrying
    report commits at once with its carry `contended`;
  - the carrying-report half of SC-041-001's quota-lease test: with one ingest of a tenant held after T8 (its own quota
    lease held), a carrying report of that tenant commits while it is held, charged to its own quota lease, not waiting
    (`pg_stat_activity`);
  - the carry's held-lock cases, moved from task T041-5 (decision D041-131) and each shown failing under its hand
    mutation: past *b* a carry answers `service_busy` and its report commits, whatever the caller's settings; a
    reserved carry writes with no subtransaction; a carry with nothing to charge opens no subtransaction (mutation 20
    of the validation report); after a top-up the next carry on that backend takes the fast path (mutation 18 rewritten
    for the top-up: a top-up that answers without granting fails it).
  Tests, in `integration_step_logs` (which T041-32 selects for these paths), each asserting the stored rows, the
  stream's `final_at` and the outcome counter, with one expected outcome:
  - a carry stored with a root completion, a root failure, a scoped completion and a scoped failure;
  - a resent report stores nothing more, also through `lose_report_ack` of T041-8, and a resend with another carry is
    answered as a replay; a batch held after T2 while its attempt's carrying report commits gets `FINAL_RECORDED`;
  - two runs of one tenant whose carries cross its `tenant_step_log_bytes` limit: both reports held inside their call,
    after its allowance read, by a `BEFORE INSERT` trigger the test installs on `step_log_lines` that waits on an advisory
    lock the test holds (the test replaces the fallback's budget function with its own bound in its database, so that
    the test, not *b*, bounds that wait), then released: one commits, the other is refused by its quota-lease charge and
    answered in the same call with a mark-only final mark, its outcome and output as without a carry, its records
    counted `tenant_quota`, no re-run; for a completion and for a failure;
  - the top-up (decision D041-124): `top_up` is true for a carry that leaves 65,535 bytes of room and false for one that
    leaves 65,536; after a carrying report with `top_up` true, the top-up runs on the backend of the report's
    transaction (`pg_backend_pid()` read by the test before the commit) and its quota lease then holds at least
    65,536 bytes (polled with `eventually_within`); with the tenant's usage row held by the test, the report's answer
    reaches the worker while its top-up still waits (`pg_stat_activity`); a top-up answered `busy` (the row held past
    *b*) leaves the next carry on that backend to fall back and land; `contended`, `limit` and `error` answers
    are counted and change no report; no top-up runs after a mark-only, `contended` or `off` carry, a report that
    rolled back, or a carry C0 refused; a worker of tenant A sending a carrying report on tenant B's run gets the answer
    it gets for a missing run, and B's usage row, quota leases and counters are unchanged;
  - the wait slots (decisions D041-125, D041-134 and D041-137), with held locks, `pg_stat_activity` and `pg_locks`, the
    store built with *Q* = 5 (*P* = 3, a pool of 20's), which its carry calls pass as `p_wait_slots`, and the fallback's
    budget function replaced by the test's own bound in its database, so that no arrival depends on timing (rule 18):
    - tenant A's usage row held by a test transaction; *P* + 2 = 5 carrying reports of A whose carries need a fallback
      (A's quota leases emptied first, as T041-6's fixture does), on one `application_name`: exactly 3 wait on the row's
      lock, each holding one tenant slot and one replica slot (`pg_locks` of the slot keys), and the other 2 commit with
      `service_busy` while it is held (3 + 2 = 5);
    - the replica cap: the usage rows of five tenants held, one waiting fallback of each (5 = *Q* replica slots held); a
      sixth tenant's carry that needs to wait answers `service_busy` at once;
    - meanwhile a carrying report of tenant B, with room in its quota lease, commits; and on a store with a pool of 8
      (*Q* = 2, *P* = 1, the Logs cap 4 filled by four ingests held after T3), with the usage rows of five tenants held
      and one fallback of each started, a heartbeat and a task lease of B get their connections (8 − 4 − 2 = 2 free),
      so B is not starved by the others' waits;
    - a carry call made with `p_wait_slots` 5 on a backend whose previous transaction set nothing: the function reads
      5, and the next transaction on that backend reads 1 (the setting is transaction-local);
    - shown failing under three hand mutations (*P* computed as *Q*; the replica slot skipped; the tenant slot skipped);
  - a `service_busy` carry: the stream's `not_stored_reason` is `service_busy` and `ceiling_reason` unchanged, asserted by
    SQL (the streams route reads it in task T041-11); the warning is written once for two such carries of a tenant
    within a minute, the second counted in the next one;
  - a guard's raise (a quota lease's `CHECK` forced by the test): the report is re-run once with a mark-only carry whose
    reason is `failed`, two transactions in all, the same outcome and output, the carry's records counted `failed`, and
    the stream keeps its final mark (ADR-0095 decision 5);
  - a failing carry statement (a trigger the test installs that raises): the report is re-run once with no carry, the
    same outcome and output, `failed` counted, no final mark; the same failure in a content fallback's mark-only carry
    runs it once more with none, the third transaction, and a guard's raise followed by a content raise and a failing
    mark-only call reaches the fourth and last;
  - the owned connection (decision D041-137), on a store whose pool has one connection, in
    `tests/fault/events-webhooks-messaging/cancellation.rs` (beside the test of `begin_safely`): two committed
    transactions in a row, a contended claim that returns `Ok(None)` after `BEGIN` and is dropped, and a refused report
    each run on the same `pg_backend_pid()`, and after each no backend of the pool is `idle in transaction`
    (`pg_stat_activity`); a report future dropped between `BEGIN` and `COMMIT` while its statement waits for a row lock
    the test holds (seen waiting in `pg_stat_activity`), the lock then released: the connection comes back rolled back
    on the same backend and nothing of the report is stored; a `COMMIT` whose backend the test terminates with
    `pg_terminate_backend` at a barrier: the connection is closed and the pool's next one is a new backend; each shown
    failing under a hand mutation (the `Drop` returning the connection with no `ROLLBACK`; the `Drop` closing on
    every drop);
  - the source-text check that only the store's top-up (in `crates/infra-postgres/src/step_logs.rs`) calls
    `sagat_quota_step_log_top_up`;
  - an ordinary `AppError::Unavailable` of a report (the database stopped) is not re-run;
  - a carry on a report whose output exceeds `tenant_content_bytes` (the content fallback, a mark-only carry); on a report
    whose output holds a declared secret, root and scoped; and on a failure whose code holds one, root and scoped: a
    mark-only carry each, no line stored;
  - C0 for a runner token on an ordinary step and a database-runner on a managed SQL and a lake step: nothing stored,
    the report unchanged;
  - an invalid carry with a valid `highest_sequence` (mark-only) and without one (ignored), capture off in the namespace,
    the deployment switch off, the store proof missing and a removed stream, each leaving the report's outcome and output
    as without a carry at the cost of section 2a; a call with a `p_mark_reason` outside the four refused by the function;
  - a long step's carrying report while its stream row is held, uncommitted, after the purge's mark has updated it
    (`removed_at`), after the purge's delete has set `purged_at`, and by the delete route held at its audit write: in each
    case CC offers no row and does not wait, CC′ finds the row locked, the carry is `contended` and the report commits at
    once (the test asserts the report's commit while the other transaction is still held);
  - the order guard: a report paused by `pause_report_after_journal` (its transaction id assigned), an ingest of its
    attempt that then begins (a higher id) and commits, the report released: the ingest's lines kept, the carry's
    records counted `out_of_order`, the final mark recorded; and a batch with a lower id that stores only duplicates
    after a higher one, which leaves `last_stored_xid` unchanged;
  - the re-run refused because the task lease expired between its two transactions (G041-021): the worker gets the
    refusal a late report gets;
  - a carry on a compensation of a stopping run stored (OD-2026-11-16-11); the counting rule over batches and a carry;
    the server canary (a secret value the test's attempt resolved and logged, as T041-8 defines it) carried on a report and
    searched as T041-8 searches it.
  Done when these pass, also as the runtime role, and `python3 scripts/check-upgrade-boundary.py` and the privilege
  audit pass.
  Refs: FR-041-004, FR-041-005, FR-041-006, FR-041-008, SC-041-001, SC-041-002, SC-041-003, SC-041-004
  Owner-gate: OD-2026-11-15-5, OD-2026-11-15-6, OD-2026-11-16-9, OD-2026-11-16-10, OD-2026-11-16-11, OD-2026-11-19-1, OD-2026-11-20-1, OD-2026-11-20-14, OD-2026-11-20-15, OD-2026-11-20-16, OD-2026-11-20-18
  Files: create the migration `<timestamp>_step_log_carry` (`.up.sql`, `.down.sql`),
  `crates/application/src/step_logs/carry.rs`, `tests/integration/step-logs/carry.rs`; delete
  `crates/infra-postgres/src/report_hook.rs`,
  `tests/performance/scalability-ha/{step_log_diag.rs,step_log_leases_tests.rs,carry_diag_rounds.sh}`; rename
  `tests/performance/scalability-ha/step_log_spike.rs` to `step_log_load.rs`; modify `crates/domain/src/step_logs.rs`,
  `crates/application/src/{worker.rs,worker/output.rs,worker/scoped_reports.rs}`,
  `crates/application/src/{worker/scoped_output_refusal.rs,execution.rs,step_logs.rs,error.rs}`,
  `crates/infra-postgres/src/{step_logs.rs,execution.rs,execution/path.rs,execution/scoped_events.rs,execution/database.rs,identity.rs,lib.rs,pool.rs}`,
  `crates/infra-postgres/src/{primitives.rs,chain_progress.rs,breaker.rs,declared_operations.rs}` (the owned
  connection), `tests/fault/events-webhooks-messaging/cancellation.rs`,
  `crates/api-grpc/src/service.rs`, `crates/api-http/src/error.rs`, `crates/telemetry/src/metrics.rs`,
  `apps/execution-engine/src/{lib.rs,settings.rs}`, `tests/support/{api.rs,engine.rs,mod.rs,step_logs.rs}`,
  `tests/integration/claim-transition-efficiency/{work_budget.rs,step_log_isolation.rs}`,
  `tests/integration/step-logs/main.rs`, `tests/integration/platform-foundation/runtime_privileges.rs`, the three oracle
  files of T041-4, `tests/performance/scalability-ha/{main.rs,run.sh,carry_spike_rounds.sh,carry_spike_judge.py}`,
  `.github/workflows/perf-spike.yml`, `crates/infra-postgres/Cargo.toml` and `tests/Cargo.toml` (the feature removed),
  `docs/engineering/architecture-principles.md`, `specs/041-step-logs/data-model.md` (§7, the metric owners).
- [ ] T041-37 Measure the built quota leases' regimes (decisions D041-124, D041-125, D041-131, D041-134 and D041-135), right after T041-36
  and T041-8 and before any later task, so that nothing is built on a quota-lease mechanism that has not been measured.
  - *Environment and rounds*, as task T041-35's fourth run: the GitHub-hosted runners of decision D041-102, through
    `.github/workflows/perf-spike.yml` on a push to a `perf/**` branch (a fast-forward of the branch measured, never a
    force), four runners of three rounds each judged in the fixed order of decision D041-104; a round is void when its
    two capture-off figures of one size differ by more than 10% or it stops; a figure is the median of the first three
    valid rounds in that order; a criterion stated as "none" or "every" holds in every valid round. The driver is the
    built mode of `tests/performance/scalability-ha/step_log_load.rs` (T041-36). Each row runs at least 90 seconds: 30
    seconds of warm-up, then a steady window of at least 60 seconds, which is what is measured.
  - *Setup*: several tenants are created by the driver, each with its own namespace and workers spread round-robin
    over the replicas, with `tenant_step_log_bytes` set as the regime states; each in-process replica's pool has its
    own `application_name` (task T041-5), which the setup asserts in `pg_stat_activity` (distinct names, one a
    replica) before the first row; a held usage row is a test connection's
    `SELECT … FROM tenant_retained_usage WHERE tenant_id = $1 FOR UPDATE`, held for the stated time and taken again
    every 2 seconds during the window; the pooler is PgBouncer 1.21 or later (1.21 is the first release that tracks protocol-level
    prepared statements in transaction mode; the package version installed is pinned in `run.sh` and recorded), started
    by `run.sh` when `SAGAT_PERF_POOLER=transaction` with `tests/performance/scalability-ha/pgbouncer.ini`:
    `pool_mode = transaction`, `max_client_conn = 400`, `default_pool_size = 128` (the sizing rule of section 2b: the
    8 replicas × 20 connections = 160 client connections onto Σ (⌊20 / 2⌋ + 5) + 8 = 8 × 15 + 8 = 128 server backends),
    `max_prepared_statements = 200` (sqlx
    prepares every query as a named statement and caches it per connection, so with the default 0 PgBouncer would
    answer the platform's statements with prepared-statement errors; the value is set explicitly whatever the
    version's default; 200 is an estimate above the statements one connection prepares on the benchmark's paths, and
    `run.sh` fails the row when PgBouncer's `SHOW STATS` reports a prepared-statement eviction), `auth_type = scram-sha-256` and `auth_file =
    tests/performance/scalability-ha/pgbouncer-userlist.txt`, the userlist holding the benchmark's own disposable role
    and its SCRAM verifier, which `run.sh` reads back from `pg_authid` after setting the role's password at start, so
    that the two match (never committed with a secret); the pool's startup `options` (its statement and idle timeouts,
    `crates/infra-postgres/src/pool.rs:59-71`): `run.sh` records how the pinned version treats that parameter, and when
    it refuses or drops it, `pgbouncer.ini` lists `options` in `ignore_startup_parameters` and the benchmark role
    carries the same timeouts server-side (`ALTER ROLE … SET`), checked through the pooler (`SHOW statement_timeout`)
    before each row; before each pooler row the driver also checks, through the pooler, that a store transaction sees
    `sagat_quota_step_log_wait_slots()` = *Q* = 5 and the next transaction 1; every pooler row records `SHOW POOLS`'
    `cl_waiting` and `maxwait` each second.
  - *Regimes*, each criterion and its consequence fixed here, before the first round:
    - *Cold start*: every replica's pools opened fresh, as after a deploy, no setup quota lease, workload (a) at
      8 × 128, once with one tenant and once with 100 tenants; this row has no warm-up: it is measured from the moment
      the pools open, so that its first carries are in the window. Measured: fallbacks and their waits, `service_busy`
      carries, top-ups by answer, the time until every backend holds a quota lease, each backend's first carrying
      report's completion time, and every pool acquire of a heartbeat or a task-lease call. Criterion: `service_busy`
      carries at most 0.1% of carries; every backend's first carrying report within twice the steady window's
      completion p99; no heartbeat or task-lease call failing to get a connection.
    - *Near the limit*, with a purge beside the workload: the tenant's `tenant_step_log_bytes` set 4 MiB above its
      usage at the window's start, and lines of a pre-filled, expired set of streams purged beside it, so that the free
      room stays under 4 MiB plus what the purge frees. Measured: refills per charge, top-ups by answer, fallbacks and
      their longest wait, the longest carry (its whole time), held refusals, the XID rate and `pg_stat_slru` for
      `subtransaction`; U and L sampled each second through `sagat_quota_step_log_usage`. Criterion: U ≤ L at every
      sample; `service_busy` carries at most 0.1%; the longest carry at most 1,000 ms (the fourth run's longest
      refill-written carry, 726.7 ms, rounded up to the next multiple of *b*, 800 ms, plus one *b* of margin).
    - *A small quota with many backends*: L = 100 MiB, every backend of 8 × 128 charging. Measured: refills per charge,
      held refusals and, at each, the unleased room, the open writers and their held room. Criterion: every held refusal
      comes with unleased room under its charge and held room within the operator's bound of section 2b (*k*_open ×
      1 MiB plus their planned charges); `service_busy` carries at most 0.1%.
    - *Many low-volume tenants*: 1,000 tenants at 8 × 128, each under 1 MiB per backend per 10 minutes. Recorded, not
      judged: fallbacks per carry, expected near every carry, since with 1,000 tenants over 160 backends nearly every
      carry is its (tenant, backend)'s first and falls back (decision D041-124). Measured besides: top-ups, the XID rate
      and `pg_stat_slru`. Criterion: the completion p99 within gate (2b)'s allowance over the same workload with one
      tenant; the subtransaction rate under 1,000 a second (the rate below which `pg_subtrans` stays in its default
      buffers on the benchmark's server); `service_busy` carries at most 0.1%.
    - *Behind a transaction pooler*: workload (a) at 8 × 128 through the pooler. Measured: fallbacks, top-ups by
      answer and `service_busy` carries per carry. Criterion for ADR-0097 option 1c (decision D041-130): carry fallbacks
      at most half of all refills (carry fallbacks plus top-ups answered `made`), and `service_busy` carries at most 0.1%.
    - *Two tenants, one the victim* (decisions D041-125 and D041-134), in the near-the-limit regime and behind the
      pooler: tenant A held in the fallback regime (its usage row held 150 ms every 300 ms), tenant B far from its
      limit, each with half the workers. Criterion: B's completion p99 and its pool-acquire wait p99 each within the
      larger of 10% and 2 ms of B's own figures in a solo row of the same round (A absent); no heartbeat or task-lease
      call of B failing to get a connection; at no sample more than *P* = 3 of a replica's backends holding a wait slot
      of A (`pg_locks` of the slot keys, by `application_name`; waits at a commit for the content class hold no slot and
      are not counted).
    - *Three or more tenants in the fallback regime* (decisions D041-134 and D041-137), with direct connections and
      behind the pooler: tenants A1, A2 and A3 each held as A above, tenant B far from its limit running 64-record
      carries with a 1 KiB output (carries that need a slot when they fall back while B's own output commit holds B's
      usage row), a quarter of the workers each. Criterion: at no sample more than *Q* = 5 of a replica's backends
      holding a replica wait slot; no heartbeat or task-lease call of B failing to get a connection, and behind the
      pooler B's `maxwait` within B's completion p99; B's completion p99 within the larger of 10% and 2 ms of its solo
      figure; B's `service_busy` carries at most 0.1% of B's carries (the loss the replica cap couples, section 2b).
    - *Ticket defaults* (decision D041-135): workload (g) at 1 × 16 and 8 × 128 with the shipped defaults (cap 10,
      tickets 7). Measured: ticket refusals and gate (4)'s formula. Criterion: refusals under 1% of batches.
    - *Fallbacks with the top-up* (the zoom-out re-read's Nit to Z-1): workload (a) at 8 × 128 far from the limit with
      direct connections. Criterion: carry fallbacks at most 0.31% of carries, the fourth run's refill share with every
      refill in a report, so that the top-up is shown to take steady refills out of reports.
    - *A fan-out run during fallback waits*, caused on purpose: a test transaction holds the usage row for 150 ms, and
      in a second row for 400 ms, while a fan-out run's carries fall back; beside it, tenant B far from its limit with a
      quarter of the workers, its own workload (a). Criterion: no report of the run waits for the run row longer than
      the longest measured fallback wait plus one report's p99; no heartbeat or task-lease call of B failing to get a
      connection; B's completion p99 within the larger of 10% and 2 ms of its solo figure.
  - *Which regimes count for OD-2026-11-20-14's rule* (question O041-041 is put again above 0.1% of carries answered
    `service_busy`): every regime above, except the rows whose busy answers are induced, as the owner confirmed
    (OD-2026-11-21-3): the fan-out's held rows (the 400 ms hold answers busy by construction, 400 ms > *b* = 200 ms)
    and tenant A of the two-tenant regime, and with them tenants A1, A2 and A3 of the three-tenant regime, held the
    same way as A (decision D041-137); B's carries count in each.
  - *Consequences*, one for each criterion: a `service_busy` rate above 0.1% in a counted regime puts question O041-041
    to the owner again (OD-2026-11-20-14); U above L at any sample, or a held refusal outside the operator's bound, is a
    defect of the quota leases, fixed and the regime run again before any later task; the pooler's criterion missed
    reopens ADR-0097 option 1c for the owner (decision D041-130); a cold-start or low-volume criterion missed (a first
    report beyond its bound, a completion p99 beyond the allowance, a connection refused) is a design change of decision
    3b, which the owner decided (OD-2026-11-20-18), put to the owner with the figures and the named alternatives (a
    background refiller, ADR-0097 option 1e, or a top-up before a backend's first carry for a tenant); the longest
    carry beyond 1,000 ms, or a fan-out wait beyond its bound, restates the risk row of section "Risks" and is reviewed
    again; the two-tenant or three-tenant criterion missed (B's busy share included) lowers *P* or *Q* or adds a
    per-tenant pool share, a design change reviewed again before any later task; tenant B's criterion in the fan-out
    regime missed is a design change of the fallback under the run lock, reviewed again before any later task; a
    pooler row whose transactions do not see *Q*, or whose timeouts do not reach the server, is a defect of the
    setup, fixed and the row run again; ticket refusals of 1% or more at the defaults raise the tickets'
    default, a design change reviewed again; fallbacks above 0.31% with the top-up are a defect of the top-up, fixed
    and run again.
  Done when every regime's figures are recorded in `specs/041-step-logs/validation-report.md`, each criterion is read
  with its consequence in `specs/041-step-logs/research.md`, and no consequence that stops the plan is open.
  Refs: NFR-041-PERF, SC-041-001, FR-041-005, FR-041-006
  Owner-gate: OD-2026-11-20-13, OD-2026-11-20-14, OD-2026-11-20-18
  Files: modify `tests/performance/scalability-ha/{main.rs,step_log_load.rs,run.sh,carry_spike_rounds.sh,carry_spike_judge.py}`,
  `.github/workflows/perf-spike.yml`, `specs/041-step-logs/{validation-report.md,research.md}`; create
  `tests/performance/scalability-ha/pgbouncer.ini` (the userlist `pgbouncer-userlist.txt` is generated by `run.sh` and
  ignored by git).

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
  purged is shown expired; a stream whose carry was answered `service_busy` shows `notStoredReason: service_busy` and
  its `ceiling` unchanged (decision D041-129); the completion predicate as a pure function, one unit test per case; a race of two first reads that
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
  sharded `integration` job of `.github/workflows/ci.yml` gains a step, under the condition that its shard's targets
  (`matrix.shard.targets`, the form of `ci.yml:155`) hold `integration_step_logs`, that runs
  `cargo test -p sagat-tests --test integration_step_logs --locked horizon_` with `SAGAT_TEST_POSTGRES_TAG=18-alpine`.
  `docs/reference/api.md` is regenerated after reading the diff. Done when every test of this task passes, also as the
  runtime role, and the `horizon_` tests pass on 16 and 18.
  Refs: FR-041-002, FR-041-010, FR-041-011, FR-041-013
  Owner-gate: OD-2026-11-13-14, OD-2026-11-13-18, OD-2026-11-15-4, OD-2026-11-15-11, OD-2026-11-16-8, OD-2026-11-20-14
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
`logs` and `logsSecret`, the `stepLogs` op and the `verdict` and `holdAnswer` faults (defined by T041-14); it adds the
micro-benchmark of section 6 and removes
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
  `mustNotStore` and its self-test with a planted canary; and a new fault action, `holdAnswer`, which
  `step-logs-transport-faults` uses, written in the harness's `faults[]` form
  (`tests/contract/sdk-conformance/README.md:558-570`) and applied, as every other fault, by each SDK runner at its
  transport hook (its gRPC interceptor; in Rust the fault proxy): `{"channel": "grpc", "match": "AppendStepLogs",
  "action": "holdAnswer", "times": 1, "after": <n − 1>, "maxHoldMs": 3000}` lets the *n*-th matching call reach the
  server, holds its answer, and releases it when the scenario runs the op `releaseAnswer` or when `maxHoldMs` passes;
  each runner records in `out.json` an observation `heldAnswers`, one entry per hold in file order,
  `{"reportSent": "beforeRelease" | "afterRelease", "afterHoldMs": <ms from the hold to the report>}`, and the
  normalizer (`normalize.rs`) replaces `afterHoldMs` by the bucket `"under2000"` or `"atLeast2000"`, so that the golden
  asserts, identically in six languages, `afterRelease`, or `beforeRelease` with `atLeast2000`); the five scenarios and their goldens, written by hand from
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
  FR-041-017 (the missing permission from the session, the missing grant from the refusal), a stream's `ceiling` and
  `notStoredReason` each as a line of text with its drop count, the busy one verbatim "not stored: the log service was
  busy (not your quota)" (decision D041-129), markers, the paging through
  empty pages of section 8, windowing, text-only rendering without links and with bidirectional and zero-width
  characters made visible, and `ScriptLogPanel` for a script marker row; unit tests, one for the busy text. The Studio
  has no quota screen today; the generated `Quota` type carries `usageWithheld` (decision D041-133), so that a later
  quota screen shows no usage figure for `tenant_step_log_bytes` when it is true. Done when `npm test` and
  `npm run check:api` pass in `frontend/apps/studio`.
  Refs: FR-041-017, FR-041-019, FR-041-022
  Owner-gate: OD-2026-11-13-18, OD-2026-11-15-9, OD-2026-11-16-8, OD-2026-11-20-14
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

- [ ] T041-9 Measure the built ingest and carry on the benchmark of SC-041-007, the scoped report path included, and
  judge SC-041-007 by its per-size criterion (decision D041-126), fixed in SC-041-007 before any of this task's rounds.
  The environment and the validity rule are task T041-37's (the hosted runners of decision D041-102, task T041-35's
  fourth run's rules); this task runs all twelve rounds (four runners, three rounds each) and judges each gate on the
  median of all its valid rounds, at least seven, with SC-041-007's rule for a run with fewer (decisions D041-135 and
  D041-136). The driver's built mode
  (`tests/performance/scalability-ha/step_log_load.rs`, T041-36) calls the application's ingest
  (`sagat_application::step_logs`) and the carrying report entry points of T041-36 on each in-process replica, as the
  benchmark calls the application's completion, with the ticket semaphore set to the cap and tickets of 10 that the
  fourth run measured, since gate (4) sizes the need (decision D041-135; the shipped defaults of decision D041-127 are
  run by task T041-37, and the gRPC tickets themselves are covered by T041-7's tests); the baseline is the same workload with capture
  off, as SC-041-007 states, and the tenant's `tenant_step_log_bytes` is set to the catalogue's maximum, so that no
  charge is refused. Workloads, named as in SC-041-007: (a) and off-1K, (f) and off-{}, at both sizes; (g) at both
  sizes, its batch's answer awaited by the driver (the SDK's wait before a report); (c) a fan-out: a workflow with a
  `forEach` over 200 items at concurrency 64 whose body is one step carrying ten lines (the scoped path), at 1 × 16;
  (h) 64-record carries at 1 × 16; (b) full batches of 500 records of 200 bytes from the tenant's ten tickets ×
  *R* concurrent attempts, *R* the replicas. It records SC-041-007's gates (1), (2b), (3) and (4) at both sizes and gates
  (1) and (2b) for (c) against its capture-off twin, recorded and not judged (the floor of SC-041-007 is derived for
  workload (a); (c) is judged by its own risk row's remedy: below 90% of its twin's rate, the scoped path carries
  nothing, section "Open Questions"); the p99 of task-lease calls while the modelled Logs class is full;
  the trigger cost and commit latency of a (b) batch; the purge rate against the ingest rate and the completion p99
  during a purge; what (g)'s awaited batch adds to a completion; the stored size per line and per stream row, the
  stream table's dead tuples after the run, and the HOT ratio of a long step's stream updates with the fill factor of
  70 and without it (section 3), in the dated section of the validation report. Done when the figures are recorded
  and SC-041-007 is read by its per-size criterion: at 8 × 128 gates (1), (3) and (4) as originally stated and (2b)
  at most +112.9 ms (OD-2026-11-21-1), at 1 × 16 the floor of decision D041-126 (gate (1) at least 80.4%, (2b) at most
  +29.1 ms, (3) at least 83.5%, (4) at most 10 tickets with refusals under 1%); a failure stops the release of the
  feature and goes to the owner with the record, never re-run. An unchanged build fails a gate whose bar is its worst
  valid round with a probability of at most 3.85% per gate with seven to twelve valid rounds, and the release, over
  all gates, with at most 34.1% at eight valid rounds and 20.1% at twelve (SC-041-007, decision D041-136). It also
  records, at each size, the median over the valid rounds of (a)'s completion p99 in percent over off-1K's and reports
  it to the owner when it exceeds the largest of the rounds the owner accepted (+36.9% at 1 × 16, +9.5% at 8 × 128),
  since the (2b) floors admit about +94% and +33.4%; that report does not stop the release, and the runbook's and the
  release notes' figures are restated from the record.
  Refs: NFR-041-COST, NFR-041-PERF, SC-041-007
  Owner-gate: OD-2026-11-19-1, OD-2026-11-20-13, OD-2026-11-20-14
  Files: modify `tests/performance/scalability-ha/{main.rs,step_log_load.rs}`, `specs/041-step-logs/validation-report.md`.
- [ ] T041-25 Alerts with their runbook entries and a promtool rule test `infra/observability/tests/step_logs_test.yaml`;
  the step "promtool test rules" of the `observability` job of `.github/workflows/ci.yml` (`:682-687`, which runs only
  `batch_stalled_test.yaml` today) changed to run every `*_test.yaml` of `infra/observability/tests/`, which also
  starts `instance_registry_test.yaml` and `trigger_schedules_test.yaml`, which must then pass too; the alerts of
  section 9, `SagatStepLogServiceBusyDrops` included (decision D041-128); the runbook `docs/runbooks/step-logs.md`
  completed, with entries for the top-up after the commit and the fallback (what `step_log_quota_top_ups_total`,
  `step_log_quota_fallbacks_total` and `step_log_quota_refusals_total{cause="busy"}` show, and the rate-limited
  warning that names a tenant), for the wait slots, and for gap G041-027: behind a pooler in transaction mode capture
  falls back more often, about the fourth run's regime with every refill inside a report, which the deployment
  accepts for the pooler's multiplexing (decision D041-130; `docs/runbooks/scaling.md` keeps its advice and is not
  changed to recommend session mode); the reference documentation (generated files regenerated after reading the
  diff), `docs/architecture/overview.md`, `docs/traceability.md`, a quickstart `docs/quickstarts/step-logs.md` with the
  happy and the failure scenario, and the release notes: the existing step-log section of `docs/releases/unreleased.md`,
  which opens with the measured cost (OD-2026-11-20-13), gains as its second line that capture is on by default
  (O041-001) and names the namespace and deployment switches, and stays marked as conditional on task T041-9. Done
  when the promtool step passes with the new test, and `contract_observability_operations` (every alert names an
  emitted metric and a runbook section) and `contract_developer_experience` pass.
  Refs: FR-041-018, FR-041-022, NFR-041-OPS
  Owner-gate: OD-2026-11-13-14, OD-2026-11-13-18, OD-2026-11-20-13, OD-2026-11-20-14
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
  Files: create `tests/e2e/step-logs/main.rs` and its `[[test]]` entry in `tests/Cargo.toml`, with its row in
  `scripts/integration-suite-durations.toml`,
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

None.

- [ ] T041-38 SC-041-007 as amended after the approval (the owner's answer "Derive them too", OD-2026-11-21-8; the
  accepted Majors OD-2026-11-21-200 and -205; decision D041-138). Task T041-9 judges SC-041-007 and records its report
  with these bars, which replace the ones SC-041-007 and T041-9 state for them:
  - at 8 × 128, gate (1) at least 88.6% (median of the rounds used 93.1%, worst valid round 88.63%, round 11,
    tolerance 4.5 points) and gate (3) at least 83.3% (median 91.5%, worst valid round 83.39%, round 9, tolerance
    8.2 points), derived as gate (2b)'s floor of D041-136 from the fourth run's eight valid rounds; gate (2b) at
    +112.9 ms and gate (4) as SC-041-007 states;
  - the false-stop bound: seven gates sit at their worst valid round (1 × 16's (1), (2b), (3) and (4); 8 × 128's (1),
    (2b) and (3)), each failed by an unchanged build with at most C(m, r) / C(8 + m, r) = 3.85%, so the release with at
    most 7 × 70/1,820 = 26.9% at m = 8, 17.9% at 7, 14.3% at 9, 20.6% at 10, 11.9% at 11 and 16.7% at 12 (a union
    bound under exchangeable rounds, replacing the 34.1% and 20.1% estimates);
  - the owner report on the tail compares like with like: the median over the valid rounds of (a)'s completion p99 in
    percent over off-1K's is reported to the owner when it exceeds +73.8% at 1 × 16 or +29.9% at 8 × 128, the worst
    valid round of the same figure in the fourth run (rounds 11 and 6), not the largest of the three rounds used
    (+36.9%, +9.5%), which the accepted build's own median (+36.1%, +10.4%) exceeded at 8 × 128; an unchanged build is
    reported with at most 3.85% per size; the report does not stop the release.
  The judge of T041-9 encodes these bars and the report threshold as constants with a test that derives each from the
  fourth run's valid rounds. Done when T041-9's record is judged by them and the runbook's and the release notes'
  figures cite them.
  Refs: SC-041-007, NFR-041-PERF
  Owner-gate: OD-2026-11-21-8
  Files: modify `tests/performance/scalability-ha/carry_spike_judge.py` (or the judge T041-9 uses) and its test,
  `specs/041-step-logs/validation-report.md`, `docs/runbooks/step-logs.md`.
- [ ] T041-39 The quota view reads the grant and the step-log usage in one snapshot (decision D041-141, amending
  D041-140 item 5; fail-closed as OD-2026-11-20-19 is). `QuotaStore::reveals_in_every_namespace` and the separate usage
  read of `tenant_step_log_bytes` are replaced by `QuotaStore::step_log_usage_if_revealed(tenant, subject)`, one
  `SELECT` of the membership role, the grant in every namespace and the usage; the application shows the usage only
  when the role may use a grant and every namespace has its grant; a view answers the grant with the usage of the same
  snapshot, or no usage. `QuotaStore::usage` refuses `tenant_step_log_bytes`. For task T041-5: the usage subquery of that
  `SELECT` (`crates/infra-postgres/src/reveal_grants.rs`, not `quotas.rs:1228-1235`) becomes a call of
  `sagat_quota_step_log_usage(m.tenant_id)` in the same statement, the function `STABLE` (checked by the privilege
  audit, and a hand mutation to `VOLATILE` fails the test below). Tests in `integration_security_multitenancy`:
  `quotas::a_view_that_reads_after_a_revoke_commits_never_shows_the_step_log_usage` (a revoke transaction deletes the
  grant, sets the usage to 7,777 bytes and holds `tenant_retained_usage`; the view a `clear` returns is seen waiting on
  it in `pg_stat_activity`; the revoke commits; 7,777 is never shown with the grant, and the next view withholds) and
  `quotas::the_store_refuses_the_step_log_usage_without_the_grant`. Done when they are seen failing (the first on the
  two-transaction read) and pass, with the quota tests, as the owner role and as the runtime role.
  Refs: FR-041-013
  Owner-gate: OD-2026-11-20-19
  Files: modify `crates/application/src/quotas.rs`, `crates/infra-postgres/src/{quotas.rs,reveal_grants.rs}`,
  `tests/integration/security-multitenancy/quotas.rs`, `specs/041-step-logs/{research.md,contracts.md,data-model.md}`,
  `docs/runbooks/step-logs.md`.

### Verification Plan

What each pull request runs, in CI and before it, is the "Gates" rule of the plan. The feature's integration suite is a
new target of `sagat-tests`, `integration_step_logs`, with real PostgreSQL, picked by the selector rule of T041-32
(constitution 4.5.0); the tests that guard the hot path live in `tests/integration/claim-transition-efficiency/`, which
the budget rule of design 038 (`scripts/select-test-targets.py:85-86`) selects for every change under
`crates/infra-postgres/` and the worker and execution modules of the application. Races use barriers or held locks,
never sleeps; an assertion on what a read returns polls until `heldBack` is false, within a bound.

| What is proven | How |
|---|---|
| The hot path is unchanged for a report without a carry (SC-041-001) | `work_budget.rs` with its constants unmodified while ingest runs between its windows; a batch commits under `lock_timeout = 50ms` while another transaction holds the run row `FOR UPDATE` and has updated the leased step row; a source-text test that the worker and execution modules never name the log tables and reach them only through `StepLogCarryTx` (T041-5, T041-36) |
| What a carry adds to a report (SC-041-001, section 2a) | The six equalities of section 2a in `work_budget.rs`, each pinned warm, and the warm-low (one top-up after the commit) and cold (the fallback, no top-up) pins; a warm carrying report runs no statement outside its budget; the second and third measures, the third derived in the test's comment and seen failing once (T041-36); the effect on time that SC-041-007 bounds, per size (T041-9); the cost with and without a carry at every other call site (T041-36) |
| A carry never changes a report | A quota refusal of the carry's charge, or a fallback past *b* or with no wait slot free, is answered in the same call by a mark-only final mark (`tenant_quota`, `service_busy`) and the report commits in its first transaction, no re-run; a guard's raise re-runs the report with a mark-only carry (reason `failed`) and a failing carry call with no carry, the same outcome and output, at most four transactions; a step-log lock held makes the carry `contended` with the report committed at once; an invalid carry, C0's roles and managed steps, capture off, the deployment switch off and a removed stream leave the report's outcome and output as without a carry; an ordinary `Unavailable` is never re-run (T041-36) |
| What a report waits for (SC-041-001) | With one ingest of a tenant held after T8 (its own quota lease held), three more ingests of that tenant commit, each on its own quota lease, none waiting (T041-6), and so does a carrying report of that tenant (T041-36); a refill held delays only a charge that needs a refill (T041-6); a carry's fallback refill waits for the usage row at most *b* and then answers `service_busy`, the report landing, and with no wait slot free answers at once (T041-36); the top-up after the commit runs while the report's answer is already sent (T041-36); an ingest held after T7 blocks neither a `lock_secrets` nor a completion (T041-6); a carrying report beside an ingest of its own attempt held after T3 commits at once with its carry `contended` (T041-36). The time bound is SC-041-007's (T041-9) and the regimes' (T041-37) |
| Quota exactness (ADR-0097 invariants I1 to I5, SC-041-001) | A property test of random interleavings of charges, refills, quota reclaims, releases and limit changes, each step checking 0 ≤ U ≤ S + G ≤ L (T041-5); U ≤ L sampled each second near the limit with a purge beside the workload (T041-37) |
| Room held by open writers is bounded (section 2b, G041-023) | `room_held_by_open_writers_is_bounded_from_the_free_room` at *k* = 1, 2, 8 and 32, a top-up's grant counted at its share; the exact remainder admitted from idle quota leases and one byte more refused; the small-quota test (T041-5); every held refusal within the operator's bound with a small quota and many backends (T041-37) |
| The quota-lease waits cannot deadlock (section 2b) | The deadlock cases of section 2b with held transactions and `pg_stat_activity`, a limit change against an open idle reclaim included; a release never waits for a held quota lease (T041-5); the lock-order test with its L3 and L3b slots (T041-6, T041-10, T041-36) |
| Quota reclaims | The idle reclaim folds only quota leases idle past the interval; an ended backend's quota lease is reclaimed at once; a change of the limit reclaims every quota lease (T041-5); the purge pass's reclaim, its skip of a tenant whose fence is held and the shard-0 publisher of the counters (T041-10) |
| The top-up (decision D041-124) | Its answers without raising, `room` at 65,536 bytes with no lock taken, a grant of exactly its share and no charge, `limit` when the share is 0 (before `room`, decision D041-137), and none when the share is not above the current room, `busy` at once with the usage row held (no wait, no slot, no quota reclaim of others, decision D041-134), `contended` with the fence held (T041-5); `top_up` true at 65,535 bytes and false at 65,536; the top-up on the report's own backend after the commit; the next carry's fast path; no top-up after a mark-only, `contended`, rolled-back or C0-refused carry; the tenant boundary (T041-36); the regimes, the pooler's included (T041-37) |
| One tenant's waits do not take another's connections (decisions D041-125 and D041-134) | With *Q* = 5 and *P* = 3, tenant A's usage row held: three of five fallbacks of A wait and two answer `service_busy` at once; with five tenants waiting, a sixth answers busy at once; tenant B's carrying report commits meanwhile, and on a pool of 8 with the Logs class filled B's heartbeat and task lease get their connections; *Q* passed per transaction and not kept by the next (decision D041-137); three hand mutations (T041-36); the same through plain inserts, and the top-up answering `busy` at once without a slot (T041-5); the two-tenant and three-tenant regimes, behind the pooler too, and the fan-out regime with a victim tenant (T041-37) |
| A transaction's connection is kept for its top-up and never returned inside a transaction (decision D041-137) | On a pool of one: the same `pg_backend_pid()` across two commits, a contended claim and a refused report; a future dropped mid-statement comes back rolled back on the same backend; a terminated `COMMIT` closes its connection; two hand mutations (T041-36, `cancellation.rs`) |
| Crash between accept and store | The `StepLogTx` wrapper fails before the commit: nothing stored, the resend stores all, once |
| Crash between store and answer | The wrapper commits and then fails (T041-6, T041-8); separately, the proxy of `tests/support/worker_proxy.rs` drops the answer of `AppendStepLogs` (T041-8): the resend stores nothing and gets `CONTINUE`; the same proxy drops the answer of a carrying report (T041-8, T041-36): the resent report is answered as a replay and stores nothing more |
| Duplicate batch | The same batch twice at a barrier: the second waits on the attempt's stream lock at T3 and gets `RETRY_LATER` at the lock timeout; its resend stores nothing; counters rise once |
| Gaps and late batches | A batch the SDK gave up on is counted as dropped by the worker once; a late batch after a newer one stores nothing and does not lower `last_sequence`; the carry's `highest_sequence` counts the drops at the end; a carry whose transaction id is below that of a batch stored after it stores none of its lines and counts them `out_of_order`; a batch with a lower transaction id never lowers `last_stored_xid` (T041-36) |
| A worker that floods (SC-041-002) | `step-logs-limits` for the SDKs; a client that ignores verdicts for the server: stored bytes stay at the ceilings, tickets hold, another tenant's completion latency is recorded; lines stored + dropped = highest sequence |
| Ceilings under concurrency | Two attempts of one run at their attempt ceilings, and two runs of one tenant at its `tenant_step_log_bytes` limit, at a barrier: the ceilings hold exactly; the quota-lease charge's raise answers `RETRY_LATER` and stores and counts nothing, and the resend, with a gap in the batch and a carry after it, keeps lines stored + dropped = `last_sequence` (T041-6, T041-8); a value that redaction lengthens is charged at its stored size |
| Revocation, session and fence | Binding removed, membership revoked, task lease expired but not reclaimed, a membership of the runner or database-runner role, a runner token, an expired session, a session of another namespace or subject or below 1.7, a replaced token, a valid task lease sent with another run's id: each stores nothing and gets its verdict; a revoked caller gets `AUTHORITY` whatever else was due, and is audited; a forward step of a stopping run gets `LEASE` and a compensation of it is stored (OD-2026-11-16-11) (T041-6, T041-8) |
| An old worker, an old server | `step-logs-negotiation`; a released (v0.2.0) engine beside a current one (T041-26 (b)) |
| Redelivery does not duplicate lines | A task delivered again under a new generation writes a new stream; the same generation's resend inserts nothing; rebuilding the projections of a run changes no log row (T041-8) |
| Reset, rerun, continue-as-new, retry | `e2e_step_logs` (T041-26): each new run starts with no log and the source keeps its own |
| Read horizon and end | Transaction A stays open, B stores a line and commits: the line waits until A ends, then both arrive in cursor order and `heldBack` was true meanwhile; a filtered read of a 12,000-line run returns its last matching lines before `complete`; an ingest held across the terminal transition is delivered before `end`; a first read of a new audit window that sees the run finished does not end early; a forged `end` for a running run is ignored; a reconnect at any point neither skips nor repeats; the held-transaction cases on PostgreSQL 16 and 18, each on a server of its own (T041-11, T041-12) |
| Lock order | An ingest, a report of the same namespace and a change of a step-log resource (T041-6), then a purge (T041-10), a carrying report beside an ingest of its attempt, the purge and a queued change of a step-log resource (T041-36), a script marker (T041-13) and a delete (T041-30), at barriers, all commit; no deadlock is reported |
| Redaction (SC-041-004) | `step-logs-redaction` in six SDKs; server tests for each shape of both scrubbers in each part of a line and each sensitive field name, of any value type (T041-6); the canary searched in the stored rows, `run_events`, `audit_records` and the captured process output (T041-8) and in the routes' bodies (T041-11, T041-29); the canary carried on a report (T041-36) |
| Tenant boundary | The cross-tenant battery on the four routes; a batch for another tenant's run (T041-8): 404 or `LEASE`, `access.denied.cross_tenant`, no count or size disclosed; a carrying report on another tenant's run leaves that tenant's usage row and quota leases unchanged (T041-36); a cursor or `stream` of another run is refused |
| Permission, grant and audit | A viewer is refused on the read routes and gets the script execution without its log; a developer without the PII grant is refused in a namespace whose schemas carry no annotation and allowed with it; two first reads in a window write one `run.logs_read` before returning, and a failure to write it returns no line (T041-11); a returned script log is audited (T041-29); a revoked grant stops a tail at its next poll (T041-12); only admin may delete, and every delete is audited (T041-30); a caller without the PII grant in one of the tenant's namespaces sees `tenant_step_log_bytes` with `usageWithheld: true` and no usage figure (T041-4, OD-2026-11-20-19) |
| Retention | A stream past its expiry returns no line, is marked, then purged 500 lines at a time and shown as expired from the first of them; quota bytes return; the stream row stays with `purged_at`; a lowered retention leaves existing streams' expiry as it was; a stream row held by an ingest is skipped by the mark and marked by the next pass (T041-10, T041-11) |
| Script marker (OD-2026-11-15-9) | The tests of section 7 (T041-13): one marker per inserted execution, written after the report commits, with its node and attempt; one per (run, node, attempt) with two iterations; none for a repeat, a second call, a test run, a finished run, capture off or the switch off; the record kept at the step-log limit; the marker found by the execution id; no deadlock with an ingest, a delete or a policy change; delivered before `end` |
| Separate step-log quota (OD-2026-11-15-5, -6) | A tenant at its `tenant_step_log_bytes` limit stores no step-log line while a script log, a dispatch error and a connector error are stored in full under `tenant_log_bytes`; an empty line is charged 128 bytes; the quota-lease charge raises at the limit (T041-5); the ingest at the limit (T041-6) |
| Runtime privileges and storage classification | The privilege audit and manifest tests pass with the new statements and columns; every function the migration creates or replaces keeps its `SECURITY DEFINER`, pinned `search_path` and grants, and `sagat_app` may execute exactly the allowance, the six definer quota-lease functions and the two invoker helpers, the counter sequences revoked from it (T041-5); `sagat_step_log_carry` is `SECURITY INVOKER`, executable by `sagat_app` only (T041-36) |
| The busy loss is shown and counted (FR-041-006, G041-026) | A `service_busy` carry sets the stream's `not_stored_reason`, leaves `ceiling` unchanged and writes one rate-limited warning a minute (T041-36); the streams route returns `notStoredReason` (T041-11); Studio shows the busy text (T041-22); the alert's rule test (T041-25); the counted regimes' rate against OD-2026-11-20-14's 0.1% (T041-37) |
| Studio (SC-041-006) | Unit tests of the states and markers; the axe run; the keyboard journey; a line holding markup, a URL, escape sequences and bidirectional controls is shown as inert text |

What this plan does not prove is registered as gaps in research.md, open unless said otherwise: G041-001 (a
transformed or foreign secret; accepted by the owner), G041-002 (the tail of an attempt that lost its task lease or its
network), G041-003 (script lines are not live), G041-004 (feature 020 must delete streams with their run; G1618 in
feature 020's research), G041-005 (a deployment whose missing store proof holds its protocol ceiling below 1.7 captures
nothing), G041-006 (SC-041-007 is measured on one machine), G041-007 (a script marker carries no iteration), G041-008
(the cursor shows the transaction counter; accepted by the owner), G041-012 (any open transaction of the cluster holds
the read horizon), G041-013 (no route removes a script execution's log early), G041-014 (an attempt whose capture
stopped before its first batch has no stream), G041-016 (the oldest-transaction gauge sees other roles' transactions only
with `pg_read_all_stats`), G041-017 (redaction against the process's secret set tells a caller who steers logged text
that a guessed value is a secret; accepted by the owner), G041-019 (the spike reached only the root report path; T041-9
measures both paths), G041-020 (a carry sent to an API replica older than 1.7 is lost during a rolling upgrade),
G041-021 (a report re-run after its carry failed it can be refused as late; narrowed by decision D041-124 to that re-run
alone, since no transaction runs before a report), G041-022 (a remote database lengthens a carrying report's hold of the
run row by its round trip; T041-9 records it), G041-023 (a charge refused while room exists only in quota leases open
transactions hold; bounded, section 2b), G041-024 (a report that stores output still takes the usage row at its commit
for the content class), G041-026 (the busy loss past the budget or with no wait slot free; accepted by the owner,
OD-2026-11-20-14, put again above 0.1% in T041-37's counted regimes) and G041-027 (behind a transaction pooler the top-up
can land on another backend and capture falls back more often; accepted by the owner, OD-2026-11-20-15; T041-37
measures it). G041-009, G041-010 and G041-011 are moot; G041-015, G041-018 (closed by OD-2026-11-20-19: the usage is
withheld without the grant) and G041-025 (closed by the fourth run, decision D041-122) are closed.

### Estimate

An estimate by reading, not a measurement. Slice A (T041-1 to T041-3, T041-33, T041-35): the reviews, two
documentation pull requests of owner answers, the carry spike (done, about 1,200 lines of Rust and scripts in the
benchmark) and the contracts. Server (T041-4 to T041-8, T041-32, T041-10, T041-36, T041-37, T041-11 to T041-13,
T041-29, T041-30): about 10,000 to 12,000 lines of Rust and SQL, most of them tests, in 14 pull requests, of which
T041-5 (about twelve definer functions of section 2b, the migrations and their held-lock, property and mutation tests,
about 3,000 lines) and T041-36 (the carry function, the top-up after the commit, the downgrade re-run and their tests,
about 3,000 lines) are the largest, and T041-32 a Python selector rule. SDKs: the specification and harness (T041-14),
six SDK tasks of 600 to 900 lines each with their tests (T041-15 to T041-20), and the matrix run (T041-21). Studio
(T041-22 to T041-24): about 1,300 to 1,600 lines with tests, in 3 pull requests. Measure, operate and close (T041-9,
T041-25 to T041-27): 4 pull requests. That is 37 task numbers (T041-1 to T041-37), three of them withdrawn (T041-28,
T041-31, T041-34): 34 tasks and 34 pull requests. A pull request's CI takes up to two hours, and they land one at a
time where they touch research.md or the contracts, so the calendar is set by the queue: about five weeks for one agent
session working alone, less with the six SDK tasks in parallel after T041-14. The places most likely to move it are
T041-37's regimes (a missed criterion stops the plan for a design change or an owner question, decision D041-131) and
the read horizon on two PostgreSQL versions.
