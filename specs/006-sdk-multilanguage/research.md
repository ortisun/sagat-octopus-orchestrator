# Research & Decisions: TypeScript, Go, Python, C# and Java SDKs

## Main decision

Protobuf codegen + idiomatic adapters and Activity worker; do not make the SDK persist its own journal. Distinguish client SDK and worker SDK from the DSL compiler.

## Rationale

Allow definition, triggers, activities and Sagas in five languages with common execution semantics. The decision respects the recovery, auditability, security and versioning properties defined in the Constitution.

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

## What was decided while building it (D119–D1xx) and what is left (G121–G1xx)

Numbering continues after feature 005 (D95–D118, G96–G120). The decision on versioning and on the conformance
suite is [ADR-0015](../../docs/adr/ADR-0015-sdks-meet-the-server-by-minor-version-and-are-held-to-one-conformance-suite.md);
how to operate it is `docs/runbooks/sdks.md`; the behaviour of the SDKs is `sdks/SPEC.md`.

### Decisions

- **D119 — One package per language holds the client, the worker host and the builders.** The plan separated
  "client SDK and worker SDK" from "the DSL compiler"; there is no compiler. The builders are side-effect-free
  helpers that return the workflow document (the IR the platform already defines) and the platform validates it
  when it is published. No SDK keeps a journal, a queue or a timer of its own (ADR-0001/0002).
- **D120 — The conformance suite is the contract, not any SDK** (ADR-0015). The first honest run found a server
  defect (D122) that the reference SDK's own tests could not have shown; the second SDK found five places where
  the first deviated from the SPEC and three where the SPEC was ambiguous.
- **D121 — Negotiation by minor version, and the server keeps the forms of every minor it accepts** (ADR-0015).
- **D122 — The heartbeat of protocol 1.0 is served.** `Heartbeat` got the run, the step and the attempt with 1.1;
  the server required `attempt >= 1` in every request, so a worker of 1.0 could not keep a lease and its task was
  leased again until the run failed with `LEASE_EXPIRED`. Now an absent attempt (0, for a heartbeat only) is the
  step's own; the lease token and the generation in the task id still fence the lease; a negative attempt is
  refused. Contract tests: `tests/contract/sdk-multilanguage/grpc/negotiation.rs` (failed on the old code).
- **D123 — `worker_registrations_total{protocol}`** counts registrations by the version they were served at, so an
  operator can tell when an old minor can be dropped. Refusals stay in `grpc_requests_total`.
- **D124 — A workflow is started by name or by id.** The OpenAPI takes `workflowId`; the SDKs resolve a name
  through `GET /workflows` (every page) and remember it for a minute. A convenience, not a new API.
- **D125 — The canonical text of a workflow is RFC 8785** (keys by UTF-16 code unit, numbers as ECMAScript prints
  them), because five languages sort and print differently and the same program must give the same bytes. The
  harness compares the documents as values; the text matters to whoever hashes it.
- **D126 — What a cancelled handler's end means depends on why it was cancelled.** `run-cancelled`: a raised
  cancellation is `FailStep(CANCELLED)`. `shutdown` (the host is stopping): not reported, the lease lapses and the
  step is leased again, because a deploy must not fail a step for good. `lease-lost`: nothing is reported.
- **D127 — Retries follow one rule in every SDK** (SPEC §5): only `TRANSPORT`, `UNAVAILABLE`, `RATE_LIMITED` and
  `INTERNAL` that say they may be repeated, and a `POST` only with a key (the SDK generates one per call and keeps
  it across the retries). A server that marks a 4xx retryable is not believed (found in the TypeScript SDK).
- **D128 — A credential never leaves for a place the caller did not name.** No SDK follows a redirect with its
  `Authorization` header; no SDK has code that turns certificate verification off (a test of each SDK that reads
  its sources); TLS and mTLS are tested with throw-away certificates for both channels in every SDK.
- **D129 — The suite boots a platform per scenario and per language, with the real execution engine.** A shared
  platform leaked one language's runs into another's `list`; a scheduler ticked by hand did not route events, which
  hid a scenario whose event never woke its run. The platform's own state (every run's graph, read over REST) is
  compared too, so the suite checks what an SDK *caused*, not only what it printed.
- **D130 — The benchmark counts calls on the server** (`grpc_requests_total` of the engine) rather than in the
  SDK, so it measures every language the same way without asking the SDKs for numbers.

### Gaps

- **G121 — The proto is at `specs/001-platform-foundation/contracts/worker.proto`, not at
  `proto/sagat_octopus_orchestrator/v1/worker.proto` as `plan.md` lists.** It has been the canonical file since
  feature 002; each SDK copies it with a script and a test fails when its copy is behind. Moving it would touch
  every feature's contracts for no gain.
