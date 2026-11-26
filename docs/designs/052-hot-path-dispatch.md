# Hot-path dispatch: a bounded and spread poll, wake on commit, and a throughput gate

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-26

--------------------------------------------------------------------------------

## Overview

The assessment of 2026-11-11 (`docs/roadmap-assessment-2026-11-11.md`, "Remedies") listed hot-path remedies that have no
specification. [Design 038](038-claim-transition-efficiency.md) built the statement budget and the cheap drop of a
stale candidate. This design specifies the next three, chosen so that each costs no new table, no protocol field and no
new authority, and is measured before anything heavier is built:

1. **A bounded and spread poll**: a poll stops walking after a run of probes that met contention and fetches its window
   again from another point, and the polls of a process that met contention start at a point spread by the process, so
   that pollers of several replicas stop trying the same candidates.
2. **Wake on commit**: a parked worker stream claims again as soon as a step it serves becomes ready, instead of
   sleeping up to `idle_retry` (500 ms), within a process and, best effort, across processes.
3. **A throughput regression gate in CI**, beside design 038's statement budget, on the pull requests that change the
   hot path.

Two heavier remedies of this design's earlier drafts are not built in 0.3.0 and wait for this design's measurement
(Open Questions 2 and 3): a queue of ready steps claimed with `SKIP LOCKED`, and the lease of the next step in the answer
to a completion. Each of the three server remedies is a setting, off until its on and off measurement keeps it.

This design was the second part of design 051's first draft, whose review found it not ready. Its second draft was
reviewed in full on 2026-11-26 (`docs/designs/052-hot-path-dispatch.reviews/2026-11-26/`, where each report lists its
findings); this draft answers that round. The earlier drafts' identifiers Requirements 052-001 to 052-006, 052-010 to
052-012, 052-020 to 052-022 and Success criteria 052-001 to 052-004 are withdrawn and not reused; design 053's
Requirement 053-006 cites them in their earlier meaning (its Task 053-8 records it).

**Terms used below.**

- A *poll* (unary or a window of a stream) asks the server for work; a *claim* is the transaction that grants a lease;
  a *candidate* is a ready step a poll considers, read by the candidate scan (`claimable_paths`,
  `crates/infra-postgres/src/workers.rs`) with every filter the scan applies in SQL; the *window* is the list of
  `min(4 × wanted, 64)` candidates a poll reads, `wanted` being what it asks for; a *probe* is a claim transaction
  begun for one candidate (`claim_run`, `crates/application/src/worker/scoped_claims.rs`).
- The *run lock* is the row lock on the run that every transaction changing the run takes; a claim takes it with
  `SKIP LOCKED` (`RUN_FOR_CLAIM`, `crates/infra-postgres/src/execution.rs`). A probe *met contention* when that lock was
  skipped, the run had no ready step left, or the step was leased meanwhile; a probe that refused the step for a reason
  of this build or this session (a definition this build cannot read, gap G460; the protocol minor; a pool refusal;
  another refusal) did not.
- *Design 038's turn* (`crates/application/src/worker/poll_order.rs`, design 038's FR-038-004) starts a poll past the
  polls of its namespace in flight in the same process; a poll alone starts at the oldest candidate. Feature 019's
  FR-019-013 limits any rotation of the start to candidates of one priority.
- A stream *parks* when its claim leased nothing: it waits for a wake permit (Requirement 052-051) or `idle_retry`,
  whichever comes first, then claims again. *Discovery admission* is the stream's per-pass admission of a claim
  (`admission.admit_discovery`, `crates/api-grpc/src/service.rs`; ADR-0029, decision 3); a refusal ends the stream with
  `RESOURCE_EXHAUSTED` and a retry hint, as today.
- *Design 038's budget* is `tests/integration/claim-transition-efficiency/work_budget.rs`: 38 statements and 2
  transactions for a lease, 50 and 1 for a completion that makes the next step ready, at `develop` `546d5aa5`; *design
  038's selector* is its rule in `scripts/select-test-targets.py`.

## The Problem

From design 038's research (`specs/038-claim-transition-efficiency/research.md`) and the code at `546d5aa5`:

- Probes are two thirds of all database round trips. A poll reads its window with no row lock and opens one transaction
  per candidate. In the closed-loop scenario of 2026-11-12 there were 58 probes per lease, a scenario with more demand
  than supply in which workers spin (the research, item 5). Design 038's turn spreads pollers within one process only,
  and a poll walks its whole window, wrapping round, even after a run of probes that met contention; with four and
  eight API replicas `LWLock:LockManager` reached 4% to 11% of active backends
  (`tests/performance/reports/scalability-ha.md`).
- A stream that finds nothing sleeps `idle_retry` (500 ms, `crates/application/src/worker.rs`) and claims again; a
  stream window that ends by its 5 s deadline tells the SDK to reopen after `idle_retry` too; a unary poll that finds
  nothing returns `retry_after_ms = 500`. A step that becomes ready while its streams sleep waits up to 500 ms: up to
  1,500 ms of idle waiting in a chain of three under light load. Broker hints go through the outbox
  (`crates/scheduler/src/outbox.rs`), each an acknowledged JetStream publish at the scheduler's pace, and nothing
  subscribes to them.
