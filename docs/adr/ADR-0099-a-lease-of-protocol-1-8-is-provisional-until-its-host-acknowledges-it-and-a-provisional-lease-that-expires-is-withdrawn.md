# ADR-0099: A lease of worker protocol 1.8 is provisional until its host acknowledges it, and a provisional lease that expires is withdrawn

**Status:** Proposed
**Decided by:** agent, delegate (Owner decision 2026-11-20-6), on the owner's answer "Yes, in 0.3.0" to delivery acknowledgement (Owner decision 2026-11-20-12, which left the variant, the protocol minor and the deadline to this design). Reviewed on 2026-11-20 by the correctness, security, architecture and design reviewers: round 1 found 5 Blockers and 32 Majors, round 2 found 2 Blockers and 21 Majors, round 3 found 1 Blocker and 16 Majors, each answered in this text; round 4 will be recorded beside them (`specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/`). Acceptance is the owner's, and with it the trust assumption of decision 12 and the reservation of minor 1.8 (Owner question 006-002).
**Refines / supersedes:** refines Architecture decision 0029 (decision 2 said a task stream introduces no durable delivery acknowledgement; decision 6 dispatched a received lease at once), Architecture decision 0098 (adds the cause `NOT_ACKNOWLEDGED` to `step.lease_withdrawn`, supplies the minor its decision 5 reserves `TASK_NOT_STARTED` for, and adds what 1.8 changes in that rule), Architecture decision 0082 (a requirement that depends on another, kept in a table the registry's trigger reads; an amendment line goes into Architecture decision 0082 at acceptance) and Architecture decision 0015 (adds minor 1.8 under its rule). Supersedes none.
**Enforced by:** to be built with the decision (`specs/006-sdk-multilanguage/tasks.md`, phase "Delivery acknowledgement"): the barrier tests of the acknowledgement against each withdrawal, against another acknowledgement, against a revocation and against a run transaction that loaded the run before it (`tests/fault/sdk-multilanguage/`, `integration_durable_execution`); a static check in `scripts/check-architecture.py` that every SQL assignment to a column of the marks is on its allow-list; the unit test of the decision function and its parity test against the SQL condition (`integration_durable_execution`); the matrix of the marks over the three statements that write a lease, including a lease written by a binary that does not know the columns; the budget lines `ACKNOWLEDGEMENT` and `FIRST_HEARTBEAT` and the unchanged `LEASE` line (`integration_claim_transition_efficiency`); the descriptor-driven test of the capability refusal (`contract_grpc_durable_execution`); the registry trigger tests; the `promtool` cases of the alerts; the host's unit tests; the conformance scenario `delivery-acknowledgement` and the override pass in six languages; the released-v0.2 scenario `provisional-lease-cutover` and its rollback case.

## Context

The engine commits a lease before it knows the task reached a host. Architecture decision 0098 withdraws the leases the server
knows it never handed to the transport, and lets a host hand back a lease it drops (`TASK_NOT_STARTED`), which its build review
reserved for the host at the minor this decision designs, under a cap of four withdrawals per attempt (Decision 006-030 and
Decision 006-031, on its branch). Two losses stay out of its reach, Gap 006-001 of `specs/006-sdk-multilanguage/research.md`: a
frame of `StreamTasks` handed to the transport and lost on the wire, and the answer of a unary `PollTasks` lost after its handler
returned. Each lapses after the lease time-to-live and, since feature 037, a lapsed lease of an `Activity` is an unknown outcome
that goes to consultation or `step.needs_review`, for work that never ran. No such loss has been observed yet: the three cases
that started this work are Architecture decision 0098's, where the server knew the frame never left the process. The owner chose
to close the residue in release 0.3.0 with a new protocol minor in all six SDKs.

What constrains the choice:

- **Exactness.** The claim to make is "a handler of a lease runs only after the server has committed that the host has the
  lease"; from it follows "a lease the server withdraws was not run by a host that keeps the protocol".
- **Cost.** Throughput follows round trips (`specs/038-claim-transition-efficiency/research.md`, finding 1). A task costs a
  lease (38 statements, 2 transactions) and a completion (50 statements, 1 transaction), the lines of
  `tests/integration/claim-transition-efficiency/work_budget.rs`. A task shorter than its heartbeat interval sends no heartbeat.
- **Locks.** A run's transitions are written under the continuation-chain lock and then the run row's lock, and `begin` restores
  the run's whole state under them. Each step row also has its own row lock, which every writer of the row takes. A binding
  revocation shortens leases under the binding's advisory lock without the run lock.
- **Later minors.** Minors are cumulative: whatever 1.8 asks of a session, every later minor asks too, and drafts 019, 020, 021,
  025, 026 and 031 will take 1.9 onward.
- **1.7 first.** A session at 1.8 also speaks 1.7 (step logs, design 041, Owner decision 2026-11-15-8). Neither the server nor
  any SDK speaks 1.7 yet.
- **One meaning per published version** (constitution principle IV): `timeoutMs` must mean the same at every minor.
- **Measured latencies.** Ready to handler entry, 99th percentile, 751 to 889 ms over the ten burst rows of five SDKs and both
  transports (`tests/performance/reports/sdk-streaming.md`); a completion, 6 to 103 ms at the 99th percentile over the nine
  rows of 2026-11-12 (`tests/performance/reports/scalability-ha.md`), on one machine with no network.

## Decision

1. **Minor 1.8 adds a flag on the lease, one call and one registration field.** `TaskLease.acknowledge_before_start` (field 23)
   is set on a lease the claim marked provisional and on no other; `WorkerCapabilities.declines_acknowledgement` (field 10) lets a
   host keep its minor and ask for no flagged lease. A host accepted at 1.8 or later MUST NOT start a handler of a flagged lease,
   and MUST NOT make any call for it other than `AcknowledgeTasks` and the hand-back (`FailStep` with `TASK_NOT_STARTED`), before
   the server answered `ACKNOWLEDGEMENT_VERDICT_ACKNOWLEDGED` for it. It dispatches an unflagged lease at once, as today. The
   server offers 1.8 on the proofs of its build alone, from the task that follows the gate of decision 13, so no later minor
   depends on an operator's switch. The wire and the host procedure are
   `specs/006-sdk-multilanguage/stream-completion.md`, "Delivery acknowledgement (protocol 1.8)". 1.8 is reserved in the register
   of `sdks/SPEC.md` §9 at the coordinator's instruction, an exception to the register's rule (Owner decision 2026-11-15-8) put to
   the owner as Owner question 006-002.
2. **The marks are lease data bound to the lease's generation.** In the statement that already writes a lease (any of the three:
   the root statement, the scoped statement of a full-run transaction, the path statement), a claim writes, for the lease's
   generation: `steps.lease_hand_back_generation` when the claiming session was accepted at 1.8 or later (the lease is under the
   reserved hand-back), and `steps.lease_offered_generation` and `steps.lease_offered_at` when it marks the lease provisional,
   clearing `steps.lease_acknowledged_at`. It marks when the session was accepted at 1.8 and does not decline acknowledgement, the
   engine knows the requirement `delivery-acknowledgement-v1` is recorded and its own `SAGAT_LEASE_WITHDRAWAL` is on, the lease's
   earlier deadline (the earlier of its expiry and its attempt deadline) is at least three receive bounds (18 s) after the claim,
   and the attempt's withdrawals so far are under the cap of Architecture decision 0098 (read from the run state the claim already
   restored). A mark counts while the step is `LEASED` or `RUNNING` and the mark's generation equals the step's, so a lease written
   later by any binary is never taken for marked. *Provisional*: `LEASED`, the offer counts, `lease_acknowledged_at` empty.
   *Acknowledged*: the offer counts, `lease_acknowledged_at` set. No statement writes a column of the marks from a value it read
   earlier: the engine's statements keep them or clear them with the lease, which a static check enforces.
