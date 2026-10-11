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
