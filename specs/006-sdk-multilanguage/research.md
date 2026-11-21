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
  (OD-2026-11-20-12). Designed in ADR-0099 (decisions D006-040 to D006-060 below: a provisional lease marked on the step row,
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



## Delivery acknowledgement (2026-11-20, D006-040 to D006-060, G006-040 to G006-046)

Owner question O006-001 was answered "Yes, in 0.3.0" (owner decision OD-2026-11-20-12), which left the variant, the protocol
minor and the deadline to this design. All decisions below are "delegate (OD-2026-11-20-6)". The numbers start at 040 because
the build of ADR-0098 records its own decisions from D006-011 and gaps from G006-004 on its branch, and its SDK build from
D006-020 and G006-020 (seen in the sibling worktrees on 2026-11-20). The contract is `stream-completion.md`, "Delivery
acknowledgement (protocol 1.8)"; the host rules are `sdks/SPEC.md` §6.1 to §6.4 and §6.8; the hard-to-reverse choice is
ADR-0099; the build plan is `tasks.md`, phase "Delivery acknowledgement". The first round of the four reviewers (2026-11-20,
`reviews/2026-11-21-delivery-ack/round-1/`) found 5 Blockers and 32 Majors in the first text; the decisions below are the answer.

### Decisions

- **D006-040 — The variant: a provisional mark on the step row, acknowledged by a batched call that locks the run and restores
  nothing of it.** Neither sketch of O006-001 is taken as written. Variant 1's exactness is kept without its step status and
  event (a status reaches every reader of `steps.status`; an event per task adds a journal append per task). Variant 2 is
  equally exact but costs, per task, a call and a transaction that restores the run's whole state (`begin`), applies a
  transition and appends `step.started`, adds a journal event to every short task, and still needs a mark of which leases carry
  the contract. The first text claimed the acknowledgement "loads no run state" while prescribing `begin`, which restores it
  (round 1: architecture Blocker 1, design Blocker 2, correctness Major 1, security Major 4); the acknowledgement now takes the
  chain and run-row locks without restoring anything (D006-043), which is what makes it cheaper than variant 2. Under light load
  a frame carries one lease, so the saving is per task in statements, not in calls: one call and one small transaction per task;
  a call carries more under a backlog or a fan-out. An acknowledgement without the run lock (one conditional statement per call)
  was weighed again (design Major 1): it would need every run-locked writer that decides from the lease columns to re-check them
  in its write and roll its journal event back, a discipline across the engine, or let the renewal race those decisions.
- **D006-041 — Protocol minor 1.8.** The register (`sdks/SPEC.md` §9, owner decision OD-2026-11-15-8) holds 1.1 to 1.7; no
  branch or sibling worktree names 1.8 (searched 2026-11-20). 1.8 is reserved in the same change as this design at the
  coordinator's instruction, and the reservation is released if ADR-0099 is not accepted. Minors are cumulative (ADR-0015;
  design 041 for 1.7 over 1.6), so the server serves 1.8 only once it serves 1.7 (task T041-7) and an SDK offers 1.8 only once it
  speaks 1.7 (tasks T041-15 to T041-20). If either is late, this feature waits; it does not make 1.8 independent of 1.7.
- **D006-042 — The mark is bound to the lease's generation.** `lease_offered_generation`, `lease_offered_at` and
  `lease_acknowledged_at` on `steps`, written by the statements that already write a lease, root and scoped; a mark counts only
  while its generation is the step's. A lease written later by a binary that does not know the columns has the next generation,
  so a stale mark never makes it provisional (round 1: architecture Blocker 2, correctness Major 6, security Major 3). Weighed: a
  `BEFORE UPDATE` trigger that clears the marks when the token changes (one more trigger on the hottest table, and its cost
  hidden from the budget, which counts top-level statements only); a `CHECK` on the status (an older binary could no longer
  write the row).