3. **The acknowledgement locks rows, never the run, and decides on what it locked.** One transaction per call, for the tasks of one
   namespace (a frame comes from one session): the permission `worker:execute`; one unlocked read, by the step primary key, of the
   rows whose lease-token hash matches, for their namespace and activity; the checks `ensure_bound` makes, under its locks (the
   binding's shared advisory lock and patterns, the namespace's shared secrets lock, the membership's row lock), with the coverage
   decided per row; then one statement that locks the matched rows `FOR UPDATE SKIP LOCKED` and reads them with the run's stop
   reason; the verdicts decided by one pure function on the locked rows with `clock_timestamp()` sampled after the locks; and one
   update of the rows that win, setting `lease_acknowledged_at`. It never waits for a row, never takes the run lock, renews nothing,
   applies no transition, writes no journal event and no outbox row.
4. **Exactly one of the acknowledgement and a withdrawal commits.** Every withdrawal of a lease whose mark counts re-reads the step
   row `FOR UPDATE` under the run lock, after its binding checks, decides with the same pure function, and withdraws only a lease
   still provisional (or, for a hand-back, not acknowledged). The acknowledgement writes only rows it holds locked and found
   provisional with at least one receive bound left. Whichever holds the row first wins; the other finds it changed and does
   nothing. The host runs only on `ACKNOWLEDGED`.
5. **A provisional lease is withdrawn at its earlier deadline; an acknowledged lease is an attempt.** When the first of its expiry
   and its attempt deadline passes, a provisional lease MUST be withdrawn as decision 3 of Architecture decision 0098 withdraws
   (cause `NOT_ACKNOWLEDGED`, the step back to `READY`, the attempt not consumed, no review); the next lease is a new claim with its
   own deadlines, so `timeoutMs` keeps its meaning. An acknowledged lease is never withdrawn: at a deadline it keeps today's meaning
   (an unknown outcome, consultation or review), and a `TASK_NOT_STARTED` for it is an ordinary retryable failure.
6. **The acknowledgement deadline is the lease's own earlier deadline, with a margin of one receive bound.** The derivation is in
   `stream-completion.md` ("Why the deadline is the lease's own"). A host of 1.8 reads every lease within the receive bound (6 s) of
   its claim (`sdks/SPEC.md` §10 for a stream; this decision extends it to `PollTasks` at 1.8) and bounds each acknowledgement
   attempt to the same 6 s; the server acknowledges only with at least 6 s left before both deadlines: before the expiry so that the
   first heartbeat, which the host sends at the earlier of its interval and a third of the time left the answer reports, commits
   before the lease can expire; before the attempt deadline so that no handler starts with less than that of its timeout. A read,
   one attempt and the margin fit when the earlier deadline is at least 18 s after the claim; a lease with less is not marked.
7. **Cancellation before the start needs no write.** An acknowledgement of a provisional lease whose step is asked to stop, or
   whose run has a stop reason, answers `ACKNOWLEDGEMENT_VERDICT_CANCELLED` and writes nothing; the host hands the lease back and
   the withdrawal's settle cancels the step; if the hand-back is lost, the withdrawal at the deadline does.
8. **A provisional lease receives no capability.** Every worker call that carries a lease token, other than `Heartbeat`,
   `CompleteStep`, `FailStep` and `AcknowledgeTasks`, MUST be refused on a provisional lease with `FAILED_PRECONDITION` and the
   stable reason `TASK_NOT_ACKNOWLEDGED`, after the token and the binding have matched, through a fence variant (root and path) that
   the report paths do not use. A served capability call records `steps.lease_capability_generation` for the lease's generation, in
   the transaction it already commits. A contract test walks the descriptor through nested, `oneof` and repeated fields and fails
   when a lease-token method has no case.
9. **What 1.8 adds to the hand-back rule of Architecture decision 0098 decision 5.** That rule (the reservation, the requirement,
   the engine's switch, `LEASED`, the cap of four per attempt) stays the server's and is not restated here. 1.8 adds: the
   reservation is read from `lease_hand_back_generation`, never from the session row, which registering again can change; a
   hand-back of an acknowledged lease, or of a lease to which a capability was served, is an ordinary retryable failure, the latter
   also recorded as a protocol violation. A host hands back only before any handler of the lease started.
10. **The fleet decides, the operator records.** Every control plane and engine of the build lists `delivery-acknowledgement-v1`
    from the task that follows the gate. It is activated by `control-plane capability-activate delivery-acknowledgement-v1`, an
    entry of the generic command of Architecture decision 0098 (Decision 006-034), whose entry says it requires
    `lease-withdrawal-v1` for every kind; the dependency is a row of `platform_requirement_dependencies`, which the registry's
    trigger reads to refuse recording a requirement before its dependency, and removing a dependency while a requirement that needs
    it is recorded. An engine marks leases only while it knows the requirement is recorded; a mark already written keeps its
    meaning until its lease ends.
11. **Older minors, declining sessions and internal runners are served exactly as today.** A lease of a session below 1.8, or of a
    session that declines acknowledgement, is never marked. The database runner, the sandbox runner and the command-line probe keep
    their minors.
12. **The trust assumption, stated for the owner.** The withdrawal of a provisional lease, and the hand-back of one, record a known
    outcome (nothing ran) because a host that declared 1.8 promised to start nothing before the acknowledgement; the server cannot
    verify that promise. Constitution principle III is applied to 1.8 leases under that assumption. What the server does see, it
    counts and audits once per step and generation: a `Heartbeat`, `CompleteStep`, or `FailStep` other than the hand-back on a
    provisional lease; a capability call refused `TASK_NOT_ACKNOWLEDGED`; a hand-back after a capability was served; and, once per
    step and withdrawn token, any report, heartbeat or capability call carrying the token of a lease withdrawn as `NOT_ACKNOWLEDGED`
    (`steps.withdrawn_lease_token_hash`).
13. **The cost is measured before the work in six languages, and the minor waits for it.** Right after the server tasks, the
    transitions scenario runs at 1.8 against 1.7 (the median of five interleaved runs each, with a network between the workers and
    the services), a 64-lease call is timed, and heartbeats are timed in a burst. The build goes on only if 1.8 is no more than 15%
    below 1.7, the call is answered within half the receive bound (3 s) at the 99th percentile, no heartbeat is shed while
    acknowledgements are admitted, and the heartbeat's 99th percentile in the burst is under a third of the receive bound (2 s).
    Then a last server task makes the server offer 1.8 and list the capability. If it fails, neither happens, Gap 006-030 of
    Architecture decision 0098 stays open, and the owner decides. These are gates of the build, not promises of the platform.
14. **What is counted** is listed in `stream-completion.md`, "Observability".
15. **The reserved code at 1.8 for the host.** A host sends `TASK_NOT_STARTED` only for a lease no handler of which has started:
    the hand-backs of `sdks/SPEC.md` §6.2 and §6.8. The SDKs rewrite a handler-raised `TASK_NOT_STARTED` to `WORKER_ERROR` at every
    minor (Decision 006-028 of the SDK build of Architecture decision 0098). The server's rule is decision 9.

## Considered options

1. **A flag on the lease, marks bound to its generation, and an acknowledgement that locks rows with `SKIP LOCKED` and never the run
   (chosen).** Exact by the row lock (decision 4). One transaction of about eight statements per call whatever the number of leases;
   it never waits for a row, so it cannot deadlock on rows or be held up by one busy run; no journal event; nothing paid where the
   operator has not activated, and nothing asked of later minors.
2. **The first round of this design: the acknowledgement under the run lock, renewing the lease.** Equally exact; the renewal is what
   needed the run lock, and with it came a transaction per run, a call bounded by the slowest run, and a renewal channel.
3. **A host rule bound to the minor** ("a session at 1.8 acknowledges every lease"). One field fewer, but offering 1.8 would depend
   on the activation, and so would every later minor.
4. **Variant 1 as sketched in Owner question 006-001: a new step status `OFFERED` and an acknowledgement event.** Equally exact; a
   status reaches every reader of `steps.status` and an event per task adds a journal append per task.
5. **Variant 2: the first heartbeat before the handler.** Equally exact; per task, a transaction that restores the run's whole state,
   applies a transition and appends `step.started`; one more journal event for every short task; it still needs the flag.
6. **A shorter deadline of its own.** Recovers a lost lease sooner, but withdraws healthy deliveries when the database is slowest.
7. **A bound of its own on repeated withdrawals** (the second and third rounds' run of three, kept on the step). It overlapped the cap
   of four per attempt of Architecture decision 0098, with other reset rules. The claim marks only while the attempt is under that
   cap, so one bound serves both.
8. **Falling back to 1.7 when the acknowledgement cannot work** (the third round). It would have cost every later minor's features;
   the registration field keeps the minor and declines only the flag.
9. **Do nothing beyond Architecture decision 0098.** Leaves Gap 006-001. Rejected by the owner (Owner decision 2026-11-20-12).

## Consequences

Good: a frame lost on the wire and a lost `PollTasks` answer go back to `READY` at the lease's earlier deadline instead of to an
operator, with the attempt number and the idempotency key unchanged; a provisional lease receives no credential, blob or log
stream; a cancellation that arrives before the start is known; a process that crashes between a claim and Architecture decision
0098's withdrawal no longer leaves a review for a marked lease (Gap 006-002 closes for them); the claim, the completion and the
meaning of every option and every later minor are unchanged; a lease's meaning never changes while it is held.

Bad: one more call and one more short transaction before the first handler of a frame starts; under light load that is one per
task, about eight statements and one transaction more on the 88 statements and 3 transactions of a task today (104 against 94
round trips with `BEGIN` and `COMMIT`), which the round-trip model of feature 038 predicts at about 10% of the transitions per
second, plus the trigger work the update fires, which the gate measures; a task with an input blob reads it only after the
acknowledgement. Six columns on `steps`, one on `worker_sessions`, one table in the registry, one migration. 1.8 waits for 1.7 in
the server and in each SDK. A lease whose earlier deadline is under 18 s, or whose attempt reached the cap, is not protected (Gap
006-047). Below 1.8 a hand-back is an ordinary failure that spends the attempt (Decision 006-030 of Architecture decision 0098),
which this design does not change and the owner weighs with that decision. Gap 006-040 to Gap 006-047 remain, of which the trust
assumption (Gap 006-042) is the one the owner accepts with this decision.

Compatibility with runs in flight: a lease issued before the activation is not flagged and keeps today's meaning; after it, only
leases claimed for sessions at 1.8 are flagged.

Rollback (`stream-completion.md`, "Rollback"): stop marking (remove the requirement, or switch `SAGAT_LEASE_WITHDRAWAL` off), wait
until no mark counts, then roll the binary back. The floor is enforced, not described: the build of Architecture decision 0098
decodes the cause `NOT_ACKNOWLEDGED` (a precondition of this build), and `lease-withdrawal-v1`, which every writer of the cause
needs, cannot be removed once a journal holds `step.lease_withdrawn` (Decision 006-032 of that build), so the registry never admits
a binary below that build once any withdrawal exists.

## Evidence

`specs/006-sdk-multilanguage/research.md` Decision 006-040 to Decision 006-061, Gap 006-040 to Gap 006-047, Owner question 006-001
and Owner question 006-002; `specs/006-sdk-multilanguage/stream-completion.md`, "Delivery acknowledgement (protocol 1.8)";
`sdks/SPEC.md` §5, §6.1 to §6.5, §6.8, §9 and §10; Architecture decision 0098 and the build decisions on its two branches;
`tests/performance/reports/sdk-streaming.md` and `tests/performance/reports/scalability-ha.md`;
`tests/integration/claim-transition-efficiency/work_budget.rs`; the review rounds under
`specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/`.
