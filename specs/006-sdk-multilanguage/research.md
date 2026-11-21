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


## Leases that never reach a host (2026-11-20, D006-001 to D006-010, G006-001 to G006-003, O006-001)

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

### Gaps

- **G006-001 — A lease handed to the transport and lost still lapses into a review** (a frame lost on the wire after
  `poll_next` returned it, the answer of a unary `PollTasks` lost after the handler returned). Closed by O006-001.
- **G006-002 — A process that stops (or crashes) between a claim and its withdrawal** leaves the lease to lapse. The withdrawal
  is spawned when the body ends and runs to its own bound; a graceful shutdown waits for it, a crash does not.
- **G006-003 — Before the operator records `lease-withdrawal-v1`, nothing changes** but the counter: an upgraded fleet keeps
  today's behaviour until the activation.

### Owner question

- **O006-001 — Delivery acknowledgement (option C).** Answered by the owner on 2026-11-20: "Yes, in 0.3.0"
  (OD-2026-11-20-12). Designed in ADR-0099 (decisions D006-011 to D006-024 below: a provisional lease marked on the step row,
  protocol 1.8, the lease expiry as the deadline). The sketch the question was asked on:
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


## Delivery acknowledgement (2026-11-20, D006-011 to D006-024, G006-004 to G006-008)

Owner question O006-001 was answered "Yes, in 0.3.0" (owner decision OD-2026-11-20-12), which left the variant, the protocol
minor and the deadline to this design. All decisions below are "delegate (OD-2026-11-20-6)". The contract is
`stream-completion.md`, "Delivery acknowledgement (protocol 1.8)"; the host rules are `sdks/SPEC.md` §6.1 to §6.4 and §6.8;
the hard-to-reverse choice is ADR-0099; the build plan is `tasks.md`, phase "Delivery acknowledgement".

### Decisions

- **D006-011 — The variant: a provisional lease, marked on the step row and acknowledged by a batched call.** Neither sketch
  of O006-001 is taken as it was written. Variant 1's exactness is kept (the handler runs only on the server's committed
  acknowledgement) without its new step status and event, which would reach every reader of `steps.status` and add a journal
  append per task; variant 2 (the first heartbeat before the handler) is equally exact but costs, per task rather than per
  frame, one call and one engine transaction that loads the run's state and appends `step.started` (a strict superset of the
  chosen acknowledgement's statements), adds one journal event to every task shorter than its heartbeat interval (most tasks,
  so most conformance goldens would change), still needs a mark of which leases carry the contract, and makes a lease
  `RUNNING` before its handler starts, so a host can no longer hand back a lease it did not start. Weighed also: a separate
  shorter deadline (D006-016), a statement without the run lock (breaks the invariant of D006-015), a bidirectional stream
  (six SDKs, nothing for a unary poll, breaks ADR-0029's finite window), the acknowledgement on the next poll (the handler
  would wait a receive). The chosen variant wins on exactness (equal), cost per task (one call per frame, no run-state load,
  no journal event, no added claim statement), behaviour with older SDKs (nothing changes for them), and the hand-back
  (still possible after the acknowledgement).
- **D006-012 — Protocol minor 1.8.** The register of `sdks/SPEC.md` §9 (owner decision OD-2026-11-15-8) holds 1.1 to 1.7, 1.7
  for feature 041; no branch or sibling worktree names 1.8 (searched 2026-11-20); drafts 020, 021, 025 and 026 take the next
  free minor when their contract merges. 1.8 is taken and registered in the same change as this design, at the coordinator's
  instruction, as "reserved by its design, served from task T006-048". Minors are cumulative (ADR-0015, and design 041 for
  1.7 over 1.6), so 1.8 is served only once 1.7 is (task T041-7). If T041-7 has not merged when task T006-048 is ready, the
  build stops there and reports; it does not make 1.8 independent of 1.7, which would change the rule of ADR-0015.
- **D006-013 — The provisional mark is lease data.** Two nullable columns of `steps`, `lease_offered_at` and
  `lease_acknowledged_at`, written by the statements that already write the lease, cleared with it. They are not journaled,
  like the lease-token hash and the expiry; the replay does not derive them; a reset clears them with the lease. Adding a
  nullable column with no default is a change of the catalogue only in PostgreSQL. The claim's budget line does not move.