- **D006-043 — The acknowledgement: one permission check, one unlocked indexed pre-read, then per matched run the chain lock,
  the run lock and one conditional write per task.** The pre-read matches the token hashes, so a caller without the tokens costs
  one indexed read and no lock (security Major 4, architecture Minor 5); runs are decided one after another, never two run locks
  at once; the binding is checked on the activity the write returns, against the caller's patterns read once per call, and a
  refused binding rolls that run back; at most one cross-tenant audit record and one binding audit record per call, through the
  bounded denial gate (security Minor 1, correctness Minor 4); a run that cannot be opened for a reason other than an unavailable
  store answers `NOT_HELD` and the call goes on (correctness Minor 3). The write checks the request's `attempt` and `generation`
  besides the token (security Nit 1).
- **D006-044 — Exactly one of the acknowledgement and the withdrawal commits, decided on values read under the lock.** The
  lost-lease pass's candidates are a hint; under the run lock it re-reads status, generation, marks, expiry and attempt deadline
  and decides with one pure function, and its write is fenced on what it read. The acknowledgement answers `NOT_HELD` for a
  provisional lease past its expiry and writes nothing, so the deadline is literally the expiry and the conformance step that
  releases a held acknowledgement after `expireLease` is deterministic (correctness Major 2, design Minor 6, architecture Minor
  10).
- **D006-045 — An acknowledged lease is an attempt.** It is never withdrawn: neither by the pass (an acknowledged lease that
  expires is an unknown outcome, as today) nor by a hand-back (`TASK_NOT_STARTED` for it is an ordinary retryable failure with a
  known outcome). The first text let a host acknowledge, resolve its secrets, run, and then hand the lease back as "nothing ran"
  (security Major 1). Capabilities are served only after the acknowledgement, and from then on the lease is treated as started.
- **D006-046 — The deadline is the lease expiry; 1.8 needs a time-to-live of at least 12 s; a full call must be answered within
  3 s.** The derivation (`stream-completion.md`, "Why the deadline is the lease expiry") uses specified bounds only: the host
  reads within R = 6 s of the claim and bounds each acknowledgement attempt to R, so one complete attempt fits when the
  time-to-live is at least 2R. The first text derived 7 s from R plus the 99th percentile of one completion, which left out that a
  call decides up to 64 runs one after another, the network and the retries (correctness Major 7, architecture Major 2, design
  Minor 1). The server-side bound of a full call, R / 2, is a requirement the measurement of task T006-064 must show, not an
  assumption: at the measured 103 ms per transaction, which overstates an acknowledgement, 64 runs would take 6.592 s.
- **D006-047 — The gate.** An engine lists `delivery-acknowledgement-v1` only when its time-to-live is at least 12 s
  (configuration, ADR-0082 decision 7; architecture Major 3, correctness Minor 2, design Minor 4); every control plane lists it.
  The dependency on `lease-withdrawal-v1` is enforced by the registry's trigger, both ways, so that the runbook's hand SQL cannot
  break it (security Minor 3, architecture Minor 2): this refines ADR-0082. The removal command expires the sessions accepted at
  1.8 once the learning bound has passed (correctness Major 5, design Major 4). Every engine of the build serves the call
  whatever it knows; a mark keeps its meaning until its lease ends, so a lease never changes meaning while it is held.
- **D006-048 — The host at 1.8.** It acknowledges before it calls or runs anything; it bounds a `PollTasks` call and each
  acknowledgement attempt to R; it gives a lease up after the lease's time left at receipt, clamped to between 12 s and 120 s, on
  its monotonic clock, never by comparing its wall clock with the server's (correctness Major 9, architecture Minor 11, design
  Major 8); a refused call (`UNIMPLEMENTED` from a proxy or a rolled-back replica, or any other status that is not retryable)
  makes it hand the leases back and register again offering 1.7 for the rest of its life, as ADR-0029 decision 6 falls back from
  `StreamTasks` (architecture Blocker 3, design Blocker 1). The clamp's lower end is 2R, the time one complete attempt needs; its
  upper end is four times the default time-to-live, so a host with a clock that is behind still gives up in bounded time.
