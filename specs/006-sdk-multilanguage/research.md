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
- **D006-004 — It is written only when every live control plane and engine honours it** (its command is superseded by
  D006-034). A new capability,
  `lease-withdrawal-v1`, is listed by every control plane and engine of the build that can replay the event (unconditionally:
  it is what the binary and every configuration of it will honour). The engine withdraws only when the requirement
  `lease-withdrawal-v1` is recorded for both kinds, which it learns from the heartbeat of the instance-capability registry (the
  gate keeps the recorded set); until then the lease lapses as before, and the counter
  `worker_leases_undelivered_total{cause, outcome="lapsed"}` shows what activation would recover. Recording the requirement is
  the operator's act (ADR-0082 decision 1): a command `control-plane lease-withdrawal-activate` with `--operator`,
  `--expect-live N` and `--dry-run`, run with the owner role, in one bounded transaction, with no step of its own. A binary of
  release 0.2 does not register, so an activation cannot see it: the runbook's rule stands (the whole fleet on a build that
  registers before any activation, `docs/runbooks/instance-registry.md` section 7). The released-v0.2 upgrade path
  (`release_upgrade_inflight.rs`) proves that no `step.lease_withdrawn` is written and no requirement recorded before the
  activation; that the current build replays a journal that holds one is proved by `verify_run` after every withdrawal of
  `tests/contract/durable-execution/grpc/lease_withdrawal.rs` (corrected 2026-11-21: the upgrade path holds no withdrawal).
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
  (Owner decision 2026-11-20-12). Designed in Architecture decision 0099 (Decision 006-040 to Decision 006-066 below: a flag on the lease, a provisional mark on the
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

### The hosts' hand-back, as built (2026-11-20, D006-020 to D006-028, G006-020 to G006-022)

The audit of the six worker hosts, every place a lease that was received is not run (before this change none reported it):

| Host | Received after `stop` began | Beyond the credit or the free slots | Rest of a frame that breaks the contract | Could not start | Duplicates |
|---|---|---|---|---|---|
| Go | stream frame dropped; a poll answer was run | poll answer and stream frame cut | input budget: the lease and the rest | none | keyed by the task id alone: a sibling iteration overwrote the in-flight entry of the other |
| Python | stream frame dropped, or lost when `stop` cancelled the loop task while a frame was read; a poll answer lost the same way | poll answer cut; stream: rest of the frame | input budget, or a duplicate: the rest of the frame | none | told by the task id alone: a sibling iteration was a duplicate and the rest of the frame was dropped |
| TypeScript | stream frame dropped; a poll answer was run | poll answer cut (by the free slots, not the credit asked for); stream: rest of the frame | input budget, or a duplicate: the rest of the frame | none | stream told by the task id alone, as Python; none at all on the poll path, where a duplicate ran a second handler |
| C# | a frame or a poll answer read in the moment between the read and the check of `stop` was run (both tests failed on the original code) | poll answer and stream frame cut | input budget: the lease and the rest | `Task.Run` refused | none: the in-flight table was keyed by a sequence number |
| Java | a stream frame and a poll answer were run | poll answer and stream frame cut | input budget: the lease and the rest | the task pool refused (`RejectedExecutionException`), after its "task taken" event | in flight only (keyed by the in-flight key) |
| Rust | a second buffered frame read in the same poll after `stop` cancelled the receive was run | poll answer and stream frame cut; the window ended at the credit, so later frames were never read | none (no input budget is checked) | no permit; the in-flight table poisoned | in flight only |

A frame that carries tasks and a retry hint is run within the credit and then ends the window in Python and TypeScript:
nothing of it is dropped, so it is not a hand-back.

All decisions below are "delegate (OD-2026-11-20-6)".

- **D006-020 — What a hand-back is.** `FailStep` with `TASK_NOT_STARTED`, `retryable = true`, the fixed safe message "the
  host did not start this task", the idempotency key of every failure (`failed-<task_id>`) and the lease's fence and
  identity, retried as a report, at most once per lease, with no heartbeat, secret resolution or handler before it.
  `TASK_NOT_STARTED` is a step failure code like `NO_HANDLER` and is not added to `sdks/error-catalog.json`: the catalog
  lists what a `SagatError` raised to a caller can carry, and a hand-back is never raised. It is observed through the
  host's existing failure event or counter with that code, and a warning log line with the task id and the reason, so no
  SDK grows a public event type. Weighed: a new event type in six SDKs (more surface for one counter the code already
  separates); a catalog entry (six embedded copies and a code no caller meets); a message naming the reason (the server
  does not need it, and the log has it).
- **D006-021 — Which leases, the same in all six.** The whole answer or frame received after `stop` began, on both
  transports (Go and TypeScript ran a poll answer that arrived while stopping, and Java a frame too; running it delays the
  stop by a whole task, while a hand-back gives it to another host at once); the leases beyond the credit or the free
  slots (all six cut them and left them to lapse); the rest of a frame that breaks the receive contract, the lease that
  broke it included; a lease the host could not start for a reason of its own. Never a duplicate (D006-022).
- **D006-022 — Duplicates are told by the in-flight key, never by the task id alone, and are not reported.** The legacy
  task id is `run/node/generation`, which sibling iterations of a collection share; Python and TypeScript took the second
  sibling of a frame for a duplicate and dropped the rest of the frame. The key is the task id and, from protocol 1.5,
  the instance path (what Java, TypeScript and Rust already keyed the in-flight table by). A real duplicate (the key is
  in flight, or already seen in the same answer or window) is ignored and logged: handing it back would carry the fence of
  the running handler and fail it.
- **D006-023 — `stop` cancels the receive call, never the code that reads a frame, and waits for the hand-backs.** Python
  cancelled the loop task, which can replace a frame already read with the cancellation and lose it; it now cancels the
  receive call (as TypeScript, Java and Go do) and the loop hands back what it read. Every host counts a hand-back on its
  way as work in flight for `stop`, within the same grace, so the connection is not closed under it.
- **D006-024 — The six-language conformance scenario waits for the server's withdrawal.** A scenario can drop a lease
  deterministically against the real server: a fault action of the conformance fault plans that raises the credit the host
  sends (`max_tasks`) while two steps are `READY` makes the server lease more than the host asked for, and the host hands
  the rest back. Its golden, though, is the server's answer: today an ordinary retryable failure (`step.failed`, then a
  retry with the next attempt), and, once the withdrawal of D006-001 to D006-005 is built and `lease-withdrawal-v1` is
  recorded in the conformance server, `step.lease_withdrawn` with the attempt not consumed. A golden blessed now would
  record the behaviour the fix replaces and be rewritten when the server lands. So the scenario (`worker-hand-back`, the
  fault action in the six fault plans, the golden read before it is blessed) is added on the design branch after the
  server side is merged into it (G006-020). Until then the deterministic proofs are the hand-back tests of each host
  against its fake server (eight cases per host, barriers, no sleeps). Weighed: a golden of today's server (a proof of the
  older-server path only, rewritten within days); no scenario (six hosts with no shared proof against the real server).
- **D006-025 — Every host reads a filled window to its end, ends a broken one, and checks its input budget.** Rust
  ended a window as soon as its credit was used, so a frame an over-sending server sent after it was never read (it
  lapsed), and it checked no input budget. Every host now reads a window whose credit is used up exactly until it ends
  (the next frame that carries tasks is handed back), and a frame that breaks the credit or the 8 MiB budget of the
  window ends it: the rest of that frame is handed back and the receive is cancelled (§10, `stream-completion.md`).
  Weighed: leaving Rust as it was (two behaviours for one contract, against rule 8 of the constitution); reading a broken
  window to its end too (it would keep a window open for a server that has already broken its contract, for frames that
  the server withdraws itself when the stream ends, D006-001; what it already handed to the transport is G006-001, and
  only a server that broke the credit sends it).
- **D006-026 — At most eight hand-backs on their way at once.** The independent review found that a server answering one
  poll with tens of thousands of small leases (an answer may be 16 MiB) made every host start one report per lease at
  once: in Java one thread each, until the thread limit, an error that killed the poll thread. Every host now runs at
  most eight hand-backs at once and queues the rest in the order received; `stop` waits for the queue within its grace,
  and what is left when the grace ends lapses. Eight is the bound of the Java heartbeat pool and well over what a server
  that keeps to the credit can cause (at most 64 leases a window, and none beyond the credit). Weighed: no bound (the
  defect); dropping the excess beyond a bound (back to lapsing into a review); a bound tied to `max_inflight` (a host
  with one slot would hand back one lease at a time while stopping).
- **D006-027 — A lease being handed back is in flight.** A hand-back still retrying (`UNAVAILABLE`) carries the lease's
  fence; a later answer that leased the same key again (only a server that repeats a lease can) would have run a handler
  while a `TASK_NOT_STARTED` for that fence was on its way. The duplicate check now counts the keys of the hand-backs on
  their way as in flight, from the moment the lease is handed back until its report ends.
