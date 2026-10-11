# Research & Decisions: Distributed observability and operations

## Main decision

Standard OTel Collector, journal projection workers, paginated search APIs and actions that generate auditable commands.

## Rationale

Diagnose and operate runs, steps, external integration, resources and Sagas from reliable data. The decision respects the recovery, auditability, security and versioning properties defined in the Constitution.

## Alternatives considered

1. **Generic ad hoc queue-based solution:** rejected as the sole durability mechanism, since message delivery is not equivalent to a transaction over workflow state.
2. **Independent services per feature from day 1:** postponed to avoid operational and consistency complexity without a benchmark.
3. **SDK-specific logic:** rejected for core invariants, to avoid behavior differences between languages.

## Internal references

- `docs/adr/` (architectural decisions)
- `docs/architecture/overview.md`
- `docs/test-strategy.md`
- `specs/001-platform-foundation/contracts/`

## Risks and mitigation

- Complexity/scope risk: deliver P1 user stories separately, running contract tests.
- Semantic ambiguity risk: encode reducers and golden fixtures, do not decide via UI alone.
- Crash/redelivery risk: test harness with real failures at checkpoints.
- Security risk: specific pentest and tenant isolation from the first milestone.

## Questions for spike validation

- Latency and overhead benchmarks on the critical path defined for this feature.
- Detail compliance and retention requirements according to the deployment environment.

## What was decided while building it (D160–D1xx) and what is left (G169–G1xx)

Numbering continues after feature 007 (D138–D159, G142–G168; D154–D159 and G160–G168 are the visual
direction of the Studio). The decision is [ADR-0017](../../docs/adr/ADR-0017-telemetry-is-derived-from-the-journal-and-conditions-are-found-by-looking.md);
how to operate it is `docs/runbooks/alerts.md` and `docs/runbooks/observability.md`.

### Decisions