- **D006-049 — The attempt deadline keeps its meaning at every minor.** The first text counted `timeoutMs` from the
  acknowledgement at 1.8, which would give one published version two meanings depending on the host's minor (constitution
  principle IV), with a shift of up to the whole time-to-live, not "at most R" (architecture Major 4, correctness Minor 1, design
  Minor 3). Now a provisional lease that reaches its attempt deadline ends its attempt as a known retryable failure,
  `ATTEMPT_TIMEOUT_BEFORE_START`: consumed by the step's own timeout, as today, never a review. The question that the first text
  would have put to the owner does not arise.
- **D006-050 — Capability refusal by a fence of its own, checked by the descriptor.** The refusal is a variant of the lease fence
  (root and path) that only the capability paths call, evaluated after the token and binding matched, so reports on a
  provisional lease are served and a caller without the token learns nothing (security Minor 2); a contract test lists every
  worker method that carries a lease token from the descriptor and fails when one of them, other than the four that keep or end a
  lease, is served on a provisional lease (architecture Major 7).
- **D006-051 — Cancellation before the start is decided by the acknowledgement.** An acknowledgement of a provisional lease whose
  cancellation is requested withdraws it (cause `NOT_STARTED`) and runs the settle in its own transaction, restoring the run or
  path on that path only, and answers `CANCELLED`. The first text had the host hand the lease back, which today's report fence
  refuses for a step that declares secrets, so the lease would have lapsed into a review (correctness Major 3, architecture Major
  10, design Minor 2).
- **D006-052 — Three withdrawals in a row end the provisional leases of a step.** `lease_unacknowledged_withdrawals`, cleared by
  an acknowledgement or a report; at three the next lease has no mark, and a fourth loss lapses as it does today. This revises
  decision D006-006 for the cause `NOT_ACKNOWLEDGED`, whose premise ("needs the server's own proof or the host's statement") does
  not hold for it: silence is enough (security Minor 4, design Blocker 1). Three: one loss on the wire is rare, a second in a row
  for the same step is already improbable, a third is treated as systematic; the added delay before today's outcome is at most
  three lease times. Weighed: consuming an attempt after the bound (architecture Blocker 3 (b): it fails a run for a delivery
  fault); a backoff on `READY` (a status the claim does not delay).
- **D006-053 — The trust assumption is the owner's to accept, and the server records what it sees.** The withdrawal of a
  provisional lease records "nothing ran" on the strength of the host's protocol promise, which the server cannot verify
  (security Major 5). A `Heartbeat`, `CompleteStep` or `FailStep` on a provisional lease is a violation: counted, and the
  session's later leases are not marked (`worker_sessions.acknowledgement_kept`). A report or heartbeat with the token of a lease
  withdrawn as `NOT_ACKNOWLEDGED` (`steps.withdrawn_lease_token_hash`) is counted, audited and alerted. The clarification of
  FR-037-006 says "for a host that keeps the protocol".
- **D006-054 — Scoped instances follow the same rules** (round 1: correctness Major 4, security Major 2, architecture Major 1,
  design Major 6). The pre-read and the write address the step row by its full address and the lock is the run row's, which
  `begin_path` takes too; the withdrawal is the ADR-0098 build's scoped withdrawal (one path transaction per run and instance
  path, with the addressed event); the refusal is a variant of `live_path_fence`; a stopping collection or loop drains a
  withdrawn instance as one never leased. Every task of the plan has a scoped row in its evidence.
- **D006-055 — Rollback: remove, expire the sessions, drain the marks, then roll back no further than a build that decodes the
  cause.** The `down` migration refuses while a mark counts (security Minor 5, correctness Major 5, design Major 4).
- **D006-056 — What is counted and alerted** is listed in `stream-completion.md`, "Observability": rates rather than windows
  (design Minor 8), the delivery delay and the call's duration measured separately (correctness Major 7 (c)), thresholds computed
  from the configured time-to-live in the rule (architecture Minor 6, correctness Minor 6).