- **D006-014 — The acknowledgement is one transaction per run, under the run lock, with the checks of `Heartbeat`, and loads no
  run state.** Tasks of one run in one call share the transaction. It writes the acknowledgement time once, moves the attempt
  deadline (D006-019), renews the lease as a heartbeat does, and answers `ACKNOWLEDGED` with the renewed expiry and the step's
  `cancel_requested`, or `NOT_HELD` when a check fails. No `RETRY` verdict: a store failure ends the call and the host repeats
  it, which is safe because a second acknowledgement is a duplicate. A run of another tenant answers `NOT_HELD` and is audited
  as `Heartbeat` audits it.
- **D006-015 — A provisional lease that expires is withdrawn with the cause `NOT_ACKNOWLEDGED`, and exactly one of the
  acknowledgement and the withdrawal commits.** The lost-lease pass re-checks its condition under the run lock and withdraws as
  ADR-0098 decision 3 does (no attempt consumed, no review). Both writers hold the run lock and fence on the lease (the
  acknowledgement on the token of a `LEASED` or `RUNNING` step, the withdrawal on a `LEASED`, unacknowledged step); the host
  runs only on `ACKNOWLEDGED`. That is the whole argument, and it uses no clock. An acknowledged lease that expires keeps its
  meaning (an unknown outcome). The `NOT_SENT` withdrawal of ADR-0098 also requires an empty acknowledgement, which closes the
  impossible case by construction.
- **D006-016 — The acknowledgement deadline is the lease expiry, and 1.8 needs a time-to-live of at least 7 s.** The
  derivation is in `stream-completion.md` ("Why the deadline is the lease expiry"): R = 6 s, the receive bound, specified; A,
  one acknowledgement, at most 0.103 s by the measured 99th percentile of the completion, which does strictly more; every
  deadline in [R + A, time-to-live] is safe; the upper end is chosen because a withdrawal of a healthy but slow delivery adds
  work when the database is slowest, while a later deadline costs time only after a loss, and because it adds no setting, no
  column and no scan. Weighed: a deadline of about 7 s (recovers a lost lease about 23 s sooner, withdraws healthy deliveries
  under load, one more parameter); the threshold under which 1.8 is not offered, R + ⌈A⌉ = 7 s, re-derived by task T006-064
  from the measured acknowledgement.
- **D006-017 — A fleet requirement gates every part, and it depends on `lease-withdrawal-v1`.** `delivery-acknowledgement-v1`
  is listed by every control plane and engine of the build. While it is not known to be recorded, an engine does not offer
  1.8, marks no lease and writes no `NOT_ACKNOWLEDGED`. The operator command `control-plane delivery-acknowledgement-activate`
  refuses while `lease-withdrawal-v1` is not recorded; the removal of `lease-withdrawal-v1` refuses while this one is.
  Weighed: gating only the mark and not the negotiation (a 1.8 host could then poll a replica of an older build that does not
  serve `AcknowledgeTasks` and never receive a verdict); no requirement (an older replica could not decode the new cause).
- **D006-018 — The host at 1.8 acknowledges before it starts anything, and bounds a `PollTasks` call to 6 s.** One call per
  frame or poll answer, concurrent with the next receive; no handler, heartbeat or capability call before the verdict;
  repeated without a limit on attempts until a verdict or until the leases expire by the host's clock; a lease without a
  verdict is handed back with `TASK_NOT_STARTED` and never run. The 6 s bound of a stream window is extended to the unary poll
  at 1.8 so that R holds for both transports (a lease the host reads, it reads within 6 s of its commit).
