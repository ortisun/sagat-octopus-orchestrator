# ADR-0099: A lease of worker protocol 1.8 is provisional until its host acknowledges it, and a provisional lease that expires is withdrawn

**Status:** Proposed
**Decided by:** agent, delegate (OD-2026-11-20-6), on the owner's answer "Yes, in 0.3.0" to delivery acknowledgement (owner decision OD-2026-11-20-12, which left the variant, the protocol minor and the deadline to this design). Reviewed on 2026-11-20 by the correctness, security, architecture and design reviewers: round 1 found 5 Blockers and 32 Majors, round 2 found 2 Blockers and 21 Majors, each answered in this text; round 3 is recorded beside them (`specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/`). Acceptance is the owner's, and with it the trust assumption of decision 12.
**Refines / supersedes:** refines ADR-0029 (decision 2 said a task stream introduces no durable delivery acknowledgement; decision 5 left a lease lost on the wire to the reaper; decision 6 dispatched a received lease at once), ADR-0098 (adds the cause `NOT_ACKNOWLEDGED`, a bound on repeated withdrawals of a marked lease, a requirement that depends on `lease-withdrawal-v1`, and makes `TASK_NOT_STARTED` a code that 1.8 reserves for the host, as the server review of pull request #260 decided), ADR-0082 (a requirement that depends on another, checked by the registry's trigger; an amendment line is added to ADR-0082 at acceptance) and ADR-0015 (adds minor 1.8 under its rule; the rule does not change). Supersedes none.
**Enforced by:** to be built with the decision (`specs/006-sdk-multilanguage/tasks.md`, phase "Delivery acknowledgement"): the barrier tests of the acknowledgement against each withdrawal in both orders (`tests/fault/sdk-multilanguage/`); the unit test of the decision function over every combination of marks, status and deadlines, and its parity test against the SQL condition; the matrix of the mark over the three statements that write a lease, including a lease written by a binary that does not know the columns (`integration_durable_execution`); the budget lines `ACKNOWLEDGEMENT` and `FIRST_HEARTBEAT` and the unchanged `LEASE` line (`integration_claim_transition_efficiency`); the descriptor-driven test that every worker method carrying a lease token, but the four that keep or end a lease, is refused on a provisional lease (`contract_grpc_durable_execution`); the registry trigger tests; the `promtool` cases of the alerts; the host's unit tests of the give-up clamp and the fallback; the conformance scenario `delivery-acknowledgement` and the override pass in six languages; the released-v0.2 scenario `provisional-lease-cutover` and its rollback case.

## Context

The engine commits a lease before it knows the task reached a host. ADR-0098 withdraws the leases the server knows it never
handed to the transport, and a host hands back the leases it drops (`TASK_NOT_STARTED`), which the server honours as a withdrawal
only from a session whose minor reserves the code, the minor this ADR designs (the server review of pull request #260); from an
older session it is an ordinary retryable failure. Two losses stay out of its reach, gap
G006-001 of `specs/006-sdk-multilanguage/research.md`: a frame of `StreamTasks` handed to the transport and lost on the wire,
and the answer of a unary `PollTasks` lost after its handler returned. Each lapses after the lease time-to-live and, since
feature 037, a lapsed lease of an `Activity` is an unknown outcome that goes to consultation or `step.needs_review`, for work
that never ran. No such loss has been observed yet: the three cases that started this work (two CI jobs and one play of feature
022) are ADR-0098's, where the server knew the frame never left the process. The owner chose to close the residue in release
0.3.0 with a new protocol minor in all six SDKs, knowing it changes the worker protocol's guarantees.

What constrains the choice:

- **Exactness.** The claim to make is "a handler of a lease runs only after the server has committed that the host has the
  lease"; from it follows "a lease the server withdraws was not run by a host that keeps the protocol".
- **Cost.** Throughput follows round trips (`specs/038-claim-transition-efficiency/research.md`, finding 1). A task costs a
  lease (38 statements, 2 transactions, 5 rows written) and a completion (50 statements, 1 transaction, 11 rows), the lines of
  `tests/integration/claim-transition-efficiency/work_budget.rs`. A task shorter than its heartbeat interval sends no heartbeat.
- **Locks.** A run's journal and state transitions are written under the continuation-chain lock and then the run row's lock,
  which `begin` takes and then restores the run's whole state. Not every write of a step row holds them: a binding revocation
  shortens leases with the binding's advisory lock only (`crates/infra-postgres/src/bindings.rs`). Each step row has its own
  row lock, which every writer of the row takes.
- **Older SDKs, later minors and released v0.2.** The server keeps serving every older minor exactly as it does (ADR-0015
  decision 1). Minors are cumulative, so whatever 1.8 asks of a session, every later minor asks too: drafts 020, 021, 025 and
  026 will take 1.9 onward. A release-0.2 binary does not register in the instance-capability registry.
- **1.7 first.** A session at 1.8 also speaks 1.7 (step logs, design 041, OD-2026-11-15-8). Neither the server nor any SDK
  speaks 1.7 yet.
- **One meaning per published version** (constitution principle IV): `timeoutMs` must mean the same at every minor.
- **Measured latencies.** Ready to handler entry, 99th percentile, 751 to 889 ms over the ten burst rows of five SDKs and both
  transports (`tests/performance/reports/sdk-streaming.md`); a completion, 6 to 103 ms at the 99th percentile over the nine
  rows of 2026-11-12 (`tests/performance/reports/scalability-ha.md`), on one machine with no network between the services and
  the database.

## Decision

1. **Minor 1.8 adds a flag on the lease and one call, and reserves one code (decision 15).** `TaskLease.acknowledge_before_start` (field 19) is set on a lease the
   claim marked provisional and on no other. A host accepted at 1.8 or later MUST NOT start a handler of a flagged lease, and
   MUST NOT make any call for it other than `AcknowledgeTasks` and the hand-back (`FailStep` with `TASK_NOT_STARTED`), before the
   server answered `ACKNOWLEDGEMENT_VERDICT_ACKNOWLEDGED` for it. It dispatches an unflagged lease at once, as today. The server
   offers 1.8 on the proofs of its build alone (once it serves 1.7), so no later minor depends on an operator's switch. The wire
   and the host procedure are `specs/006-sdk-multilanguage/stream-completion.md`, "Delivery acknowledgement (protocol 1.8)". 1.8
   is reserved in the register of `sdks/SPEC.md` §9 at the coordinator's instruction, an exception to the rule of
   OD-2026-11-15-8 ("when its contract merges") put to the owner; it is released if this ADR is not accepted.
2. **The mark is lease data bound to its generation.** A claim marks a lease provisional, in the statement that already writes
   the lease (any of the three: the root statement, the scoped statement of a full-run transaction, the path statement), when
   the session was accepted at 1.8, the engine knows the requirement `delivery-acknowledgement-v1` is recorded, the lease's earlier
   deadline (its expiry, or its attempt deadline when `timeoutMs` is set) is at least 18 s after the claim (decision 6), and the
   step's run of withdrawals is under the bound (decision 9). It sets `steps.lease_offered_generation` to the lease's generation
   and `steps.lease_offered_at` to the claim's time, and clears `steps.lease_acknowledged_at`. A mark *counts* while the step is
   `LEASED` or `RUNNING` and `lease_offered_generation` equals its `generation`, so a lease written later by any binary, including
   one that does not know the columns, is never taken for provisional. *Provisional*: the step `LEASED`, its mark counts,
   `lease_acknowledged_at` empty. *Acknowledged*: its mark counts, `lease_acknowledged_at` set. No statement writes
   `lease_acknowledged_at` from a value it read earlier: the engine's statements keep it or clear it.
3. **The acknowledgement writes one column under the step row's lock, and nothing else.** One transaction per call: the
   permission `worker:execute`; one unlocked read, by the step primary key, of the addressed rows whose lease-token hash matches;
   for each namespace among them, the checks `ensure_bound` makes, under the same locks (the binding's shared advisory lock, the
   namespace's shared secrets lock, the membership's row lock); then one conditional statement over the matched rows, in
   primary-key order, that sets `lease_acknowledged_at` and clears the step's run of withdrawals, only while the lease is still
   provisional with the request's token, attempt and generation, its cancellation not requested, and at least R = 6 s left before
   its earlier deadline by `clock_timestamp()`. It does not take the run lock, restore the run, renew the lease, apply a transition,
   append a journal event or write the outbox.
4. **Exactly one of the acknowledgement and a withdrawal commits.** Every withdrawal of a lease whose mark counts (the expiry or
   attempt deadline of decision 5, ADR-0098's hand-back and its `NOT_SENT` withdrawal) re-reads the step row `FOR UPDATE` under
   the run lock, decides with one pure function, and withdraws only a lease still provisional. The acknowledgement's statement
   takes the same row lock. Whichever takes it first wins; the other finds the row changed and does nothing (`NOT_HELD` for the
   acknowledgement, `fence_missed` for the withdrawal). The host runs only on `ACKNOWLEDGED`.
5. **A provisional lease is withdrawn at its earlier deadline; an acknowledged lease is an attempt.** When the first of its
   expiry and its attempt deadline passes, a provisional lease MUST be withdrawn as ADR-0098 decision 3 withdraws (one fenced
   transaction per run or path, `step.lease_withdrawn` with the cause `NOT_ACKNOWLEDGED`, the step back to `READY`, the attempt not
   consumed, no review); the next lease is a new claim with its own expiry and attempt deadline, so `timeoutMs` keeps its meaning.
   An acknowledged lease is never withdrawn: at its expiry or attempt deadline it keeps today's meaning (an unknown outcome,
   consultation or review), and a `TASK_NOT_STARTED` for it is an ordinary retryable failure.
6. **The acknowledgement deadline is the lease's own earlier deadline; a lease is marked only when that leaves 18 s.** The
   derivation is in `stream-completion.md` ("Why the deadline is the lease's own"). A host of 1.8 reads every lease within the
   receive bound R = 6 s of its claim (`sdks/SPEC.md` §10 for a stream; this decision extends it to `PollTasks` at 1.8), bounds
   each acknowledgement attempt to R, and the server acknowledges only with R left, so that the first heartbeat (at a third of
   the time left, at least 2 s) commits before the lease can expire. One read, one complete attempt and the margin fit when the
   earlier deadline is at least 3R = 18 s after the claim; at the default 30 s time-to-live with no attempt timeout, at most
   (30 − 6 − 6) / 6 = 3 complete attempts fit. A lease whose earlier deadline is shorter is not marked and keeps today's meaning.
7. **Cancellation before the start needs no write.** An acknowledgement of a provisional lease whose cancellation is requested
   answers `ACKNOWLEDGEMENT_VERDICT_CANCELLED` and writes nothing; the host hands the lease back, ADR-0098's withdrawal runs the
   settle, which cancels the step. If the hand-back is lost, the lease is withdrawn at its deadline and the settle cancels it then.
8. **A provisional lease receives no capability.** Every worker call that carries a lease token, other than `Heartbeat`,
   `CompleteStep`, `FailStep` and `AcknowledgeTasks`, MUST be refused on a provisional lease with `FAILED_PRECONDITION` and the
   stable reason `TASK_NOT_ACKNOWLEDGED`, after the token and the binding have matched, through a fence variant (root and path)
   that the report paths do not use. A contract test walks the descriptor, through nested, `oneof` and repeated fields, and fails
   when such a method has no case.
9. **Repeated withdrawals are bounded, and a host that cannot acknowledge falls back.** Every withdrawal of a lease whose mark
   counts increments `steps.lease_unacknowledged_withdrawals`; only an acknowledgement (and a reset) clears it; at 3 the next lease is not
   marked, so a fourth loss lapses as it does today. A host falls back, handing back what waits and registering again offering
   1.7 until it restarts, on `UNIMPLEMENTED` from `AcknowledgeTasks`, or after three calls in a row refused with another
   non-retryable status, or after three leases in a row given up with no `ACKNOWLEDGED` between them.
10. **The fleet decides, the operator records, the registry checks the dependency.** Every control plane and engine of the build
    lists `delivery-acknowledgement-v1` from the task that follows the measurement of decision 13; before it, the feature is
    dormant. `control-plane delivery-acknowledgement-activate` and `-remove` (owner role, one bounded transaction each,
    `--operator`, `--expect-live N`, `--dry-run`) check under the epoch lock that `lease-withdrawal-v1` is recorded, with a typed
    refusal; the registry's trigger checks the same as a backstop (an invoker function with schema-qualified names, which takes the
    epoch row first), and refuses removing `lease-withdrawal-v1` while `delivery-acknowledgement-v1` is recorded. The removal of
    `lease-withdrawal-v1` also refuses, in its command, while a mark counts. An engine marks leases only while it knows the
    requirement is recorded; a mark keeps its meaning until its lease ends.
11. **Older minors and internal runners are served exactly as today.** A lease of a session below 1.8, and an unflagged lease at
    1.8, are never provisional. The database runner, the sandbox runner and the command-line probe keep their minors.
12. **The trust assumption, stated for the owner.** The withdrawal of a provisional lease records a known outcome (nothing ran)
    because a host that declared 1.8 promised to start nothing before the acknowledgement; the server cannot verify that promise.
    Constitution principle III is applied to 1.8 leases under that assumption, for the withdrawals of decision 5 and for the
    hand-back of a provisional lease. What the server does see, it records, once per step and generation, in a counter and a
    bounded audit record: a `Heartbeat`, `CompleteStep` or `FailStep` other than the hand-back on a provisional lease; and any call
    other than a hand-back that carries the token of a lease withdrawn as `NOT_ACKNOWLEDGED` (`steps.withdrawn_lease_token_hash`).
13. **The cost is measured before the work in six languages.** Right after the server tasks, the transitions scenario runs at 1.8
    against 1.7 (the median of five interleaved runs each, with a network between the workers and the services) and the
    acknowledgement of a 64-lease frame is timed. The build goes on to the SDKs only if 1.8 is no more than 15% below 1.7, the call
    is answered within R / 2 = 3 s at the 99th percentile, and the 99th percentile of ready to handler entry rises by no more than
    the call's; otherwise it stops and the owner decides. These are gates of the build, not promises of the platform.
14. **What is counted** is listed in `stream-completion.md`, "Observability": verdicts, withdrawals by cause and outcome, leases
    left unmarked by reason, the delivery delay and the call's duration, provisional leases in flight, refused capability calls,
    protocol violations and calls after a withdrawal, with alerts on ratios, on the bound and on the delay.
15. **Minor 1.8 reserves `TASK_NOT_STARTED` for the host.** The code is the host's statement that no handler of a lease it
    received has run or will run; the host sends it only before any handler of that lease started (the hand-backs of
    `sdks/SPEC.md` §6.2 and §6.8), never for a task whose handler started. A handler cannot send it: the SDKs rewrite a
    handler-raised `TASK_NOT_STARTED` to `WORKER_ERROR`, keeping the handler's `retryable`, as they rewrite any code a handler may
    not use. From a session accepted at 1.8 or later, the server MUST withdraw the lease (cause `NOT_STARTED`, no attempt
    consumed, no review) when the step is still `LEASED` with the issued fence, the lease is not acknowledged (when its mark
    counts), and the step's withdrawals on `TASK_NOT_STARTED` for that attempt number are under the per-attempt cap the server
    applies; beyond the cap, or when any condition fails, it is an ordinary retryable failure with the flag the host sent. From a
    session below 1.8 it is always an ordinary retryable failure. The cap's value is derived by the server build of ADR-0098
    (pull request #260) and recorded in its decisions; this design names it and does not choose it. Every such withdrawal of a
    lease whose mark counts also increments the run of withdrawals of decision 9.

## Considered options

1. **A flag on the lease, a mark bound to its generation, and an acknowledgement that writes one column under the row lock
   (chosen).** Exact by the row lock (decision 4). One transaction of about 6 statements per call whatever the number of leases;
   no run lock, so it never waits behind a run's transaction and never holds two; no journal event; nothing paid where the operator
   has not activated, and nothing asked of later minors. A host can still hand back a provisional lease.
2. **The first round of this design: the acknowledgement under the run lock, renewing the lease.** Equally exact. The renewal is
   what needed the run lock, since the expiry, the timeout and the cancellation decide from the lease columns under it; with it
   came one transaction per run in the call, a call bounded by the slowest run, and a renewal channel that kept unmarked leases
   `LEASED` (round 2: design Major 1, security Major 2). Without the renewal the lease keeps its claim-time expiry, and the margin
   of decision 6 gives the first heartbeat the time the renewal gave.
3. **A host rule bound to the minor ("a session at 1.8 acknowledges every lease").** One field fewer, but offering 1.8 would then
   depend on the activation and on the fleet's time-to-live, and so would every later minor (round 2: architecture Blocker 1).
4. **Variant 1 as sketched in owner question O006-001: a new step status `OFFERED` and an acknowledgement event.** Equally exact;
   a status reaches every reader of `steps.status` and an event per task adds a journal append per task.
5. **Variant 2: the first heartbeat before the handler.** Equally exact, reuses `Heartbeat` and `step.started`; per task, one call
   and one transaction that restores the run's whole state, applies a transition and appends `step.started` (the build pins both
   lines); one more journal event for every short task; it still needs the flag.
6. **A shorter deadline of its own.** Recovers a lost lease sooner, but withdraws healthy deliveries when the database is slowest;
   an operator who wants faster recovery shortens the time-to-live, down to 18 s.
7. **Consume an attempt at the attempt deadline of a provisional lease** (the second round of this design). Keeps `timeoutMs`, but
   fails a step with one attempt for a delivery fault (round 2: correctness Major 4). Withdrawing at the earlier deadline keeps
   `timeoutMs` too, since the next lease is a new claim.
8. **The acknowledgement in a bidirectional stream, or carried by the next poll.** The first needs bidirectional streams in six
   SDKs and gives nothing for a unary poll; the second makes the handler wait for the next receive.
9. **Do nothing beyond ADR-0098.** Leaves G006-001. Rejected by the owner (OD-2026-11-20-12).

## Consequences

Good: a frame lost on the wire and a lost `PollTasks` answer go back to `READY` at the lease's earlier deadline instead of to an
operator, with the attempt number and the idempotency key unchanged; a provisional lease receives no secret, credential, blob or
log stream; a cancellation that arrives before the start is known; a process that crashes between a claim and ADR-0098's
withdrawal no longer leaves a review for a marked lease (gap G006-002 closes for them); the claim, the completion, the meaning of
every option and every later minor are unchanged; a lease's meaning never changes while it is held.

Bad: one more call and one more short transaction before the first handler of a frame starts; under light load that is one per
task, about 6 statements and 1 transaction more on the 88 statements and 3 transactions of a task today (102 against 94 round
trips with `BEGIN` and `COMMIT`), which the round-trip model of feature 038 predicts at about 8% of the transitions per second, an
estimate the gate of decision 13 measures; a task with an input blob reads it only after the acknowledgement. Five columns on
`steps`, two commands, one migration, a change of the registry's trigger. 1.8 waits for 1.7 in the server and in each SDK. Leases
whose earlier deadline is under 18 s are not protected (gap G006-047). Gaps G006-040 to G006-047 remain, of which the trust
assumption (G006-042) is the one the owner accepts with this ADR.

Compatibility with runs in flight: a lease issued before the activation is not flagged and keeps today's meaning; after it, only
leases claimed for sessions at 1.8 are flagged.

Rollback (`stream-completion.md`, "Rollback"): remove the requirement; within one registry heartbeat no engine marks a lease;
wait until no mark counts; only then may `lease-withdrawal-v1` be removed; roll the binary back no further than a build that
decodes the cause `NOT_ACKNOWLEDGED` (the ADR-0098 build, if it adds the cause to its closed set before it merges, as asked in
decision D006-059; otherwise this build). A session accepted at 1.8 meeting a binary below 1.7 is design 041's case, not a new one:
an unflagged lease asks nothing of 1.8.

## Evidence

`specs/006-sdk-multilanguage/research.md` decisions D006-040 to D006-061, gaps G006-040 to G006-047 and owner question O006-001;
`specs/006-sdk-multilanguage/stream-completion.md`, "Delivery acknowledgement (protocol 1.8)"; `sdks/SPEC.md` §5, §6.1 to §6.4,
§6.8, §9 and §10; ADR-0098 and the build decisions of its branch; `tests/performance/reports/sdk-streaming.md` and
`tests/performance/reports/scalability-ha.md`; `tests/integration/claim-transition-efficiency/work_budget.rs`; the review rounds
under `specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/`.
