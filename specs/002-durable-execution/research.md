# Research & Decisions: Durable execution engine and scheduler

## Main decision

Planner reduces events into a pure state machine; scheduler uses `FOR UPDATE SKIP LOCKED` and fencing in transactions; outbox dispatcher sends hints to JetStream; worker protocol confirms via Command API.

## Rationale

Ensure DAG progress, checkpoints, suspension, retries and failure recovery without re-executing commits. The decision respects the recovery, auditability, security and versioning properties defined in the Constitution.

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

## Gaps found while implementing (T002-001)

Recorded on 2026-10-08 after checking FR-002-001..009, the success criteria and the ADRs against what was built. Numbering continues from `specs/001-platform-foundation/research.md` (G1-G18, D1-D24).

| # | Gap | Resolution | Status |
|---|---|---|---|
| G19 | There is no blob store, so payloads (run input, step output) are limited to 256 KiB inline. `input_blob_ref` / `output_blob_ref` of the worker protocol exist but the server never fills the first and refuses the second with `INVALID_ARGUMENT`. FR-002-008 ("control payload/result size via blob refs") is therefore met only for the *control* half. | The limit is enforced at every entry point (REST, gRPC, `payloads` check constraint, child input). A content-addressed `payloads` table already stores inline payloads by digest, so a blob store can take over the same digests. Pending decision: which store, retention and classification (spec open question). | Partly open |
| G20 | `foreach` and `humanApproval` are in the IR schema but no feature owns them. | The codec refuses them with `IrError::Unsupported` and the reason, at publish time, so nothing unexecutable is ever stored. `saga` (003) and `script` (005) have owners. | Open (unowned) |
| G21 | The result of a child run is not available to its parent: the parent step's output is empty. A run has no "result" (the IR `end.outputExpr` is parsed but never evaluated or stored). | Decision D36. Defining a run result (and exposing it as `steps.<child>.output`) needs a product decision about what a run returns. | Open |
| G22 | Timers, retries and lost leases are acted on one transaction at a time, about 100 per second per loop, and several engines read the *same* oldest candidates, so more replicas do not multiply that (the losers skip what the winner already did, which is safe but wasteful). The same herd shows in the p99 of task claims when many workers poll at once. | Measured in `tests/performance/reports/002-durable-execution.md`. Bounded concurrency inside a tick and disjoint candidate selection are feature 010 (scalability). Nothing is lost while a burst drains: due work waits in an index. | Open (010) |
| G23 | `FailStep.safe_message` is length-checked but not stored. | By design (D31): the journal never holds free text from a worker, because it cannot be proven free of secrets. A structured, bounded error detail could be added later. | Accepted |
| G24 | `TaskLease.tenant_alias` and `input_blob_ref` are always empty. | The tenant is implicit in the worker's identity and the blob store does not exist (G19). | Accepted |
| G25 | A poll may return up to 64 tasks whose inputs are each up to 256 KiB, more than the 4 MiB a gRPC client accepts by default. | The server does not cap the response; SDKs (feature 006) must size `max_tasks` by their receive limit. A server-side cap by total bytes is a follow-up. | Open |
| G26 | The gRPC server has no panic-catching layer (the REST server has `CatchPanicLayer`). | The workspace lints forbid `unwrap`/`expect`/`panic` in non-test code, and `unsafe` is forbidden, so a handler panic is not expected; if one happened it would drop the connection, not the process. | Accepted |
| G27 | The worker API is plaintext gRPC (HTTP/2 prior knowledge) and relies on the ingress or mesh for TLS, as the REST API does. Mutual TLS between engine and workers is not enforced. | Feature 009 (security and multi-tenancy). The runbook states the requirement. | Open (009) |
| G28 | `run_events` and `payloads` grow without bound; there is no partitioning or retention. | Feature 010. Delivered outbox messages are purged (retention 24 h, bounded batches) and so are idempotency records and worker sessions. | Open (010) |
| G29 | `outbox_backlog` is a `count(*)` over the partial index, executed once per dispatch round: O(backlog). | Fine up to hundreds of thousands of pending messages; switch to an estimate or a counter when a benchmark says so. | Open (010) |
| G30 | The two journal vocabularies of the saga engine (compensation) and the event gateway (signals from outside the tenant API) do not exist yet. | Features 003 and 004. Signals through REST are implemented and audited. | Out of scope |

## Decisions taken in this feature