- Two branches of the candidate scan (the eligible compensations and the forward pool-refused sample) lack the path
  filter of protocol minors below 5 that the other two branches have, so a session below minor 5 receives scoped
  candidates its claim then drops without a transaction, and design 038's counter counts them as probes.
- No CI job gates throughput. Design 038's budget gates statement counts, which would have caught the fourfold
  regression of November had it run on that change, but not a slowdown that adds no statement.

## Requirements

### The poll

- **Requirement 052-040 The scan's filters agree.** The eligible compensation branch and the forward pool-refused branch
  of the candidate scan MUST carry the path filter of the other two branches (`$5 >= 5 OR
  cardinality(st.instance_path_nodes) = 0`), and design 038's `probed` counter MUST count only a candidate for which a
  claim transaction was begun.
- **Requirement 052-041 A bounded walk that fetches again.** A poll MUST stop walking its eligible window after 8
  consecutive probes that met contention (probes that did not meet contention reset nothing and count nothing), in place
  of design 038's rule that a poll walks every candidate of its window (FR-038-004, refined: the order stays a
  preference, and no candidate is dropped, only deferred). A poll so stopped with untried eligible candidates fetches its
  window once more, starting past the candidates its first walk tried (its first start plus the number tried) and
  excluding the runs it already probed, and walks it under the same bound; it then walks the
  ineligible sample that ADR-0075's activation condition (d) puts behind every eligible candidate (`INELIGIBLE_SAMPLE`, 4,
  passed to `claim()` with the eligible window's length), uncounted; and if it leased nothing while candidates remain
  untried, a stream claims again after 50 ms instead of parking, once, before it may park, and a unary poll answers
  `retry_after_ms = 50`, unless the same session received that answer less than `idle_retry` ago, in which case it
  answers `idle_retry` (the last such answer kept per session in memory for `idle_retry`, at most one entry per session
  with a poll in that interval). The bound counts only when untried eligible candidates remain and no other ending applies (the
  session full, the consumer gone, the byte budget, `MAX_CLAIM_FAILURES`); errors count towards `MAX_CLAIM_FAILURES`
  only.
- **Requirement 052-042 A spread start under contention, per process.** The polls of a namespace in a process MUST
  start as design 038's turn says, plus, while the namespace is *contended* in the process, an offset of `r mod len`
  positions, `len` being the window's length and `r` the process's registry instance identifier (Architecture decision
  0082) read as an integer (its first eight bytes, little-endian), the rotation applied within one priority when
  feature 019 adds priorities (FR-019-013). A namespace is contended in a process from a probe there that met contention
  until a poll of the namespace there ends having met none, or 1 s after its last contended probe, whichever is first
  (a map of the namespaces contended in the last second, at most 10,000 entries, the oldest dropped first). The turn
  stays a permutation inside a process; processes differ by `r`; a poll of a namespace that is not contended keeps the
  scan's order. The offset is the server's, never the client's.
- **Requirement 052-043 Counted.** The poll MUST count, beside design 038's counters, the polls stopped by the bound,
  the windows fetched again and the polls started spread (`worker_poll_bounded_total`, `worker_poll_refetched_total`,
  `worker_poll_spread_total`, no label), and the statements of a poll (`stage="poll"` in design 038's
  `worker_claim_candidates_total`, for the authentication, the session read and each scan), so that statements per lease
  are measured, not only probes; and the wake counts its claims by outcome (`worker_wake_claims_total{outcome}`,
  `leased` or `empty`), its messages published, dropped, received, rate-limited, refused for an absent key and coalesced, and
  its local-only starts and checks, with closed labels; a woken claim carries the span event `woken` (no tenant
  attribute), and the publish and the receipt each a span of their own.

### Wake on commit