- **G122 — Resolved as a product decision: MIT.** The owner selected the MIT License on 2026-10-11. The root licence and all five SDK package licences/metadata are aligned, and clean-consumer package tests pass. Registry publication remains G123.
- **G123 — Local distribution is verified; registry release remains open.** `scripts/package-sdks.sh` builds eight artifacts with checksums, verifies embedded MIT licences and installs all five SDKs into clean consumers. Paired builds with a fixed source revision matched hashes. `.github/workflows/sdk-packages.yml` runs this credential-free and stores CI artifacts. The candidate includes release notes and Maven sources/Javadocs (T006-037). Actual npm/PyPI/NuGet/Maven/Go-proxy publication, release signing and durable release retention remain open; see `docs/runbooks/sdk-packaging.md`. *Update 2026-11-01:* the owner chose GitHub Releases as the only channel on 2026-10-16 (`docs/owner-decisions.md`, item 12), so registry publication is out of scope rather than open; release v0.2.0 carries the five SDK packages with `SHA256SUMS` and a SLSA provenance attestation, verified by download, checksum and clean-consumer install (`tasks/todo.md`, FR-006-008).
  **Decided and built 2026-10-16 (owner): GitHub Releases only.** `.github/workflows/release.yml` builds the SDK packages with `scripts/package-sdks.sh` on a tag `v<semver>`, adds them to the release with `SHA256SUMS` and a Sigstore build-provenance attestation per file; no npm, PyPI, NuGet or Maven publication; Go is consumed from the GitHub module path. Runbook: `docs/runbooks/release.md`. The first tagged release is the evidence that is still to come.
- **G124 — A server restart is not in the conformance suite.** The scenarios fail the transport and the session
  (`worker-transport-faults`: a registration that never leaves, refused polls, an UNAUTHENTICATED poll, refused
  heartbeats and completions) but do not stop the engine; the engine would come back on another port. Each SDK's
  unit tests cover a session that is gone.
- **G125 — Implemented and measured (2026-10-12).** ADR-0029
  and protocol 1.4 add finite-credit task streams in all five SDKs. The 95 common conformance
  executions, six E2E and nine fault tests pass. T006-042 records the 30-row poll/stream
  comparison and final five clean-consumer packages. The report preserves unknown clock offset
  and reconstructed claim-snapshot attribution limitations; no universal speedup is claimed.
- **G126 — Resolved for opt-in ecosystem integration (2026-10-12).** TypeScript, Python, Go and
  Java bridge the existing hooks to real OpenTelemetry APIs; C# retains native ActivitySource/Meter.
  In-memory SDK exporter tests prove parentage, bounded attributes/metrics and exporter failure
  isolation. Providers/exporters remain application-owned; no global exporter is installed by an SDK.
  See `docs/runbooks/sdk-observability.md` and each SDK's OpenTelemetry tests.
- **G127 — Runtime matrices say only what was run on this machine** (macOS arm64): one version of each runtime,
  except Python 3.10 (conformance) and 3.11–3.13 (unit tests, before the last change). Other versions and
  operating systems are declared, not tested. Each README says which.
- **G128 — The heartbeat interval depends on the SDK's clock** (the lease carries no TTL): a clock skewed by more
  than the lease's TTL distorts it (clamped to 0.5–10 s; `heartbeatIntervalMs` overrides). Adding the TTL to
  `TaskLease` is an additive field for a later minor.
- **G129 — `waitForCompletion` that times out raises `TRANSPORT` with the reason `WAIT_TIMEOUT`, retryable.** It is
  not a transport failure; a code of its own is cleaner and is a change to the five SDKs and the catalog.
- **G130 — `Retry-After` is read as seconds only**, not as an HTTP date (SPEC §8: a date is ignored, in every SDK).
- **G131 — Payloads over 256 KiB are refused (`PAYLOAD_TOO_LARGE`)**; blob references are the server's G19.
- **G132 — Merging with `bugfix/spec-alignment`**: it uses protocol 1.3 (blobs) and `PROTOCOL_MINOR` here is 2;
  when both are merged the constant becomes 3 and the SDKs' newest offer follows it. Numbers D72–D77/G66–G72/ADR-0011
  are that branch's.
- **G133 — `@grpc/grpc-js` cannot do TLS to a bare IP address on Node 26** (it sends the host as the TLS server
  name). The TypeScript host asks for `tls.serverName` and says so; other SDKs are not affected.

### Decisions made after the reviews (D131–D137)

- **D131 — The host cancels the token and nothing else.** It never interrupts a handler, in Python's `asyncio` too (the
  first Python SDK injected `CancelledError` into the handler's task on a cancel, so a handler that ignored the token
  was interrupted mid-charge and failed for good as `CANCELLED`, and a saga does not compensate a step reported failed
  whose effect may have landed). A handler that ignores the token finishes and is reported (unless the lease was
  lost). Conformance: `cancellation` (a stubborn handler), `worker-shutdown`.
