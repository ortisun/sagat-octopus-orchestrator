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
- **D006-006 — No cap on withdrawals of one step.** A withdrawal needs the server's own proof that the frame never left the
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
- **D006-014 — `TASK_NOT_STARTED` is decided on the code.** The withdrawal does not depend on the `retryable` flag: the code is
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
  (OD-2026-11-20-12). Designed in ADR-0099 (decisions D006-040 to D006-061 below: a flag on the lease, a provisional mark on the
  step row, an acknowledgement under the row lock, protocol 1.8, the lease's own deadline). The sketch the question was asked on:
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




## Delivery acknowledgement (2026-11-20, D006-040 to D006-061, G006-040 to G006-047)

Owner question O006-001 was answered "Yes, in 0.3.0" (owner decision OD-2026-11-20-12), which left the variant, the protocol
minor and the deadline to this design. All decisions below are "delegate (OD-2026-11-20-6)" unless they say otherwise. The
numbers start at 040 because the builds of ADR-0098 record theirs from D006-011 and G006-004 (server) and from D006-020 and G006-020
(SDKs) on their branches. The contract is `stream-completion.md`, "Delivery acknowledgement (protocol 1.8)"; the host rules are
`sdks/SPEC.md` §5, §6.1 to §6.5 and §6.8; the hard-to-reverse choice is ADR-0099; the build plan is `tasks.md`, phase "Delivery
acknowledgement". The four reviewers ran three rounds on 2026-11-20 (`reviews/2026-11-21-delivery-ack/`, the directory named by
the coordinator): round 1 found 5 Blockers and 32 Majors, round 2 found 2 Blockers and 21 Majors; the decisions below answer
both, and each names the findings it answers.

### Decisions

- **D006-040 — The variant: a flag on the lease, a mark bound to its generation, an acknowledgement that writes one column under
  the step row's lock.** Variant 1 of O006-001 is kept for its exactness without its status and event; variant 2 (the first
  heartbeat before the handler) costs, per task, a transaction that restores the run, applies a transition and appends
  `step.started`. The first round's acknowledgement took the run lock and renewed the lease; the renewal is what needed the run
  lock, and with it came a transaction per run in a call, head-of-line blocking on a busy run, a renewal channel that kept
  unmarked leases `LEASED`, and a call bounded by the slowest run (round 2: design Major 1, Major 4, Major 5; security Major 2;
  correctness Minor 10). Without the renewal, the acknowledgement writes `lease_acknowledged_at` and the run of withdrawals only;
  the only writers that decide from that column are the withdrawals, which take the same row lock (D006-044). The lease keeps its
  claim-time expiry, and the margin of D006-046 gives the first heartbeat the time the renewal gave.
- **D006-041 — Protocol minor 1.8, offered on build proofs alone.** The register (`sdks/SPEC.md` §9, OD-2026-11-15-8) holds 1.1 to
  1.7; no branch or worktree names 1.8 (searched 2026-11-20). 1.8 is reserved now at the coordinator's instruction, which is an
  exception to the register's rule ("when its contract merges") and goes to the owner; it is released if ADR-0099 is not
  accepted, or if the build stops at its gate (D006-057). Minors are cumulative (ADR-0015, design 041), so the server serves 1.8
  only once it serves 1.7 (task T041-7) and an SDK offers it only once it speaks 1.7 (tasks T041-15 to T041-20). Round 2 found that
  a host rule bound to the minor ("a session at 1.8 acknowledges every lease"), with 1.8 offered only after the activation, would
  gate every later minor behind the operator's switch and the fleet's time-to-live (architecture Blocker 1); the per-lease flag
  `acknowledge_before_start` (TaskLease field 19) puts the contract on the lease, so 1.8 depends on build proofs only.
- **D006-042 — The mark is bound to the lease's generation and counts only while the step holds a lease.** `lease_offered_generation`,
  `lease_offered_at` and `lease_acknowledged_at` on `steps`, written by the three statements that write a lease (root, scoped in a
  full-run transaction, path; round 2: correctness Minor 2), and by the acknowledgement and the withdrawals. A mark counts while the
  step is `LEASED` or `RUNNING` and its generation is the step's (round 2: correctness Minor 6), so a stale mark never counts
  (round 1: architecture Blocker 2, correctness Major 6, security Major 3). The engine's statements keep or clear
  `lease_acknowledged_at` and never write it from a value they read. Weighed: a trigger that clears the marks (a trigger on the
  hottest table, hidden from the budget); a `CHECK` on the status (an older binary could no longer write the row).