- **D006-028 — `TASK_NOT_STARTED` is reserved for the host** (a blocker of the server review). The server withdraws a
  lease on that code without consuming the attempt or opening a review, on the word that no handler ran; a handler that
  failed with it would make an attempt that may have had an effect look like one that never began, and escape feature
  037's review. So every host reports a handler's failure with that code (forward, compensation or reconciliation) as
  `WORKER_ERROR`, keeping its message and its `retryable`, the rule §6.5 already applies to a code that is not
  `[A-Z0-9_]{1,64}`. Weighed: refusing the code as a programming error (`retryable = false`: it would fail a step for good
  for a handler's mistake in naming); leaving it to the server (the server cannot tell the host's report from the
  handler's: both come on the same lease).

- **G006-020 — No conformance scenario proves the hand-back yet** (D006-024): to be added on the design branch once the
  server's withdrawal is merged into it.
- **G006-021 — The `poll` event does not count the same thing in every host.** Go and Python count the tasks taken;
  TypeScript and Java count the leases of the answer, and Java's stream path emits none. It was so before the
  hand-back and changes no lease; one meaning (the tasks taken) is to be chosen in a later delivery.
- **G006-022 — A report made after `stop` closed the connection** (a handler that ignored its token and returned late) is
  sent on a connection TypeScript opens again, where the other hosts refuse it; a hand-back is not one, since `stop`
  waits for them. Found in the review of this delivery, older than it.

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
  rewrite alone (older SDKs and raw clients still send the code); the minor is read never above this build's own
  ceiling (ADR-0099 decision 9: a session another build registered may store a minor this build does not serve). Weighed
  also: the cap alone (a few free re-runs per attempt for any
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
  5. The causes of protocol 1.8 count toward the same four (request (5) of D006-059, Task 006-073): a provisional lease
     withdrawn at its deadline (`NOT_ACKNOWLEDGED`) and a hand-back after the acknowledgement each end one delivery before
     any handler ran, as the four above do, so each takes one of them. A cap of its own for 1.8 would let a client that
     never acknowledges, or a host that always hands back, take four more free leases of the same attempt; an attempt
     whose deliveries keep ending is a property of the task, the host or the client, which must spend it.
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
  check in `scripts/check-architecture.py`, and the test `capability_lists_follow_the_code` of `crates/infra-postgres` (which
  runs on every pull request that changes that crate or the application crate), fail when the newest definition of any of the
  three lists differs from the tokens of `Capability::ALL`. Down migrations are held to the rule too (D006-083). The branch that
  merges second **restamps** its capability migration and its validation after the other's newest, never adds a third one
  beside an older stamp: sqlx applies a pending older version without complaint, after newer ones (round-3 architecture
  Minor 1).
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
  knows. ADR-0098 says so plainly. Its acceptance went to the owner, through the coordinator, because it
  overturns part of an accepted decision about the guarantees of the worker protocol. Accepted on 2026-11-21 (OD-2026-11-21-4), after the round-3 reviews closed with no Blocker and no Major.

- **G006-030 — No session honours `TASK_NOT_STARTED` before protocol 1.8 negotiates.** A host of 1.6 or 1.7 that hands a
  lease back gets an ordinary failure: the attempt is spent and retried with its backoff. That is a known outcome, never a
  review. Closed when the delivery acknowledgement build (ADR-0099) accepts 1.8.
- **G006-031 — Until 1.8, a host's hand-back spends the attempt.** The six SDKs hand leases back on every minor (D006-020
  to D006-023), and the server honours the code only from 1.8 (D006-030). So, today, a host that stops with leases it has
  not started turns each into an ordinary retryable failure: retried with its backoff while the budget allows, and, for a
  step with one attempt, a failure for good (its `onError` route or the run's failure) where before it lapsed into a review.
  It is a known outcome either way, never a review. Who chooses between the two for one-attempt steps (hand back only from
  1.8 and let older minors lapse, or keep the ordinary failure) is the coordinator, then the owner: it is a guarantee of the
  worker protocol. **Closed on 2026-11-21 by D006-080** (delegate, OD-2026-11-20-6): the hosts hand back only on a session
  of 1.8 or later; below it a dropped lease lapses as on `develop`.

### Decisions of the last round (2026-11-21, D006-080 to D006-091, G006-032; G006-031 closed)

What the handoff of pull request #260 left open, and the coordinator's answer to gap G006-031. Each decision is "delegate
(OD-2026-11-20-6)" unless it says otherwise. The numbers 040 to 079 are left to the design of delivery acknowledgement,
which uses 040 to 066 and may need more, so this list starts at 080. The acceptance of ADR-0098 is the owner's:
OD-2026-11-21-4 ("Accept, if the final review is clean"), applied when the round-3 reviews of
`reviews/2026-11-21/round-3-*.md` close with no Blocker and no Major.

- **D006-080 — The hosts hand back only on a session of protocol 1.8 or later** (closes G006-031). Below 1.8 a host drops
  the same leases it would hand back (`sdks/SPEC.md` §6.2, items 1 to 4), logs each one with its reason and that it lapses,
  and leaves it to lapse, as `develop` does. From 1.8, the minor whose server reserves the code (D006-030), it hands back as
  D006-020 to D006-023 say. The minor is the one the registration accepted, read for the session that received the lease.
  The reason: a server below 1.8 records `TASK_NOT_STARTED` as an ordinary failure that spends the attempt, so with a
  hand-back on every minor a stopping host would fail a one-attempt `Activity` for good where `develop` sends its lapsed
  lease to a review, which an operator can retry. That is a regression of a guarantee the delegation does not cover; this
  decision removes it and changes no guarantee: below 1.8 every host behaves as on `develop`, and from 1.8 the server
  withdraws the lease without spending the attempt. Weighed: keeping the hand-back on every minor (the ordinary failure is
  a known outcome, but it is final for one-attempt steps, a step back from `develop`); handing back only for steps with
  attempts left (the host does not know the budget); a new minor of its own for the hand-back (1.8 is the minor that
  reserves the code, so a second minor would mean the same thing). A handler's `TASK_NOT_STARTED` is still rewritten into
  `WORKER_ERROR` on every minor (D006-030 (a)): that is a property of the report, not of the hand-back. Each SDK has a
  test of both sides: a session below 1.8 sends no report for a lease it drops, a session of 1.8 sends the hand-back.
- **D006-081 — The six-language conformance scenario of the hand-back moves to the build of protocol 1.8** (AGENTS rule
  8; round-2 architecture Major 6, "NM4"). With D006-080 no SDK hands back before 1.8, and no server of this build
  negotiates 1.8, so a scenario run today could observe only that nothing is reported, which each SDK's unit tests already
  prove for its own host. The scenario that proves the hand-back across the six languages needs a server that negotiates
  1.8 and the harness's fault actions to hold a frame (`hold` and the rest of Task 006-058 on the delivery
  acknowledgement branch). So it is recorded as a task of that build, next to its conformance scenarios (Tasks 006-058 to
  006-064): a stopping host of each language hands back a leased, unstarted task, and the step is `READY` with its attempt
  unspent. Until then rule 8 holds for the server semantics this build changes: none of the six SDKs sends the code on a
  session the server serves.
- **D006-082 — An activation entry may require a live process of each kind** (round-2 architecture Major 5, "NM3").
  `capability-activate` gains the entry field `each_kind_live`: when it is set, the census reports which of the entry's
  kinds have at least one live process, and the command refuses, naming the kinds with none, unless every kind has one;
  under the epoch lock the count is made again and judged the same way. `--expect-live N` still counts the kinds
  together. Feature 022 requires at least one live capable process of each kind, so its entries set it; the entry of
  `lease-withdrawal-v1` does not (an empty fleet may activate it: `empty_fleet`). The census function
  `platform_capability_census` returns the kinds that are live (`out_kinds_live`) beside the counts, so one statement still
  answers both questions. Weighed: a census per kind (one statement per kind, and the lock count must repeat each); a
  rule in 022's own command (two commands again, which D006-034 removed).
  What feature 022 adopts, whichever branch merges second:
  - its two capabilities as entries of `ACTIVATABLE`, each with `kinds: &InstanceKind::ALL`, `empty_fleet: false`,
    `each_kind_live: true` and its own `after` sentence, and its tokens in `Capability::ALL` and the parity test;
  - `flow-control-activate` and its own census step removed, the operator using `control-plane capability-activate <token>
    --operator NAME [--expect-live N] [--dry-run]`;
  - the capability lists: if 022 merges after this branch, its capability migration and its validation are restamped after
    `20270208000200`, the up listing the union of both features' values and the down restoring this branch's list (the
    newest up before it); if it merges first, this branch restamps instead (D006-033). The two tests of
    `crates/infra-postgres/tests/capability_lists.rs` say which list is wrong.
- **D006-083 — Down migrations are held to the capability lists too** (round-2 architecture "NM2", correctness 2). The
  test `capability_lists_follow_the_code` of `crates/infra-postgres` and the check of `scripts/check-architecture.py` also
  require that each down migration that redefines one of the three checks restores exactly the list of the newest up
  migration before its own, so a rollback of one feature can never erase another feature's values.
- **D006-084 — The replay floor refuses a removal outside READ COMMITTED** (round-2 security minor 2, correctness minor
  2). The guard sees a withdrawal that committed while the removal waited because, under READ COMMITTED, the statement
  that checks the journal takes a new snapshot after the row lock is granted. Under REPEATABLE READ or SERIALIZABLE (a
  session or a role with `default_transaction_isolation`) it keeps the transaction's first snapshot and would let the
  removal through. So the guard refuses to remove `lease-withdrawal-v1` in any isolation level but READ COMMITTED, with a
  message that says to run it there. The withdrawal's side needs no such check: its `FOR SHARE` of a row deleted after its
  snapshot waits and finds no row under READ COMMITTED, and fails with a serialization error under the stricter levels;
  in both it writes nothing. Two tests cross the withdrawal's share lock and the removal in both orders, with
  `pg_stat_activity` as the barrier. Weighed: making the guard independent of the level (no statement can see a commit
  newer than a REPEATABLE READ snapshot); refusing only when an event exists (the guard cannot know it under that
  snapshot).
- **D006-085 — The removal of the requirement is bounded, and its check is an index probe** (round-2 security minor 3,
  correctness note). `platform_requirement_remove` holds the epoch lock while it waits for the withdrawals' share locks
  and while the guard looks for the event, and every registration waits for the epoch lock (it runs on the shared pool,
  whose statement timeout is 5 s; round-3 security Minor 2 corrected an earlier "2 s lock timeout", which is the
  operator transactions' bound).
  - A partial index `run_events_lease_withdrawn_idx` on `run_events (tenant_id) WHERE event_type = 'step.lease_withdrawn'`
    turns the guard's look into an index probe (the down migrations drop it first, so their offline check scans); it holds only the rare withdrawal rows, so a
    journal write pays for it only when it is a withdrawal. It is built in the migration, as the repository's other
    indexes on large tables are, with `IF NOT EXISTS` so an operator can build it `CONCURRENTLY` first
    (`docs/runbooks/lease-withdrawal.md`); an invalid index is refused.
  - The removal runs with `lock_timeout = 1s`, which bounds each of its lock waits, and the runbook runs it under
    `statement_timeout = 1500ms`, which bounds the whole call. The derivation: a registration that queues behind the
    removal waits at most for the whole removal; that must stay under the registration's 5 s statement timeout with a
    margin for its own work and for a slow database, so the whole removal is held to 1.5 s, and one lock wait to 1 s of
    it, leaving half a second for an index probe, one delete, one update, one insert and the commit. `lock_timeout`
    alone does not bound the call: a new `FOR SHARE` of the requirement row joins the share holders without queueing
    behind the waiting delete, so overlapping withdrawals could make the delete wait again and again (round-3 security
    Minor 2, correctness Minor 1); the statement timeout ends that. A removal that times out changes nothing and says so;
    the operator sets `SAGAT_LEASE_WITHDRAWAL=off` first, which stops the share locks, and runs it again. The bound holds
    for the removal of every requirement (the function is shared); a removal by hand of `schedules-v1` waits as little.
- **D006-086 — A handed-back token is dead for heartbeats too** (round-2 correctness minor 1, security minor 4). A
  withdrawal does not record the token as the one that reported the step, so a heartbeat with it gets `STALE_ATTEMPT`
  instead of an OK with `cancel_requested=false`. D006-031 step 1 ("its token is dead") then holds for every call.
- **D006-087 — Studio names `step.lease_withdrawn`** (round-1 architecture minor 6, AGENTS rule 10): "A lease no worker
  started was taken back; the attempt was not spent", in the normal tone. The attempt view keeps merging the leases of one
  attempt number, which is what a withdrawal means (the attempt is reused); it is not a second attempt.
- **D006-088 — Every guard of leases holds the withdrawal tracker until its leases are handed on** (found by the test of
  the unary claim on the shutdown tracker, the handoff of round 2). A unary claim's leases travel in the output of its
  task. When the handler is gone, tokio drops that output after the task has left the tracker, so a drain that waits at
  that moment found the tracker empty and ended before the withdrawal was spawned: on a graceful shutdown the pool could
  close and the lease lapse. Each `PendingLeases` now holds a token of the tracker; its `drop` hands the leases to their
  withdrawal, which holds the tracker itself, before the token goes. A drain therefore waits for every lease committed and
  not yet handed to the transport, wherever its guard is. The test `a_graceful_shutdown_waits_for_a_unary_claim_in_progress_and_its_withdrawal`
  failed on the old code with the step still `LEASED` after the drain.
- **D006-089 — From 1.8, a hand-back on a fleet that does not withdraw leaves the lease to lapse** (round-3 correctness
  Major 1). A session of 1.8 or later reserves the code, so its `TASK_NOT_STARTED` is the host's word that nothing ran.
  When the server will not withdraw for a reason of the fleet, not of the lease (the requirement not recorded, `Lapsed`;
  the engine's switch off, `Disabled`), it records nothing and answers `STALE_ATTEMPT`, which every SDK ends quietly
  (`sdks/SPEC.md` §6.2). The lease then lapses as on `develop`: an `Activity`'s goes to a review an operator can retry, and
  the attempt of work that never ran is not spent. Without this, D006-080's regression would come back on the day 1.8
  negotiates, on every fleet not yet activated. The ordinary failure stays for the other refusals: a step no longer
  `LEASED` to this lease (`FenceMissed`: a handler started, or the lease moved on), the cap of four (`Capped`, D006-031:
  a cause that repeats is a property of the task), and a session below 1.8 (`Lapsed` for the minor: the code is not
  reserved there, so it may be a handler's). Weighed: the ordinary failure everywhere (spends the attempt of work that
  never ran, the regression itself); withdrawing without the requirement (an older replica could meet the event: the
  reason the requirement exists); a new answer code (the SDKs already end a hand-back on `STALE_ATTEMPT`). Tested: a
  session of 1.8 hands back with no gate, with a gate that does not keep the requirement, and with the switch off; each
  time the host gets `STALE_ATTEMPT`, the step stays `LEASED` and nothing is written.
- **D006-090 — What ADR-0099 asks of this build, and when** (round-3 architecture Major 1; the requests are D006-059 of the
  delivery acknowledgement design, on its branch). This build is not yet the rollback floor of protocol 1.8, and no build
  of it negotiates 1.8. The status of each request:
  - (3) the holder's minor judged at the build's own ceiling: done (`hand_back_ceiling`, tested by
    `a_stored_minor_above_the_build_ceiling_is_judged_at_the_ceiling`).
  - (4) "Refined by ADR-0099" pointers on ADR-0098's decisions 5 and 8: when pull request #264 (ADR-0099, accepted on its
    branch by OD-2026-11-21-5) merges, because the ADR it points to is not on this branch.
  - (1), (2) the cause `NOT_ACKNOWLEDGED` in the closed set and its codec, (5), (6), (7) the refusal of capability calls
    by an engine whose registry is silent, (8) and (9): moved to the follow-up of pull request #260 that D006-059 itself
    names, which must merge before any build negotiates 1.8. They are ADR-0099's design (its decisions and their tests);
    building them here would implement another feature's design inside this one (AGENTS rule 2), and nothing in this build
    depends on them, because no session reaches 1.8.
  - The release consequence: a release that carries this build without that follow-up cannot be the floor a 1.8 build
    rolls back to. Recorded as G006-032.
- **D006-091 — The minors of round 3, fixed or recorded.** Fixed: the crossing test runs with the removal's lock bound
  raised on its own database, so it cannot fail on the clock; the index test reads the plan of the guard's look; the
  scoped half of D006-086 has a test (`bounded_worker.rs`); the under-lock judgement of `each_kind_live` is a named
  function with a unit test; the stale sentences of D006-004 and D006-033; the decision ranges; the 1.8 rule in
  `stream-completion.md`; the rollback text of ADR-0098 and of the down migration; the runbook's order of the index
  build; the generic command in `docs/runbooks/instance-registry.md` section 5, the release notes, the fitness index row,
  the 1.8 row of the register of minors (`sdks/SPEC.md` §9) and a compile-time check that this build's ceiling stays below
  the minor that reserves the code. Recorded and left, each with its reason in G006-033: the stamp-order fitness check, the Rust SDK's test-only
  ceiling, the four round-2 minors the architecture review lists again (the cap trusts its caller, the test seams in
  production ports, `with_withdrawal_concurrency`'s call order, no alert on `capped`), fairness across tenants and the
  hot requirement row, the vacuous release-0.2 assertion and the expired lease handed back.

- **G006-032 — This build is not the rollback floor of protocol 1.8.** ADR-0099's requests (1), (2) and (5) to (9) are a
  follow-up of pull request #260 (D006-090). Closed when that follow-up merges; it must merge before any build negotiates
  1.8. Built on 2026-11-24 as Tasks 006-068 to 006-074 (D006-094 to D006-099); what of request (1) is behaviour of 1.8 is
  G006-037, and the reader of `lease-withdrawal-v2` under its lock is G006-034. Closed with the merge of that pull request.
- **G006-033 — Minors left after round 3** (D006-091). None weakens a guarantee of this build: each is unreachable before
  1.8, an operability improvement, or a test seam.
  - No check refuses a migration stamped below one already applied; the restamp rule (D006-033) is prose until the
    migration runner refuses it.
  - The Rust SDK's newest understood minor is 8 under `cfg(test)` and 6 otherwise, so its unit tests do not exercise the
    shipped refusal of a newer minor.
  - The domain's cap of four trusts the count its caller reads; the delivery probe, the commit probe of D006-092 (which
    holds the run's locks while it waits, bounded by `idle_in_transaction_session_timeout`) and the test hooks are public
    types of production crates (behind no feature; `scripts/check-architecture.py` refuses their use outside the tests,
    D006-093); `with_withdrawal_concurrency` must be called before `withdrawals()`.
  - `tasks.md` has no rows for the withdrawal (D006-001 to D006-093): it was built from this register and ADR-0098, and
    its evidence is the pull request's CI (round 4 correctness Minor 3).
  - No alert on `worker_leases_undelivered_total{outcome="capped"|"error"}`; the withdrawal semaphore is per engine, not
    per tenant; every withdrawal takes `FOR SHARE` on one requirement row.
  - `release_upgrade_inflight.rs`'s "no withdrawal before the activation" cannot fail on a 0.2 journal; a lease handed back
    after its expiry has no test of its own (the fence that refuses it is tested through the reaper), and a 1.8 hand-back
    retried past the lease's expiry but before the reaper is an ordinary failure, not a lapse: unreachable before 1.8, to be
    settled with ADR-0099's expiry rule (its request (1) of D006-059).
  - The scoped half of D006-089 (`scoped_reports.rs`) is covered by the verdict's unit test and by reading, not by a test
    of a scoped hand-back on a fleet that does not withdraw.

### The claim that waits when its consumer goes, and the final review (2026-11-24, D006-092 and D006-093)

The finding. `protocol::cancelling_while_a_claim_waits_rolls_back_before_the_fence_opens` failed on every CI run of pull
request #260 since the server side of the withdrawal (commit 31c28347; runs 44850791606, 44853482693, 44864595812,
44917207464 and 44917210114): the step read `READY`, then the recovery stream received no task. The cause is in the code,
not in the test. D006-002 replaced the abort of the producer with a stop signal, and D006-035 asks whether the consumer is
gone only before each run's transaction. A claim that is inside a run's transaction, waiting on a lock (in the test the
drain lock that `worker_drained` takes), when its client goes away, therefore goes on once the lock opens and commits the
lease for a consumer that is gone. On `develop` the abort dropped that transaction before its commit, so it was rolled
back. On a fleet that has not recorded `lease-withdrawal-v1` (every fleet until the operator's activation; the test's
platform has no gate) the lease then lapses: the defect this pull request exists to remove, made more likely than on
`develop`. On a fleet that has recorded it, the lease is withdrawn, at the cost of two more transactions, a journal event and
one of the four withdrawals the attempt may have (D006-031), where a rollback costs nothing. The step read `READY` in the
test only because the claim had not committed yet when it was read: the second wait of the test ends as soon as the claim
holds the lock, not when it ends.

- **D006-092 — A run's transaction whose only writes are its leases is rolled back when its consumer is gone before it
  commits** (delegate, OD-2026-11-24-4). Just before the commit of a run's transaction that leased at least one step, the
  claim asks again whether its consumer is gone. If it is, and the transaction wrote nothing but its leases, it is dropped,
  which rolls it back: no lease is committed, no attempt is spent, nothing goes to the withdrawal, and the run is counted
  `worker_claim_skipped_total{reason="consumer_gone"}`. A transaction that also wrote something that is not a lease (the
  audit of a step refused outside its secret's worker pools or of a refused database claim, a permanent failure of an input
  that cannot be made) commits as before, and its leases go to the withdrawal: those writes belong to no consumer, an audit
  of a refusal must not disappear because the refused client cancelled (constitution, auditing), and the worker that was
  refused may never claim again to have it recorded anew (security and correctness reviews of round 4, Minor 1 of each). A
  transaction that leased nothing commits as before. The claim then stops, as D006-035 says. What is left is the consumer going away between this question and the end of the commit; those leases go
  to the withdrawal as before (D006-001). D006-002 still holds: nothing cuts a claim between a commit and the hand-over.
  Weighed:
  - Leave it to the withdrawal (the branch before this decision): a lapse on every fleet before its activation, a
    regression against `develop`; after it, two transactions and a slot of the cap for each cancelled claim.
  - Cut the database wait when the consumer goes (cancel the backend, or race the stop signal against the transaction and
    drop it): the lock wait ends sooner, but the store port would have to split a phase that may be dropped from a commit
    that may not, because a drop that reaches the commit is the loss D006-002 forbids. What it saves is bounded by the
    store's lock timeout. Not taken.
  - The question before the commit: one check, no new state, the same rule on the root path and on the scoped path, and
    the same result `develop` had for the case the abort covered. Taken, with the exception for kept writes: rolling those
    back too (what `develop`'s abort did) would lose an audit, and writing them again in a transaction of their own would add
    a store port for a case that the withdrawal already covers.

  The test barrier (D006-008): the claim awaits `ClaimWatch::before_commit` just before that question; a test sets it with
  `WorkerGrpc::with_commit_probe`, which is given the same `Abandonment` as the delivery probe, so the test releases the
  claim exactly when the server knows its consumer is gone. Production never sets it. The test is renamed
  `cancelling_while_a_claim_waits_on_a_lock_rolls_its_run_back`, waits for the claim to end (the tracker of D006-036)
  before it reads the step, and asserts that nothing went to the withdrawal. The other tests:
  `a_claim_whose_consumer_goes_before_its_commit_leases_nothing_and_withdraws_nothing` (a stream and a unary poll on a
  platform that withdraws: no `step.leased`, no `step.lease_withdrawn`, the next lease is the step's first),
  `a_scoped_claim_whose_consumer_goes_before_its_commit_is_rolled_back` (the scoped path, on the real store),
  `a_claim_whose_consumer_goes_before_its_commit_keeps_the_audit_of_a_refusal` (`secret_pools.rs`: one step refused and
  audited, one leased, in one transaction: it commits) and the unit test of `roll_back_for_a_gone_consumer` (the exception
  and the counter). Seen failing before the check: the first two with `handed_over() == 1` (the lease committed), the scoped
  one with the lease returned; the audit test with no audit row under the rule without the exception.

- **D006-093 — The final review of pull request #260 (round 4, 2026-11-24), settled** (delegate, OD-2026-11-24-4). The
  correctness and security reviewers read the branch after the merge of `develop` and D006-092: no Blocker, no Major.
  Fixed: Minor 1 of both (the mixed transaction, above); correctness Nit 1 (the barrier waits of the new contract test are
  bounded); Nit 2 (the new counter value has a test); Nit 3 (the documentation of `ClaimWatch` and `ConsumerWatch`, the
  decision ranges of ADR-0098). Settled in writing:
  - Correctness Minor 2 (no test that a transaction that leased nothing commits): with the exception for kept writes the
    condition `leased` only spares the barrier; the audit test fails if the exception is removed, which is the case the
    reviewer's mutation would break.
  - Correctness Minor 3 (no green run recorded; `tasks.md` has no rows for the withdrawal): the runs are in the pull
    request's CI; the withdrawal was built from `research.md` and the ADR, and its tasks are not rewritten after the fact.
    Recorded in G006-033.
  - Correctness, governance note (an accepted ADR edited in place): ADR-0098's decision 2 keeps its guarantee (nothing is
    cut between a commit and the hand-over; what was committed and not handed over is withdrawn). D006-092 adds that nothing
    is committed for a consumer known gone, which decision 1 already intends; it is a clarification dated in the decision,
    not a change of mind, so no new ADR.
  - Security Minor 2 (the probes are public and not behind a feature): no configuration, request or variable reaches them;
    a feature `test-support` would have to be threaded through every crate that builds the API for tests. Instead the
    architecture check refuses a call of `with_delivery_probe` or `with_commit_probe` outside `tests/` and the crate's own
    tests. Recorded in G006-033 with the probe's locks.
  - Security Nit 3 (no test pins the two newer definer functions): fixed, the privilege test lists them.

### The follow-up of pull request #260 (2026-11-24, D006-094 to D006-100, G006-034 to G006-038)

What ADR-0099 asks of the build of ADR-0098 and pull request #260 left out (D006-090, G006-032), built as Task 006-068 to
Task 006-073 on `feature/delivery-acknowledgement` and cut as a pull request of its own. Each decision is "delegate
(OD-2026-11-24-4)" unless it says otherwise.

- **D006-094 — The cause `NOT_ACKNOWLEDGED` is a value of the closed set, with the list of all of them** (Task 006-068). The
  domain gains `WithdrawalCause::NotAcknowledged` and `WithdrawalCause::ALL`, which the parse, the codec's tests and the
  domain's tests iterate, so a fourth cause cannot be added without them. Nothing writes it in this build; the transition
  that replays it is the one of the other causes (`LEASED` of that attempt and generation to `READY`, the attempt given back).
  The Studio names the event, not its causes, so it has nothing to change. Tested: `the_cause_has_three_closed_values` (seen
  failing with two values), the transition test over every cause, `a_withdrawn_lease_is_an_exact_codec_inverse` over every
  cause. Weighed: a separate event for 1.8 (an older replica would have to learn a second event for the same transition) and
  an open text field (a replica would replay a cause it does not know); the closed value is ADR-0099's choice.
- **D006-095 — The two functions of the replay floor are rewritten in place, and the requirement counts only for both kinds**
  (Task 006-070; requests (6) and (8) of D006-059). Migration `20270208000000_lease_withdrawal` may have been applied, so the
  new migration `20270211000000_lease_withdrawal_functions` rewrites both functions with `CREATE OR REPLACE FUNCTION`, which
  keeps their grants, and its down restores the bodies of `20270208000000` and `20270208000200` (nothing is stored, so
  nothing is refused). `lease_withdrawal_required()` answers yes only when the row of `lease-withdrawal-v1` lists both kinds
  (`kinds @> ARRAY['CONTROL_PLANE', 'EXECUTION_ENGINE']`), the question an engine's gate asks (`ServingGate::requires` over
  both kinds): a requirement recorded for the engines only, by hand SQL, leaves control planes that cannot replay the event
  free to register, so nothing is withdrawn under it; the lease lapses as before and a 1.8 hand-back is answered
  `STALE_ATTEMPT` (D006-089). The replay-floor guard says that the switch stops "new withdrawals of unmarked leases", which
  is all it does from 1.8 (ADR-0099 decision 10). The guard had run as the role that deletes the row; it now runs as its
  owner with `search_path = pg_catalog, pg_temp`, like every other function of the registry, so its look at `run_events`
  never depends on the deleting role's privileges or search path, and the privilege test lists it with the other eight.
  Tested: `a_requirement_recorded_for_one_kind_withdraws_nothing` (`contract_grpc_durable_execution`), the guard's text in
  `the_requirement_cannot_be_removed_once_a_journal_holds_a_withdrawal`, `the_requirement_counts_only_for_both_kinds_and_the_rollback_restores_the_old_bodies`
  (`integration_platform_foundation`), and the privilege test of `instances.rs`. Weighed: editing `20270208000000` in place
  (a database that applied it fails sqlx's checksum, and the old bodies stay where it ran); equality with the pair of kinds
  instead of containment (a requirement recorded for a kind added later would turn every withdrawal off); keeping the guard
  as an invoker's function (it would read the journal through the deleting role's search path, the one function of the
  registry that does).
- **D006-096 — `lease-withdrawal-v2`: a capability of its own, an entry that follows `lease-withdrawal-v1`, and a floor of
  one cause** (Task 006-071; request (9) of D006-059). `Capability::LeaseWithdrawalV2` (`lease-withdrawal-v2`) joins the
  closed set, and the migrations `20270211000100_lease_withdrawal_v2` and `20270211000200_lease_withdrawal_v2_validate`
  add it to the three checks (the floor below is `20270211000150_lease_withdrawal_v2_floor`, split out by D006-100)
  (the union with every value of `develop`; feature 022's are still on its own branch, and the
  branch that merges second takes the union, D006-033), `NOT VALID` then validated, its down restoring the list of
  `20270208000000` (D006-083). The processes list it from Task 006-072 on, in the same commit as the rule it promises (an
  engine whose registry is silent serves no capability call): a commit that listed it first would announce a rule the binary
  did not apply. The rules:
  - **The entry.** `capability-activate lease-withdrawal-v2` covers both kinds, may be activated with nothing live, as
    `lease-withdrawal-v1` may, and gains the entry field `follows`: the command refuses, saying so, while
    `lease-withdrawal-v1` is not recorded for both kinds, once before its census and again in its step under the epoch
    lock, where a removal of `lease-withdrawal-v1` (which takes the same lock) cannot cross it. The order is the command's,
    not a trigger's: ADR-0100 retired the trigger that would have ordered requirements, and an order broken by hand SQL is
    harmless here, because a binary that lists `lease-withdrawal-v2` lists `lease-withdrawal-v1` too, and every
    withdrawal still reads `lease-withdrawal-v1` under its lock (D006-032).
  - **The floor.** The replay-floor guard refuses to remove `lease-withdrawal-v2` while a journal holds
    `step.lease_withdrawn` with the cause `NOT_ACKNOWLEDGED`, the one cause a build without it cannot replay, and, as for
    `lease-withdrawal-v1` (D006-084), outside READ COMMITTED. So recording it stays reversible until the first withdrawal
    of 1.8. Its look is a probe of a new partial index, `run_events_lease_not_acknowledged_idx` on `run_events (tenant_id)
    WHERE event_type = 'step.lease_withdrawn' AND detail->>'cause' = 'NOT_ACKNOWLEDGED'`, built as
    `run_events_lease_withdrawn_idx` is (`IF NOT EXISTS`, an invalid one refused, the runbook's `CONCURRENTLY` first).
    The down refuses while the requirement or its history exists, while a registered process lists the capability, or while
    a journal holds a `NOT_ACKNOWLEDGED` withdrawal.
  - **The tests that said it was not activatable** (`the_command_activates_only_its_listed_capabilities`, the command-line
    test of `lease_withdrawal_activation.rs`) are inverted, and new ones show the activation refused without
    `lease-withdrawal-v1`, refused with it recorded for one kind, recorded with it for both, and the removal refused with a
    `NOT_ACKNOWLEDGED` row written by SQL fixture (nothing of this build writes one) and allowed with only a `NOT_SENT` one.
  Weighed: a trigger that refuses recording `lease-withdrawal-v2` before `lease-withdrawal-v1` (ADR-0100's option, retired:
  it orders nothing the reads under lock do not already make safe); reusing `run_events_lease_withdrawn_idx` and filtering
  the cause (the probe would read every withdrawal of every cause under the epoch lock, so the removal's 1.5 s bound would
  depend on how many withdrawals the platform ever made, D006-085); a guard that refuses the removal once any withdrawal
  exists (it would make the activation one-way from the first `NOT_SENT`, which a build without `lease-withdrawal-v2`
  replays).
- **D006-097 — A hand-back of a lease still `LEASED` passes the report fence whatever the cancellation says** (Task
  006-069; the part of request (1) of D006-059 that belongs to this build). A run whose cancellation was requested, or that
  is stopping, still has steps `LEASED` to hosts that never started them, and a host that stops hands those back. Without
  declared secrets the report reached the verdict of D006-030 already; with them, `checked_secret_report` (root) and
  `live_path_fence` (scoped) refused it `STALE_ATTEMPT`, because the fence let a stop through only for an acknowledged
  cancellation (`CANCELLED`, not retryable): the lease then lapsed into a review instead of being withdrawn and cancelled
  at once (D006-011). The fence now takes the closed `StopAllowance`: `None` (any other report), `Cancellation` (the
  acknowledgement, as before) and `HandBack` (a `TASK_NOT_STARTED`), which lets the stop through only while the step is
  still `LEASED` to this attempt and generation, judged again on the second fence after the scan. A hand-back of a step that
  started (a heartbeat made it `RUNNING`) keeps the fence it had, so a secret step whose cancellation was requested cannot
  record an ordinary failure past it. The verdict that follows is unchanged (the minor that reserves the code, the
  requirement, the cap), and the withdrawal settles the stopping run in the same command, so the step ends `CANCELLED` with
  its attempt unspent. What request (1) also asks, the expiry of an acknowledged lease and the lifted expiry of a lease whose
  mark counts, is behaviour of protocol 1.8 (G006-037). Tested, red first for the two cases with declared secrets (the
  hand-back answered `STALE_ATTEMPT`, the step still `LEASED`):
  `a_hand_back_of_a_run_whose_cancellation_was_requested_is_withdrawn` for a root and a scoped lease, each with and without
  declared secrets (`contract_grpc_durable_execution`). Weighed: allowing every stop for `TASK_NOT_STARTED` (a started
  secret step whose cancellation was requested could then fail past the fence with a code a handler may have raised before
  1.8); deciding the hand-back's verdict before the fence (the verdict needs the requirement's lock, which the scan's
  transaction must not hold across the provider's input and output).
- **D006-098 — An engine whose registry is silent serves no capability call** (Task 006-072; request (7) of D006-059; the
  amendment of ADR-0082 decision 5, recorded in its Amendments section and in section 3 of `docs/runbooks/instance-registry.md`).
  - **The beat.** The migration `20270211000300_instance_heartbeat_reporting_change` adds
    `instance_heartbeat_reporting_change(p_instance uuid, p_token bytea)`, a function of its own name (the two-argument
    `instance_heartbeat` keeps its contract for older binaries, and no call is ambiguous). It does what `instance_heartbeat`
    does, then takes the epoch row `FOR SHARE SKIP LOCKED` and returns that locked read's epoch with
    `out_change_in_progress = false`; only when an owner transaction holds the row does it read the committed epoch plainly
    and return `true`. It raises when the row is missing. It takes the epoch after the process's own row and the day rule,
    so the share lock is held only for the counts and the commit, and it never waits for it, so it adds no wait to the lock
    order of the registry. `EXECUTE` is revoked from `PUBLIC` and granted to the runtime role; it runs as its owner with the
    fixed search path. Both kinds call it (their membership is the shared `Member`). Under the share lock an activation
    waits for the beat, and its census, a statement after its lock under READ COMMITTED, sees the beat's `last_seen`, so it
    refuses an engine that lacks the capability.
  - **The decision.** The gate keeps the start of the last registration or beat that succeeded with no change in progress and
    whose decision under its epoch succeeded (the requirements read again when the epoch moved). Capability calls are served
    while the gate is open and that start is at most `CAPABILITY_WINDOW` = 25 s old (`LIVENESS` less 5 s). A beat that
    reports a change in progress is a successful beat for liveness, readiness and the closing rule, applies an epoch that
    moved (closing the gate when the process lacks what was recorded), keeps the current decision only while it is inside its
    window and never renews it, so a lapsed one waits for a beat with no change in progress; it is counted in
    `instance_heartbeats_change_in_progress_total{kind}`. A gate no registry decides (`ServingGate::opened`, a process or
    a test without a registry) is not bounded. The derivation: a census counts a process live when its `last_seen` is
    within 30 s, and `last_seen` is written after the beat's start, so a process the census cannot see stopped serving
    capability calls at least 5 s before; the 5 s cover the clocks of the process and of the database and the time from the
    second check to the answer.
  - **The refusal.** `ResolveSecrets`, `ResolveDatabase`, `ReadBlob` and `WriteBlob`, the lease-token calls other than
    `Heartbeat`, `CompleteStep` and `FailStep` (`AcknowledgeTasks` does not exist before 1.8), are refused `UNAVAILABLE` with
    the envelope reason `REGISTRY_SILENT` and `retry-after-ms` 1000, counted in
    `worker_capability_calls_refused_total{method, reason="REGISTRY_SILENT"}`. The gate is checked when the call arrives and
    again before the first byte of an answer that hands data out (after the secrets or the database credential are
    resolved and before the answer is built; at the first item of a blob download) and before the store's write of an
    upload starts. (A resolution refused there is rolled back with its audit: D006-100.) While the window has lapsed
    the heartbeat tries every second
    (`RETRY_WHILE_CLOSED`), as for a closed gate, so the calls come back within about a second of the registry answering.
  - **The connection.** The registry's pool carries the process's statement timeout, at most 5 s
    (`REGISTRY_STATEMENT_TIMEOUT`); a longer setting is lowered, not refused. With a beat every 12 s at most, each bounded to
    5 s, a healthy process renews its decision well inside the 25 s.
  - **The capability.** Every control plane and engine of this build lists `lease-withdrawal-v2` from this task on (D006-096).
  Tested, each red first: the gate's window at its edges and the beat that reports a change (unit tests of
  `crates/application/src/instances.rs`); `a_beat_held_behind_an_owner_transaction_reports_the_change_and_does_not_wait`,
  `an_activation_waits_for_a_beat_that_holds_the_epoch_and_sees_its_last_seen` and the privilege tests
  (`integration_schedules_bulk_operations`); `a_silent_engine_serves_no_capability_call_and_keeps_heartbeats_and_reports`
  (`contract_grpc_durable_execution`). Weighed: closing the whole gate at 25 s (stops reports and heartbeats too, so a
  short silence would fail leases whose work is done; the requests asked for capability calls only); measuring from the end
  of the beat (a beat that waited would count from a moment after the database last saw the process); a fallback that
  serves while a change is in progress (the open owner transaction is exactly the activation that may forbid the call);
  checking only on arrival (a call admitted just before the window ends could hand out a secret after it).
- **D006-099 — A hand-back from a session stored at 1.8 that this build cannot honour lapses** (Task 006-074; request (10) of
  D006-059, round 16 of the delivery acknowledgement design). A session's stored minor is read for the lease and clamped to
  this build's ceiling (ADR-0099 decision 9). Before this decision a session stored at 8 or more under a build whose ceiling
  is below 8 gave an ordinary failure, because the clamped minor does not reserve the code. That happens to a binary rolled
  back to this build while hosts registered at 1.8 hand back before they register again: each hand-back would spend its
  attempt, and fail a one-attempt `Activity` for good, for work the host says never ran. Now `hand_back_verdict` takes the
  stored minor before the clamp beside the clamped one: when the stored one reserves the code and the clamped one does not,
  the verdict is `Lapse` (nothing written, `STALE_ATTEMPT`, counted `lapsed`), as for a fleet that does not withdraw
  (D006-089); the clamped minor alone still decides `Withdraw`, so this build never withdraws a hand-back of 1.8. A session
  stored below 8 is judged as before (the ordinary failure: the code may be a handler's there). Tested, red first: the unit
  test of the pure verdict (stored 8 with ceiling 6 gives `Lapse`, stored 6 an ordinary failure) and
  `a_hand_back_from_a_session_stored_at_1_8_that_this_build_cannot_honour_lapses`, root and scoped, with the build's own
  ceiling (`contract_grpc_durable_execution`); `a_stored_minor_above_the_build_ceiling_is_judged_at_the_ceiling`, which
  asserted the ordinary failure, is replaced by it. Weighed: the ordinary failure (the regression of D006-080 after a
  rollback); withdrawing at the stored minor (this build would honour a reservation it does not implement, the opposite of
  ADR-0099 decision 9).
- **D006-100 — Review round 1 of pull request #283, settled** (delegate, OD-2026-11-24-4; every finding fixed; the reports and
  the commit of each fix are in `reviews/2026-11-25-lease-withdrawal-v2/`).
  - **The migration of `lease-withdrawal-v2` is split.** `20270211000100` held `ACCESS EXCLUSIVE` on three registry tables
    to its commit and then built an index over `run_events`, so every heartbeat of a large platform would have waited past
    its timeout. The index and the guard rewrite moved to `20270211000150_lease_withdrawal_v2_floor`, which touches no
    registry table; a crate test refuses any later up migration that does both. The downs hold `run_events` in SHARE mode
    while they look for a `NOT_ACKNOWLEDGED` withdrawal.
  - **The pause never outruns the window.** After any beat the heartbeat pauses at most until the last start that can
    still renew the decision (the 25 s window less the registry's 5 s statement timeout and a 1 s margin: 19 s after it),
    and at most a second once that has passed, so one failed beat never stops a healthy engine's capability calls (a paused
    runtime test: without the bound the second beat starts at 22.4 s or later). A member's clocks can be given.
  - **Two clocks.** The decision keeps the wall clock beside the monotonic one, and the window lapses on whichever says
    so first: the monotonic clock does not advance while a machine is suspended. A wall clock set back counts as no time.
  - **A process the full table did not take serves no capability call.** No census counts it, so neither its registration
    nor a beat that registers it again as `FULL` decides the window; its gate still opens for the rest.
  - **The refusal rolls back what it would have recorded.** Each capability use case (secrets, database credentials, blob
    reads) asks the gate inside its own transaction just before it commits the audit of the resolution, the pin of a
    database credential or the audit of a read, and refuses with the new `AppError::RegistrySilent` (`UNAVAILABLE`,
    `REGISTRY_SILENT`, a retry hint of a second); the counter of secrets handed out moves only after the commit. The
    transport keeps its checks, and a download asks the gate before every item, not only its first poll. This replaces the
    sentence of D006-098 that a resolution refused at the second check was audited.
  - **A statement timeout of 0** (no timeout to PostgreSQL) gives the registry's connection its 5 s.
  - **`HandBack` is granted only to a session whose minor reserves the code**, judged at the build's ceiling, at the root
    and in a scope (this narrows D006-097): below 1.8 the code may be a handler's, and a fleet below 1.8 keeps the outcome
    it had (`STALE_ATTEMPT` from the fence of a cancelled secret step). In this build it is dormant; the tests raise the
    ceiling.
  - **Tests added:** a replay of a journal holding a `NOT_ACKNOWLEDGED` withdrawal (SQL fixture); the plan of the floor's
    probe; the rollback of the follow-up compared, body digest, security and settings, with a database that only ran the
    migrations up to `20270208000200`; a capability call held behind its run's row while the window lapses, which hands out
    and records nothing (secrets, blob read, upload); `CompleteStep` and `FailStep` served while silent.
  - **Starvation, measured.** Eight processes beating in a loop with no pause, while ten activations and removals run
    under the operator's 2 s lock bound: every one recorded, and the beats went on meanwhile. A beat's share lock lasts only
    its counts and its commit, so no change was made; the test stays (`beats_without_pause_do_not_starve_an_activation`).
  The weighed options were the reviewers': the in-transaction check rather than a transport check alone (a refusal after
  the commit records a hand-out that never happened); a guard of its own migration rather than a lower lock in one
  (an `ALTER TABLE ... CHECK` takes `ACCESS EXCLUSIVE` whatever is done); a pause bounded on every beat rather than only
  after a failed one (the same bound, with no state to track which beats renewed).
- **G006-034 — The writer of `NOT_ACKNOWLEDGED` reads `lease-withdrawal-v2` under the lock its removal waits for in the
  build of 1.8.** This build writes no such withdrawal, so it has no reader of the requirement under a share lock; the guard
  of D006-096 is the half that is here. Closed by the task of the delivery acknowledgement build that writes the cause.
- **G006-035 — A host below 1.8 does not repeat a `REGISTRY_SILENT` refusal.** `sdks/SPEC.md` §5 asks the repetition of 1.8
  hosts; an older host turns the refusal into the handler's error, so an attempt that resolves a secret, a database or a
  blob in the window from 25 s of silence fails retryably where it was served before. The window adds 5 s to what ADR-0082
  decision 5 already closed at 30 s for a failure that is not a lost connection, and a lost connection leaves no database to
  resolve from. Closed when every host of the fleet is of 1.8 or later; no SDK change is part of this follow-up.
- **G006-036 — No alert on `worker_capability_calls_refused_total` or `instance_heartbeats_change_in_progress_total`.** The
  silence that causes them already raises `SagatInstanceRegistrySilent` after a minute; a change left open shows on the
  second counter only. To be added with the alerts of the delivery acknowledgement build.
- **G006-038 — The refusal of a database credential at the commit has no end-to-end test.** The check inside
  `resolve_database` is the one the secret resolution and the blob read use (`capability_window_holds`), tested through
  them; the contract suite has no managed database step to hold behind its run's row. Closed with a fixture of a managed
  database step in `contract_grpc_durable_execution` (D006-100).
- **G006-037 — The rest of request (1) of D006-059 is behaviour of protocol 1.8.** The expiry of an acknowledged lease and the
  lifted expiry of a lease whose mark counts read the acknowledgement marks, which this build does not write. Closed by the
  tasks of the delivery acknowledgement build that write them.

## Delivery acknowledgement (2026-11-20, Decision 006-040 to Decision 006-066, Gap 006-040 to Gap 006-048, Owner question 006-002)

Owner question 006-001 was answered "Yes, in 0.3.0" (Owner decision 2026-11-20-12), which left the variant, the protocol minor and
the deadline to this design. All decisions below are "delegate (Owner decision 2026-11-20-6)" unless they say otherwise. The
numbers start at 040 because the builds of Architecture decision 0098 record theirs on their branches: the server from Decision
006-011 to Decision 006-018 and from Decision 006-030 to Decision 006-039 with Gap 006-004 and Gap 006-030, the SDKs from Decision
006-020 to Decision 006-028 with Gap 006-020. The contract is `stream-completion.md`, "Delivery acknowledgement (protocol 1.8)"; the
host rules are `sdks/SPEC.md` §5, §6.1 to §6.5 and §6.8; the hard-to-reverse choice is Architecture decision 0099; the build plan is
`tasks.md`, phase "Delivery acknowledgement". The four reviewers ran their rounds on 2026-11-20 (`reviews/2026-11-21-delivery-ack/`,
the directory named by the coordinator); each decision names the findings it answers. This design follows Architecture decision
0098 as amended by its build review (its decisions 5, 7 and 8, Decision 006-030 to Decision 006-039 above), merged into this branch
on 2026-11-21 with its base; it is rebased onto `develop` after that branch merges and before it is accepted (round 4: architecture
Minor 11). The requests of Decision 006-059 are still to be made on that branch. "The receive bound" is the 6 s a host gives
one receive call.

### Decisions

- **D006-040 — The variant: a flag on the lease, marks bound to its generation, an acknowledgement that locks rows and never the
  run.** Variant 1 of Owner question 006-001 is kept for its exactness without its status and event; variant 2 costs, per task, a
  transaction that restores the run, applies a transition and appends `step.started`. The first round's acknowledgement took the
  run lock and renewed the lease; the renewal is what needed the run lock (round 2: design Major 1, security Major 2). The third
  round's single conditional update decided verdicts on an unlocked read, could wait on a busy row and held locks in a plan order
  (round 3: correctness Major 1 and Minors 1, 2 and 9; design Major 4; security Minor 3; architecture Minor 3); the acknowledgement
  now locks the matched rows `FOR NO KEY UPDATE OF steps SKIP LOCKED` by primary key, with the clock as a column of that statement
  (it never waits, so the clock is sampled when the locks are held), decides on what it locked, and answers
  `RETRY` for a row it could not lock, so it never waits for a row, cannot deadlock on rows and is not held up by one busy run.
- **D006-041 — Protocol minor 1.8, offered on build proofs alone and only after the gate.** The register (`sdks/SPEC.md` §9, Owner
  decision 2026-11-15-8) holds 1.1 to 1.7; no branch or worktree names 1.8 (searched 2026-11-20). 1.8 is reserved now at the
  coordinator's instruction, an exception to the register's rule ("when its contract merges") registered as Owner question
  006-002. The per-lease flag (round 2: architecture Blocker 1) puts the contract on the lease, so 1.8 depends on build proofs only.
  The server offers it from Task 006-057, after the gate, never before (round 3: architecture Major 4, design Major 3): until then
  the tests raise a session's stored minor, as Decision 006-030 of the server build does. Minors are cumulative (Architecture
  decision 0015, design 041), so 1.8 follows 1.7 in the server (Task 041-7) and in each SDK (Task 041-15 to Task 041-20). Field 23
  of `TaskLease` and field 10 of `WorkerCapabilities` are above every number drafts 019 and 031 name (round 3: architecture Minor 8).