- **D25 — A run is a pure reducer over a journal; the rows are projections of it.** `RunState::apply` validates an event against the current state *before* mutating, the planner `plan()` is a pure function of `(definition, state, now)`, and every command returns the events to journal. The application writes the events, the step rows, the run row, payloads, outbox, audit and idempotency record in **one transaction**; `PgRunStore::verify_run` replays the journal and compares it with the projections, and the property tests do the same after every random action. Time and jitter are passed in; nothing in the domain reads a clock.
- **D26 — Retry defaults: three attempts, exponential from 1 s to 60 s, equal jitter.** A node that says nothing gets `RetryPolicy::DEFAULT`; the delay after attempt *n* lies in `[base/2, base)` and never exceeds the maximum. Lease expiry and attempt timeout consume an attempt and are retryable (the effect may have happened, so the retry reuses the same idempotency key).
- **D27 — Fencing is the triple (attempt, generation, lease token).** A lease grant raises the generation, hands one worker a random 256-bit token and stores only its SHA-256. A report must present all three, plus (when sent) the step id derived from version, run and node. Anything else is `STALE_ATTEMPT` and changes nothing. A report about a run of another tenant, an unknown run, a finished run and a reclaimed lease are the *same* answer.
- **D28 — Every deadline uses the database clock.** Leases, timers, retries, wait deadlines and run deadlines are computed from and compared with `clock_timestamp()` in PostgreSQL, so engines with skewed clocks agree. Tests move time by updating the stored timestamps, never by sleeping.
- **D29 — Expressions are a bounded subset of CEL, implemented here.** The reference crate recurses without bound (depth 12 overflows a 2 MiB stack) and its macros can burn unbounded CPU while a run lock is held. `sagat-infra-expr` has hard limits (1 KiB source, 256 tokens, depth 16, no macros, no regular expressions, no time), is checked against the reference implementation in a differential test, and every expression in a definition must compile at publish time.
- **D30 — Payloads are content-addressed and inline up to 256 KiB; an activity with no result completes with an empty output, recorded as JSON `null`.** `output_sha256`, when sent, must match the bytes received.
- **D31 — Workers never put free text in the journal.** A failure carries a code (`[A-Za-z0-9_.:-]{1,64}`; anything else becomes `WORKER_ERROR`) and a retryable flag. `safe_message` is validated for length and dropped. Idempotency: a completion or failure is keyed `(scope step:<run>:<node>, Idempotency key)` with a hash of the report; the same key and report replays the original answer (`already_applied`), the same key with a different report is `IDEMPOTENCY_KEY_REUSED`, a new key on a finished attempt is `STALE_ATTEMPT`.
- **D32 — NATS carries hints, never authority.** The outbox row is written in the transaction that makes a step ready (or a run finish); a dispatcher publishes it at-least-once with the outbox id as `Nats-Msg-Id` (the server drops duplicates inside its window) to `sagat.v1.<event>.<tenant>` (and, for `task.ready`, `.<activity>`): a subject per tenant, event type and activity, bounded by what a tenant declares and never by runs, so a worker pool can filter on exactly its activities and a tenant's hints never mix with another's. A worker still has to *claim* the step through the worker API. The adapter refuses subjects outside `sagat.v1.`, bounds every wait, and the engine starts and keeps working when NATS is down.
- **D33 — The scheduler finds candidates without locking and decides under the run's lock.** Each pass is an index range scan over a small partial index; the domain command re-checks the condition under the run's row lock, so a candidate someone else already handled is a benign skip and several engines are safe. The lock order is always run first, then its steps, and no transaction holds two run locks.
- **D34 — The worker protocol reuses the REST identity pipeline.** The gRPC adapter calls the same `authenticate_bearer` as the REST extractor (verify token, per-subject budget before any lookup, tenant lookup, per-tenant budget and concurrency slot), maps errors with the same classification (conflict → `ABORTED`, budget → `RESOURCE_EXHAUSTED` + `retry-after-ms`, stable code in `x-sagat-error`), and delegates every rule to the application's `WorkerService`. A session token is bound to the identity that opened it and expires when idle. The `worker:execute` permission is held by the `worker` role and, as for every permission, by `admin`.
- **D35 — The engine proves its own least privilege and reports honestly.** Outside `SAGAT_ENV=dev` it refuses to start as a role that could rewrite the audit trail (exit 2), and refuses a plaintext key URL. `/livez` fails when the scheduler loop stops ticking; `/readyz` additionally needs the execution schema. Metrics and probes are on a separate loopback listener.
- **D36 — A child run is reserved by the parent and created by the scheduler, under the parent's lock.** Reaching a `child` node journals `child.started` with an id *derived* from `(parent run, node)`, parks the parent on it and stores the evaluated input. The scheduler then creates the child in one transaction that first locks the parent and confirms the step still waits for exactly that run, so a child cannot exist for a step cancelled a moment earlier. The child inherits namespace, trace and correlation, gets `min(parent deadline, now + node timeout)` and depth + 1 (limit 8, then `CHILD_DEPTH_EXCEEDED`); the target workflow is looked up by name inside the parent's tenant *and* namespace. Two flags written by the parent under its own lock (`parent_settled`, `parent_cancel`) make every scheduler query a small partial-index scan, and no transaction ever holds both runs' locks. Policies: `propagate` (default; the parent does not wait for the cancelled child), `request_cancel` (the parent waits until the child has settled), `detach`. A child that ends, however it ends, fails or completes the parent step (`CHILD_FAILED`, `CHILD_CANCELLED`, `CHILD_TIMED_OUT`; `CHILD_NOT_FOUND` when the workflow or the pinned version does not exist), and the parent's `onError` route applies. The parent's step has no output (G21).