- **D006-043 — The acknowledgement: one transaction per call, no run lock.** The permission once; one unlocked pre-read by primary
  key matching the token hashes, so a caller without the tokens costs one indexed read and no lock (round 1: security Major 4);
  per namespace, the checks of `ensure_bound` under its own locks, so a revocation is ordered with it (round 2: security Major 3,
  correctness Minor 4); one conditional statement over the bound rows in primary-key order, with `clock_timestamp()` evaluated under
  the row lock (round 2: design Major 2, correctness Minor 3); at most one cross-tenant and one binding audit record per call through
  the paths that exist (`missing_task_error`, `spawn_audit_access_denied`; round 2: security Minor 1); transient errors end the call
  retryable and are never `NOT_HELD` (round 2: correctness Minor 5). About 6 statements per call whatever its size.
- **D006-044 — Exactly one of the acknowledgement and a withdrawal commits, by the step row's lock.** Every withdrawal of a lease
  whose mark counts (the pass's, the hand-back's, `NOT_SENT`) re-reads the step row `FOR UPDATE` under the run lock and decides
  with one pure function, which a parity test holds equal to the SQL condition (round 2: architecture Minor 2); the acknowledgement
  takes the same row lock. The candidate query of the pass is only a hint (round 1: correctness Major 2).
- **D006-045 — An acknowledged lease is an attempt.** It is never withdrawn; a `TASK_NOT_STARTED` for it is an ordinary retryable
  failure (round 1: security Major 1). Capabilities are served only after the acknowledgement. An unmarked lease keeps the rules of
  ADR-0098, and the acknowledgement never touches it (round 2: security Major 2).
- **D006-046 — The deadline is the lease's own earlier deadline; a lease is marked only when that leaves 3R = 18 s.** R = 6 s for
  the read (specified), R for one acknowledgement attempt (the host's bound), R of margin before the earlier deadline (so the first
  heartbeat, at a third of the time left and at least 2 s, commits before the lease can expire, since the acknowledgement does not
  renew it). The first round's 7 s left out the call and the network (round 1: correctness Major 7, architecture Major 2); the
  second round's 12 s had no margin and checked only the expiry (round 2: correctness Major 1, architecture Major 1, design Major 3).
  The threshold is applied at the claim, by the claiming engine, to the lease's own deadlines, not to the engine's listing of the
  capability, so an engine with a short time-to-live is not refused, it only marks nothing (round 2: design Major 6).
- **D006-047 — The gate.** `delivery-acknowledgement-v1`, listed by every control plane and engine of the build from the task that
  follows the measurement (round 2: architecture Major 5); recorded and removed by two commands that check the dependency on
  `lease-withdrawal-v1` under the epoch lock with a typed refusal; the registry's guard, an invoker function with schema-qualified
  names that takes the epoch row first, checks it as a backstop (round 2: security Minor 5, architecture Minor 3); the removal of
  `lease-withdrawal-v1` refuses while a mark counts (round 2: correctness Minor 7, architecture Major 4, design Minor 3). This
  refines ADR-0082; an amendment line goes into ADR-0082 at acceptance. With the per-lease flag, no session has to be expired on a
  removal.
- **D006-048 — The host at 1.8.** It acknowledges flagged leases before it calls or runs anything for them but the hand-back
  (round 2: correctness Major 3, security Major 1, architecture Major 2, design Blocker 1); it bounds a `PollTasks` call and each
  acknowledgement attempt to R; it gives a lease up after its time left at receipt minus R, clamped to between R and 120 s, on its
  monotonic clock (round 1: correctness Major 9); it treats `CANCELLED`, `UNKNOWN`, `ABORTED` and `DEADLINE_EXCEEDED` as transport
  failures (round 2: correctness Minor 8); it falls back to 1.7 until it restarts on `UNIMPLEMENTED`, after three calls in a row
  refused with another non-retryable status, or after three leases in a row given up (round 2: correctness Major 2, architecture
  Minor 6, design Minor 6); a stopping host waits at most R for an acknowledgement in flight and runs what is acknowledged (round 2:
  correctness Minor 9).
- **D006-049 — `timeoutMs` keeps its meaning at every minor, and a provisional lease is withdrawn at its earlier deadline.** The
  second round consumed an attempt at the attempt deadline of a provisional lease, which failed a step with one attempt for a
  delivery fault (round 2: correctness Major 4, design Minor 2); counting `timeoutMs` from the acknowledgement would give one
  published version two meanings (round 1: architecture Major 4). Withdrawing at the first of the expiry and the attempt deadline
  keeps the attempt and keeps the meaning, because the next lease is a new claim with its own deadlines; the bound of D006-052
  ends a step whose timeout is shorter than its deliveries. No new failure code.