- **D006-042 — The marks are lease data bound to the lease's generation and to the database server's incarnation.**
  `lease_hand_back_generation`, `lease_claimed_at`, `lease_claim_incarnation`, `lease_offered_generation`, `lease_acknowledged_at`,
  `lease_capability_generation`, `withdrawn_lease_token_hash` and `withdrawn_on_hand_back` (written with the hash, in one
  statement, from the withdrawal's cause; round 14: correctness Minor 2, security Minor 1, architecture Minor 2) on `steps` (the attempt flag `provisional_withdrawn_attempt` is
  attempt data, not a mark: Decision 006-066), written by the three lease statements (root, scoped in a
  full-run transaction, path), by the acknowledgement, by the capability paths and by the withdrawals. A mark counts while the step
  holds a lease and its generation is the step's, so a stale mark never counts (round 1: architecture Blocker 2, correctness Major 6,
  security Major 3), while `lease_claim_incarnation` is not null and equals the current server's start time (Decision 006-062), and,
  where the requirement is read, while the lease was claimed after its current recording (Decision 006-064; round 6: correctness Nit
  5). Any other value, null included, never counts (round 5: correctness Minor 6, design Minor 1, architecture Nit 4). No statement
  assigns a mark from a value it read earlier; the allowed forms are written out, the claim's conditioned in SQL on a new lease, and
  a static check with negative tests and a barrier test with a run transaction loaded before an acknowledgement enforce them (round
  3: architecture Major 6; round 4: architecture Minor 6). The session upsert sets `declines_acknowledgement` from the registration
  (round 4: correctness Nit 6). The pure function receives "the mark is current" as a boolean, and the gauges, the drain and the
  `down` use the same macro; each gauge is defined (round 5: architecture Nit 5; round 6: design Major 2).