- **D006-019 — The attempt deadline of a provisional lease counts from its acknowledgement.** The `TIMED_OUT` arm of the
  lost-lease pass skips a provisional, unacknowledged lease, and the acknowledgement moves `attempt_deadline_at` later by the
  time the lease waited. Without this a step whose `timeoutMs` is shorter than its delivery would either be withdrawn at every
  attempt and loop without consuming one, or time out right after an acknowledgement into a review. With it, `timeoutMs`
  measures the attempt from the moment its host has the task. This narrows the meaning of `timeoutMs` for 1.8 leases (delivery
  is no longer counted, at most R); older minors keep today's meaning. Reported to the owner as a change of a public option's
  meaning made under the delegation.
- **D006-020 — A provisional lease receives no capability.** `ResolveSecrets`, `ResolveDatabase`, `WriteBlob`, `ReadBlob` and
  `AppendStepLogs` are refused with `FAILED_PRECONDITION`, reason `TASK_NOT_ACKNOWLEDGED`, in the query that already checks the
  lease. A credential or a blob is not handed to a lease the server may withdraw and give to another host. `Heartbeat`,
  `CompleteStep` and `FailStep` are served as today: each takes the step beyond the withdrawal, the safe direction for a host
  that did not acknowledge. Weighed: making every such call an implicit acknowledgement (a write on five read paths for a case
  a conforming host never produces).
- **D006-021 — Cancellation before the start is known and immediate.** The verdict carries `cancel_requested`; a host told so
  does not start the handler and hands the lease back with `TASK_NOT_STARTED`; the withdrawal runs the engine's settle in its
  transaction, and the planner cancels a `READY` step of a stopping run (`crates/domain/src/execution/plan.rs`). The same holds
  for any withdrawal of ADR-0098 (to be confirmed against its build, task T006-050).
- **D006-022 — Older minors and the internal runners are unchanged.** A session below 1.8 gets no mark and keeps today's
  meaning of every lease. The database runner (1.6), the sandbox runner (1) and the CLI's worker probe keep their minors; moving
  them is not part of this design (gap G006-007).
- **D006-023 — What is counted and alerted.** `worker_task_acknowledgements_total{verdict}`,
  `worker_leases_undelivered_total{cause="not_acknowledged", outcome}`, the histogram
  `worker_task_acknowledgement_delay_seconds` (the measured input of D006-016), the gauges `worker_leases_provisional` and
  `worker_leases_provisional_expired` (from the operations statistics query, not the hot path),
  `worker_capability_calls_refused_total{method, reason}`, and the alerts `SagatLeasesNotAcknowledged` and
  `SagatLeaseAcknowledgementSlow`.
- **D006-024 — The tests are deterministic and the goldens do not move.** Races are held with the run lock or a barrier,
  never a sleep; the conformance suite gains fault actions that hold a call until the scenario releases it and drop one
  stream frame, and the scenario `delivery-acknowledgement`; every other scenario must pass at 1.8 with its existing golden,
  unblessed, which is the evidence that the happy path writes nothing new.

### Gaps

- **G006-004 — A host that stops or crashes between the verdict `ACKNOWLEDGED` and the start of the handler** leaves an
  acknowledged lease that lapses into a review. The window is inside the process and holds no call. A stopping host hands the
  lease back instead; a crash cannot.
- **G006-005 — An acknowledgement whose answer is lost, followed by a loss of the network for the rest of the renewed lease,**
  leaves an acknowledged lease that the host did not start and cannot hand back: it lapses into a review.
- **G006-006 — The guarantee binds hosts that keep the protocol.** A hand-written client at 1.8 that runs a handler without
  acknowledging, uses no capability and dies before any call, has its lease withdrawn and its work run again with the same
  attempt number and idempotency key. External effects stay at least once (constitution principle III); the refusals of
  D006-020 make most such clients fail loudly.
- **G006-007 — The internal runners stay below 1.8** (the database runner at 1.6, held there by design 041 for 1.7; the
  sandbox runner at 1; the CLI probe). Their leases keep today's meaning; a frame lost on the way to them still lapses.
- **G006-008 — For one registry heartbeat after an activation**, an engine that has not learned of it can claim for a 1.8
  session without the mark; that lease keeps today's meaning. After a removal, for one heartbeat, an engine may still mark
  leases or withdraw with `NOT_ACKNOWLEDGED`: the rollback waits for it (ADR-0099, "Rollback").