- **D132 — The server says when it does not have the call.** A router-level 404 and 405 were the generic envelope,
  indistinguishable from a missing run, so `UNSUPPORTED_BY_SERVER` (SPEC §9) could never be raised against this
  platform whatever the SDKs' unit tests (which modelled an HTML 404) said. The envelope now carries
  `details.reason` `ROUTE_NOT_FOUND` / `METHOD_NOT_ALLOWED` (OpenAPI note; additive). Only a server of this version
  or later tells; an SDK newer than that will.
- **D133 — Retry by code, and 408 is `UNAVAILABLE`.** One rule in five languages (SPEC §5): the code is `TRANSPORT`,
  `UNAVAILABLE`, `RATE_LIMITED` or `INTERNAL` and it says it may be repeated; `QUOTA_EXCEEDED` never; a 4xx that
  claims it on another code is not believed. The shared catalog maps 408 to `UNAVAILABLE`.
- **D134 — The canonical text is a contract compared byte for byte** (the build output carries it; the harness compares
  it across the five languages and with the golden file). The first comparison of *parsed* documents hid that
  TypeScript listed integer-like keys first (JavaScript objects do) and that Python printed a big integer exactly.
  An integer beyond 2^53-1 is refused in the canonical text in every SDK; an exponent is never expanded.
- **D135 — What a hostile or broken server can make an SDK do is bounded** (SPEC §10): a body is read up to 32 MiB, a
  wait taken from the server is at most a minute, a message is cut at 512 characters, a trace id that is not 32 hex
  is not repeated, a poll answer runs at most the tasks asked for, a refused re-registration is backed off. Each was
  a finding with a measurement (up to 17,844 registrations in two seconds; 1.96 GiB for a 300 MiB answer; an
  `OutOfMemoryError` from a 19-byte number in Java).
- **D136 — The harness is a good citizen of its process.** One PostgreSQL for a whole play (`hold_server`; the
  container was restarted for every platform, 65 times in a full run); `TestApi` aborts its servers when dropped (they
  and their pools lived until the process ended, which only a suite of seventy platforms notices: connections climbed
  to the limit of 100 and the API answered 503 — the restarts had been masking it); `expireLease` keeps expiring
  until the platform has reclaimed the lease (a heartbeat between the update and the next tick renewed it); files are
  named with the process id; masking is scoped (a `message` of an error, a `version` of a Run); the deadline of a Run is
  compared as `<createdAt+3600s>`.
- **D137 — The Go SDK needs Go 1.25.** `govulncheck` found reachable advisories in grpc 1.80 (heap exhaustion by HTTP/2
  DATA fragmentation, through `ResolveSecrets`) and `x/net` 0.49 (an infinite loop on a bad `SETTINGS_MAX_FRAME_SIZE`);
  the fixed versions need Go 1.25 (grpc 1.83.2, x/net 0.58.0).

### Gaps after the reviews (G134–G141)

- **G134 — The lease is not fenced locally.** An SDK cannot cancel a handler when the lease ran out by its own clock
  (the clocks differ, and the lease carries no TTL); only a `STALE_ATTEMPT` does. A worker cut off from the platform
  keeps running its handler; its report is refused as stale and external effects stay at-least-once. SPEC §6.4.
- **G135 — Nine `govulncheck` findings remain in the Go SDK**, standard-library and `x/net` advisories published on
  2026-10-08 and fixed in Go 1.26.9 and `x/net` 0.60.0, younger than the two-week rule for dependencies. The README
  lists them and says it was not established whether the SDK is exposed to the ones that could concern a client.
- **G136 — Java has no `tls.serverName`; the Maven build downloads `protoc` and `protoc-gen-grpc-java` binaries**
  with no checksum pin (the plugin offers none; the Maven wrapper and the dependency checksums are strict).
- **G137 — What the suite does not see on the wire.** That a worker of protocol 1.0 sends a heartbeat without a step
  identity, that TLS verifies, that a redirect is not followed and that a throwing logger costs nothing are
  unit-tested in each SDK, not observed by the conformance suite (a scenario has no tap on the wire).
- **G138 — The lease-lost and shutdown scenarios depend on a control action**, not on a real silent worker: the
  platform's lease is back-dated until it is reclaimed. Real partitions are the fault tests of feature 002.
- **G139 — Not run:** .NET unit tests on net8.0 (the conformance suite ran once on net8.0), Java 17 and 21, Go other
  than 1.25.14, Node other than 26.8.1, any OS but macOS arm64, Python 3.10 beyond the unit tests and one conformance run.
- **G140 — The benchmark measures this machine in release mode** with the platform and the runner on the same host over
  loopback; it says what the SDKs do to the worker protocol (calls, pauses, backpressure), not what a network costs.