- **D006-043 — The acknowledgement: one transaction per call, one namespace per call, no run lock, bounded from its arrival.** A
  deadline of half the receive bound from the call's arrival, pool acquisition included, checked before each statement, with the
  timeouts set once to 1 s, so a call ends at most 1 s past it (4 s), counted in neither the statements nor the round trips (round 7:
  correctness Minor 6); at the deadline the transaction is rolled back and the connection returned rather than discarded (round 6:
  correctness Minor 7, architecture Minor 6, design Nit 3); the answer is `UNAVAILABLE` with the reason `ACKNOWLEDGEMENT_DEADLINE`, never
  `DEADLINE_EXCEEDED`, which the host keeps for its own bound (round 4: design Minor 2; round 5: correctness Minor 2, security Minor
  2, design Minor 3; the pinned PostgreSQL 16 has no `transaction_timeout`); a commit whose synchronous-standby wait is cut short has
  committed locally, which the incarnation covers (round 6: correctness Nit 7, architecture Nit 5). The permission once; one settings
  statement for the timeouts and for `synchronous_commit`, raised to `local` when it is `off` (round 5: architecture Major 1); an
  unlocked read by primary key for namespace and activity (a caller without the tokens costs one indexed read); the checks of
  `ensure_bound` under its locks, before any row lock, with coverage per row (round 3: security Minors 1 and 2), through one function
  shared with the run transactions (round 4: architecture Minor 7); the locking read by primary key alone, `FOR NO KEY UPDATE OF steps
  SKIP LOCKED`, which the foreign-key checks on `steps` do not conflict with (round 4: design Nit 2, correctness Nit 5), with the
  clock and the server's start time as its columns (round 4: correctness Nit 1) and the join with `runs` on the tenant and the run
  (round 4: security Nit 2); the verdicts; one update. One namespace, because a frame comes from one session (round 3: security Minor
  1). At most one cross-tenant record, spawned off the answer path through a bounded gate (round 4: security Nit 1), and one binding
  denial per call through the paths that exist (round 2: security Minor 1). Transient errors are `UNAVAILABLE`, never `NOT_HELD`.
  About eight statements per call, whatever its size (round 3: architecture Minor 2), through a port of its own,
  `AcknowledgementTx` (round 3: architecture Minor 1); its application request type has no `Debug` and its span records no fields
  (round 5: security Nit 4). Its admission limit keeps the requests in flight within the limit and half the reserve for the class
  `Lease` at every limit, tested as properties (round 4: security Minor 3); a shed answers `ACKNOWLEDGEMENT_SHED`. After any failure
  that carries the server's envelope the host splits an attempt of more than eight leases in two (round 6: correctness Minor 6; round
  7: correctness Minor 3).