- **D006-057 — The cost and its acceptance bound.** By count of the statements of D006-043, a task gains about 6 statements and
  1 transaction under light load (102 against 94 round trips with `BEGIN` and `COMMIT`), plus the call's authentication; the
  round-trip model of feature 038 predicts about 8% fewer transitions a second. The build measures it (task T006-064) with a
  network between the workers and the services, and stops and reports to the owner if 1.8 is more than 15% below 1.7 on the same
  machine and day. 15% is the prediction with a margin about as large again for the variation between runs on a loaded
  machine, which the 2026-11-12 run of feature 038 warns about (its load average was 26); the margin is a choice recorded
  here, not a measurement. Release 0.3.0 ships the requirement unrecorded, as ADR-0098 does: the operator activates it (architecture Major 6,
  design Major 2).
- **D006-058 — The conformance plan says how the evidence is produced** (correctness Major 8, architecture Major 5, design Major
  7). Of the 78 scenarios, 26 pin no minor and run at 1.8 by the SDK default; 29 pin 1.5 and run once more at 1.8 through a harness
  override (`SAGAT_SDK_PROTOCOL_OVERRIDE=1.8`, which only raises 1.5, whose task shape 1.8 keeps); those pinned at 1.4 and at 1
  remain the evidence for older minors; `protocol-negotiation`, `worker-stream-faults` and `worker-transport-faults` run with a
  3 s time-to-live, below 1.8 by construction. All of them must keep their goldens unblessed; only `protocol-negotiation` (a new
  probe with its own server block at 30 s, and one at 6 s that expects 1.7) and the new `delivery-acknowledgement` get new
  goldens. The new scenario adds a refused acknowledgement (`UNIMPLEMENTED`), a skewed host clock and a frame of several runs
  with one held busy.
- **D006-059 — What this design asks of the ADR-0098 build, before it merges.** (1) `TASK_NOT_STARTED` on a `LEASED` step that
  is not acknowledged is accepted whatever `cancel_requested`, the stop reason and the declared secrets say: today
  `checked_secret_report` refuses it for a cancel-requested step with declared secrets (round 1, correctness Major 3), on every
  minor. (2) The cause `NOT_ACKNOWLEDGED` joins the closed set of `WithdrawalCause` and its codec, so that the ADR-0098 build can
  replay a journal of this one and is the rollback floor. Both go to the coordinator; neither changes ADR-0098's decisions.
- **D006-060 — The frequency is unknown.** No loss of a frame on the wire or of a `PollTasks` answer has been observed; the
  three cases that started this work are ADR-0098's (design Major 3, correctness Minor 7). The owner chose to close the class
  anyway (OD-2026-11-20-12). At 1.8 `worker_leases_undelivered_total{cause="NOT_ACKNOWLEDGED"}` measures it; below 1.8 it stays
  invisible among the lapses.

### Gaps

- **G006-040 — A host that crashes between the verdict `ACKNOWLEDGED` and the start of the handler** leaves an acknowledged
  lease that lapses into a review. The window is inside the process and holds no call. A stopping host hands the lease back
  instead, an ordinary retryable failure.
- **G006-041 — An acknowledgement whose answer is lost, followed by the host giving the lease up,** turns into an ordinary
  retryable failure (the hand-back of an acknowledged lease consumes the attempt), or, when the hand-back cannot reach the server
  either, into a lapse and a review.
- **G006-042 — The trust assumption** (D006-053). A client at 1.8 that runs a handler without acknowledging and makes no call
  during the lease (a slow handler, a lost report, a crash) has its lease withdrawn and its work run again with the same attempt
  number and idempotency key. External effects stay at least once (constitution principle III); a later call with its token is
  counted and alerted; the refusals of D006-050 make most such clients fail loudly.
- **G006-043 — The internal runners stay below 1.8** (the database runner at 1.6, which design 041 also keeps below 1.7; the
  sandbox runner at 1; the command-line probe). A frame lost on the way to them still lapses.
- **G006-044 — For one registry heartbeat after an activation**, an engine that has not learned of it claims for a 1.8 session
  without the mark (that lease keeps today's meaning); after a removal, for one heartbeat, an engine may still mark leases, which
  is why the removal command waits before it expires the sessions.
- **G006-045 — The throughput cost and the call bound are predictions** until task T006-064 measures them.
- **G006-046 — The journal does not record that a lapsed lease had been acknowledged**; the counter
  `worker_leases_expired_total{mark}` does.