- **D006-050 — A provisional lease receives no capability.** A fence variant (root and path) that only the capability paths call,
  after the token and binding matched (round 1: security Minor 2); a contract test walks the descriptor recursively, through
  nested, `oneof` and repeated fields, and fails when a lease-token method outside the four exemptions has no case (round 2:
  security Minor 6, architecture Minor 13).
- **D006-051 — Cancellation before the start needs no write.** `CANCELLED`, nothing written; the host hands back; ADR-0098's
  withdrawal runs the settle; if the hand-back is lost the withdrawal at the deadline does (round 2: design Minor 4, correctness
  Minor 1, security Minor 7). It depends on request (1) of D006-059 only for its speed.
- **D006-052 — The run of withdrawals: three, whatever their causes, cleared only by an acknowledgement.** Every withdrawal of a lease
  whose mark counts increments it; a hand-back does not clear it (round 2: correctness Major 2, security Minor 3). At three the next
  lease is not marked and a fourth loss lapses as today. This revises D006-006 for marked leases, whose premise does not hold for
  them: silence is enough. Three: one loss on the wire is rare, a second in a row for one step is already improbable, a third is
  treated as systematic; the delay before today's outcome is at most three earlier deadlines. Weighed: consuming an attempt after
  the bound (fails a run for a delivery fault); a backoff on `READY` (a status the claim does not delay).
- **D006-053 — The trust assumption is the owner's, and the server records what it sees.** The withdrawal of a provisional lease,
  and the hand-back of one, record "nothing ran" on the host's protocol promise (round 1: security Major 5). Counted and audited once
  per step and generation: a `Heartbeat`, `CompleteStep` or a `FailStep` other than the hand-back on a provisional lease; any call
  other than a hand-back with the token of a lease withdrawn as `NOT_ACKNOWLEDGED` (once per step and withdrawn hash; round 2:
  security Minor 4). Hand-backs are exempt from both (round 2: security Major 1, design Blocker 1). The session flag of round 2 is
  dropped: it could not be attributed to a session (security Minor 2), and the run of withdrawals already ends a loop.
- **D006-054 — Scoped instances follow the same rules** (round 1: correctness Major 4, security Major 2, architecture Major 1, design
  Major 6). The acknowledgement addresses the row by its full address; the withdrawal is the ADR-0098 build's scoped withdrawal with
  the same re-read; the refusal is a variant of `live_path_fence`.
- **D006-055 — Rollback: remove, drain the marks, then remove `lease-withdrawal-v1` if needed, then roll back no further than a
  build that decodes the cause.** The `down` migration refuses while a mark counts, while the history names the capability, or
  while a live process lists it (round 2: architecture Minor 4); the closed capability checks are extended `NOT VALID` and then
  validated, as the ADR-0098 build's migration does.
- **D006-056 — What is counted and alerted** is listed in `stream-completion.md`, "Observability": ratios, not absolute rates
  (round 2: design Minor 5); a metric behind every alert (architecture Minor 7); sessions by minor (design Minor 7).
- **D006-057 — The cost and its gate, before the six SDKs** (round 2: architecture Major 5, design Major 7). By count of the
  statements of D006-043, a task gains about 6 statements and 1 transaction under light load (102 against 94 round trips with
  `BEGIN` and `COMMIT`), plus the call's authentication; the round-trip model of feature 038 predicts about 8% fewer transitions a
  second. Task T006-056 measures it with a network between the workers and the services, the median of five interleaved runs of
  each minor (round 2: architecture Minor 9), with a fan-out run and a burst that fills the reserve. The build goes on only if 1.8
  is no more than 15% below 1.7, a 64-lease call is answered within 3 s at the 99th percentile, the 99th percentile of ready to
  handler entry rises by no more than the call's, and no heartbeat is shed while acknowledgements are admitted. 15% is the
  prediction with a margin about as large again for the variation between runs on a loaded machine, which the 2026-11-12 run of
  feature 038 warns about (its load average was 26): a choice recorded here, not a measurement. If it fails, the capability stays
  unlisted (the feature dormant) and the owner decides. Release 0.3.0 ships the requirement unrecorded, as ADR-0098 does: an
  operator activates `lease-withdrawal-v1`, then `delivery-acknowledgement-v1`.
- **D006-058 — The conformance evidence.** The 26 scenarios that pin no minor start no worker and are no evidence for 1.8 (round 2:
  architecture Major 6, design Minor 1). The harness override (`SAGAT_SDK_PROTOCOL_OVERRIDE=1.8`) raises every host scenario pinned
  at 1.4 or 1.5 to 1.8, with step logs off (round 2: architecture Minor 11), and compares only two fields differently: the
  negotiated `protocolVersion` against the pinned one, and, for scenarios pinned at 1.4, the instance identity that 1.5 adds to a
  lease (round 2: correctness Major 5). Under it the 51 host scenarios other than `protocol-negotiation` must keep their goldens
  unblessed, the three with a 3 s time-to-live among them, whose leases stay unflagged. The new scenario adds the cases the 1.4
  scenarios carry (secrets and an input blob after `ACKNOWLEDGED`, refused before it; a failure retried), a refused call, a skewed
  clock, a busy run in a frame, four give-ups in a row, a handler that raises `TASK_NOT_STARTED`, and a collection item.