- **D006-044 — Exactly one of the acknowledgement and a withdrawal commits, by the step row's lock.** Every withdrawal of a marked
  lease re-reads the row `FOR UPDATE` under the run lock after its binding checks (round 3: design Minor 5) and decides with the pure
  function, which extends `LeaseDeadlines` and has a parity test against the SQL macro in `integration_durable_execution` (round 3:
  architecture Minor 4). The candidate query of the pass is a hint.
- **D006-045 — A lease that has run is an attempt; one not yet `RUNNING`, handed back by its host, is not.** The pass never
  withdraws an acknowledged lease, and a `TASK_NOT_STARTED` for a `RUNNING` lease, or for one to which a capability was served, is an
  ordinary retryable failure (round 1: security Major 1). A lease whose mark counts, still `LEASED` and with no capability served, is
  withdrawn on its hand-back whether provisional or acknowledged; one whose mark does not count is an ordinary failure, since its
  capability record may have been lost (round 9: correctness Minor 1, security Minor 2). `LEASED` is not proof that no handler
  started: a task shorter than its first heartbeat completes from `LEASED`, so the window from `ACKNOWLEDGED` to the first heartbeat
  rests on the host's promise alone, and the withdrawn token's hash is kept for this withdrawal too, so that a later call with it is
  counted and alerted (round 9: security Minor 1, design Minor 2, correctness Minor 4). Otherwise an acknowledgement whose answer was
  lost, or one answered too late to start, would charge an attempt for work that never ran, while an unmarked 1.8 lease's identical
  hand-back is withdrawn (round 8: correctness Major 3, design Minor 2). This revises the first round's choice; it is the owner's to
  confirm with the trust assumption, and a first heartbeat before every flagged handler, which would make `LEASED` mean "not
  dispatched" at one transaction per short task, is the alternative left to the owner. Capabilities are served only after the
  acknowledgement.
- **D006-046 — The deadline is the lease's own earlier deadline, with one receive bound of margin; a lease is marked only when that
  leaves three receive bounds (18 s).** The read within the receive bound (specified), one attempt within it (the host's bound), and a
  margin of it, with a reason for each deadline (round 3: design Minor 1): before the expiry, because the acknowledgement renews
  nothing and the first heartbeat must commit, which the host sends at the earlier of its interval and a third of the
  `lease_time_left_ms` the answer reports less its attempt's duration, before the handler starts under 3 s, whatever interval it is configured with
  (round 3: correctness Major 2; round 4: correctness Minor 1); before the attempt deadline, because a heartbeat does not move it and
  a handler that started with less of its timeout would most likely end as an unknown outcome. The threshold is applied at the claim
  (round 2: design Major 6). The gate checks the margin's assumption: with `T − a` at least 3 s at receipt, the beat leaves within a
  third of it and has at least two thirds, 2 s, to commit, which is the gate's heartbeat bound; the 3 s threshold below is what makes
  them fit (round 5: correctness Nit 1; round 10: correctness Nit 1). Because the commit and the answer's transit are not bounded, the answer reports the
  time left before both deadlines: under 3 s before the expiry the host sends the first heartbeat before the handler starts (since the
  call can end 4 s after its arrival, (6 − 4) × 2 / 3 would be under the 2 s bound; round 8: correctness Minor 2), and
  under 2 s before the attempt deadline, which a heartbeat cannot move, it hands the lease back (round 7: design Minor 3).
- **D006-047 — The gate, the requirements and the switch.** `delivery-acknowledgement-v1` is listed by engines from Task 006-057,
  after the gate, and means that the engine serves `AcknowledgeTasks`, replays the cause, refuses capabilities to provisional leases
  and withdraws a marked provisional lease at its deadline and on its hand-back (round 4: architecture Major 1). It is recorded for
  engines only: a control plane serves no worker call and runs no pass, and the floor build's control planes will decode the cause
  once request (2) of Decision 006-059 is built, which a released build must have before Task 006-057 (round 5: architecture Minor 2;
  round 6: architecture Nit 7; round 10: architecture Minor 1); but the floor's capability, `lease-withdrawal-v2` (request (9) of
  Decision 006-059), is read as recorded only for both kinds, because control planes replay the event too (round 7: security Minor
  5, architecture Minor 3; round 11: correctness Major 2, security Major 1, architecture Major 1, design Major 1). Its entry of the generic
  `control-plane capability-activate` (Decision 006-034 of the server build) is added in the same task with `empty_fleet: false`
  (round 4: architecture Minor 5) and a field `requires` of `Activatable`, refusing while `lease-withdrawal-v2` is not recorded for
  both kinds, a convenience (round 7: architecture Nit 6), and while any registered instance, live or not, lacks it, which is not
  (round 11: security Major 1); both refusals are a trigger of this design's registry migration (Task 006-046), which also refuses removing
  `lease-withdrawal-v2` while `delivery-acknowledgement-v1` is recorded (round 13: design Minor 1: otherwise the sequence of a removal, a registration of a build of #260 without the follow-up, its
  registry falling silent, and a new recording would admit a binary without the liveness rule that the census at insert never sees),
  on `platform_requirements`, enabled
  `ALWAYS`, so hand SQL cannot skip them (round 12: security Minor 2, architecture Minor 1). That trigger is not the ordering
  trigger of the fifth round, which bought nothing for the order; it exists for the second refusal, which is load-bearing. The guarantee is elsewhere: an engine marks only while it knows both are
  recorded so, and every withdrawal of a marked lease reads both under the locks their removals wait for, through a witness type
  that the decision requires, so no path can decide without the read (round 7: architecture Minor 4); the wrong order writes no event
  and costs only marks that are never honoured, counted `requirement_missing` and alerted (round 6: design Minor 5; round 7:
  architecture Minor 2). The fifth round's trigger, made a decision of its own in the sixth, is withdrawn; its number, Architecture
  decision 0100, is retired with a stub (round 7: architecture Nit 1). The engine switch `SAGAT_LEASE_WITHDRAWAL=off` stops new marks
  and the withdrawals of unmarked leases and does not reach a mark already written (round 4: architecture Major 1, correctness Minor
  2). This refines Architecture decision 0098 decision 8; a "Refined by" pointer goes into it at acceptance (round 4: architecture Nit
  3).
- **D006-048 — The host at 1.8.** It acknowledges flagged leases before it calls or runs anything for them but the hand-back (round 2:
  correctness Major 3, security Major 1, architecture Major 2, design Blocker 1), and never dispatches one without `ACKNOWLEDGED`,
  whatever it declined (round 4: correctness Minor 6, security Minor 5, design Minor 1); it bounds a `PollTasks` call and each
  attempt to the receive bound; it asks again for the leases answered `RETRY`, together, as part of the same call, on a short backoff
  of their own that is not a failed call (round 4: design Minor 3, correctness Nit 3); it gives a lease up on its monotonic clock,
  clamped, and never while an attempt is in flight (round 3: correctness Minor 3); it applies the two 2 s rules of Decision 006-046.
  It declines acknowledgement, at its own minor and for a bounded, alerted period (round 3: architecture Major 5, design Major 1;
  round 4: correctness Major 3, security Minor 4, design Major 2), only on evidence of its own path (Decision 006-065). A stopping host
  waits, within its `stop(timeout)`, at most the receive bound for an acknowledgement in flight (round 4: correctness Nit 4). Its
  normative text in `sdks/SPEC.md` §6.2 lists the verdicts first and starts the fallback with its own sentence (round 7: correctness
  Minor 7, design Minor 2).