- **Requirement 052-050 A registry without a runtime.** The application MUST keep, per process, a wake registry
  (`crates/application/src/worker/wake.rs`) of per-key permits behind a standard mutex, a key being (tenant, namespace,
  activity), with no `tokio` feature beyond `rt` (ADR-0092's check unchanged): a stream registers its keys from before its claim
  begins until it leaves its park (or its window ends), an entry exists only while a stream is registered for its key,
  so the registry holds at most the stream permits times `MAX_ACTIVITIES` entries, and goes with the last registration.
  Parking, its timer and the coalescing (the parked streams of a key are woken at most once per 10 ms; a pass hands out
  the permits gathered meanwhile) are in `crates/api-grpc`, which already parks with `tokio::time`.
- **Requirement 052-051 Permits, not versions.** A commit that makes `k` steps of a key ready MUST add `k` permits to
  the key's registered entry (none when no stream is registered), the permits of a key never more than the streams
  registered for it, and a claim that leases `n` tasks of a key removes up to `n` of its permits, so that a fan-out
  leased by one stream leaves no permit behind; a stream about to park MUST take a permit if one is
  there and claim again, or park, and a claim on a permit validates the session in its transaction as every claim does
  today, so a session revoked while its stream was parked leases nothing; a parked stream is woken by a permit added while it waits, and a permit added between
  its empty claim and its park is taken by the check before the park, so no wake is lost. A stream whose claim on a
  permit leased nothing takes no further permit until `idle_retry` has passed, and a stream whose session is full or
  whose window is drained parks without registering. A barrier test commits after a stream's claim has scanned and before its
  transaction ends, and sees the stream claim again at once; another makes one step ready with eight streams ending
  empty claims and sees at most one more claim; a third makes five steps ready, lets one stream lease them all, then
  makes a sixth ready within `idle_retry` and sees it leased at once.
- **Requirement 052-052 Published after a successful commit, from every writer.** Every transaction that writes a
  `task.ready` outbox row (`crates/infra-postgres/src/execution.rs`, `execution/path.rs`, `chain_progress.rs`,
  `loop_progress.rs` and `primitives.rs`) MUST collect its keys and counts as it writes the row and, inside its
  `commit()` after the commit succeeded, hand them to the process's wake sink (`WakeSink`, a port of the application),
  never on a failed commit; the sink adds the local permits and queues the cross-process wake. One sink per process is
  handed to every store and to the gRPC service.
- **Requirement 052-053 A best-effort wake across processes, on the selected transport.** With an internal event
  transport configured, the sink MUST publish one message per key per commit through a port on that transport (core
  NATS publish, or Redis `PUBLISH`), never through the outbox and never awaiting the broker: it `try_send`s into a
  bounded queue drained by one task on a connection of its own, and drops and counts what does not fit or cannot be
  sent. Each execution engine subscribes on a connection of its own and, for each message, looks the key's registry entry up
  first: a message for an absent key touches no state; otherwise, unless the message is the process's own (its payload, the publishing
  instance's identifier, equals the receiver's), it adds one permit, at most 20 per second per key (a token bucket kept
  inside the entry). One message adds one permit whatever the number of steps the remote commit made ready, so a remote
  fan-out wider than one stream's credit leaves the other streams to `idle_retry`. The control plane, whose run starts make steps
  ready, publishes and does not subscribe. A lost wake costs at most `idle_retry`; the paths that make work leasable
  with no commit of a step (a breaker closing with time, quota freed by a lease's expiry, a binding added) stay bounded by
  it. Without a transport, the wake covers the commits of the same process only.
- **Requirement 052-054 The wake's channel is a contract, and only the platform may use it.** The message is an
  internal channel of `specs/001-platform-foundation/contracts/events.asyncapi.yaml`, unstored, under the contract
  baseline check: subject (NATS) or channel (Redis) `sagat.wake.v1.<tenant>.<namespace>.<activity token>`, the tenant and
  namespace as their identifiers and the activity as the hexadecimal of the first 16 bytes of its name's SHA-256, outside
  the platform stream's filter `sagat.v1.>` so that JetStream never stores it; the payload is the publishing instance's
  identifier (16 bytes), read only to drop a process's own messages; the key is read from the subject alone, a
  malformed message dropped and counted. The cross-process wake is on only when the broker authenticates the
  platform with a wake user of its own, on a connection and credentials of its own (`nats.wakeCredentialsSecret`, or a
  Redis wake URL secret, `redis.wakeUrlSecret`; the outbox keeps its own user, which needs the JetStream API), whose
  permissions are: for engines, publish and subscribe on `sagat.wake.v1.>`; for the control plane, publish only; no
  JetStream API and no other subject (NATS), no other command, channel or key (Redis ACLs, asserted by the chart's contract test: engines
  `-@all +publish +subscribe +psubscribe resetchannels &sagat.wake.v1.*`, the control plane
  `-@all +publish resetchannels &sagat.wake.v1.*`); and
  when no other user or anonymous connection may publish or subscribe on those channels (Redis's `default` user disabled
  or given `resetchannels`; no NATS `no_auth_user` and no open account). The chart refuses to render the wake on without
  the wake's credentials secret, outside the development profile; and each process checks at start, before it turns the
  cross-process wake on, that the broker refuses an anonymous connection (NATS), or that `ACL WHOAMI` is not `default`
  and an unauthenticated connection's `SUBSCRIBE` and `PUBLISH` on the probe channel `sagat.wake.v1.probe` are both
  refused (Redis); otherwise it stays local-only, counts it, logs why, and checks again with a backoff capped at 5 minutes, each
  attempt counted. Compose's
  development broker is the development profile. The control plane's egress to the broker is opened only when the wake
  is on (`infra/helm/templates/networkpolicy-egress.yaml`).
- **Requirement 052-055 A deadline window reopens at once.** A stream window that ends by its deadline with credit left
  MUST carry `retry_after_ms = 50` in its terminal frame (the SDKs clamp below 50 ms to 50, and read 0 as 1,000), so that
  a worker under light load is registered again within about 50 ms; a window that ends otherwise carries today's hint.

### The gate

- **Requirement 052-030 Throughput gate.** A pull request that changes the hot-path files (design 038's selector, which
  sets a new `throughput_gate` output of `scripts/select-test-targets.py`, also set by a change of the gate's own files)
  MUST run, in a job `throughput-gate` on `pull_request` only, on the standard hosted runner, with
  `permissions: contents: read`, no `pull_request_target` or `workflow_run`, its results as a job summary and an
  artefact, nothing committed back, `timeout-minutes: 60` and a concurrency group per pull request that cancels the run
  in progress, the in-process transitions scenario of `tests/performance/scalability-ha/` in one configuration (one
  replica and 16 workers, `SAGAT_PERF_TRANSITION_RUNS=1000`, whose workers end when every run's three transitions are
  complete and sleep the SDK's `receive_delay(retry_after_ms)` on an empty poll), its metric the completed transitions
  per second written to a JSON file, for the merge base and the head in the order A B B A A B B A A B B A (six pairs of
  adjacent runs), each on a fresh database, both driven by the merge base's scenario and configuration (a pull request
  that changes the scenario gets the gate's verdict "not comparable", which fails the job until the scenario's change
  merges on its own). `scripts/throughput-gate.py` (with tests in `scripts/tests/`), read with the
  threshold from the merge base's tree, fails the job when the head is slower in all six pairs and the median of the six
  paired differences exceeds the threshold T = 1.425 × sd, sd being the standard deviation of the paired differences of
  a pilot of 30 A/A′ pairs (two builds of the merge base differing by a no-op commit) on that runner; 1.425 is
  (t(0.975, 5) + t(0.8, 5)) / √6 = (2.571 + 0.920) / 2.449, a heuristic, since the sign rule decides first. Under no
  change the sign rule alone fires with probability 1/64, about 1.6%; the pilot's report states the false-positive rate
  and the power at a 15% loss of transitions per second, estimated by resampling its pairs with that loss injected; if
  that power is below 0.8, the gate runs ten pairs (A B B A five times) and fails on nine of ten slower (probability
  11/1024, about 1.1%, under no change) and the median beyond T10 = (t(0.975, 9) + t(0.8, 9)) / √10 × sd =
  (2.262 + 0.883) / 3.162 × sd = 0.995 × sd. Both factors are a paired t-test's, applied to the median as a heuristic. A one-statement regression is the statement budget's to catch; the scenario uses unary polls, so a
  slowdown of streams or of the wake, or one from a migration or a dependency's update, is the release benchmark's.

### Success criteria

- **Success criterion 052-005** With two application instances and eight pollers each on one activity (`wanted` 16),
  in an open-loop benchmark run at 80% of the maximum sustained rate (the highest start rate at which completions keep
  pace for three minutes), statements per lease and probes per lease fall from today's values, measured first in the
  same benchmark run, probes per lease to at most 4, and leases per second do not fall; above 4, Open Question 2's queue
  is written.
- **Success criterion 052-006** With wake on commit and one worker per activity, in the chain of three at 10% of its
  maximum sustained rate on the free standard runner for ten minutes, with two application instances and runs started
  through the instance that does not hold the worker's streams, the 99th percentile of the time from a step becoming
  ready to its lease is under 50 ms, against up to 500 ms today, reported for the first step (whose wake crosses
  instances) and for the others (woken in the process of the completion) apart; the medians are reported.
- **Success criterion 052-007** The gate fails on a hand mutation that removes 15% of the scenario's transitions per
  second and passes on its revert, both seen in CI, and the pilot's false-positive rate and power are reported.
- **Success criterion 052-008** Each server remedy is measured on and off, five benchmark runs each, on the free standard
  runner, and is removed if it improves neither the maximum sustained rate nor the 99th percentile latency by more than
  the range of its five runs.

## Goals

- A poll's cost follows what it leases, across replicas, not the number of pollers.
- A step's wait for a lease under light load is not bounded below by a polling interval.
- A slowdown of the hot path fails the pull request that causes it.

## Non-Goals

- **No change of the journal, of the transaction boundary or of who decides a lease.** The run lock and the run's
  reducer decide, as today.
- **No new table and no protocol change in 0.3.0**: the queue and the next-step lease wait for measurement (Open
  Questions 2 and 3). The wake's channel is internal to the platform's processes.
- **No hints for work re-armed without a commit of a step**: `idle_retry` bounds them.
- **No push dispatch or worker routing** (feature 019's). **No long polls.**

## Open Questions

Questions 1 to 3 were decided under the delegation of Owner decisions 2026-11-20-6 and 2026-11-24-4, with their
reasoning; question 4 is the owner's own decision.

1. **Is this design in release 0.3.0's scope?** Yes (Owner decision 2026-11-24-513): the owner wants to surpass Temporal
   and Windmill on performance. Its remedies are justified by their own on and off measurement on the free standard
   runner (Success criterion 052-008), not by design 051's paid campaign, which is unscheduled (Owner decision
   2026-11-25-1).
2. **A queue of ready steps?** Not in 0.3.0 unless Success criterion 052-005 is missed. The earlier drafts' queue needed
   what its review listed (`critic_report.md`, `zoom_out_report.md`): the scan's filters joined under the row lock, one
   row per run and activity for fan-out, an unlocked peek before a single locked row, a safe switch and backfill, a
   counted budget and a trigger that can change under design 053's rules; a design of its own, written only if the
   measurement asks for it (Owner decision 2026-11-24-525).
3. **The next step in the answer to a completion?** Not in 0.3.0. A woken stream already receives its next task by push,
   so what remains is one claim transaction; the remedy would be a second lease producer (its locks after the
   completion commits, under ADR-0098 and ADR-0099's withdrawal rules, marked provisional and acknowledged from protocol
   1.8), in a protocol minor behind those `sdks/SPEC.md` already reserves (1.9 for design 045, 1.10 to 1.12 for draft
   019), in six SDKs. A follow-up only if Success criterion 052-006 shows the claim after a wake dominates (Owner
   decision 2026-11-24-525).
4. **Does the CI gate need a constitution amendment?** Yes: the constitution enumerates the suites a pull request runs
   ("and only those"); the owner approved the gate for constitution 5.1.0 (Owner decision 2026-11-25-3, point 3), which
   the coordinating session is drafting (its placement in 5.1.0 was Owner decision 2026-11-24-512). Task 052-10 checks
   that its text admits the gate as Requirement 052-030 writes it; Task 052-11 waits for the merge.

**Gap 052-G1 (security, found by this design's review, outside its scope).** A tenant's NATS or Redis connector runs in
the engine's pod, whose network policy reaches the platform's broker; the connectors' guard checks the addresses a name
resolves to and the client then resolves the name again (a rebinding passes it), and the NATS client follows the servers
a broker advertises; a tenant administrator may allow a private range that covers the broker. Requirement 052-054's
credentials and permissions keep the wake's channel to the platform regardless; the guard's fix (pin the checked
addresses, `ignore_discovered_servers()`, keep the platform's stores out of tenants' egress rules as ADR-0035 decision 4
does for others, the platform's broker addresses among those the guard must exclude) is feature 004's, handed to its
research register by Task 052-12.

**Gap 052-G2 (feature 019's order).** Feature 019's FR-019-013 says a lone poller is served strictly in priority and
`ready_at` order. Under Requirement 052-042 that holds while its namespace is uncontended in the process; a poller alone
in the fleet whose own completions or timers hold run locks can be spread for up to 1 s, and its order then holds within
one priority only. FR-019-013's wording is feature 019's to settle (Task 052-12); feature 019's conformance ordering
scenario has no concurrent holder of a run lock, so it is unaffected.

## Owner decisions needed

None open. The constitution's amendment is approved (Owner decision 2026-11-25-3, point 3) and only its merge is
awaited (Task 052-11). The owner pays for no runner in 0.3.0 (Owner decision 2026-11-25-1, "Nada pago"): every
measurement here runs on the free standard runner, where an on and off pair on one machine is a fair comparison; this
design claims no number measured on a machine it was not run on.

## Critical User Journeys (CUJs)

### CUJ 1: Many workers on one activity across replicas

*   **Given**: 16 workers on one activity through two application instances, and a backlog larger than they ask for.
*   **When**: they poll at once.
*   **Then**: polls that meet contention set their process's flag, and the next polls of each process start at a point
    spread by the process; a poll that meets contention in 8 probes in a row fetches its window again past what it tried
    start, and a poll that leased nothing with work left asks again after 50 ms.

### CUJ 2: A chain under light load

*   **Given**: one worker per activity of a chain of three, its streams parked on instance B; the run is started through
    instance A.
*   **When**: the run starts.
*   **Then**: A's commit publishes a wake that B receives; the first activity's stream claims at once; its completion
    commits on B and wakes the second activity's stream through B's registry, and so on.

### CUJ 3: A slow change to the hot path

*   **Given**: a pull request that touches the claim and slows it without adding a statement.
*   **When**: CI runs the gate.
*   **Then**: the head is slower in all six pairs and by more than the threshold, and the pull request fails.

## Technical Plan (Architecture)

**The poll.** `claim()` in `crates/application/src/worker.rs` receives the eligible window's length, counts consecutive
probes that met contention (`claim_run` reports which), stops at 8, fetches once more, then walks the ineligible
sample; `poll_order.rs` adds the process's offset while the namespace's in-flight entry carries the contention flag.
The scan's two branches gain the missing filter.

**Wake.** `WakeRegistry` and the `WakeSink` port in `crates/application/src/worker/wake.rs`; the stores collect keys
where they write `task.ready` and call the sink inside `commit()`; the gRPC stream loop keeps its `SessionRecord`,
registers its activities from before its claim until it leaves its park, takes a permit or parks with its timer; a `NatsWake` (core client, publish
and subscribe) in `crates/infra-nats` and a `RedisWake` in `crates/infra-connectors/src/redis.rs`, beside the platform's Redis publisher and on a connection of its own, implement the transport port; the engine
builds one registry and sink and hands them to every store and to the gRPC service; the control plane builds a
publish-only sink.

**The gate.** `scripts/throughput-gate.py` and its threshold constant, the scenario's JSON output, the selector's
`throughput_gate` output and the `changes` job's, a `workflow_dispatch`-only pilot workflow, the job.

| Change | Files | Requirement |
|---|---|---|
| The scan's filter and the counter | `crates/infra-postgres/src/workers.rs`, `crates/application/src/worker/claim_counters.rs`, `crates/application/src/worker/scoped_claims.rs` | 052-040 |
| The bounded and spread poll and its counters | `crates/application/src/worker.rs`, `crates/application/src/worker/poll_order.rs`, `tests/integration/claim-transition-efficiency/` | 052-041, 052-042, 052-043 |
| The registry, the permits and the parking | `crates/application/src/worker/wake.rs`, `crates/api-grpc/src/service.rs` | 052-050, 052-051, 052-055 |
| The sink in every writer of `task.ready` | `crates/infra-postgres/src/execution.rs`, `execution/path.rs`, `chain_progress.rs`, `loop_progress.rs`, `primitives.rs` | 052-052 |
| The transports, the processes and their configuration | `crates/infra-nats/src/lib.rs`, `crates/infra-connectors/src/redis.rs`, `apps/execution-engine/src/{lib,settings,cli}.rs`, `apps/control-plane/src/{config,lib}.rs`, `infra/helm/templates/` (both components, the wake's credentials secrets, the control plane's egress rule), `infra/helm/values.yaml`, `infra/docker/compose.yaml`, the chart's contract test, `docs/runbooks/execution-engine.md`, `docs/runbooks/events.md` | 052-053, 052-054 |
| The channel's contract | `specs/001-platform-foundation/contracts/events.asyncapi.yaml` and its baseline | 052-054 |
| The scenario, the gate and the selector | `tests/performance/scalability-ha/`, `scripts/throughput-gate.py`, `scripts/tests/`, `scripts/select-test-targets.py`, `.github/workflows/ci.yml`, a pilot workflow, `tests/performance/reports/throughput-gate.md` | 052-030 |

**Settings.** `SAGAT_POLL_BOUND` (an integer, 8; 0 turns the bound off), `SAGAT_POLL_SPREAD` and `SAGAT_WAKE` (booleans),
parsed in the engine's and the control plane's settings with their help text, carried in `WorkerSettings`, each off
until Task 052-9 keeps it, then on by default. **Compatibility and rollback.** Nothing is added to the schema or the
protocol; a rollback is a setting, and a mixed fleet of releases needs no capability (design 053, Requirement 053-006):
an older process walks its window, sleeps its `idle_retry` and does not subscribe to the wake's channel, whose payload
is the publishing instance's 16-byte identifier (a reader ignores any bytes after the 16th and drops a message whose
subject it cannot parse).

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| The bound defers eligible work | Only contention counts; a second fetch from another point; a 50 ms retry with work left; the counters |
| Unreadable or refused runs at the head end every poll | They do not count towards the bound |
| The spread breaks the oldest-first order | Only within a second of contention in the process; an uncontended poll keeps the order |
| Wakes spin streams | Permits, capped by the registered streams and debited by leases; no permit for a stream whose permitted claim leased nothing; full streams unregistered; own messages dropped |
| A forged or flooded wake | The platform's own broker user and channel permissions; one permit per message, 20 per second per key |
| The broker stalls a request | A bounded queue drained by one task; drops counted |
| A lost wake | `idle_retry` stays the upper bound; the claim suite runs a wake-off case and the lost-wake test |
| The gate flakes or is gamed | A pilot of 30 A/A′ pairs, six of six pairs and the threshold, read from the merge base |
| The cheap remedies are not enough | Success criterion 052-005's threshold writes the queue's design |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | No transition changes; a lease is decided under the run lock as today |
| II. Modular domain, ports and adapters | Pass | The poll's order, the bound and the registry in the application without a runtime; the transports behind the `WakeSink` port in the adapters; parking in `api-grpc` |
| III. External effects are at-least-once; sagas compensate | Pass | Unchanged |
| IV. Compatibility is versioned and pinned | Pass | No schema or protocol change; the wake's channel versioned in the AsyncAPI contract with a reader rule |
| V. Code, tenant boundary and secrets are isolated | Pass | The channel's key from the subject alone, no activity name on the wire, the platform's own broker user with channel permissions; gap 052-G1 recorded for the connectors' owner |
| VI. Observability and audit are part of the feature | Pass | Bounded, refetched and spread polls, poll statements, wakes published, dropped, received, rate-limited and coalesced, a span event "woken" with no tenant attribute, all with closed labels |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | The wake carries no record, is never the only signal of work, and is not an outgoing message in this principle's sense: the `task.ready` outbox row stays the durable signal (ADR-0134 refines ADR-0002 and ADR-0069) |
| VIII. Failure is tested and contracts are verified, honestly | Pass | The barrier tests, the broker stopped, the commit failed, the suite with the wake off, the gate's mutation and pilot, each seen failing first |
| Delivery Constraints | Pass with conditions | Constitution 5.0.0 as merged; the CI gate needs the CI gates of constitution 5.1.0, approved by the owner (Owner decision 2026-11-25-3, point 3) and being drafted, so Task 052-11 waits for its merge |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| A bound, a second fetch and a spread start | Probes per lease across replicas | Nothing: the walk stays at 64 |
| A wake registry, permits and a transport port | Up to 500 ms of idle per step under light load, across replicas | A shorter `idle_retry`: an empty claim per idle stream per interval |
| A throughput gate | A slowdown without an added statement passes the budget | Release-time benchmarks only |

## Alternatives Considered

- **A queue of ready steps claimed with `SKIP LOCKED`** (this design's earlier drafts). Open Question 2.
- **The queue as the authority** (Windmill's model). One statement per claim; it moves the lease decision out of the
  run's transaction and changes every guard of features 002, 009 and 010.
- **The next step in the answer to a completion** (the earlier drafts). Open Question 3.
- **Jumping to another offset after k empty probes, without fetching again.** The window is a snapshot shared with the
  polls in flight; its later candidates are as likely leased as its first.
- **A session's hash for the spread** (this design's second draft). Two sessions of one process collide one time in
  four, which breaks design 038's turn inside a process; the process's identity keeps the turn a permutation.
- **`LISTEN` and `NOTIFY` for the wake.** Its commit-time lock serializes the commits of every notifying transaction.
- **The outbox's `task.ready` hint for the wake.** An acknowledged JetStream publish at the scheduler's pace.
- **A shared turn across processes.** A hot counter per namespace.

## Non-Functional Requirements

- **Non-functional requirement 052-001 Hot path.** A lease and a completion stay within design 038's budget unchanged:
  the bound and the spread change only which candidates a poll tries; the wake's publish is after the commit, outside
  the database and never awaited.
- **Non-functional requirement 052-002 Security.** No new database grant; a wake carries identifiers and an opaque
  activity token only, on the `sagat.wake.v1.` channels, through the platform's own broker user; a forged wake, if the
  broker's permissions were bypassed, costs at most 20 claims per second per key and process, each re-checked by the
  claim's authority and binding checks.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

- [ ] T052-1 Amend `specs/006-sdk-multilanguage/stream-completion.md`'s rule on `retry_after_ms` for a window ended by its deadline (Requirement 052-055); revise Architecture decisions 0133 (the bounded and spread poll, refining design 038's FR-038-004, within Owner decision 2026-11-12-3 as Owner decision 2026-11-24-526 records) and 0134 (the wake, refining ADR-0002, ADR-0029 and ADR-0069); update the claim paragraph of `docs/architecture/execution-semantics.md`, the wake line of `docs/architecture/worker-runtime-protocol.md`, the comment of `poll_order.rs`, and add a follow-up task to design 038 noting FR-038-004's refinement; add rows for the new checks to `docs/engineering/architecture-principles.md`.
  Refs: Requirement 052-041, Requirement 052-042, Requirement 052-053
- [ ] T052-2 Add the missing path filter to the two branches of the candidate scan and count `probed` only with a transaction begun, with the test `a_minor_four_session_reaches_its_forward_work_behind_nine_scoped_compensations` and the counters' test `probed_counts_only_a_begun_claim_transaction`, beside `a_poll_counts_each_stage_once_with_the_stage_as_its_only_label`.
  Refs: Requirement 052-040
- [ ] T052-3 Bound the walk, fetch again, walk the ineligible sample uncounted (in the ChildRoot capacity guard's suite, `tests/integration/workflow-primitives/`, the guard being off in production), and the 50 ms retry, with `SAGAT_POLL_BOUND`, the counters and the tests `a_poll_stops_after_eight_probes_that_met_contention` (`wanted` 3 or more, the ninth candidate still `READY`, `probed` 8), `the_second_fetch_starts_past_what_the_first_tried` (with an instance identifier of offset 0, set by the test constructor, and the namespace marked contended by a test hook), `two_bounded_unary_polls_in_a_row_answer_fifty_then_idle_retry`, `unreadable_runs_ahead_do_not_end_the_poll` (nine runs whose definition this build cannot read before one it can), `a_static_backlog_behind_eight_locked_runs_is_leased_within_one_hundred_milliseconds` and `the_ineligible_sample_is_walked_after_the_bound`.
  Refs: Requirement 052-041, Requirement 052-043
- [ ] T052-4 Spread the start by the process while a poll of the namespace met contention, with `SAGAT_POLL_SPREAD`, a unit test of the offset on fixed identifiers, `a_poll_alone_keeps_the_order_of_the_scan` and `a_lone_poll_is_served_oldest_first` kept, `k_polls_in_flight_together_probe_once_per_lease` kept unmodified, `a_second_round_after_contention_probes_once_per_lease_in_each_of_two_instances` (two application instances on one database with identifiers set by a test constructor to offsets at least K apart modulo the window's length, round 2 started while round 1's locks are still held so that the namespace stays contended, a contended first round, then K pollers per instance asserting K probes for K leases), `one_poller_per_instance_stays_spread_within_a_second_of_contention` and `the_contended_map_drops_its_oldest_entry_past_ten_thousand`; the report of Task 052-9 records the instances' offsets.
  Refs: Requirement 052-042, Requirement 052-043
- [ ] T052-5 Add the registry, the permits, the parking with its timer, the unregistered full streams, the deadline window's hint and `SAGAT_WAKE`, with, in the target `integration_hot_path_wake` of Task 052-6 (created here), the tests `a_commit_between_an_empty_claim_and_the_park_wakes_the_stream` (a park probe beside the `ConsumerWatch` probes, `idle_retry` 60 s, a lease within 1 s), `one_step_made_ready_wakes_at_most_one_of_eight_streams`, `a_second_step_after_a_fan_out_is_woken_at_once`, `the_permits_of_a_key_never_exceed_its_registered_streams`, `a_full_stream_is_not_woken_and_a_free_one_is`, `a_stream_whose_woken_claim_leased_nothing_waits_its_idle_retry`, `a_deadline_window_asks_to_reopen_within_fifty_milliseconds` and `the_registry_holds_no_entry_without_a_registered_stream`.
  Refs: Requirement 052-050, Requirement 052-051, Requirement 052-055
- [ ] T052-6 Collect the keys in every writer of `task.ready`, call the sink after a successful commit, add the NATS and Redis transports with their bounded queues and connections, the control plane's publish-only sink, the channel in the AsyncAPI contract, the broker users and permissions in Helm and Compose, the chart's refusal, and, in `integration_claim_transition_efficiency` (which the constitution already runs for a change of lease or completion code), the wake-off case `the_claim_suite_passes_with_the_wake_off`, with the tests `a_wake_from_another_instance_claims_at_once` (a registry per instance, the NATS-received counter asserted), `a_failed_commit_publishes_nothing`, `the_wake_prefix_is_outside_the_stream_filter`, `a_stopped_broker_does_not_slow_a_completion`, `a_wake_flood_is_limited_per_key`, `a_wake_for_an_absent_key_touches_no_state`, `a_process_drops_its_own_wake`, `an_open_broker_keeps_the_wake_local`, `a_session_revoked_while_parked_leases_nothing_on_a_wake`, `a_lost_wake_is_bounded_by_idle_retry` (seen failing first on a park that waits for the wake alone), and the telemetry vocabulary test over the new counters; these tests, and Task 052-5's, in a new target `integration_hot_path_wake` of `tests/Cargo.toml`, which a new rule of `scripts/select-test-targets.py` (with its selector test) selects for changes of `crates/application/src/worker/wake.rs`, `crates/api-grpc/src/service.rs`, `crates/infra-nats/` and the Redis transport; the Redis transport's `a_redis_wake_from_another_instance_claims_at_once` and `an_open_redis_keeps_the_wake_local` (the `default` user enabled) in `integration_redis_streams`.
  Refs: Requirement 052-052, Requirement 052-053, Requirement 052-054
- [ ] T052-7 Give the in-process transitions scenario its gate configuration (workers end on completions and sleep the SDK's `receive_delay`), a JSON output, and an open-loop mode (runs started at a fixed rate, the maximum sustained rate found by stepping it) in `tests/performance/scalability-ha/`.
  Refs: Requirement 052-030, Success criterion 052-005, Success criterion 052-006
- [ ] T052-8 Write `scripts/throughput-gate.py` with its tests, the selector's `throughput_gate` output with its tests and the `changes` job's output, and a `workflow_dispatch`-only pilot workflow, merged to `develop` before it runs (a dispatched workflow must be on the default branch); run the pilot of 30 A/A′ pairs, A the `develop` commit it is dispatched on and A′ that commit plus a no-op commit on a branch of the pilot's own, and record sd, the false-positive rate and the power at a 15% loss in `tests/performance/reports/throughput-gate.md` and the script's constant.
  Refs: Requirement 052-030
- [ ] T052-9 Measure each server remedy on and off, five benchmark runs each, on the free standard runner, Success criteria 052-005 and 052-006 with two application instances, and record the gain or the removal in `tests/performance/reports/hot-path-dispatch.md` with the runner named; when Success criterion 052-005 is missed, open the queue's design (Open Question 2).
  Refs: Success criterion 052-005, Success criterion 052-006, Success criterion 052-008
- [ ] T052-10 When constitution 5.1.0 merges, check that its CI gates admit the throughput gate as Requirement 052-030 writes it, and record the merge commit here.
  Refs: Requirement 052-030
  Owner-gate: pending, the merge of constitution 5.1.0 (approved by Owner decision 2026-11-25-3, point 3)
- [ ] T052-11 After Tasks 052-8 and 052-10, add the `throughput-gate` job and prove it fails on the hand mutation and passes on its revert.
  Refs: Requirement 052-030, Success criterion 052-007
  Owner-gate: pending, the merge of constitution 5.1.0 (approved by Owner decision 2026-11-25-3, point 3)
- [ ] T052-12 Hand gap 052-G1 to feature 004's research register and gap 052-G2 to feature 019's, each under its owner's next free gap number, checked against the other open branches.
  Refs: Requirement 052-054

#### Follow-up tasks

None yet.

### Verification Plan

Each pull request runs `integration_durable_execution` (also as the runtime role), design
038's budget target `integration_claim_transition_efficiency`, the fault suites of features 002 and 010, `cargo clippy
--workspace --all-targets --locked -- -D warnings` and the repository's checkers (`scripts/check-*.py`). The seven
guards that design 038 lists in its Verification Plan pass unmodified. Each new test is seen failing first, with a hand
mutation where the behaviour already exists.