- **G141 — A heartbeat due at the moment a step ends is answered `aborted`, and the platform counts it as an error.**
  Found by the benchmark (backpressure scenario: steps of 2 s, 3 s lease, a beat about every second): TypeScript and Java
  sent two beats per task and 14 and 23 of 32 tasks had one answered `aborted` because `CompleteStep` had already been
  processed; Go and Python had none, C# one. SPEC §6.3/§6.4 keeps the heartbeat running until the report is made
  (so that a report that needs retries does not lose its lease), which makes this race possible in every SDK. Nothing
  is lost (all runs completed, the late beat is the lease protocol working), but `grpc_requests_total{status="aborted"}`
  would read as failures in feature 008's alerts. Options, not decided: (a) the SDK sends no beat while the first
  report attempt is in flight and resumes only if it fails; (b) the server answers a beat for an attempt that the same
  worker already completed with a distinct, non-error status; (c) feature 008 excludes `Heartbeat:aborted` from error
  rates. Why the SDKs differ (2.0 against 1.3 beats per task) was not investigated.
  **Decided 2026-10-16 (owner): option (b), built the same day.** The step keeps the hash of the lease that reported its attempt (`steps.reported_lease_token_hash`, migration `20261108000000`, cleared on every new lease); a heartbeat whose token is that lease gets OK with `lease_expires_at_epoch_ms = 0` and `cancel_requested = false`, counted in `worker_heartbeats_after_report_total`. The protocol has no new field and the five SDKs do not change (they only take a positive deadline; SPEC §6.4). A lost or forged lease stays `STALE_ATTEMPT`: `contract_grpc_durable_execution` covers both, and the reporting-lease case failed before the change.


## Leases that never reach a host (2026-11-20, D006-001 to D006-018, G006-001 to G006-004, O006-001)

The finding. The engine commits a lease before it knows the task reached a host, and nothing takes the lease back: it lapses
after the lease time-to-live. Since feature 037 a lapsed lease of a declared `Activity` is an unknown outcome
(`expire_attempt` records `LEASE_EXPIRED`, which `end_attempt` sends to reconciliation or `step.needs_review`), so work that
never ran waits for an operator. Seen twice in CI (`reset` in Go, jobs 136567175914 and 136453650156: no `step.started` within
30 s, the lease time-to-live) and once in feature 022 (`flow-rate-limit` in C#, PR #251: `step.leased` attempt 1 then
`step.needs_review`, the handler never ran). Where the server drops a committed lease (`crates/api-grpc/src/service.rs`):
(1) `timeout_at(producer_end, claim)` drops a claim future that already committed one or more runs (`claim` commits one
transaction per run); (2) at the window deadline `TaskBody::poll_next` discards a queued frame that carries leases; (3) when
the body is dropped (client deadline or cancel, lost connection) or the token expires, the producer is aborted, possibly
between a commit and the send, and the queue is drained; (4) a frame already handed to the transport can be lost on the wire,
and the answer of a unary `PollTasks` likewise. Where a host drops a lease it received: Go and Python when the host is
stopping, Python on a malformed frame, and the excess of a frame over the credit in the hosts that truncate it; none reports
it. The contract said "After commit, before delivery: lease remains durable; expiration/reaper creates later fenced
attempt" (stream-completion.md), which stopped being true for activities with feature 037.

All decisions below are "delegate (OD-2026-11-20-6)" unless they say otherwise.

### Decisions

- **D006-001 — The server withdraws a lease it knows never left the process** (cases 1 to 3, and a unary poll whose handler is
  dropped before it returns). Withdrawn: in one fenced transaction per run, only while the step is still `LEASED` (no
  heartbeat), with the same generation and lease-token hash the server issued; the step goes back to `READY`, its lease is
  cleared, no attempt is consumed and no review is opened. A fence that does not hold (the step moved on) withdraws nothing
  and is counted. Weighed: leaving the lease to lapse (today: a false review); shortening the lease time-to-live (makes every
  slow first heartbeat a lapse); treating an expiry with no heartbeat as unambiguous (unsafe: a handler can have its effect
  before its first heartbeat, at least 500 ms by default); a delivery acknowledgement (closes more, changes the protocol:
  O006-001). Withdrawal is the only option that fixes what the server can see without changing a guarantee.