- **D006-049 — `timeoutMs` keeps its meaning, and a provisional lease is withdrawn at its earlier deadline** (round 2: correctness
  Major 4; round 1: architecture Major 4). The next lease is a new claim with its own deadlines. Under repeated losses one attempt
  number can take up to four of its earlier deadlines in wall time, which the workflow-document reference says (round 3: correctness
  Nit 4). The host learns the attempt deadline of a flagged lease from the lease itself: `TaskLease.attempt_deadline_epoch_ms` (field
  24, 1.8) is set only on a flagged lease whose attempt deadline comes before its expiry, and the host's give-up counts from the
  earlier of the two; the stored expiry is not shortened, which would change what an acknowledged lease's lapse means (round 11:
  correctness Minor 4).
- **D006-050 — A provisional lease receives no capability, and a served one is recorded before it is served**, through fence variants
  only the capability paths call, after the token and binding matched (round 1: security Minor 2); the descriptor-driven test walks
  nested, `oneof` and repeated fields, fails when a lease-token method has no case, and fails when a message with a field ending in
  `_token` is not in `skip_debug` (round 3: security Nit 4; round 4: security Nit 3). The record is one conditional update of
  `lease_capability_generation`, once per generation, under the run or path lock the path holds, committed before anything is handed
  over or accepted, and the call is refused when the lease is no longer held: secrets in their final transaction before the values
  are returned, `ReadBlob` in its audit transaction before a byte is sent, root `WriteBlob` in its check transaction, which now
  commits (today it commits nothing), scoped `WriteBlob`, `ResolveDatabase` and `AppendStepLogs` in the transaction each already
  commits before it answers (round 3: security Major 2; round 4: correctness Minor 4, security Minor 1). A barrier test orders a
  capability call against a hand-back in both orders.
- **D006-051 — Run nothing, hand back: `CANCELLED` needs no write, and it is for `FORWARD` tasks only.** `CANCELLED` when the step is
  asked to stop or the run has a stop reason (round 3: security Nit 2), but only for a `FORWARD` task: a compensation or
  reconciliation, which the claim leases on purpose while a run is stopping, is acknowledged whatever the run's stop reason, as its
  heartbeat is never told to stop (`sdks/SPEC.md` §6.4; round 8: design Major 2). The step's `cancel_requested` is read under the row
  lock (round 3: correctness Major 1, architecture Minor 3), the run's stop reason with it but unlocked, so a stop committed after
  that read is ordered after the acknowledgement and reaches the host through its first heartbeat (round 4: design Nit 3, security
  Nit 2); the host hands back and the withdrawal's settle cancels. `CANCELLED` is also the answer to a repeated acknowledgement
  (`duplicate`) or a lease whose mark does not count (`recovered`) with such a stop or less than the margin left (round 5:
  correctness Minor 4, design Minor 4); its hand-back is withdrawn while the lease is `LEASED` and its mark counts, and is otherwise
  an ordinary failure (Decision 006-045), through the report fence of a step with declared secrets too (Decision 006-059 (1); round 6:
  correctness Minor 1). A `FORWARD` step of a saga whose cancel policy would let work in flight finish is cancelled too: nothing ran,
  so nothing is lost (round 9: correctness Nit 9); a domain property test checks that a stop is never planned for a compensation or
  reconciliation (round 9: security Nit 3).
- **D006-052 — One bound on repeated withdrawals, in one place: the cap of four per attempt, applied by the withdrawal command.** The
  second and third rounds kept a run of three withdrawals on the step (round 3: architecture Major 2, design Major 2, correctness
  Minor 4, security Minor 6); the fourth had the claim check the cap from "the run state", which does not hold the count: the
  withdrawal command takes it from the journal (`lease_withdrawals`, Decision 006-031 of the server build), so the check would have
  needed a journal read in every claim and would have repeated the command's (round 4: correctness Major 1, architecture Major 3,
  design Major 3). The claim now marks without the count, the `LEASE` line keeps its statements, and the withdrawal command applies
  the cap to the causes of 1.8 as to every other, at the cost of one count statement in the pass's withdrawal (round 5: architecture
  Nit 7). The lost-lease pass is the only pass that ends expired leases, so in it every outcome other than a withdrawal (the cap, a
  requirement no longer recorded) ends the attempt with `expire_attempt` in the same transaction, as today, and never leaves an
  expired lease for a later pass, which would pick it first forever (round 5: correctness Major 1). At the cap the pass ends a
  provisional lease as an unknown outcome while a hand-back is an ordinary failure (Architecture decision 0098 decision 7): the pass
  has only silence, four times over for one attempt, which is evidence that the path is not to be trusted, while the hand-back is the
  host's word; both orders are tested (round 5: correctness Minor 5). A provisional lease's hand-back is withdrawn after a deadline
  too, until the pass has withdrawn it, because nothing ran and the host's give-up regularly lands there when no attempt is answered
  (round 4: correctness Major 2). Whether the server build counts the causes of 1.8 in its derivation of the cap is its answer to
  request (5) of Decision 006-059.
- **D006-053 — The trust assumption is the owner's, and the server records what it sees** (round 1: security Major 5). Counted and
  audited once per step and generation: a `Heartbeat`, `CompleteStep` or a `FailStep` other than the hand-back on a provisional lease;
  a capability call refused `TASK_NOT_ACKNOWLEDGED`; a hand-back after a capability was served (round 3: security Major 2); a report,
  heartbeat or capability call with the token of a marked lease withdrawn as `NOT_ACKNOWLEDGED` or on its hand-back (round 9:
  security Minor 1); a heartbeat after a hand-back's withdrawal (`NOT_STARTED`) is counted under its own kind, `heartbeat_after_hand_back`,
  audited and not alerted, since a heartbeat sent before the handler can be overtaken by its own hand-back (round 10: security Minor
  3, correctness Minor 6); a heartbeat after any other withdrawal, `NOT_ACKNOWLEDGED` above all, where no host that keeps the
  protocol heartbeats, is counted under `heartbeat` and alerted, the earliest sign of a handler run without an acknowledgement
  (round 11: correctness Minor 5, security Minor 1); the stale path takes the cause from `withdrawn_on_hand_back`, written with the
  withdrawn token's hash in one statement, since the journal has no index that would bound a read of it and a holder of a withdrawn
  token could repeat that read at will (round 12: architecture Minor 4, security Nit 2; round 13: security Minor 2, design Nit 4). "Once" is the identical-record rule of the
  audit trail (`crates/infra-postgres/src/identity.rs`, an insert guarded by `NOT EXISTS` on tenant, actor, action, resource and
  reason): the resource is the step (run and step), and the reason is built only from stored values, the row's step id and its stored
  generation, never from the request, so a holder of a withdrawn token cannot vary it; the new actions are added to
  `collapses_repeats` with no repeat window and a distinct cap of 100 (round 5: security Minor 3); the refusal path commits its audit
  as the secret denial does (round 4: security Minor 2). Hand-backs and acknowledgements with such a token are counted apart, as
  evidence that nothing ran (round 2: security Major 1, design Blocker 1; round 3: security Minor 4).
- **D006-054 — Scoped instances follow the same rules** (round 1: correctness Major 4, security Major 2, architecture Major 1, design
  Major 6).
- **D006-055 — Rollback: switch marking off and confirm it, drain the provisional leases, remove the requirement, roll back, switch
  back on; the floor is enforced by the registry** (round 3: architecture Blocker 1). The switch is read at start, so the engines
  restart, and the evidence that it holds is named: every live engine started since, the `switch_off` skips rising, the provisional
  gauge falling (round 6: design Minor 3). The drain waits for `worker_leases_provisional` alone, because only provisional leases
  change meaning when the requirement goes and nothing renews them; waiting for every mark would never end while 1.8 workers work,
  and would wait for the longest acknowledged task (round 6: design Major 2, correctness Minor 2, architecture Minor 4). Removing the
  requirement first would end the protection of the provisional leases in flight, because engines of the older build may then join
  (Decision 006-064). The switch is set back afterwards, so the floor keeps Architecture decision 0098's own withdrawals. The build
  of Architecture decision 0098 must decode `NOT_ACKNOWLEDGED` in a released build before Task 006-057 (precondition (a) of the task
  phase; round 10: architecture Minor 1), every writer of
  the cause reads `lease-withdrawal-v2`, the floor's own capability (request (9) of Decision 006-059), under the lock its removal
  waits for (round 4: architecture Major 2), and that requirement cannot be removed once a journal holds the event with that cause, as
  Architecture decision 0098 decision 8 has it for `lease-withdrawal-v1` and any withdrawal, so no binary below the floor is admitted once any `NOT_ACKNOWLEDGED` withdrawal exists,
  whatever was released before it: a release of pull request #260 without the follow-up lists only `lease-withdrawal-v1` (round 11:
  correctness Major 2, security Major 1, architecture Major 1, design Major 1). On the floor a 1.8 session's hand-back is an ordinary failure, because that build clamps the holder's minor to
  its own ceiling (Decision 006-059 (3); round 4: security Major 1, correctness Minor 3, architecture Minor 10). The `down` migration
  refuses while a provisional mark counts, while the history names the capability, or while a live process lists it.
- **D006-056 — What is counted and alerted** is listed in `stream-completion.md`, "Observability": ratios over
  `worker_lease_marks_total`, the count of marked claims (round 10: architecture Minor 5), and one closed `mark` vocabulary for both
  withdrawal and expiry counters (round 10: security Nit 5) (round 3:
  design Minor 7), `NOT_HELD` with a reason (round 3: design Minor 4), the delay alert from the measured baseline (round 3: design Nit 2).
- **D006-057 — The cost and its gate, before the six SDKs and before 1.8 is offered** (round 2: architecture Major 5, design Major 7;
  round 3: architecture Major 4). By count of the statements of Decision 006-043, a task gains about eight statements and one
  transaction under light load (104 against 94 round trips with `BEGIN` and `COMMIT`), plus the triggers the update fires; the
  round-trip model of feature 038 predicts about 10% fewer transitions a second. Task 006-056 measures it with a network between the
  workers and the services, the median of five interleaved runs of each minor, with a fan-out run and a burst. The build goes on only
  if 1.8 is no more than 15% below 1.7 in the transitions runs, the timed 64-lease calls are answered within half the receive bound
  at the 99th percentile over every call with fewer than 0.1% answered other than `OK`, which would otherwise make the first criterion
  true by construction (round 5: correctness Minor 1; round 6: correctness Minor 3), and, in the burst run, which sheds the class by
  design, no heartbeat is shed while acknowledgements are admitted (round 7: design Nit 3), and the heartbeat's 99th percentile in the burst is under a third of the receive bound.
  The rise of ready to handler entry and the share of claims unmarked as `short_deadline` are reported, not gated (round 3: design
  Minors 2 and 7). 15% is the prediction with a margin about as large again for the variation between runs on a loaded machine, which
  the 2026-11-12 run of feature 038 warns about (its load average was 26): a choice recorded here, not a measurement. If the gate fails,
  the minor stays at 7 and the capability unlisted, Gap 006-030 of the server build stays open, and the owner chooses between shipping
  1.8 as "the code reserved, the flag never set" and leaving that gap open. The first fixes the meaning of 1.8 for good (constitution
  principle IV): no later server could flag a 1.8 session, delivery acknowledgement would take a later minor, and the SDKs would carry
  only the negotiation and the reserved code at 1.8, which the SDK build already rewrites (round 4: architecture Minor 9, design Minor
  4). The messages of Task 006-043 then stay in the contract with the comment "reserved, not offered" (round 4: architecture Major
  4). Release 0.3.0 ships the requirement unrecorded.
- **D006-058 — The conformance evidence.** The 26 scenarios that pin no minor start no worker and are no evidence for 1.8. The harness
  override (`SAGAT_SDK_PROTOCOL_OVERRIDE=1.8`) raises every host scenario pinned at 1.4 or 1.5 to 1.8, with step logs off, and compares
  differently only the negotiated `protocolVersion` (against the pinned one) and, for 1.4, the lease's instance identity. Under it the
  harness asserts that each raised scenario acknowledged at least one flagged lease, except the three with a 3 s time-to-live
  (`protocol-negotiation`, `worker-stream-faults`, `worker-transport-faults`), and that no `TASK_NOT_STARTED` was sent where the
  scenario's golden does not expect one; the scenarios that hand back by design are found by a run without the override that
  records which send `TASK_NOT_STARTED` (`worker-shutdown` is expected among them), listed by name in the harness, and run without the
  override (round 3: architecture Minor 5, correctness Minor 6; round 4: correctness Minor 5). The new scenario adds the cases the 1.4
  scenarios carry, a refused call (the host declines and keeps its minor, then registers again without declining after its period,
  shortened by the harness), a 30 s `UNAVAILABLE` outage of the call that does not fall back, a flagged frame received after the
  declining registration (handed back), a skewed and a slow clock, a long configured heartbeat interval, three calls whose every
  attempt goes unanswered while the stream is answered (the fallback), two minutes of a 503 without the server's envelope for this
  call alone (the fallback at the third counted call, no attempt spent), also with `max_inflight` equal to the frame size so that
  only the hand-backs are answered, and at an 18 s time-to-live, two minutes of answers with the envelope (no fallback, no attempt
  spent, each lease handed back once), a handler that raises `TASK_NOT_STARTED`, a hand-back held
  across a re-registration, a hand-back that lands after the expiry, and a collection item. A busy row (`RETRY`) needs a database
  session holding the step row, which the harness has no action for, so it is tested at the server (Task 006-049 and Task 006-051)
  and in each SDK's unit tests. An informative re-run of `sdk-streaming` at 1.8 is recorded (round 3: architecture Minor 14).