- **D160 — Telemetry is derived from the journal, off the write path** (ADR-0017). The write path adds one
  upsert (`telemetry_pending`: the run's last journal sequence) in the transaction that changes a run; a
  projector in the engine turns what is new into spans and measurements, and records its progress only when the
  collector has the spans. Tested: any split of a journal into batches gives the spans of one batch and none
  twice (`crates/application/tests/telemetry_derive.rs`); a collector that is down delays the spans and loses
  none; a projector that dies after exporting exports the same ids again; a round stuck on a collector that
  never answers makes no transition wait (`tests/fault/observability-operations/projection.rs`).
- **D161 — OTLP/HTTP with JSON, written here on `reqwest`, and no OpenTelemetry SDK.** The Collector is the
  standard; the wire format is the standard; the SDK's tracer assumes a span starts and ends in the process that
  created it, which is not what an attempt is (D160), and its dependency set (`opentelemetry`, `-sdk`, `-otlp`,
  `tracing-opentelemetry`, a batch thread of its own) changes shape every few months. The exporter is a client
  with retries on `429/502/503/504` and `Retry-After`, no retry on a refusal, a bounded queue that drops and
  counts instead of waiting, and a `tracing` layer that exports only the spans of this workspace's crates that
  belong to a trace. `OTEL_EXPORTER_OTLP_*`, `OTEL_SERVICE_NAME` and `OTEL_TRACES_SAMPLER*` configure it; a
  protocol or sampler it does not speak is an error, not a fallback (G169).
- **D162 — The spans of a run and of an attempt have computed ids.** From the run and the step, so the lease
  carries the span of its attempt before it is exported, the scheduler parents its work to the run's span
  without asking, and a span exported twice is one span. The first sampling function drew its random from
  bytes a UUID's variant bits fix, so no trace under a ratio of one half was ever kept; the test found it.
- **D163 — Every run belongs to a trace and hangs from something.** `runs.trace_parent_span` (the request that
  started it) and `inbox_events.trace_id`/`trace_parent_span` (the publisher's trace): a run started by an event
  continues the publisher's trace; one started by the clock, or by an event with no trace, begins its own; a
  child hangs from its parent's span. This closes G79. The migration is additive and nullable.
- **D164 — Worker calls with no `traceparent` are logged and not exported.** A poll or a registration is a
  trace nobody asked for: exporting it would add a one-span trace per poll of every idle worker. A report of an
  attempt carries the lease's traceparent and joins the run's trace under the attempt's span.
- **D165 — `run.finished` and its webhook carry the trace.** `data.traceId` in the body, and a `traceparent`
  header on the delivery whose parent is the delivery attempt, itself a child of the run's span, so what a
  subscriber does about a finished run is in the run's trace. Additive in the AsyncAPI document.
- **D166 — Logs and span attributes are redacted on the way out.** By the name a value is logged under
  (`authorization`, `token`, `secret`, `password`, `cookie`, `credential`, `api_key`, `private_key`, `payload`,
  `body`, `bearer`, `signature`: textual values only, a count under such a name is kept), by shape (a web token, a
  `Bearer` or `Basic` credential, the password of a URL, keys with a known prefix), and by length (2048
  characters). A field cannot forge the level, the time or the service; a line break cannot start a second line.
  It is the net under the rule that no call site logs a secret, and it is heuristic (G170).
- **D167 — Every `*_seconds` histogram has buckets.** Only `*duration_seconds` had a rule, so every lag and
  latency (`event_dispatch_lag_seconds`, `webhook_delivery_lag_seconds`, `script_run_seconds`, …) was rendered
  as a summary, whose quantiles cannot be added across replicas: no dashboard could compute a percentile from
  two engines. From a millisecond to an hour.
- **D168 — One projector and one monitor at a time, by a lease row.** `engine_leases` holds a holder and an
  expiry on the database's clock; a process that dies is replaced when its lease runs out (30 s, 60 s). A
  session-level advisory lock would leak through a connection pool.
- **D169 — The notes of a projected run are not deleted when it ends.** Deleting would let a late upsert start
  the run's projection again from the beginning; they are purged a day after the run finished.
- **D170 — Alerts are found by aggregate queries per namespace and reconciled.** `operational_alerts`, one row
  per namespace, kind and subject, toggling `FIRING`/`RESOLVED`. Five kinds: `OUTBOX_STUCK`,
  `LEASE_NOT_REAPED` (critical), `STEP_UNCLAIMED`, `REVIEW_PENDING`, `DEAD_ITEMS` (warning). A change and the
  obligation to tell the namespace's subscribers (`ops.alert.fired`, `ops.alert.resolved`) are written in one
  transaction. A look that fails resolves nothing. At most 25 alerts of a kind per namespace per look. Tested
  with two monitors at once: one alert, one notification.
- **D171 — Two permissions: `ops:read` and `worker:manage`.** `ops:read` (Developer, Operator, Viewer, Admin)
  sees alerts and the workers serving a namespace; `worker:manage` (Operator, Admin) drains a worker.
- **D172 — The cause of a transition is the platform's own vocabulary.** `RunEvent::cause()`: a closed kind
  (`failure`, `retry`, `review`, `abort`, `unknown_outcome`, `reconciliation`, `stop`, `timeout`, `cancel`,
  `decision`, `request`, `outcome`) and the code that goes with it.
- **D173 — An operator may act on a step that has not started, and the first commit wins.** `RETRY_STEP` also
  accepts a step in `RETRY_WAIT` (it runs now instead of at `not_before`; the attempts before it stay), and
  `MARK_MANUAL_REVIEW` holds a `READY` or `RETRY_WAIT` step that is not a lookup (`StepNeedsReview`, code
  `MARKED_FOR_REVIEW`, no fence, so it needs the same decision as any review). The scheduler's own retry and
  the operator's race on the run's row: the second to commit finds an illegal transition and answers `409`.
- **D174 — `RERUN_AS_NEW` is for a run that has ended.** Same version, same input, a new run with
  `runs.rerun_of` pointing at the old one; idempotent by the key, audited as `run.rerun`. A live run is refused
  (`409`, "only a run that has ended"): cancel it first. The sagas suite that expected otherwise was changed
  with this (`tests/e2e/sagas-compensations/us04.rs`).
- **D175 — A worker identity is drained per namespace, and what it holds goes on.** A row in `worker_drains`;
  the `use_session` update computes `draining`, a poll answers empty, registration and heartbeat are unchanged
  (so a restarted worker is still drained), leases already held finish. `listWorkers` says when `inFlight` is 0.
- **D176 — Reading the audit trail is audited, scrubbed, and withholds the denials.** `GET /audit` needs
  `audit:read` (administrators). The reading is written in the transaction that reads it and collapses repeats;
  a reason that has the shape of a credential is replaced on the way out (the trail keeps what was written).
  `access.denied.cross_tenant` is **not returned**: it is written only when the identifier exists in another
  tenant, so listing it was an existence oracle that the `404` takes care not to be. The cross-tenant battery of
  009 found it the first time it met `GET /audit`; `AuditAction::WITHHELD_FROM_TENANT` is the list.
- **D177 — Errors say what to do.** `remediation` is in the envelope, from a closed table keyed by the error
  code (a sentence, never text taken from the request), and the Studio shows it.
- **D178 — A run says how far its telemetry is.** `GET /runs/{id}/telemetry`: `CURRENT`, `CATCHING_UP`, `BEHIND`
  (older than the lag budget), `NO_TRACE`, `UNTRACKED`. The timeline and the graph read the transactional read
  models, so their own lag stays 0 (`projectionLagMs`); what lags is the export to the backend, and that is what
  this reports.
- **D179 — The reviews changed five things the design had taken for granted.** (1) A look of the monitor is
  bounded (25 groups per namespace, 100 per tenant, 1000 in all, ranked in the query) and says where it stopped
  short; an alert is resolved only where the look saw everything, so an older condition appearing cannot resolve a
  live alert, and one tenant cannot fill the look and hide the others'. (2) An unserved step is one a worker that
  served it would be given: the conditions of the claim, so a paused run or a compensation behind an open breaker
  is not an alert. (3) Marking a run projected says whether it moved the note, only that one counts the
  measurements, and the projector asks for its lease again before each batch. (4) A collector's `401`, `404` or a
  redirect is a wait, not a verdict on the batch: only `400`, `413` and `422` drop spans; the rest stay in the
  journal. (5) The sampler decides the projected spans as it does the live ones.
- **D180 — A hold keeps what is known about the last attempt.** `MARK_MANUAL_REVIEW` of a step waiting out a retry
  after a lost lease kept `MARKED_FOR_REVIEW` as its failure and so forgot the effect may have happened; a saga stopped
  afterwards did not undo it. The hold says why the step waits now and leaves the failure alone.
- **D181 — A drain is coupled to the polls that lease.** The lease transaction checks the drain again under a shared
  advisory lock; the drain takes it exclusively. A drain that has returned has stopped every poll that had not
  leased yet (`a_drain_cannot_return_while_a_poll_that_found_the_worker_serving_is_still_leasing`). A namespace holds at
  most 500 drained identities.
- **D182 — The audit read follows the index of the tenant, and the reading is recorded again after five minutes.**
  Ordering by the primary key walked the table backwards across every tenant (`Rows Removed by Filter: 10144` for 51 rows
  with 200 tenants); a test holds the plan. An administrator who read the trail once and again a month later left one
  record: the same reading within five minutes is one record, after that a new one.

### Gaps (G169–G1xx)

- **G169 — OTLP/HTTP with JSON only.** `http/protobuf` (the specification's default) and gRPC are refused with a
  clear error. A backend that accepts only protobuf needs a Collector in front. A protobuf encoding with `prost`
  is the small step if it is asked for.
- **G170 — Redaction by name and shape is a heuristic.** A secret logged as `note = "hunter2"` has no name and
  no shape. The first line of defence is structural (secrets and payloads are types whose `Debug` hides them) and
  stays the rule.
- **G171 — Measurements are counted by the process that holds the projector's lease, after the progress is
  recorded.** A crash between the two loses those counts; a change of leader continues a counter in another
  process's series. Exactly-once counting is not promised; the journal is the truth.
- **G172 — The run's span is exported when the run ends.** A long run's attempts reach the backend before their
  parent; backends show them without a root until then.
- **G173 — No exemplars.** The `metrics` facade has no exemplar API; trace ids are not on metrics. The timeline
  has the trace id of every transition.
- **G174 — Alert thresholds are global settings**, not per namespace or tenant.
- **G175 — Alerts leave by webhook** (and the log, the gauges and the API). No pager or email integration;
  the Prometheus rules for platform-level conditions ship with the dashboards.
- **G176 — Runs created before this feature hang from nothing** (`trace_parent_span` is null); their spans are
  exported with no parent request.
- **G177 — One projector and one monitor cap their own throughput.** Sharding by tenant is feature 010.
- **G178 — The monitor looks at the whole platform every 30 s with aggregate queries**; their cost grows with the
  number of namespaces in a condition and not with history, but it is not benchmarked beyond the suite (T008-033).
- **G179 — A rerun uses the version of the run it repeats.** There is no "rerun on the latest version"; the
  caller starts a new run for that.
- **G180 — The quota panel draws refusals, not usage against the limit.** `quota_exceeded_total` is there; a
  gauge of usage per limit is not, because the exact count is taken under a lock on the write path (feature 009)
  and sampling it would repeat it.
- **G181 — The Studio has no control for draining a worker, and no list of alerts.** Both are API-only. The
  timeline carries the operator actions that existed before (retry, cancel, pause, resume, signal) and the new
  ones (retry now, hold for review, rerun).
- **G182 — The audit trail has no retention or export.** Its growth is bounded only by the rules that collapse
  repeats; partitioning is feature 010 together with G28. A tenant that needs the trail elsewhere pages the API.
- **G183 — A tenant cannot see attempts to reach it from outside, nor its own probes of other tenants.** The
  denials are kept (D176) and are read by whoever runs the platform in the database; an operator endpoint for
  them is not built.
- **G184 — Drain is by identity and namespace, not by version or by worker process.** Two processes sharing an
  identity are drained together.
- **G185 — The alerts are not tied to a run.** An alert names a namespace and a kind (and the oldest affected
  item); the runs it concerns are found in the timeline and the dead-item lists.
- **G186 — The detection of a stuck outbox reads every undelivered message.** Measured at about 9 µs a message
  (150 ms at 17 000 rows, `tests/performance/reports/observability-operations.md`), so it grows with the failure it
  is meant to find: a dispatcher that is down for an hour at the write rates of the benchmark leaves millions of
  rows and a round that takes seconds. The other four detections are index-bound and take milliseconds. The
  remedy is to examine at most the oldest N rows in the order of `event_outbox_pending_idx` and report the count as
  "at least"; it is not done, because it changes what an alert says (`items`) and needs its own test.
- **G187 — A caller who sends a `traceparent` chooses whether its request is sampled.** The decision is made from the
  trace id, which the caller supplies, so a client can force every one of its requests to be exported (or none). The
  exporter's queue is bounded and counts what it drops, which protects memory and not the fairness between
  tenants' spans. Sampling on a value drawn by the server, or honouring only the parent's `sampled` flag from
  trusted callers, are the remedies; neither is built.
- **G188 — The monitor's cost grows with the failure it detects, beyond the outbox (G186).** `sample()` counts the
  ready steps, the outbox and the inbox without a bound, and the detection of unserved steps reads every ready
  step older than the threshold. Both are measured at 5 000 and not beyond.
- **G189 — A cross-tenant probe takes longer than a probe of an id that exists nowhere.** The denial is written
  before the `404` is sent when the id exists in another tenant. The difference is a database write; it was there
  before this feature and the new endpoints inherit it. The remedy is to write the denial off the response path.
- **G190 — The migrations of this feature create indexes and constraints on `runs` and `inbox_events` without
  `CONCURRENTLY`**, which a transaction cannot do. On a table with many millions of rows the lock window is the time to build
  a partial index and to validate the new check constraints: plan it.
- **G191 — The worker list shows the 200 newest sessions** and no cursor; an identity whose sessions are older is
  seen only in `drains`.
- **G192 — The schedule latency of an attempt that was held by an operator includes the time it was held.** The
  attempt starts from the step's last ready event, not from the release.