- **D006-002 — The producer is never aborted in the middle of a claim.** The body signals the producer to stop; the producer
  finishes the claim it is in (the claim is already bounded by the store's statement and lock timeouts), queues nothing more,
  and hands every lease it holds and did not hand to the transport to the withdrawal. A lease is "handed to the transport"
  when `TaskBody::poll_next` returns its frame; from then on only O006-001 can recover it.
- **D006-003 — The journal records a withdrawal with a new event, `step.lease_withdrawn`.** Weighed against reusing an event
  old engines already replay: `step.released` moves `PENDING` to `READY` only, so an older replica replaying it on a `LEASED`
  step fails with an illegal transition; `step.retry_scheduled` then `step.retry_due` mean "an attempt failed and the budget
  allows another", consume the attempt and the retry budget, and would fail a step for good when no attempt is left. Neither
  fits exactly. The event carries `node`, `attempt`, `generation` and `cause` (`NOT_SENT` when the server withdrew it,
  `NOT_STARTED` when a host handed it back, D006-005). It moves `LEASED` (that attempt and generation) to `READY` and sets the
  attempt back by one, so the next lease reuses the attempt number that never began; the generation is kept, so the next lease
  has the next generation and every fence stays monotonic. It is part of the step's history: a reset copies it like
  `step.leased`.
- **D006-004 — It is written only when every live control plane and engine honours it.** A new capability,
  `lease-withdrawal-v1`, is listed by every control plane and engine of the build that can replay the event (unconditionally:
  it is what the binary and every configuration of it will honour). The engine withdraws only when the requirement
  `lease-withdrawal-v1` is recorded for both kinds, which it learns from the heartbeat of the instance-capability registry (the
  gate keeps the recorded set); until then the lease lapses as before, and the counter
  `worker_leases_undelivered_total{cause, outcome="lapsed"}` shows what activation would recover. Recording the requirement is
  the operator's act (ADR-0082 decision 1): a command `control-plane lease-withdrawal-activate` with `--operator`,
  `--expect-live N` and `--dry-run`, run with the owner role, in one bounded transaction, with no step of its own. A binary of
  release 0.2 does not register, so an activation cannot see it: the runbook's rule stands (the whole fleet on a build that
  registers before any activation, `docs/runbooks/instance-registry.md` section 7), and the released-v0.2 upgrade path proves
  that no `step.lease_withdrawn` is written before the activation and that the current build replays a journal that holds one.
- **D006-005 — A host that drops a lease it received hands it back** through `FailStep` with the code `TASK_NOT_STARTED`,
  retryable, before any handler of it runs, with the lease's fence: when it is stopping, when a frame carries more than it may
  run, and when the rest of a malformed frame is dropped. Never for a task whose handler started. All six SDKs. A server that
  has the requirement of D006-004 recorded and finds the step still `LEASED` withdraws it as in D006-001 with the cause
  `NOT_STARTED`; otherwise (an older server, the requirement not recorded, the step already `RUNNING`) it is an ordinary
  retryable failure, which is never ambiguous: it is retried while the budget allows and fails for good after. Weighed: a plain
  retryable failure always (consumes an attempt for work that never ran, and fails a step without retries for good); saying
  nothing (today: a false review).
- **D006-006 — No cap on withdrawals of one step.** (Superseded by D006-031 on 2026-11-21.) A withdrawal needs the server's own proof that the frame never left the
  process, or the host's own statement while it holds the lease; neither repeats without new polling, which the pace of the
  receive loop bounds. The counter `worker_leases_undelivered_total{cause, outcome="withdrawn"}` and the attempt that is not consumed make a loop
  visible. Revisit if a cause other than stopping or a window deadline shows in it.
- **D006-007 — Feature 037 is clarified, not changed.** A withdrawn lease is not an attempt and has a known outcome (nothing
  ran), so FR-037-006 does not apply to it; a lease that lapses keeps its meaning (unknown outcome, consultation or review).
- **D006-008 — The tests are deterministic.** A barrier after the commit of a claim and before the send, and a channel slot
  the test holds, never a sleep. Each asserts: the step back to `READY`, no `step.needs_review`, the attempt not consumed, and
  the handler run exactly once afterwards.
- **D006-009 — The residue is named, not hidden.** A frame handed to the transport and lost, and a lost answer of a unary
  poll, still lapse into a review after this change (G006-001); O006-001 closes them.
- **D006-010 — Where this lives.** The contract is `stream-completion.md` ("Leases that never reach a host"), the host rule is
  `sdks/SPEC.md` §6.2 and §10, the clarification of feature 037 is in `specs/037-compensation-safety/spec.md` after FR-037-006,
  and the hard-to-reverse choice (a new journal event behind a fleet requirement) is ADR-0098.

### Decisions of the build (2026-11-20)

What the code showed the decisions above do not settle. All "delegate (OD-2026-11-20-6)".

- **D006-011 — A withdrawal on a step whose cancellation was asked for ends in a cancellation.** The withdrawal clears
  `cancel_requested`, as a new lease does, and the planner settles the run in the same transaction: a step of a run that is
  stopping, or of a branch that lost, goes from `READY` to `CANCELLED` there. Nothing ran, so the cancellation needs no
  acknowledgement from a host. Weighed: keeping the flag on a `READY` step (no reader expects it there, and the step would wait
  for a host that will never come).