- **D006-059 — What this design asks of the build of Architecture decision 0098** (amendments of a proposed decision, made on its
  branch; round 3: architecture Minor 10; round 4: architecture Minor 13; round 5: architecture Minor 3). Against its merged head
  (checked 2026-11-21) only (3) is present; the others go into a follow-up of pull request #260 (round 9: architecture Major 1, design Major 1, security Minor 4).
  Whether a build of #260 is released before that follow-up is a release choice, not a guarantee this design rests on: request (9)
  makes the floor the registry's (round 11: correctness Major 2, security Major 1, architecture Major 1, design Major 1). (1) A `TASK_NOT_STARTED` on a `LEASED` lease is accepted
  whatever `cancel_requested` and the stop reason say, through the report fence of a step with declared secrets as well (not yet
  there: `checked_secret_report` allows a stop only for `CANCELLED`); the expiry for an acknowledged lease is this build's code,
  since the floor knows no acknowledgement (round 6: correctness Minor 1; round 7: correctness Nit 9); this build also lifts the
  expiry for a lease whose mark counts (round 5: architecture Nit 1). (2) The cause `NOT_ACKNOWLEDGED` joins the closed set of
  `WithdrawalCause` and its codec, so that build replays it: missing, and a hard precondition, because it makes that build the
  rollback floor (Decision 006-055). (3) It decides the holder's minor as the smaller of the stored minor and its own compiled
  `PROTOCOL_MINOR`: present (`hand_back_ceiling`). (4) Its decisions 5 and 8 get only a "Refined by Architecture decision 0099"
  pointer at acceptance (round 5: architecture Minor 3). (5) Its derivation of the cap of four says whether the causes of 1.8
  count: missing. (6) The error text of its replay-floor guard says "of unmarked leases" (round 5: architecture Nit 2), and (8)
  `lease_withdrawal_required()` returns the requirement as recorded only for both kinds (round 7: architecture Minor 3): both
  missing; if migration `20270208000000` has been applied anywhere, they come as a new migration with `CREATE OR REPLACE FUNCTION`,
  keeping `SECURITY DEFINER` and `search_path = pg_catalog, pg_temp`, with a privilege test of `prosecdef` and `proconfig` (round 9:
  security Minor 4). (7) An engine whose registry has been silent for the liveness window less 5 s (25 s), measured from the start
  of its last successful beat, serves no capability call (every lease-token call but `Heartbeat`, `CompleteStep`, `FailStep` and
  `AcknowledgeTasks`) until a beat and the gate's decision under the epoch it returned have both succeeded, the gate checked again
  before the first byte of the answer is sent; the registry's connection carries a statement timeout of at most 5 s, so no
  configuration is refused (round 9: architecture Minor 5); the beat never waits on the epoch: it takes the epoch row
  `FOR SHARE SKIP LOCKED`, reads it plainly only when an owner transaction holds it, and reports that, so an activation or a removal
  by hand SQL left open never closes the fleet's gates; when the beat holds the row while an activation waits, the activation,
  in `READ COMMITTED` with its census in a statement after its lock, sees the beat's `last_seen` and refuses an engine that lacks
  the capability (round 13: security Nit 1) (round 9: security Major 1, correctness Nit 7); the refusal is
  `UNAVAILABLE` with the envelope reason `REGISTRY_SILENT` and a retry hint, counted in
  `worker_capability_calls_refused_total{method, reason="REGISTRY_SILENT"}` (round 11: architecture Minor 3), which the SDKs repeat within the handler's deadline (`sdks/SPEC.md` §5; round 9: design Nit 7); heartbeats, reports,
  hand-backs and claims keep flowing. This amends Architecture decision 0082 decision 5 for capability calls only, recorded in
  Architecture decision 0098 and in the Amendments section of Architecture decision 0082 at acceptance, with section 3 of
  `docs/runbooks/instance-registry.md` the floor build's to update; it is missing, a hard precondition, and this build inherits it
  (round 6: security Minor 3; round 7: architecture Major 1, design Major 3, correctness Minors 1 and 2, security Minor 4). The beat
  of (7) is a new function, `instance_heartbeat_reporting_change(p_instance uuid, p_token bytea)`, which returns what
  `instance_heartbeat` returns and `out_change_in_progress`: it first tries the epoch row `FOR SHARE SKIP LOCKED` and returns that
  locked read's epoch with false, and only when the row is held reads the committed epoch plainly and returns true, so an
  activation committing during the beat is never seen as the old epoch with no change in progress (round 12: security Minor 1); under the
  same privilege constraints as (6) and (8), with `EXECUTE` revoked from `PUBLIC` and tested so, and called by both kinds, whose gate
  member is shared; a function of its own name, not an overload, leaves the two-argument function's contract for older binaries and
  no call ambiguous (round 11: security Nit 2, architecture Nit 8, design Nit 7); it raises when the epoch row is missing. A process
  always applies a committed epoch that moved and closes its gate when it lacks what was recorded, which is always safe (round 11:
  security Minor 2); a beat that reports a change in progress is a successful beat for liveness, readiness and the closing rule,
  keeps an engine's current capability decision only while its window has not lapsed, and never opens a lapsed one, which waits for
  a beat with no change in progress (round 10: security Major 1 part 5 and Minors 1 and 2, architecture Major 2 and Minor 4, design
  Minor 5); such beats are counted in `instance_heartbeats_change_in_progress_total{kind}` (round 11: design Nit 8). The second gate check is before the first byte of a call that hands data out,
  and before the write of a call that writes (round 10: security Nit 4). There is no fallback: the round-9 fallback, in which this
  design's dormant build would have become the floor, could not be activated (no control plane lists the capability) and did not
  enforce its floor after the rollback's removal (round 10: architecture Major 1, security Major 1, design Major 1). Instead, a
  released build of the follow-up with (2), (7) and (9) is a hard precondition of Task 006-057, the task that leaves dormancy; the
  tasks that write or decode the cause, use the secret fence, read a requirement's kinds or run the server end to end wait for the
  follow-up's merge, precondition (a) of the task phase names them, and the other dormant tasks proceed (round 11: architecture
  Major 2, correctness Minor 1, design Nit 2). (9) The follow-up lists a capability of its own on both kinds, `lease-withdrawal-v2`,
  meaning that the binary replays the causes of 1.8 and applies (7); it joins the closed lists of known capabilities, gains an entry
  of the generic `capability-activate` for both kinds that refuses while `lease-withdrawal-v1` is not recorded for both kinds, and
  the replay-floor guard refuses its removal while a journal holds `step.lease_withdrawn` with the cause `NOT_ACKNOWLEDGED`,
  the one cause a build without it cannot replay, so recording it stays reversible until the first withdrawal of 1.8 (round 12:
  architecture Minor 2, correctness Nit 4, design Nit 3); the merged head's tests that assert it is not activatable
  (`apps/control-plane/src/capability_activation.rs`, `tests/integration/schedules-bulk-operations/lease_withdrawal_activation.rs`)
  are the follow-up's to invert (round 12: design Nit 4); and the forward order is: the follow-up's build, `lease-withdrawal-v2`
  activated, the build of Task 006-057, `delivery-acknowledgement-v1` activated. The release rule of round 11 was the alternative,
  rejected because it rests on what was released rather than on what the registry admits. Every withdrawal of a marked lease reads `lease-withdrawal-v2` where it read `lease-withdrawal-v1`, so the
  floor is the registry's whatever the order of releases: a release of #260 without the follow-up lists only `lease-withdrawal-v1`
  and is refused registration once `lease-withdrawal-v2` is recorded, which it stays once any `NOT_ACKNOWLEDGED` withdrawal exists and, by the trigger of Task 006-046, while
  `delivery-acknowledgement-v1` is recorded; and the entry of
  `delivery-acknowledgement-v1` also refuses while any registered instance, live or not (the registry deletes a row silent for a
  day), lacks `lease-withdrawal-v2`, so no binary without (7) still holds an open gate when the first provisional lease is claimed
  (round 11: correctness Major 2, security Major 1, architecture Major 1, design Major 1). Requests (1) and (5) belong to the same
  follow-up. Its decision 5 also makes a hand-back below 1.8 an ordinary failure that spends the attempt (Decision 006-030 there), which
  this design does not change: the owner weighs it with that decision (round 3: correctness Major 4, design Major 3).
- **D006-060 — The frequency is unknown.** No loss of a frame on the wire or of a `PollTasks` answer has been observed; the three cases
  that started this work are Architecture decision 0098's (round 1: design Major 3). At 1.8 the withdrawals with the cause
  `NOT_ACKNOWLEDGED` measure it.
- **D006-061 — Minor 1.8 is the minor at which `TASK_NOT_STARTED` is reserved for the host** (the server review of pull request #260,
  given by the coordinator on 2026-11-20; Decision 006-030 of the server build). The server's rule stays in Architecture decision 0098
  decision 5 (round 3: architecture Major 3); this design adds what 1.8 changes (Decision 006-059, and Decision 006-045 for a lease
  not yet `RUNNING`; round 9: architecture Major 2). The SDK rewrite of a handler-raised code is that of Decision 006-028 of the SDK build, at every minor (round 3:
  architecture Minor 7, design Major 1).
- **D006-062 — A committed acknowledgement must survive, and a mark that might not have counts for nothing** (round 4: design Major 1;
  round 5: architecture Major 1, design Major 1, correctness Minor 3, security Minor 1, architecture Minor 6; round 6: architecture
  Major 1, design Major 1, security Minors 1 and 2; round 7: design Minors 4, 5 and 6). The exactness of decision 4 of Architecture
  decision 0099 rests on it. (a) The acknowledgement, and the capability record, which protects the hand-back of an unmarked 1.8
  lease, commit with `synchronous_commit` at least `local`, raised by their settings statement when it is `off` (draft 032 asks the
  same of its replay paths, Requirement 032-009): a crash that the postmaster recovers without restarting keeps the start time and
  loses the write-ahead log not yet written. (b) The claim stamps `lease_claim_incarnation` with `pg_postmaster_start_time()`, which
  every role can read, which never fails on a standby, and which changes whenever another server takes over (a promotion, a recovery,
  a snapshot or base backup started as a server, a managed restore or failover, a cut-over, an old primary's return) and with the loss
  of the operating system; a mark of another incarnation, or none, does not count. (c) `fsync` has no condition of its own and is
  stated as an assumption: with it off a commit reaches only the kernel, which the loss of the operating system loses (another
  server, seen) but a device's write-back error can also lose with the server running (round 8: security Nit 4); the round-6
  condition, its skip reason and its alert clause are dropped. (d) A restore that does not start the server (a logical restore into
  the same running server, a virtual machine reverted with its memory, a storage snapshot rolled back under a running server) is
  done with every engine stopped and followed by a restart of PostgreSQL before they start, checked by the start time (round 8:
  correctness Minor 4, security Minor 4); the round-6 command `end-lease-marks`,
  its function and its table `platform_operator_actions` are dropped, since their census refused in the one case they existed for
  (round 7: correctness Minors 8 and 9, security Minor 3, architecture Minors 5 and 6, design Minor 4). It assumes one writable
  server, which the row lock assumes too, and the gauges read the primary. The fifth round's timeline was replaced because it missed
  the restores that start a server without archive recovery and needed a guarded function PostgreSQL cannot honour. One column and
  one function call per claim of a 1.8 lease, per acknowledgement and per withdrawal re-read; no statement. Task 006-044 records
  whether a crash restart changes the start time; if it does, (a) can go too.
- **D006-063 — The SDK copies of the contract move with it, and internal runners that rise decline** (round 4: architecture Major 4
  and Minor 8). Task 006-043 runs each SDK's synchronisation script and regenerates its checked-in stubs with no host behaviour, as
  Task 041-3 does for 1.7, so the byte-equality tests of the copies stay green between the contract and the SDK tasks. An internal
  runner that rises to 1.8 or later (drafts 023 and 024 will, through 1.9) registers with `declines_acknowledgement = true` unless it
  implements the host procedure; notes in those drafts say so, and in drafts 032 and 035, which also take later minors (round 5:
  architecture Minor 8).
- **D006-064 — The 1.8 hand-back is honoured only while `delivery-acknowledgement-v1` is recorded, for a lease claimed after that
  recording** (round 5: security Major 1; round 6: correctness Nit 5, security Minor 3). In a fleet that mixes this build with the
  build of Architecture decision 0098, an engine of the floor build serves a capability without recording `lease_capability_generation`,
  so a hand-back withdrawn by this build could hide a capability that was served. The requirement, once recorded, means that every
  live engine is of this build, and the registry refuses any that is not; an engine silent at the census serves no capability call
  after it reconnects until its gate is decided again (Decision 006-059 (7)). So the hand-back, and every withdrawal of a marked lease, read both
  requirements and the former's `required_since` under the locks their removals wait for (`delivery_acknowledgement_required()`, one
  statement, both rows `FOR SHARE` in the order removals must take, `SECURITY DEFINER` as `lease_withdrawal_required()` is), and a
  removal waits for them; a lease claimed before the current recording does not count, so a removal followed by a re-activation does
  not give protection back to leases a floor engine may have served in between. A `NOT_SENT` withdrawal needs only
  `lease-withdrawal-v1`, since its token never left the server (round 6: correctness Minor 8). Before the activation, and after a
  removal, a 1.8 hand-back is an ordinary failure that spends the attempt, as below 1.8, and the provisional leases in flight lapse
  as today, which is why the rollback switches marking off and drains before it removes the requirement (Decision 006-055).