- **D006-059 — What this design asks of the ADR-0098 build, before it merges.** (1) `TASK_NOT_STARTED` on a `LEASED` lease that is
  not acknowledged is accepted whatever `cancel_requested` and the stop reason say; only those conditions of
  `attempt_authority_fence` are relaxed for it, the token, binding, attempt, generation and expiry checks stay (round 2: security Nit
  4); today `checked_secret_report` refuses it for a cancel-requested step with declared secrets (round 1: correctness Major 3). (2)
  The cause `NOT_ACKNOWLEDGED` joins the closed set of `WithdrawalCause` and its codec, so that the ADR-0098 build can replay a
  journal of this one and is the rollback floor; a closed set is kept over an open one because it catches a corrupt row, and a
  future cause comes with its own build and requirement (round 2: architecture Minor 12). (3) Its withdrawals of a lease whose mark
  counts re-read the step row `FOR UPDATE` (D006-044); this build adds that, since only this build knows the columns. (4) Its
  hand-back withdrawal requires that no capability was served to the attempt, or names that residue as its own gap (round 2:
  security Major 2): an unmarked lease is ADR-0098's, which this design no longer widens. All four go to the coordinator; none
  changes ADR-0098's decisions.
- **D006-060 — The frequency is unknown.** No loss of a frame on the wire or of a `PollTasks` answer has been observed; the three
  cases that started this work are ADR-0098's (round 1: design Major 3). At 1.8,
  `worker_leases_undelivered_total{cause="NOT_ACKNOWLEDGED"}` measures it.
- **D006-061 — Minor 1.8 reserves `TASK_NOT_STARTED` for the host** (the server review of pull request #260, given by the
  coordinator on 2026-11-20, a decision of that review). The server honours the code as a withdrawal (cause `NOT_STARTED`) only
  from a session accepted at 1.8 or later, only while the step is `LEASED` with the issued fence and the lease is not acknowledged,
  and only under the per-attempt cap the server applies; from an older session, beyond the cap or when a condition fails, it is an
  ordinary retryable failure with the flag the host sent. The code is the host's statement that no handler of the lease ran or will
  run; a handler cannot send it: the SDKs rewrite a handler-raised `TASK_NOT_STARTED` to `WORKER_ERROR`, keeping the handler's
  `retryable` (§6.5 already rewrites a code a handler may not use). The cap's value is derived by the server build of ADR-0098 and
  recorded in its decisions; this design names it and does not choose it. The cap bounds hand-backs of one attempt number; the run
  of D006-052 bounds withdrawals of marked leases of one step; both apply.

### Gaps

- **G006-040 — A host that crashes between the verdict `ACKNOWLEDGED` and the start of the handler** leaves an acknowledged lease
  that lapses into a review. The window is inside the process and holds no call; a stopping host runs the task instead.
- **G006-041 — An acknowledgement whose answer is lost, followed by the host giving the lease up,** becomes an ordinary retryable
  failure (the hand-back of an acknowledged lease consumes the attempt), or, when the hand-back cannot reach the server either, a
  lapse and a review.
- **G006-042 — The trust assumption** (D006-053). A client at 1.8 that runs a flagged lease without acknowledging and makes no call
  during it (a slow handler, a lost report, a crash) has its lease withdrawn and its work run again with the same attempt number
  and idempotency key; external effects stay at least once (constitution principle III); a later call with its token is counted and
  alerted, but only the last withdrawn token of a step is kept; the refusals of D006-050 make most such clients fail loudly. A
  hand-written client can also send `TASK_NOT_STARTED` after running a handler; the code is the host's word, as in ADR-0098.
- **G006-043 — The internal runners stay below 1.8** (the database runner at 1.6, the sandbox runner at 1, the command-line probe).
- **G006-044 — For one registry heartbeat after an activation or a removal**, an engine may still claim without marks, or with them.
- **G006-045 — The throughput cost and the call bound are predictions** until task T006-056 measures them.
- **G006-046 — The journal does not record that a lapsed lease had been acknowledged**; `worker_leases_expired_total{mark}` does.
- **G006-047 — A lease whose attempt timeout, or whose engine's time-to-live, is under 18 s is not marked**, so a frame lost on the
  way to it still lapses into a review: the protection needs the time of a read, an attempt and the margin.
