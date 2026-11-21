# ADR-0099: A lease of worker protocol 1.8 is provisional until its host acknowledges it, and a provisional lease that expires is withdrawn

**Status:** Proposed
**Decided by:** agent, delegate (OD-2026-11-20-6), on the owner's answer "Yes, in 0.3.0" to delivery acknowledgement (owner decision OD-2026-11-20-12, which left the variant, the protocol minor and the deadline to this design). Reviewed on 2026-11-20 by the correctness, security, architecture and design reviewers: round 1 found 5 Blockers and 32 Majors over the four reports, answered in this text; round 2 is still to run (`specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/`). Acceptance is the owner's, and with it the trust assumption of decision 13.
**Refines / supersedes:** refines ADR-0029 (decision 2 said a task stream introduces no durable delivery acknowledgement; decision 5 left a lease lost on the wire to the reaper; decision 6 dispatched a received lease at once), ADR-0098 (adds the cause `NOT_ACKNOWLEDGED`, a bound on repeated withdrawals of that cause, and a requirement that depends on `lease-withdrawal-v1`), ADR-0082 (a requirement that depends on another, enforced by the registry's trigger) and ADR-0015 (adds minor 1.8 under its rule; the rule does not change). Supersedes none.
**Enforced by:** to be built with the decision (`specs/006-sdk-multilanguage/tasks.md`, phase "Delivery acknowledgement"): the barrier tests of the acknowledgement against the withdrawal in both orders and with the candidate read before the acknowledgement (`tests/fault/sdk-multilanguage/`); the matrix of the provisional mark, root and scoped, including a lease written by a binary that does not know the columns (`integration_durable_execution`); the budget lines `ACKNOWLEDGEMENT` and `FIRST_HEARTBEAT` with no step scan, no journal and no outbox insert in the former, and `LEASE` unchanged (`integration_claim_transition_efficiency`); a descriptor-driven contract test that every worker method carrying a lease token, other than the four that keep or end a lease, is refused on a provisional lease (`contract_grpc_durable_execution`); the registry trigger tests of the dependency; the conformance scenario `delivery-acknowledgement` in six languages; the released-v0.2 upgrade scenario `provisional-lease-cutover` and its rollback case.

## Context

The engine commits a lease before it knows the task reached a host. ADR-0098 withdraws the leases the server knows it never
handed to the transport, and a host hands back the leases it drops (`TASK_NOT_STARTED`). Two losses stay out of its reach, gap
G006-001 of `specs/006-sdk-multilanguage/research.md`: a frame of `StreamTasks` handed to the transport and lost on the wire,
and the answer of a unary `PollTasks` lost after its handler returned. Each lapses after the lease time-to-live and, since
feature 037, a lapsed lease of an `Activity` is an unknown outcome that goes to consultation or `step.needs_review`, for work
that never ran. No such loss has been observed yet: the three cases that started this work (two CI jobs and one play of
feature 022) are ADR-0098's, where the server knew the frame never left the process. The owner chose to close the residue in
release 0.3.0 with a new protocol minor in all six SDKs, knowing it changes the worker protocol's guarantees.

What constrains the choice:

- **Exactness.** The claim to make is "a handler of a lease runs only after the server has committed that the host has the
  lease"; from it follows "a lease the server withdraws was not run by a host that keeps the protocol".
- **Cost.** Throughput follows round trips (`specs/038-claim-transition-efficiency/research.md`, finding 1). A task costs a
  lease (38 statements, 2 transactions, 5 rows written) and a completion (50 statements, 1 transaction, 11 rows), the lines of
  `tests/integration/claim-transition-efficiency/work_budget.rs`. A task shorter than its heartbeat interval (a third of the
  lease, 500 ms to 10 s) sends no heartbeat and writes no `step.started`.
- **The run lock.** Every write of a run's rows happens under the continuation-chain lock and then the run row's lock
  (`SELECT run_version FROM runs … FOR UPDATE`), taken by `begin` for the whole run and by `begin_path` for a scoped path;
  `begin` also restores the run's whole state, which is what makes a heartbeat cost what it costs.
- **Older SDKs and released v0.2.** The server keeps serving every older minor exactly as it does (ADR-0015 decision 1); a
  release-0.2 binary does not register in the instance-capability registry.
- **Minors are cumulative.** A session at 1.8 also speaks 1.7 (step logs, design 041, OD-2026-11-15-8), 1.6 and 1.5. The
  server does not serve 1.7 yet and no SDK speaks it.
- **One meaning per published version.** The six SDKs must not fork the server's semantics (constitution principle IV): an
  option of a published workflow, such as `timeoutMs`, must mean the same whatever minor the host that takes the task speaks.
- **Measured latencies.** Ready to handler entry, 99th percentile, 751 to 889 ms over the ten burst rows of five SDKs and both
  transports (`tests/performance/reports/sdk-streaming.md`); a completion, 6 to 103 ms at the 99th percentile over the nine
  rows of 2026-11-12 (`tests/performance/reports/scalability-ha.md`), on one machine with no network between the services and
  the database.

## Decision

1. **Minor 1.8 adds `AcknowledgeTasks` and one host rule.** A host accepted at 1.8 MUST NOT start a handler of a lease, and MUST
   NOT make any other call for it, before the server answered `ACKNOWLEDGEMENT_VERDICT_ACKNOWLEDGED` for that lease. It
   acknowledges the leases of a frame, or of a `PollTasks` answer, in one call sent as soon as the frame is read. The wire and
   the host procedure are `specs/006-sdk-multilanguage/stream-completion.md`, "Delivery acknowledgement (protocol 1.8)". Minor
   1.8 is cumulative and is served only once 1.7 is; it is reserved in `sdks/SPEC.md` §9 and the reservation is released if
   this ADR is not accepted.
2. **The provisional mark is lease data bound to its generation.** A claim for a session accepted at 1.8, made while the
   requirement `delivery-acknowledgement-v1` is known to be recorded, MUST mark the lease provisional in the statement that
   already writes the lease, root or scoped: `steps.lease_offered_generation` is set to the lease's generation and
   `steps.lease_offered_at` to the claim transaction's time, and `steps.lease_acknowledged_at` is cleared. A mark counts only
   while `lease_offered_generation` equals the step's `generation`, so a lease written later by any binary, including one that
   does not know the columns, is never taken for provisional. A lease is *provisional* when its step is `LEASED` and its mark
   counts and `lease_acknowledged_at` is empty; *acknowledged* when its mark counts and `lease_acknowledged_at` is set. Nothing
   else about the lease changes: token, generation, attempt, expiry (the claim's time plus the lease time-to-live), attempt
   deadline and credit.
3. **The acknowledgement locks the run and loads nothing of it.** Per call the server checks `worker:execute` once and reads,
   with one indexed statement and no lock, the addressed step rows whose lease-token hash matches; only the runs that matched are
   opened. Per run it takes the continuation-chain lock and then the run row's lock, in the order `begin` and `begin_path` take
   them, and writes each matched step row with one conditional statement (the token hash, the attempt and the generation of the
   request, the step `LEASED` or `RUNNING`, and, when the lease is provisional, not past its expiry), setting
   `lease_acknowledged_at` once and renewing the lease as a heartbeat renews it. The binding is checked on the activity that
   statement returns, against the caller's bound patterns read once per call; a refused binding rolls that run back. The
   acknowledgement MUST NOT restore the run's state, apply a transition, append a journal event or write the outbox, except on
   the path of decision 8. A run that cannot be opened for a reason other than an unavailable store answers `NOT_HELD` for its
   tasks; the others go on.
4. **Exactly one of the acknowledgement and the withdrawal commits, and the deadline is literally the expiry.** The lost-lease
   pass re-reads the lease's marks, status, generation and expiry under the run lock, decides with one pure function, and writes
   the withdrawal fenced on what it read. The acknowledgement refuses a provisional lease that is past its expiry
   (`NOT_HELD`, nothing written). Both writers hold the run lock; the host runs only on `ACKNOWLEDGED`. No clock enters the
   argument.
5. **A provisional lease that expires is withdrawn; an acknowledged lease is an attempt.** At its expiry a provisional lease
   MUST be withdrawn as ADR-0098 decision 3 withdraws (one fenced transaction per run or path, `step.lease_withdrawn` with the
   cause `NOT_ACKNOWLEDGED`, the step back to `READY`, the attempt not consumed, no review), whatever the requirement says by
   then: a mark keeps its meaning until its lease ends. An acknowledged lease is never withdrawn: when it expires it is an unknown
   outcome, consultation or review, as today, and a `TASK_NOT_STARTED` for it is an ordinary retryable failure, never a
   withdrawal.
6. **The acknowledgement deadline is the lease's expiry, and 1.8 needs a time-to-live of at least 12 s.** The derivation is in
   `stream-completion.md` ("Why the deadline is the lease expiry"). A host of 1.8 reads every lease it will read within the
   receive bound R = 6 s of its claim (`sdks/SPEC.md` §10 for a stream; this decision extends it to `PollTasks` at 1.8) and
   bounds each acknowledgement attempt to R as well, so one full attempt fits before the expiry when the time-to-live is at
   least 2R = 12 s, and (30 − 6) / 6 = 4 attempts fit at the default 30 s. Within that bound every deadline is equally safe
   (decision 4); the expiry is chosen because it adds no setting, no column and no scan, and because a deadline nearer the
   healthy delivery time would withdraw healthy deliveries when the database is slowest. The server MUST answer a call of 64
   leases over 64 runs within R / 2 = 3 s at the 99th percentile on the measured topology (task T006-064).
7. **The attempt deadline keeps its meaning.** `timeoutMs` is counted from the claim at every minor. At its attempt deadline a
   provisional lease ends its attempt as a known retryable failure with the code `ATTEMPT_TIMEOUT_BEFORE_START`: the attempt is
   consumed by the step's own timeout, as today, and no review opens, because nothing ran. An acknowledged lease that reaches
   its attempt deadline keeps today's meaning.
8. **Cancellation before the start is decided by the acknowledgement.** An acknowledgement of a provisional lease whose
   cancellation is requested MUST NOT acknowledge it: in the same transaction it restores the run (or the path), withdraws the
   lease with the cause `NOT_STARTED` and runs the settle, which cancels the step as the withdrawal of a cancel-requested lease
   does in the ADR-0098 build; it answers `ACKNOWLEDGEMENT_VERDICT_CANCELLED`, and the host drops the lease without reporting.
9. **A provisional lease receives no capability.** Every worker call that carries a lease token, other than `Heartbeat`,
   `CompleteStep`, `FailStep` and `AcknowledgeTasks`, MUST be refused on a provisional lease with `FAILED_PRECONDITION` and the
   stable reason `TASK_NOT_ACKNOWLEDGED`, after the token and the binding have matched, through a fence variant of its own (root
   and path) that the report paths do not use. Today these are `ResolveSecrets`, `ResolveDatabase`, `WriteBlob`, `ReadBlob` and
   `AppendStepLogs`; a later call is covered by the contract test that lists the methods from the descriptor.
10. **Repeated withdrawals are bounded.** A step whose leases were withdrawn with the cause `NOT_ACKNOWLEDGED` three times in a
    row (`steps.lease_unacknowledged_withdrawals`, cleared by an acknowledgement or a report) gets its next lease without the
    mark, so a fourth loss lapses as it does today. A host that gets `UNIMPLEMENTED`, or any other status that is not retryable,
    for `AcknowledgeTasks` hands the call's leases back and registers again offering 1.7 for the rest of its life.
11. **The fleet decides, the operator records, and the registry enforces the dependency.** An engine lists
    `delivery-acknowledgement-v1` only when its configuration can honour it (its lease time-to-live is at least 12 s, ADR-0082
    decision 7); every control plane of the build lists it. The operator records it with
    `control-plane delivery-acknowledgement-activate` and removes it with `control-plane delivery-acknowledgement-remove` (owner
    role, `--operator`, `--expect-live N`, `--dry-run`). The registry's `ENABLE ALWAYS` trigger MUST refuse recording it while
    `lease-withdrawal-v1` is not recorded and removing `lease-withdrawal-v1` while it is. The removal command, after the removal
    commits and the registry's learning bound has passed, expires every worker session accepted at 1.8, so that their hosts
    register again below 1.8. Every engine of the build serves `AcknowledgeTasks` whatever it knows of the requirement; an engine
    offers 1.8 and writes new marks only while it knows the requirement is recorded.
12. **Older minors and internal runners are served exactly as today.** A lease of a session below 1.8 is never provisional.
    The database runner, the sandbox runner and the command-line probe keep their minors.
13. **The trust assumption, stated for the owner.** The withdrawal of a provisional lease records a known outcome (nothing ran)
    because a host that declared 1.8 promised to start nothing before the acknowledgement; the server cannot verify that
    promise. Constitution principle III is applied to 1.8 leases under that assumption. What the server does see, it records: a
    `Heartbeat`, `CompleteStep` or `FailStep` on a provisional lease marks its session as not keeping the protocol (no later
    lease of it is provisional) and is counted; a report or heartbeat that carries the token of a lease withdrawn as
    `NOT_ACKNOWLEDGED` (kept as `steps.withdrawn_lease_token_hash`) is counted, audited and alerted.
14. **What is counted** is listed in `stream-completion.md`, "Observability": verdicts, withdrawals by cause and outcome, the
    delivery delay and the call's duration, the provisional leases in flight, refused capability calls, protocol violations and
    reports after a withdrawal, with alerts on rates and on the bound of decision 10.

## Considered options

1. **A provisional mark on the step row, acknowledged by a batched call that locks the run and loads nothing (chosen).** Exact by
   the run lock (decision 4). Per call it adds one indexed read; per run in the call, one transaction with the chain lock, the run
   lock and one write per task. Under light load a frame carries one lease, so this is one call and one small transaction per
   task; under a backlog or a fan-out the call carries up to 64. The claim keeps its statements; the happy path writes no journal
   event.
2. **Variant 1 as sketched in owner question O006-001: a new step status `OFFERED` and an acknowledgement event.** Equally exact.
   A new status reaches every reader of `steps.status` (the session's lease counts, the reaper, the planner, reset, Studio, the
   operations statistics, the replay of every replica), and an event per task adds a journal append per task. The provisional
   state is a property of the lease, like its expiry and its token, which are not journaled either.
3. **Variant 2: the first heartbeat before the handler.** Equally exact, and it reuses `Heartbeat` and `step.started`. Per task,
   not per call: one call and one transaction that restores the run's whole state, applies a transition and appends
   `step.started`; more statements than option 1 (the build pins both lines). One more journal event for every task shorter than
   its heartbeat interval. The server must still mark which leases carry the contract.
4. **A shorter deadline of its own.** Recovers a lost lease sooner, but withdraws healthy deliveries when the database is slowest,
   and each such withdrawal claims and delivers the task again: load added under overload; and one more parameter. An operator
   who wants faster recovery shortens the lease time-to-live, down to 12 s, which also shortens every heartbeat interval; that
   trade-off is the operator's, not a second deadline's.
5. **An acknowledgement without the run lock** (one conditional statement over the call's step rows). One statement per call;
   the step row's own lock would order it against a withdrawal whose write is conditional. Rejected: every writer that decides
   from the lease columns under the run lock (the expiry, the attempt timeout, the withdrawal, the cancellation, reset) would have
   to re-check them in its write and roll its journal event back when they changed, a conditional-write discipline across the
   engine, or the renewal written by an acknowledgement would race those decisions. The run lock is the one serialization the
   engine uses for a run's rows.
6. **Count `timeoutMs` from the acknowledgement at 1.8.** Measures the handler exactly, but gives one published version two
   meanings depending on the host's minor (constitution principle IV). Rejected for decision 7.
7. **Consume an attempt after repeated withdrawals.** Bounds the loop through the retry policy, but fails a run for a delivery
   fault. Rejected for decision 10, which ends a loop in today's outcome.
8. **The acknowledgement in a bidirectional stream, or carried by the next poll.** The first needs bidirectional streams in six
   SDKs and gives nothing for a unary poll; the second makes the handler wait for the next receive. Rejected.
9. **Do nothing beyond ADR-0098.** Leaves G006-001. Rejected by the owner (OD-2026-11-20-12).

## Consequences

Good: a frame lost on the wire and a lost `PollTasks` answer go back to `READY` at the lease's expiry instead of to an operator,
with the attempt number, and so the idempotency key, unchanged; a provisional lease receives no secret, database credential,
blob or log stream; a cancellation that arrives before the start is known and immediate; a process that crashes between a claim
and ADR-0098's withdrawal no longer leaves a review for a 1.8 session (gap G006-002 closes for them); the claim, the completion
and the meaning of every option are unchanged; a lease's meaning never changes while it is held.

Bad: one more call and, per run in it, one more transaction before the first handler of a frame starts; under light load that
is one per task: by count of the statements named in decision 3, about 6 more statements and 1 more transaction on the 88
statements and 3 transactions of a task today, plus the call's authentication, which the budget does not count. With `BEGIN`
and `COMMIT` counted as round trips, that is 102 against 94, so the round-trip model of feature 038 predicts a throughput cost
of about 8% in the transitions scenario; it is an estimate, not a measurement (task T006-064 measures it,
and the build stops if it exceeds 15%). Five columns on `steps` and one on `worker_sessions`, two commands, one migration, a
trigger change in the registry. 1.8 cannot be served before 1.7 is, in the server and in each SDK. Gaps G006-040 to G006-046
remain, of which the trust assumption (G006-042) is the one the owner accepts with this ADR.

Compatibility with runs in flight: a lease issued before the activation has no mark and keeps today's meaning; after it, only
leases of sessions accepted at 1.8 are provisional, and a session is accepted at 1.8 only after the activation.

Rollback (`stream-completion.md`, "Rollback"): run `delivery-acknowledgement-remove`, which removes the requirement, waits the
registry's learning bound and expires the sessions accepted at 1.8; wait until no provisional lease is in flight
(`worker_leases_provisional` is 0, at most one lease time-to-live later); then roll the binary back, no further than a build that
decodes the cause `NOT_ACKNOWLEDGED`. If the ADR-0098 build adds that cause to its closed set before it merges (requested of it,
decision D006-059), the floor is the ADR-0098 build; otherwise it is this one. The `down` migration refuses while a mark counts.

## Evidence

`specs/006-sdk-multilanguage/research.md` decisions D006-040 to D006-060, gaps G006-040 to G006-046 and owner question O006-001;
`specs/006-sdk-multilanguage/stream-completion.md`, "Delivery acknowledgement (protocol 1.8)"; `sdks/SPEC.md` §6.1 to §6.4, §6.8,
§9 and §10; ADR-0098 and the build decisions of its branch; `tests/performance/reports/sdk-streaming.md` and
`tests/performance/reports/scalability-ha.md` for the measured latencies; `tests/integration/claim-transition-efficiency/work_budget.rs`
for the budget lines; the two review rounds under `specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/`.