- **D006-065 — Only the host's own path makes the host decline, and the server's envelope tells the two apart** (round 5: correctness
  Major 2; round 6: correctness Major 1, security Major 1, design Minors 1 and 2; round 7: all four reviewers; round 8: correctness
  Major 1 and Minor 3, security Minors 2 and 3, design Major 1; round 9: correctness Major 2 and Minors 5 and 6, design Minors 3 to 5,
  security Minor 3). The worker service and its admission put the error envelope (`x-sagat-error`) on every status they make, which
  Task 006-049 checks, so a transient answer carrying it is the server's own trouble and never counts toward a decline; a gateway's
  503 or a mesh's timeout carries none. The server's deadline starts in the platform's own outermost layer, the per-connection
  concurrency limit being the platform's layer under it, with a reserve of 4 places for the class while the compiled minor is at
  least 8, so the other classes have 60 of the default 64 from Task 006-057 and the dormant build changes none (round 11: design
  Minor 4, architecture Minor 4). The early give-up is decided by an attempt sent after the evidence: the evidence is, after
  an attempt of a lease ended with no verdict, another call of the host that reaches the database, sent after that attempt ended,
  answered `OK`, or a frame with leases arriving after it; the host gives the lease up at the end of an attempt sent after the
  evidence only when that attempt ends with no verdict and neither a shed (`RESOURCE_EXHAUSTED` with `ACKNOWLEDGEMENT_SHED`) nor an
  unreachable store (`UNAVAILABLE` with `ACKNOWLEDGEMENT_STORE_UNREACHABLE`, the server's reason when its deadline passed before it
  held a connection or its connection failed: an engine that has not reached the database decides nothing, round 12: correctness
  Major 1); with the envelope or not, any other status counts, so a mesh's cut counts as well as a deadline; no attempt of the lease
  was shed; one receive bound has passed since its first attempt; and no try of another call of the host that reaches the database
  (a heartbeat, a report, a hand-back, a receive; never an `AcknowledgeTasks`) ended with a transport failure, `DEADLINE_EXCEEDED`,
  `UNAVAILABLE`, `RESOURCE_EXHAUSTED` or a retryable `INTERNAL` since the evidence, a list a host can observe (round 12: correctness
  Minor 2, design Minor 1). Attempts keep the backoff of §5 and any retry hint; only once one receive bound has passed and no attempt
  was shed does the evidence send the next attempt early, after a random delay of up to 200 ms, at most once per lease per receive
  bound, so dense evidence is no retry loop and a promotion no herd (round 12: correctness Minor 1, security Minor 3, design Minor 2,
  architecture Nit 7). A `RETRY` restarts the window, and the halves of a split call share it. Evidence must
  follow a failure, so an `OK` from before a failover counts for nothing (round 10: correctness Major 1, design Minor 2), and a
  failure must follow the evidence, so the first answer after a promotion leads a lease past one receive bound to an attempt sent
  early, within 200 ms, which a working primary answers, and not to a hand-back of a lease that was waiting in its backoff (round
  11: correctness Major 1, design Minor 1). The reason `ACKNOWLEDGEMENT_STORE_UNREACHABLE` covers every way an engine has not
  reached a writable database: a transaction that did not begin, its acquisition timing out or failing with a store error the
  registry counts as unreachable, in the handler or in the authentication's lookups once the token has verified, and `25006` from a
  server that answers read-only; a call short of time after the concurrency layer answers `ACKNOWLEDGEMENT_DEADLINE` (round 13:
  correctness Minor 3 and Nit 6, security Minor 1). The blocking tries are counted between the latest evidence before the deciding
  attempt and its end, a receive ended by the host's own bound is not one (round 13: correctness Minor 2, design Nit 2), and an
  attempt sent before the first evidence that follows the previous failed attempt does not decide, whatever evidence arrives while
  it is in flight (round 13: correctness Nit 1; round 14: correctness Minor 1). A statement or `COMMIT` failing with an unreachable
  error after the transaction began answers the same reason, by one shared classification (round 14: correctness Minors 4 and 5,
  architecture Nit 6, security Nit 2); only the current acknowledgement connection redials, one old connection drains at a time
  and a failed dial waits one backoff step (round 14: correctness Nit 1, design Minor 1, security Nit 4); and every acknowledgement
  attempt is bounded to 6 s whatever the configured call timeout (round 14: design Minor 2). What is left: a
  failover that begins during the deciding attempt, after the evidence, with no other call failing in that time, or during a
  hand-back, and, on a stream, a frame sent before a failure and read after a failed attempt on a host with no other call, cost the
  lease its attempt (Gap 006-047). Sheds, a busy engine, and a host with no other call and no frame wait for the ordinary
  give-up. With the bound of
  Decision 006-066 a lease given up early runs unflagged on its next lease after one receive bound and its deciding attempt, more
  when heartbeats are the only evidence. The host declines on `UNIMPLEMENTED`,
  three refusals of a closed list, or three counted calls in the run since the last `ACKNOWLEDGED`: given up with no verdict, every
  attempt ended with no answer or without the envelope, and its probe failed the same way, the probe being one `AcknowledgeTasks` for
  one of its leases, sent once that lease's hand-back is answered `OK` or refused stale with the envelope (the pass won), evidence
  only, which a working path answers `NOT_HELD` with the reason `withdrawn`; a verdict other than `NOT_HELD` or `RETRY` leads to one more
  hand-back and no second probe. The host sends `AcknowledgeTasks`, its probes included, on a transport connection of its own
  (a channel is not enough in grpc-js and grpcio, which share subchannels unless `grpc.use_local_subchannel_pool` is set; round 12:
  architecture Minor 3, design Minor 4) with at most 4 calls in flight, held by the SDK's own permit taken per attempt, frames waiting
  for it merged into a new call up to 64 leases (round 12: design Minor 3), and the connection dialled again when an attempt on
  it ends with no answer while the main connection answers (round 12: correctness Minor 4), or after two
  `ACKNOWLEDGEMENT_STORE_UNREACHABLE` in a row while the main connection answers, the old connection drained and an attempt ended by
  the host's own close sent again and counted for nothing (round 13: correctness Minor 1, architecture Minor 3, design Minor 4);
  retries take a free permit before new frames, waiting frames form a new call, and each lease keeps its own window in it (round
  13: design Minor 3); the connection is built by the same function as the main one, the same credentials, TLS settings and
  interceptors (round 13: security Minor 3); each 1.8 host holds two connections against the server's caps, which the runbook says
  (round 13: architecture Minor 2); an acknowledgement waiting for that
  permit has not been sent, is not an attempt and counts toward nothing, a rule every SDK can build, unlike whether a transport gave
  a call a stream; behind a proxy that pools connections upstream only the server's reserve holds, which is why it stays (round 10: architecture Minor
  6, design Minor 6; round 11: correctness Minor 7, architecture Minor 5, design Minor 3). Calls are ordered by the give-up of the last of their leases, a call that does not qualify
  neither counts nor breaks the run, a run with no counted call for 10 minutes starts again, and there is no time
  floor. The sixth round's engine breaker is withdrawn (round 7: correctness Major 1 and Minors 4 and 5, security Major 1 and Minor 1,
  architecture Minors 7 and 8, design Major 2 and Nit 5). The declining registration waits a random delay of up to one receive bound
  (round 6: architecture Nit 9).
- **D006-066 — An attempt is flagged at most once after a hand-back, and never at the cap** (round 7: correctness Major 1, design
  Major 1; round 8: correctness Major 2, architecture Major 1 and Minor 1, security Minor 1, design Minor 1). `steps.
  provisional_withdrawn_attempt` is attempt data, not lease data: no generation binds it and nothing clears it, and a later attempt
  number never equals it. The claim, the withdrawal and every other transition write a step row through the same three projection
  statements, so its one form is in each of them, for every caller: `CASE WHEN $flag AND status = 'LEASED' AND generation = $fence
  THEN attempt ELSE provisional_withdrawn_attempt END`, where `$flag` is bound true only by the application's withdrawal service (round 11: architecture Minor 2), for the hand-back's
  withdrawal of a lease whose mark counts or for the withdrawal `withdraw_lease` reports as the last the cap allows, and false by every
  other caller, the claim included; it is bound per instance, through a port method of the run and path transactions keyed by
  the `ExecutionInstanceId` (the node and its path indices; the root instance for a root step) and the generation, which the adapter
  consumes in the withdrawn instance's projection call alone and whose commit fails with an internal error when a mark was never
  consumed, so a `LEASED` sibling with the same generation touched by the same append, an item of the same collection node
  included, keeps its value (round 10: correctness Minor 2, architecture Minor 2; round 11: correctness Minor 3, architecture Minor
  1); the hand-back sets it only for a lease flagged at its claim;
  the old-row fence keeps a later append of the same transaction from writing a row already withdrawn, and `attempt` is the stored
  value before the same statement sets it back by one, so the withdrawn lease's attempt (round 9: correctness Major 1, architecture
  Minor 1). No other statement names the column, which the static check enforces with its own
  allowed set and negative tests; this is the one bound flag allowed, since a stale caller can only bind false. The claim reads it from the row it already writes and marks only while it
  differs from the attempt claimed, so it costs no statement and is not the journal read of considered option 7. So whatever fails on
  the acknowledgement's path, a lease is handed back once per attempt, its next lease runs unflagged, and no withdrawal cycling spends
  an attempt. The pass's `NOT_ACKNOWLEDGED` does not set it: silence is the loss this feature exists for, a lossy path loses
  deliveries more than once, and the cap still bounds it; where the hand-backs fail too, the host's path is down and its reports
  fail anyway. A planned hand-back (a host's stop, a decline, `CANCELLED`) also sets it, because the server sees one code; a reason
  field on the hand-back that would tell it apart was considered and left to the owner (Gap 006-047). A `NOT_SENT` withdrawal below
  the cap does not set it.

### Gaps

- **G006-040 — A host that crashes between the verdict `ACKNOWLEDGED` and the start of the handler** leaves an acknowledged lease that
  lapses into a review. The window is inside the process and holds no call; a stopping host runs the task instead.
- **G006-041 — An acknowledged lease that does not get back to the server unrun:** its host cannot hand it back (the lease lapses into
  a review), its hand-back loses to the pass past the expiry (`expire_attempt`, a review for an `Activity`), or its first heartbeat,
  sent before the handler, commits with its answer lost (the lease is `RUNNING`, and its hand-back spends the attempt). One that its
  host hands back in time is withdrawn (Decision 006-045).
- **G006-042 — The trust assumption** (Decision 006-053). A client at 1.8 that runs a flagged lease without acknowledging and makes no
  call during it has its lease withdrawn and its work run again with the same attempt number and idempotency key; external effects stay
  at least once (constitution principle III); a later call with its token is counted and alerted, but only the last withdrawn token of a
  step is kept. A hand-written client can also send `TASK_NOT_STARTED` after running a handler that used no capability; the code is the
  host's word, as in Architecture decision 0098.
- **G006-043 — The internal runners stay below 1.8** (the database runner at 1.6, the sandbox runner at 1, the command-line probe);
  one that rises declines (Decision 006-063).
- **G006-044 — For one registry heartbeat after an activation or a removal**, an engine may still claim without marks, or with them.
  The registry cannot exclude a binary of release 0.2, which does not register; the runbook's rule that no such binary runs at an
  activation is what excludes it. A process that could not register into a full table (Gap 2000 of the registry) serves for one beat
  unseen. The trigger that refuses `delivery-acknowledgement-v1` while a registered instance lacks `lease-withdrawal-v2` cannot see
  a process whose row is gone (deleted after a day of silence, pushed out of a full table, never written): such a process of a
  build without the liveness rule can serve capability calls until its first successful beat after the activation; the runbook
  allows removing a silent row only once its process is confirmed stopped (round 12: security Minor 2, architecture Minor 1, design
  Minor 6, correctness Nit 6). A database clock stepped backwards across a removal and a re-activation could make a lease claimed before the
  re-activation count. A floor engine's withdrawal at the cap, within the learning window, does not set the attempt flag.
- **G006-045 — The throughput cost and the call bound are predictions** until Task 006-056 measures them.
- **G006-046 — The journal does not record that a lapsed lease had been acknowledged**; `worker_leases_expired_total{mark}` does.
- **G006-047 — A lease whose earlier deadline is under three receive bounds (18 s) is not marked, nor the next lease of an attempt
  after a hand-back (planned ones included: a host's stop, a decline, `CANCELLED`) or at the cap; an attempt past the cap is not
  withdrawn, nor a mark across another server's takeover, planned or not**, so a frame lost on the way to such a lease still lapses
  into a review. A failover that outlasts a lease's give-up, or begins during its hand-back or during the one attempt that follows
  the evidence of an early give-up, spends the attempt: the hand-back is an ordinary failure once the promotion has come, and when
  its tries of §5 end before the promotion the lease lapses into `recovered` at the pass and, for an `Activity`, a review (round 11:
  correctness Minor 2, design Minors 1 and 2, architecture Nit 9). So does, on a stream, a frame sent before a failure and read
  after a failed attempt on a host with no other call, which counts as evidence (round 12: correctness Nit 2, design Nit 2), and an
  engine whose stale pool outlasts the ordinary give-up after a promotion, each stale connection costing an acquisition its whole
  bound (round 13: correctness Nit 7). Task 006-056 reports the share of claims left unmarked; `flagged_before`, `capped` and `recovered` count the
  others.
- **G006-048 — The database's durability.** What the incarnation and the durable commit do not cover: a restore that does not start
  the server (a logical restore into the same running server, a virtual machine reverted with its memory, a storage snapshot rolled
  back under a running server) done with the engines running or without the restart of PostgreSQL that the runbook asks for;
  `fsync` off, which a device's write-back error can make lossy with the server still running; and more than one writable server.
  The rule is proven on self-managed PostgreSQL 16 through `infra/dr/drill.sh`, which also checks a snapshot-style restart. The owner
  accepts this with decision 12 of Architecture decision 0099.

### Owner question

- **O006-002 — The reservation of minor 1.8 before its contract merges.** The register's rule (Owner decision 2026-11-15-8) takes a
  minor "when its contract merges"; the coordinator instructed this design to take and register 1.8 now. The reservation is released if
  Architecture decision 0099 is not accepted; if the gate fails, the owner chooses between shipping 1.8 as "the code reserved, the flag
  never set", which fixes that meaning of 1.8 for good and moves delivery acknowledgement to a later minor, and releasing it (Decision
  006-057). Asked through the coordinator. **Answered 2026-11-21: "Reserve 1.8 now" (Owner decision 2026-11-21-6).**
