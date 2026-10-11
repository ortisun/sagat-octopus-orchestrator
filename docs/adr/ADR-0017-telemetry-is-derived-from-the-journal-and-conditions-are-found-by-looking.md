# ADR-0017 — Telemetry is derived from the journal, never produced on the write path; conditions that need a person are found by looking

Status: **Accepted** | Date: 2026-10-11 | Feature: 008 | Builds on ADR-0002 (PostgreSQL is the authority; the outbox carries obligations), ADR-0008 (what is shown of a run is derived from the persisted history, not from ephemeral telemetry) and ADR-0012 (events and deliveries are rows); changes none of them

## Context

Feature 008 asks that a person diagnose a run from reliable data: a trace that crosses the request, the
scheduler, the worker and the provider; a timeline of every transition with its cause; alerts for the runs
that are stuck; actions on a run that are audited. The platform already had a durable journal (the
authority), a W3C `traceparent` parsed at the door and written to the journal, Prometheus counters with no
tenant or run in any label, and no exporter of any kind. The tempting answers each put telemetry on the
path of the thing it describes:

1. **Spans produced by the code that does the work** (a span opened when a step is leased, closed when
   it completes). The work spans processes and time: a lease is taken by one request, completed by another
   an hour later, and its worker may die in between. A span that lives in a process cannot describe it, and
   one that is lost with the process is a hole in the trace exactly where the failure is.
2. **An OpenTelemetry SDK tracer in every handler.** It brings a dependency set that changes shape every few
   months, a batch processor with its own threads, and a model of spans that start and end where they are
   created, which is the problem of the first answer.
3. **Metrics incremented where the event happens**, so a retry that commits twice, or a transaction that
   rolls back, is counted regardless.
4. **Alerts in a metrics backend only.** The condition "the outbox of *this namespace* is stuck" has no
   namespace in a metric (and must not: a tenant is an unbounded label), so it cannot be evaluated there, or
   told to the person whose namespace it is.

## Decision

**1. The journal is the source; the write path only notes that it moved.** In the transaction that changes a
run, one upsert records the run's last journal sequence (`telemetry_pending`). A **projector** in the engine
(one at a time, by a lease row with an expiry on the database's clock) reads the notes, derives the spans and
the measurements of the run from its journal with a pure function (`application::telemetry::derive`), hands
the spans to the collector, and only when the collector has them records how far the run has got. A
collector that is down delays the spans and loses none; a projector that dies between exporting and
recording exports the same spans again. Nothing the projector does takes a lock on a run or makes a
transition wait (tested: a round stuck on a collector that never answers while runs start and complete).

**2. Ids are computed, not drawn, where they must be known before the span exists.** The span of a run
and the span of each attempt of a step are computed from the run and step identifiers, so the lease a worker
receives can carry the span of its attempt, a scheduler pass can name the run's span as its parent without
asking anyone, and a span exported twice is the same span. The request's span id is drawn by the HTTP
middleware (so the handler can record it on the run it starts) and travels with the run and the event.

**3. A trace is the same trace everywhere.** A run started by a request continues the request's trace and hangs
from its span; one started by an event continues the publisher's (a signed webhook or the publish API), or
begins a trace of its own when there is none, so no run is without one; a child hangs from its parent's span;
the lease, the `run.finished` webhook (as the `traceparent` header and `data.traceId`) and the outbox
message carry it. Live spans (the request, a worker call that reports an attempt, a scheduler action, an
outbox publish, a delivery attempt) are exported by a `tracing` layer; polls and registrations, which belong
to no trace worth keeping, are logged and not exported.

**4. The exporter speaks OTLP/HTTP with JSON, and says so.** The standard `OTEL_*` variables configure it. A
protocol or a sampler it does not speak is a configuration **error**, not a silent fallback: spans sent to a
port that expects another protocol are lost without a sign. Sampling is decided from the trace id alone, so
every process, and the projector long after the request, reach the same answer.

**5. Logs and span attributes are redacted on the way out.** A value logged under a sensitive name is
replaced, a credential-shaped string (a web token, a `Bearer` or `Basic` credential, the password of a URL, a
key with a well-known prefix) is scrubbed, a long value is cut, and a line cannot be forged with a line
break. This is the net under the rule that no call site logs a secret or a body; it is tested as the net it
is.

**6. Conditions that need a person are found by looking, in the database that knows them.** The engine's
monitor (one at a time, by a lease) runs aggregate queries over the indexes the scheduler itself uses and
gets counts and ages **per namespace**. They are reconciled against `operational_alerts`: a new condition
begins to fire, one no longer seen resolves, and each change is written in the same transaction as the
obligation to tell the namespace's webhook subscribers (the outbox pattern of ADR-0012), so an alert is
announced exactly when it is recorded and once however many engines run. A look that fails resolves nothing.
An alert carries counts, ages and names, never data of a run, and a runbook. Platform-level conditions (the
telemetry projection is behind; the engine's scheduler stopped) are metrics with rules in the monitoring
stack, because they have no namespace.

**7. Read models stay transactional.** The graph and the journal are as current as the last commit, and say
so (`projection.state` is `LIVE`); only what is derived afterwards (spans and measurements) has a lag, and its
lag is measured and exposed (`telemetry_projection_lag_seconds`, per run in the timeline).

## Consequences

- A new table (`telemetry_pending`) is written once per transaction that changes a run: a measurable cost on
  the write path (the benchmark of feature 008 measures it) in exchange for telemetry that survives anything.
- The run's span is exported when the run ends: a long run's attempts are in the backend before their parent.
- Measurements are counted by the process that holds the projector's lease; when the lease moves, a counter is
  continued by another process's series (Prometheus handles that), and a crash between recording progress and
  counting loses those counts. Exactly-once counting is not promised (G).
- Trace ids are not on metrics (the facade has no exemplars); a person goes from an alert or a spike to the
  runs by the timeline, which has the trace id of every transition.
- One projector and one monitor at a time is a ceiling on throughput that feature 010 (scale) moves with
  sharding; the notes are per run, so sharding by tenant hash needs no new design.
- A change of the shape of a span or an attribute is a change of a contract with whoever draws dashboards from
  it: `sagat.*` attributes are listed in `docs/runbooks/observability.md`.