- **D006-012 — The grain of the transaction.** The root leases of one run are withdrawn in one transaction under the run lock;
  scoped leases in one transaction per run and instance path (the store's path transaction, which takes the same run lock).
  Each lease is fenced on its own: one whose fence does not hold is skipped and counted, the others are withdrawn.
- **D006-013 — What the readers make of it.** The attempts reader drops a withdrawn lease (it is not an attempt, D006-007): it
  has no span, and the next lease of the step keeps the ready time of the first for its scheduling latency. The journal and
  the timeline show `step.lease_withdrawn` with the cause kind `withdrawal` and the code `NOT_SENT` or `NOT_STARTED`. A
  withdrawal makes the step ready again, so it sends the task-ready hint `step.released` sends. A reader that looks up the
  lease of an attempt by its number takes the newest `step.leased` of that number (`ORDER BY seq DESC`), which is the lease
  that ran.
- **D006-014 — `TASK_NOT_STARTED` is decided on the code.** (Narrowed by D006-030 on 2026-11-21: on the code, the minor of
  the session that holds the lease and the cap of D006-031.) The withdrawal does not depend on the `retryable` flag: the code is
  the host's statement that no handler ran. When the server does not withdraw, the report is an ordinary failure with the
  flag the host sent. A managed database task keeps its closed list of failure codes, so a managed runner that sends
  `TASK_NOT_STARTED` is refused as today (G006-004).
- **D006-015 — The gate learns the recorded requirements** right after the process registers and whenever the requirement
  epoch moves, from the same read that decides whether it serves. Until it has read them it withdraws nothing (the lease
  lapses and is counted). The withdrawal runs in the execution engine, which serves the worker gRPC API.
- **D006-016 — A graceful shutdown waits for the withdrawals.** They run on a tracker that the worker gRPC server waits for
  after it stops serving, bounded by the grace of the shutdown; a crash still loses them (G006-002).
- **D006-017 — A unary poll claims in a task of its own.** A client that goes away in the middle of a claim drops the handler,
  and with it, before, the leases already committed for the runs claimed so far. The claim now runs to its end in its own
  task: its leases come back to the handler, or, when the handler is gone, go to the withdrawal.
- **D006-018 — The counter.** `worker_leases_undelivered_total{cause, outcome}`: `cause` is `NOT_SENT` or `NOT_STARTED`;
  `outcome` is `withdrawn`, `fence_missed` (the step moved on), `lapsed` (the requirement is not recorded) or `error` (the
  store failed; the lease lapses).

### Gaps

- **G006-001 — A lease handed to the transport and lost still lapses into a review** (a frame lost on the wire after
  `poll_next` returned it, the answer of a unary `PollTasks` lost after the handler returned). Closed by O006-001.
- **G006-002 — A process that stops (or crashes) between a claim and its withdrawal** leaves the lease to lapse. The withdrawal
  is spawned when the body ends and runs to its own bound; a graceful shutdown waits for it, a crash does not.
- **G006-003 — Before the operator records `lease-withdrawal-v1`, nothing changes** but the counter: an upgraded fleet keeps
  today's behaviour until the activation.
- **G006-004 — The managed database runners do not hand leases back.** Their failure codes are a closed list
  (`DatabaseFailure`); a lease such a runner drops lapses as before. Their own design decides whether to add the code.

### Owner question

- **O006-001 — Delivery acknowledgement (option C).** Answered by the owner on 2026-11-20: "Yes, in 0.3.0"
  (OD-2026-11-20-12). The sketch the question was asked on:
  - *Variant 1, a provisional lease.* A claim grants a provisional lease (`OFFERED`) with a short acknowledgement deadline;
    the host acknowledges each task (a new RPC, or a frame of a bidirectional stream) before it runs the handler; only the
    acknowledgement makes the step `LEASED`; an offer not acknowledged in time is withdrawn by the engine. Exact ("the handler
    runs only after the server knows the host has the task"); costs one round trip and one statement per task; needs a new step
    status and a new event.
  - *Variant 2, the first heartbeat before the handler.* Every host of the new minor sends its first heartbeat when it
    dispatches a task and starts the handler only when that heartbeat is answered; the engine withdraws a lease still `LEASED`
    (no heartbeat) when the acknowledgement deadline passes, because a host of that minor cannot have started it. Reuses
    `Heartbeat` and `step.started` (the first heartbeat already moves `LEASED` to `RUNNING`) and the withdrawal of D006-003;
    costs the same round trip per task, moved before the handler instead of after one interval.
  - Both need a new worker-protocol minor (the next free one in the register of `sdks/SPEC.md`, after 1.7 of design 041:
    OD-2026-11-15-8), all six SDKs, conformance scenarios in six languages, and the server must keep serving older minors
    unchanged (their leases keep today's semantics). The acknowledgement deadline must be derived from measured delivery and
    heartbeat latency, not guessed. The variant, the minor and the deadline are decided in the ADR that follows (design first,
    the four reviewers before any code).

### Decisions after the reviews (2026-11-21, D006-030 to D006-039, G006-030)

The three reviews of the build (`reviews/2026-11-21/`) found three Blockers and six Majors. The coordinator chose the fixes;
each decision below is "delegate (OD-2026-11-20-6)". Numbers 019 to 029 and 040 to 061 belong to the SDK branch and to the
design of delivery acknowledgement, so this list starts at 030.

- **D006-030 — `TASK_NOT_STARTED` belongs to the host, never to a handler** (security Blocker 1, architecture Blocker 1).
  Three layers. (a) Every SDK rewrites a `TASK_NOT_STARTED` that a handler raises into an ordinary error (`WORKER_ERROR`), as
  it does for a code that is not valid; only the host's own hand-back sends it (the SDK branch `bugfix/withdraw-leases-sdks`).
  (b) The server honours it as a withdrawal only when the worker session that holds the lease (`steps.leased_by`) negotiated
  a protocol minor that reserves the code: the minor of delivery acknowledgement, 1.8 (OD-2026-11-20-12, ADR-0099). From a
  session of an older minor it is an ordinary failure: the attempt is spent, the backoff applies and the host's
  `retryable` flag is honoured. SDKs already in the field and raw clients forward any code, so the reservation must be the
  server's. (c) The cap of D006-031. This build negotiates at most 1.6, so no session reaches the withdrawal yet (G006-030);
  the path is tested on a session whose stored minor the test raises to 8, the only way before 1.8 negotiates, and the
  delivery acknowledgement build replaces that with a real 1.8 session. Narrows D006-005 and D006-014. Weighed: the SDK
  rewrite alone (older SDKs and raw clients still send the code); the cap alone (a few free re-runs per attempt for any
  handler); refusing the code outright before 1.8 (a host that follows `sdks/SPEC.md` §6.2 would get an error for an honest
  hand-back, where an ordinary failure is a known outcome).
- **D006-031 — At most four withdrawals for one attempt** (security Major 1; replaces D006-006). The count is derived from
  the journal: the step's withdrawals that carry the attempt it is about to withdraw. A withdrawal gives the attempt back,
  so every withdrawal before the attempt is spent carries the same number; the event's shape does not change. The
  derivation of the bound:
  1. One delivery is withdrawn at most once. After a withdrawal the step is `READY`, its token is dead, and the fence of a
     second report with that token misses.
  2. Four causes end a delivery of an attempt before any handler runs. The server's one: the consumer ended before the
     frame reached the transport (a window that ended, a deadline, a client gone; for the count they are one cause, the
     consumer's call ending). The host's three: it is stopping, a frame carries more than its credit, the rest of a
     malformed frame is dropped. On the wire the host's three are one code.
  3. Each cause is a fault of one delivery, not of the task. A host stops once in its life. A frame over the credit and a
     malformed frame are faults of that frame. A cause that repeats for the same attempt is a property of the task, the
     host or the client: a frame that never decodes, a client that always drops its call. It must spend the attempt like
     any other failure.
  4. So an honest attempt meets each cause at most once: four withdrawals. The cap is on the total, because the host's
     three causes cannot be told apart.
  The fifth withdrawal of one attempt is refused, and the outcome is counted as `capped`. Past the cap a `TASK_NOT_STARTED`
  is the ordinary failure (the attempt spent, the backoff, the flag honoured), never a review. A lease the server did not
  send is left to lapse, as before this feature: the server has no failure of a host to record, and a lapse keeps today's
  meaning. A cap set too low costs one attempt, for example a rolling restart that hands the same step back a fifth time.
  A cap set too high gives four free leases. The domain refuses the fifth in the command, not in the replay: a journal is
  replayed whatever it holds. Tested at its edges for both causes: the fourth is withdrawn, the fifth is not.
- **D006-032 — The requirement is a replay floor, and an engine-local switch stops new withdrawals** (architecture Blocker
  2). `lease-withdrawal-v1` meant two things: permission to write the event, and the ability to replay it. Removing it
  re-opened the gate for builds that cannot replay `step.lease_withdrawn`. Now:
  - A trigger on `platform_requirements` refuses to delete `lease-withdrawal-v1` while any journal holds the event. A new
    durable event is forward-only.
  - Each withdrawal transaction reads the requirement under a share lock, through an owner function the runtime role may
    run (`lease_withdrawal_required()`). The removal deletes the row, so the two cannot cross. A removal that commits first
    leaves the withdrawal with no requirement, and it writes nothing. A withdrawal that commits first is seen by the
    removal's check, which refuses.
  - `SAGAT_LEASE_WITHDRAWAL=off` on an engine stops it writing new withdrawals. Its leases lapse as before, counted as
    `disabled`. The engine still lists the capability, because it can still replay the event. This switch is the way to
    stop withdrawing.
  - Rolling a binary back below this feature is safe only while no journal holds the event. The down migration already
    refuses otherwise.
  Weighed: removing the requirement as the way to stop (lets older builds serve journals they cannot replay); a marker row
  written by the first withdrawal (one more table and grant for the same answer the journal already gives).
- **D006-033 — The capability lists in SQL follow the code** (architecture Blocker 3). Feature 022
  (`20270130000200_flow_control_capabilities`) and this feature redefine the same three checks: `serving_instances_capabilities_known`,
  `platform_requirements_capability_known` and `platform_requirement_history_capability_known`. Each lists its own values
  only, so whichever branch merges second takes the union of both lists in its own migration, the newest one. A fitness
  check in `scripts/check-architecture.py` fails when the newest definition of any of the three lists differs from the
  tokens of `Capability::ALL`, so the order of the merges cannot drop a capability.
- **D006-034 — One operator command activates a fleet requirement** (architecture Major 5): `control-plane
  capability-activate <capability> --operator NAME [--expect-live N] [--dry-run]`. `<capability>` is the registry token
  (`lease-withdrawal-v1`). The command holds a closed list of the capabilities an operator may activate this way. Each
  entry gives the kinds the requirement covers, whether an empty fleet may be activated with `--expect-live 0`, and the
  sentence that says what follows and where the runbook is. Every entry gets the same steps:
  - the operator's name is checked (printable, a subject);
  - the live processes of the kinds are counted, and the count must equal `--expect-live` (without it, none live is a
    refusal);
  - a live process that does not list the capability is refused by its instance id;
  - a dry run checks that the database role may record the requirement, and changes nothing;
  - the fleet is counted again under the epoch lock;
  - a requirement already recorded exits 0.
  Feature 022 adds its two capabilities as entries and drops `flow-control-activate`. `lease-withdrawal-activate` was never
  released, so it goes away with no alias. The one-way handover of `schedules-v1` (`schedule-trigger-handover`, ADR-0082)
  stays separate: it changes the platform's state in the same transaction and has no way back.
- **D006-035 — A claim stops leasing when its consumer is gone** (correctness Major 1). Before each run's transaction the
  claim asks whether its consumer is gone (the stream's body dropped or past its window, the unary handler dropped). If so,
  it stops and returns what it already committed, which then goes to the withdrawal. Stopping between two run
  transactions leaves nothing half done. A test probe runs after each run's commit, so a test can drop the consumer
  between two runs (correctness Major 2).
- **D006-036 — The producer and the unary claim run on the withdrawal tracker** (correctness Major 3, architecture Major 2).
  A graceful shutdown then waits for a claim in progress and for the withdrawal it hands over, not only for the
  withdrawals already started. The drain is bounded by the grace that is left, not by a new grace of its own.
- **D006-037 — A unary poll keeps the tenant's concurrency slot until its claim ends** (security Major 2). The slot moves
  into the claim's task, so a client that cancels at once cannot run more claims at the same time than the tenant's cap.
- **D006-038 — At most a quarter of the pool withdraws at once** (security minor 3, architecture Major 3). An engine runs at
  most `max(1, SAGAT_DB_MAX_CONNECTIONS / 4)` withdrawals at a time: 5 with the default pool of 20. The derivation:
  - The pool's other users (the scheduler's passes, up to the pool less two; the lease keeper; the claims and reports of
    the worker API) can each fill it on their own. The withdrawal is the one user that can wait: a lease lasts its
    time-to-live, and the run lock it takes is the one a claim takes.
  - With a quarter, three quarters of the connections stay for the rest at every moment.
  - A withdrawal that waits holds no connection. The leases waiting are bounded by what claims can commit (the process's
    stream bound times the credit of 64, plus the unary polls in flight), so the wait needs no bound of its own.
  - A withdrawal that waits past its lease's expiry is still fenced. A lease the reaper ended misses the fence and is
    counted `fence_missed`.
- **D006-039 — ADR-0098 supersedes ADR-0029 decision 5 for the leases the server never handed to the transport**
  (architecture Major 1). ADR-0029 decision 5 says never to release a committed lease because a write, stream or client
  was cancelled, because a closed stream never proves that a handler did not start. That reason still holds for a frame
  handed to the transport. It does not hold for a frame `poll_next` never returned: no host can have it, and the server
  knows. ADR-0098 says so plainly and stays Proposed. Its acceptance goes to the owner, through the coordinator, because it
  overturns part of an accepted decision about the guarantees of the worker protocol.

- **G006-030 — No session honours `TASK_NOT_STARTED` before protocol 1.8 negotiates.** A host of 1.6 or 1.7 that hands a
  lease back gets an ordinary failure: the attempt is spent and retried with its backoff. That is a known outcome, never a
  review. Closed when the delivery acknowledgement build (ADR-0099) accepts 1.8.
