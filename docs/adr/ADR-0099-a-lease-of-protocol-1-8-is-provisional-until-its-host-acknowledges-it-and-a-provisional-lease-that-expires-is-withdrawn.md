# ADR-0099: A lease of worker protocol 1.8 is provisional until its host acknowledges it, and a provisional lease that expires is withdrawn

**Status:** Proposed
**Decided by:** agent, delegate (Owner decision 2026-11-20-6), on the owner's answer "Yes, in 0.3.0" to delivery acknowledgement (Owner decision 2026-11-20-12, which left the variant, the protocol minor and the deadline to this design). Reviewed on 2026-11-20 and 2026-11-21 by the correctness, security, architecture and design reviewers: round 1 found 5 Blockers and 32 Majors, round 2 found 2 Blockers and 21 Majors, round 3 found 1 Blocker and 16 Majors, round 4 found no Blocker and 11 Majors, round 5 found no Blocker and 5 Majors, round 6 found no Blocker and 5 Majors, round 7 found no Blocker and 6 Majors, round 8 found no Blocker and 6 Majors, round 9 found no Blocker and 6 Majors, round 10 found no Blocker and 5 Majors, each answered in this text; round 11 is recorded beside them (`specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/`). Acceptance is the owner's, and with it the two assumptions of decision 12 and the reservation of minor 1.8 (Owner question 006-002). It follows Architecture decision 0098 as amended by its build review (decisions 5, 7 and 8; Decision 006-030 to Decision 006-039), merged into this branch, and relies, once they are built, on the requests Decision 006-059 makes of that build (the cause, the liveness amendment of Architecture decision 0082 decision 5 and the others), which its merged head does not have yet: they go into a follow-up of pull request #260, and a released build with them is a hard precondition of the task that takes this design out of dormancy (Task 006-057; Decision 006-059).
**Refines / supersedes:** refines Architecture decision 0029 (decision 2 said a task stream introduces no durable delivery acknowledgement; decision 6 dispatched a received lease at once), Architecture decision 0098 (adds the cause `NOT_ACKNOWLEDGED` to `step.lease_withdrawn`; supplies the minor its decision 5 reserves `TASK_NOT_STARTED` for and adds what 1.8 changes in that rule; its decision 8's switch does not reach a marked lease; "Refined by" pointers go into its decisions 5 and 8 at acceptance, which otherwise keep describing their own build; it relies on that build's amendment of Architecture decision 0082 decision 5, an engine silent past the liveness window serving no capability call until its gate is decided again, Decision 006-059 (7)) and Architecture decision 0015 (adds minor 1.8 under its rule). Supersedes none.
**Enforced by:** to be built with the decision (`specs/006-sdk-multilanguage/tasks.md`, phase "Delivery acknowledgement"): the barrier tests of the acknowledgement against each withdrawal, against another acknowledgement, against a revocation and against a run transaction that loaded the run before it, of a hand-back after a deadline against the pass, of a capability call against a hand-back, and of a requirement's removal against the pass's withdrawal (`tests/fault/sdk-multilanguage/`, `integration_durable_execution`); a pass test that every outcome other than a withdrawal ends the attempt; a static check in `scripts/check-architecture.py` that every SQL assignment to a column of the marks, and to the attempt flag, has an allowed form; the unit test of the decision function and its parity test against the SQL condition (`integration_durable_execution`); the matrix of the marks over the three statements that write a lease, including a lease written by a binary that does not know the columns and a mark of another server incarnation or none; a test that the acknowledgement and the capability record commit with `synchronous_commit` at least `local`; the matrix of the attempt flag (no withdrawal cycling spends an attempt); the budget lines `ACKNOWLEDGEMENT` and `FIRST_HEARTBEAT` and the unchanged `LEASE` line (`integration_claim_transition_efficiency`); the descriptor-driven test of the capability refusal and record (`contract_grpc_durable_execution`); the `promtool` cases of the alerts; the host's unit tests; the conformance scenario `delivery-acknowledgement` and the override pass in six languages; the released-v0.2 scenario `provisional-lease-cutover` and its rollback and mixed-fleet cases; the failover drill of `infra/dr/drill.sh`; a witness type that a withdrawal cannot be decided without the requirements read.

## Context

The engine commits a lease before it knows the task reached a host. Architecture decision 0098 withdraws the leases the server
knows it never handed to the transport, and lets a host hand back a lease it drops (`TASK_NOT_STARTED`), which its build review
reserved for the host at the minor this decision designs (its decision 5) under a cap of four withdrawals per attempt (its decision
7). Two losses stay out of its reach, Gap 006-001 of `specs/006-sdk-multilanguage/research.md`: a frame of `StreamTasks` handed to
the transport and lost on the wire, and the answer of a unary `PollTasks` lost after its handler returned. Each lapses after the
lease time-to-live and, since feature 037, a lapsed lease of an `Activity` is an unknown outcome that goes to consultation or
`step.needs_review`, for work that never ran. No such loss has been observed yet: the three cases that started this work are
Architecture decision 0098's, where the server knew the frame never left the process. The owner chose to close the residue in
release 0.3.0 with a new protocol minor in all six SDKs.

What constrains the choice:

- **Exactness.** The claim to make is "a handler of a lease runs only after the server has committed that the host has the
  lease"; from it follows "a lease the server withdraws was not run by a host that keeps the protocol".
- **Cost.** Throughput follows round trips (`specs/038-claim-transition-efficiency/research.md`, finding 1). A task costs a
  lease (38 statements, 2 transactions) and a completion (50 statements, 1 transaction), the lines of
  `tests/integration/claim-transition-efficiency/work_budget.rs`. A task shorter than its heartbeat interval sends no heartbeat.
- **Locks.** A run's transitions are written under the continuation-chain lock and then the run row's lock, and `begin` restores
  the run's whole state under them. Each step row also has its own row lock, which every writer of the row takes. A binding
  revocation shortens leases under the binding's advisory lock without the run lock.
- **Durability.** `docs/runbooks/disaster-recovery.md` allows an asynchronous standby ("a risk to take on purpose"), a
  point-in-time recovery and "your platform's snapshot tool"; a database may run with `synchronous_commit = off`. Each can lose
  transactions the platform had committed.
- **Mixed fleets and shared capacity.** During a rolling upgrade or rollback, engines of this build and of the build of Architecture
  decision 0098 serve the same workers; admission is per process and shared by every tenant; a gateway or a mesh may sit between a
  host and the engines.
- **Later minors.** Minors are cumulative: whatever 1.8 asks of a session, every later minor asks too, and drafts 019, 020, 021,
  023, 024, 025, 026, 031, 032 and 035 will take 1.9 onward.
- **1.7 first.** A session at 1.8 also speaks 1.7 (step logs, design 041, Owner decision 2026-11-15-8). Neither the server nor
  any SDK speaks 1.7 yet.
- **One meaning per published version** (constitution principle IV): `timeoutMs` must mean the same at every minor.
- **Measured latencies.** Ready to handler entry, 99th percentile, 751 to 889 ms over the ten burst rows of five SDKs and both
  transports (`tests/performance/reports/sdk-streaming.md`); a completion, 6 to 103 ms at the 99th percentile over the nine
  rows of 2026-11-12 (`tests/performance/reports/scalability-ha.md`), on one machine with no network.

## Decision

1. **Minor 1.8 adds a flag on the lease, one call and one registration field.** `TaskLease.acknowledge_before_start` (field 23)
   is set on a lease the claim marked provisional and on no other; `WorkerCapabilities.declines_acknowledgement` (field 10) lets a
   host keep its minor and ask for no flagged lease, which it does only on evidence that the call cannot work on its path, and for
   a bounded period (`sdks/SPEC.md` §6.2). A host accepted at 1.8 or later MUST NOT start a handler of a flagged lease, and MUST NOT
   make any call for it other than `AcknowledgeTasks` and the hand-back (`FailStep` with `TASK_NOT_STARTED`), before the server
   answered `ACKNOWLEDGEMENT_VERDICT_ACKNOWLEDGED` for it, whatever it has declined. It dispatches an unflagged lease at once, as
   today. The server offers 1.8 on the proofs of its build alone, from the task that follows the gate of decision 13, so no later
   minor depends on an operator's switch. The wire and the host procedure are `specs/006-sdk-multilanguage/stream-completion.md`,
   "Delivery acknowledgement (protocol 1.8)". 1.8 is reserved in the register of `sdks/SPEC.md` §9 at the coordinator's
   instruction, an exception to the register's rule (Owner decision 2026-11-15-8) put to the owner as Owner question 006-002.
2. **The marks are lease data bound to the lease's generation and to the database server's incarnation.** In the statement that
   already writes a lease (any of the three: the root statement, the scoped statement of a full-run transaction, the path
   statement), a claim for a session accepted at 1.8 or later writes, for the lease's generation, `steps.lease_hand_back_generation`
   (the lease is under the reserved hand-back), `steps.lease_claimed_at` and `steps.lease_claim_incarnation` (the database server's
   start time, `pg_postmaster_start_time()`); when it also marks the lease provisional, it writes `steps.lease_offered_generation`
   and clears `steps.lease_acknowledged_at`. It MUST mark only when the session does not decline acknowledgement, the engine knows
   `lease-withdrawal-v1` is recorded for both process kinds and `delivery-acknowledgement-v1` for engines, its `SAGAT_LEASE_WITHDRAWAL`
   is on, the lease's earlier deadline (the earlier of its expiry and its attempt deadline) is at least three receive bounds (18 s)
   after the claim, and the attempt is not already spent for flags (`steps.provisional_withdrawn_attempt`, attempt data written
   through one fenced form in the three projection statements, whose condition only the withdrawal command binds true). A mark counts while the step is `LEASED` or `RUNNING`, its generation
   equals the step's, its `lease_claim_incarnation` is not null and equals the current server's start time, and, for a withdrawal,
   the lease was claimed after the current recording of `delivery-acknowledgement-v1`. A mark that does not count is, for the
   acknowledgement, an attempt, and for every withdrawal an unmarked lease, except that its hand-back is an ordinary failure. A promotion, a point-in-time recovery, a snapshot or base
   backup started as a server, a managed restore or failover and a cut-over to another cluster each bring another server, so they end
   every mark written before them without an operator. *Provisional*: `LEASED`, the offer counts, `lease_acknowledged_at` empty.
   *Acknowledged*: the offer counts and `lease_acknowledged_at` is set. No statement writes a column of the marks from a value it
   read earlier, which a static check enforces.
3. **The acknowledgement locks rows, never the run, and decides on what it locked.** One transaction per call, for the tasks of one
   namespace (a frame comes from one session), under a deadline of half the receive bound from the call's arrival, pool acquisition
   included, checked before each statement (so a call ends at most one statement timeout, 1 s, past it): the permission
   `worker:execute`; one statement setting its lock and statement timeouts to 1 s and raising `synchronous_commit` to `local` when it
   is `off`; one unlocked read, by the step primary key, of the rows whose lease-token hash matches, for their namespace and activity;
   the checks `ensure_bound` makes, through the same function, under its locks (the binding's shared advisory lock and patterns, the
   namespace's shared secrets lock, the membership's row lock), with the coverage decided per row; then one statement that locks the
   matched rows by primary key `FOR NO KEY UPDATE OF steps SKIP LOCKED` and reads them with the run's stop reason, the server's start
   time and `clock_timestamp()`; the verdicts decided by one pure function on the locked rows; and one update of the rows that win,
   setting `lease_acknowledged_at`. At the deadline the transaction is rolled back and the connection returned (discarded only when a
   rollback does not return), and the call answers `UNAVAILABLE` with the reason `ACKNOWLEDGEMENT_DEADLINE`. It
   never waits for a row, never takes the run lock, renews nothing, applies no transition, writes no journal event and no outbox row.
4. **Exactly one of the acknowledgement and the pass's withdrawal commits; a hand-back rests on the host's word.** Every withdrawal
   of a lease whose mark counts re-reads the step row `FOR UPDATE` under the run lock, after the report path's binding checks (the
   lost-lease pass, a system actor, has none) and after reading both requirements under the locks their removals wait for, and
   decides with the same pure function: the pass withdraws only a lease still provisional, the hand-back only a lease still
   `LEASED` with no capability served. The acknowledgement writes only rows it holds locked and found provisional with at least one
   receive bound left. The row lock orders the acknowledgement against the pass: whichever holds the row first wins, and the other
   finds it changed and does nothing. A hand-back can follow an acknowledgement and still withdraw; its "nothing ran" rests on the
   host's promise (decision 12) and the capability record, not on the row lock. The host runs only on `ACKNOWLEDGED`. This rests on a
   committed acknowledgement surviving (decision 12).
5. **A provisional lease is withdrawn at its earlier deadline; a lease that has not run is withdrawn on its hand-back; an attempt is
   flagged at most once after a hand-back.** When the first of its expiry and its attempt deadline passes, a provisional lease MUST
   be withdrawn as decision 3 of Architecture decision 0098 withdraws (cause `NOT_ACKNOWLEDGED`, the step back to `READY`, the attempt
   not consumed, no review); the next lease is a new claim with its own deadlines, so `timeoutMs` keeps its meaning. A hand-back
   under the reserved code of a lease whose mark counts, that is still `LEASED` (no heartbeat has made it `RUNNING`) and to which no
   capability was served, MUST be withdrawn (cause `NOT_STARTED`), provisional or acknowledged, whether or not a deadline has passed,
   until the pass has ended it (past the expiry the two race, and the pass ends an acknowledged lease with `expire_attempt`): the host's word that nothing started is the same either way (decision 12), so an acknowledgement whose answer was lost
   costs no attempt. That withdrawal sets `provisional_withdrawn_attempt` to the withdrawn lease's attempt (the stored attempt, before
   the withdrawal sets it back by one), and so does any withdrawal that brings the attempt to the cap of four of Architecture decision
   0098 decision 7; the next lease of that attempt is then not marked and runs with today's meaning. So whatever fails on the
   acknowledgement's path, a lease is handed back at most once per attempt and no withdrawal cycling can spend an attempt. The pass's
   `NOT_ACKNOWLEDGED` does not set it: silence is the loss this feature exists for, and the cap still bounds it. In the lost-lease
   pass, which is the only pass that ends expired leases, every outcome other than a withdrawal (the cap reached, a requirement no
   longer recorded) MUST end the attempt with `expire_attempt` in the same transaction, as today: an unknown outcome, counted
   `capped` or `lapsed`. The pass never withdraws an acknowledged lease: at a deadline it keeps today's meaning.
6. **The acknowledgement deadline is the lease's own earlier deadline, with a margin of one receive bound.** The derivation is in
   `stream-completion.md` ("Why the deadline is the lease's own"). A host of 1.8 reads every lease within the receive bound (6 s) of
   its claim (`sdks/SPEC.md` §10 for a stream; this decision extends it to `PollTasks` at 1.8) and bounds each acknowledgement
   attempt to the same 6 s; the server answers `ACKNOWLEDGED` only with at least 6 s left before both deadlines, for a first
   acknowledgement, a repeated one and a lease whose mark does not count alike: before the expiry so that the first heartbeat, which
   the host sends at the earlier of its interval and a third of the time left the answer reports less the time its attempt took,
   commits before the lease can expire; before the attempt deadline so that no handler starts with less than that of its timeout.
   The answer reports the time left before both deadlines; a host that finds less than 3 s before the expiry, once its attempt's
   duration is taken off, sends its first heartbeat before it starts the handler, and one that finds less than one heartbeat bound
   (2 s) before the attempt deadline treats the answer as `CANCELLED`. A read, one attempt and the margin fit when the earlier
   deadline is at least 18 s after the claim; a lease with less is not marked.
7. **Run nothing, hand back.** An acknowledgement of a `FORWARD` task's provisional lease whose step is asked to stop, or whose run
   has a stop reason, and of an acknowledged lease or a lease whose mark does not count that has such a stop or less than the margin
   left, answers `ACKNOWLEDGEMENT_VERDICT_CANCELLED` and writes nothing; the host hands the lease back, which is withdrawn while its
   mark counts (decision 5) and is otherwise an ordinary failure, and the withdrawal's settle cancels a stopped step (also of a saga
   whose cancel policy would have let work in flight finish: nothing ran, so nothing is lost). A compensation or reconciliation, which the server leases on purpose while
   a run is stopping, is never answered `CANCELLED` for a stop, as its heartbeat is never told to stop (`sdks/SPEC.md` §6.4).
8. **A provisional lease receives no capability, and a served one is recorded durably before it is served.** Every worker call that
   carries a lease token, other than `Heartbeat`, `CompleteStep`, `FailStep` and `AcknowledgeTasks`, MUST be refused on a
   provisional lease with `FAILED_PRECONDITION` and the stable reason `TASK_NOT_ACKNOWLEDGED`, after the token and the binding have
   matched, through a fence variant (root and path) that the report paths do not use. On any other lease, a capability call writes
   `steps.lease_capability_generation` for the lease's generation, once per generation, under the run or path lock it already holds
   and in a transaction that commits, with `synchronous_commit` at least `local`, before anything is handed over or accepted. A
   contract test walks the descriptor through nested, `oneof` and repeated fields and fails when a lease-token method has no case.
9. **What 1.8 adds to the hand-back rule of Architecture decision 0098.** The server's rule stays in its decisions 5, 7 and 8 and is
   not restated here. 1.8 adds: the reservation is read from `lease_hand_back_generation`, never from the session row, which
   registering again can change; it is honoured only while `delivery-acknowledgement-v1` is recorded, read under the lock its
   removal waits for, and only for a lease claimed on the current server incarnation after that recording, because only then does no
   live engine serve a capability without recording it; the hand-back of a `LEASED` lease with no capability served is withdrawn
   whether provisional or acknowledged and after a deadline too (decision 5); a hand-back of a `RUNNING` lease, of a lease to which a
   capability was served, or of a lease whose reservation does not count is an ordinary retryable failure, a served capability also
   recorded as a protocol violation. A `NOT_SENT` withdrawal, whose token never left the server, needs only `lease-withdrawal-v1`. A
   build of Architecture decision 0098 never honours a hand-back from a 1.8 session: it clamps the holder's minor to its own ceiling
   (Decision 006-059). A host hands back only before any handler of the lease started.
10. **The fleet decides, the operator records.** Every engine of the build lists `delivery-acknowledgement-v1` from the task that
    follows the gate, meaning it serves `AcknowledgeTasks`, replays the cause, refuses capabilities to provisional leases and
    withdraws a marked provisional lease at its deadline and on its hand-back; the requirement MUST be recorded for engines only,
    since a control plane serves no worker call and the floor build's control planes will decode the cause (request (2) of Decision
    006-059), which a released build must have before this design leaves dormancy. It is activated by
    `control-plane capability-activate delivery-acknowledgement-v1`, an entry of the generic command of Architecture decision 0098,
    which refuses while `lease-withdrawal-v1` is not recorded for both kinds. An engine MUST mark only while it knows both are
    recorded so, and every withdrawal MUST read both through one function whose result the decision requires as a type, so no order
    of activations and removals, by the command or by hand, can write an event without its replay floor; a wrong order by hand is
    counted (`requirement_missing`) and alerted. The switch `SAGAT_LEASE_WITHDRAWAL=off` stops new marks and Architecture decision
    0098's withdrawals of unmarked leases, and MUST NOT reach a mark already written, which keeps its meaning until its lease ends.
    Removing the requirement ends the protection of the provisional leases in flight, because engines of the older build may join;
    so a rollback switches marking off and drains the provisional leases before it removes the requirement. An engine of the floor
    build silent at an activation's census serves no capability call until its gate is decided again (Decision 006-059 (7)), so it
    cannot serve one unrecorded; its beat reads the epoch `FOR SHARE SKIP LOCKED`, so an open owner transaction never blocks it; the
    refusal is `UNAVAILABLE` with the envelope reason `REGISTRY_SILENT` and a retry hint, which the SDKs repeat within the handler's
    deadline; and this build inherits the rule with the floor build's code, for engines of either build.
11. **Older minors, declining sessions and internal runners are served exactly as today.** A lease of a session below 1.8, or of a
    session that declines acknowledgement, is never marked. The database runner, the sandbox runner and the command-line probe keep
    their minors; an internal runner that rises to 1.8 or later registers declining unless it implements the host procedure.
12. **Two assumptions, stated for the owner.** The first is the host's: the withdrawal of a provisional lease, and the withdrawal of a
    `LEASED` lease that its host hands back, record a known outcome (nothing ran) because a host that declared 1.8 promised to start
    nothing before the acknowledgement and to hand back only what it did not start; the server cannot verify that promise.
    Constitution principle III is applied to 1.8 leases under that assumption. What the server does see, it counts and audits once
    per step and generation, by the identical-record rule of the audit trail with a reason built from stored values only: a
    `Heartbeat`, `CompleteStep`, or `FailStep` other than the hand-back on a provisional lease; a capability call refused
    `TASK_NOT_ACKNOWLEDGED`; a hand-back after a capability was served; and any report, heartbeat or capability call carrying the token
    of a marked lease withdrawn as `NOT_ACKNOWLEDGED` or on its hand-back (`steps.withdrawn_lease_token_hash`). Since a task shorter
    than its first heartbeat completes from `LEASED`, the window from `ACKNOWLEDGED` to the first heartbeat is covered by the promise
    alone. The second is the database's: one writable server,
    with `fsync` on, whose committed acknowledgements and capability records survive. They commit with `synchronous_commit` at least
    `local`, so a crash that the postmaster recovers keeps them; every other loss of committed transactions (a failover, a recovery, a
    restore that starts a server, the loss of the operating system) brings another server start, whose start time ends every earlier
    mark; and a restore that does not start the server (a logical restore into the same running server, a virtual machine reverted
    with its memory, a storage snapshot rolled back under a running server) is done with the engines stopped and followed by a restart
    of PostgreSQL before they start (`docs/runbooks/disaster-recovery.md` section 7). What remains, Gap 006-048, is that procedure
    skipped, `fsync` off, or more than one writable server.
13. **The cost is measured before the work in six languages, and the minor waits for it.** Right after the server tasks, the
    transitions scenario runs at 1.8 against 1.7 (the median of five interleaved runs each, with a network between the workers and
    the services), a 64-lease call is timed, and heartbeats are timed in a burst. The build goes on only if 1.8 is no more than 15%
    below 1.7 in the transitions runs, the timed 64-lease calls are answered within half the receive bound (3 s) at the 99th
    percentile over every call with fewer than 0.1% answered other than `OK`, and, in the burst run, no heartbeat is shed while
    acknowledgements are admitted and the heartbeat's 99th percentile is under a third of the receive bound (2 s). Then a last server task makes the server offer 1.8
    and list the capability. If it fails, neither happens, Gap 006-030 of Architecture decision 0098 stays open, and the owner
    decides; shipping 1.8 with only the reserved code would fix that meaning of 1.8 for good, and delivery acknowledgement would take
    a later minor. These are gates of the build, not promises of the platform.
14. **What is counted** is listed in `stream-completion.md`, "Observability".
15. **The reserved code at 1.8 for the host.** A host sends `TASK_NOT_STARTED` only for a lease no handler of which has started:
    the hand-backs of `sdks/SPEC.md` §6.2 and §6.8. The SDKs rewrite a handler-raised `TASK_NOT_STARTED` to `WORKER_ERROR` at every
    minor (Decision 006-028 of the SDK build of Architecture decision 0098). The server's rule is decision 9.

## Considered options

1. **A flag on the lease, marks bound to its generation and to the server's incarnation, and an acknowledgement that locks rows
   with `SKIP LOCKED` and never the run (chosen).** Exact by the row lock (decision 4). One transaction of about eight statements
   per call whatever the number of leases; it never waits for a row, so it cannot deadlock on rows or be held up by one busy run; no
   journal event; nothing paid where the operator has not activated, and nothing asked of later minors.
2. **The first round of this design: the acknowledgement under the run lock, renewing the lease.** Equally exact; the renewal is what
   needed the run lock, and with it came a transaction per run, a call bounded by the slowest run, and a renewal channel.
3. **A host rule bound to the minor** ("a session at 1.8 acknowledges every lease"). One field fewer, but offering 1.8 would depend
   on the activation, and so would every later minor.
4. **Variant 1 as sketched in Owner question 006-001: a new step status `OFFERED` and an acknowledgement event.** Equally exact; a
   status reaches every reader of `steps.status` and an event per task adds a journal append per task.
5. **Variant 2: the first heartbeat before the handler.** Equally exact; per task, a transaction that restores the run's whole state,
   applies a transition and appends `step.started`; one more journal event for every short task; it still needs the flag.
6. **A shorter deadline of its own.** Recovers a lost lease sooner, but withdraws healthy deliveries when the database is slowest.
7. **A bound of its own on repeated withdrawals** (the second and third rounds' run of three), or **the claim checking the cap**
   (the fourth round). The first overlapped the cap of four per attempt with other reset rules; the second needed a journal read on
   every claim and repeated the check the withdrawal command makes. The cap stays in the withdrawal command; the bound of decision 5
   is different: it is one column of the row the claim already writes, needs no count and has no reset rule beyond the attempt.
8. **Falling back to 1.7 when the acknowledgement cannot work** (the third round), **declining until the host restarts** (the
   fourth), **counting the server's own shedding toward the host's decline** (the sixth), or **an engine breaker that stops marking
   while the server's answers fail** (the seventh). The first cost every later minor's features; the second let one database
   incident turn the protection off fleet-wide, silently; the third let any tenant switch the protection off for every host on an
   engine with a short burst; the fourth needed thresholds that a flood of cheap calls could hold closed, starved itself of evidence
   once open, and still let a quiet engine cycle leases to the cap. The bound of decision 5 makes every failure of the path cost at
   most one give-up per attempt, so the host declines, for a bounded period, only on evidence of its own path, and nothing else is
   needed.
9. **The database's timeline** (the fifth round), **an operator's fence alone, or a command that ends the marks after a restore**
   (the sixth). The timeline does not change for a snapshot or a base backup started as a server, a managed restore or a cut-over,
   and reading it needed a guarded function; an operator is not in time for an automatic failover; the command needed a table, a
   function and a census that refused in the one case it existed for. The server's start time changes for all of them, every role can
   read it, and it never fails on a standby, and a restart of PostgreSQL does what the command did; its cost is that a planned restart
   ends the protection of the leases in flight.
10. **A trigger that refuses recording a requirement before the one it depends on** (the fifth round, as its own decision). Once
    every withdrawal reads both requirements and the engine marks only while both are recorded, the wrong order writes no event and
    costs only marks that are never honoured; the command's refusal is a convenience and the trigger bought nothing more.
11. **Do nothing beyond Architecture decision 0098.** Leaves Gap 006-001. Rejected by the owner (Owner decision 2026-11-20-12).

## Consequences

Good: a frame lost on the wire and a lost `PollTasks` answer go back to `READY` at the lease's earlier deadline instead of to an
operator, with the attempt number and the idempotency key unchanged; a provisional lease receives no credential, blob or log
stream; a cancellation that arrives before the start is known; a process that crashes between a claim and Architecture decision
0098's withdrawal no longer leaves a review for a marked lease (Gap 006-002 closes for them); the claim, the completion and the
meaning of every option and every later minor are unchanged; a lease's meaning never changes while it is held, whatever an engine's
switch says; a restore or failover of any kind that starts a server ends the marks by itself; no withdrawal cycling can spend an
attempt, and, within the cap and while the requirement is recorded, a lease whose mark counts and that its host hands back while it
is `LEASED` with no capability served, before the pass ends it, is not charged an attempt.

Bad: one more call and one more short transaction before the first handler of a frame starts; under light load that is one per
task, about eight statements and one transaction more on the 88 statements and 3 transactions of a task today (104 against 94
round trips with `BEGIN` and `COMMIT`), which the round-trip model of feature 038 predicts at about 10% of the transitions per
second, plus the trigger work the update fires, which the gate measures; the pass's withdrawal of a marked lease reads both
requirements and the attempt's withdrawals with two statements; a task with an input blob reads it only after the acknowledgement;
the first capability call of a lease writes one column with a local flush. Eight columns on `steps`, one on `worker_sessions`, one
function (the requirements read). An engine whose registry connection fails while its request pool works stops serving capability
calls after the liveness window, until its gate is decided again. 1.8 waits for 1.7 in the
server and in each SDK. Before the activation a 1.8 hand-back spends the attempt, as below 1.8. A planned restart of PostgreSQL ends
the protection of the leases in flight. A lease whose earlier deadline is under 18 s is not protected, nor is an attempt past the
cap, nor the next delivery of an attempt after a hand-back, a planned one (a host's stop, a decline, `CANCELLED`) included (Gap
006-047). While the acknowledgement fails at the server, each flagged lease waits one receive bound before its host hands it back,
and the second claim and withdrawal it then costs fall on a server already in trouble. An acknowledged lease whose host cannot hand
it back, whose hand-back loses to the pass past the expiry, or whose first heartbeat sent before the handler commits with its answer
lost, ends as today: a review or a spent attempt (Gap 006-041). While the liveness rule refuses capability calls (a registry
connection failing while the request pool works, or an owner transaction held open on the epoch during an engine's lapse), a host
below 1.8 does not repeat them, so its handler fails and spends the attempt. Below 1.8 a hand-back is an ordinary failure that spends
the attempt (Decision 006-030 of Architecture decision 0098), which this design does not change and the owner weighs with that
decision. Gap 006-040 to Gap 006-048 remain, of which the two assumptions (Gap 006-042 and Gap 006-048) are the ones the owner
accepts with this decision. There is no emergency stop for the provisional leases in flight short of removing the requirement,
which makes them lapse.

Compatibility with runs in flight: a lease issued before the activation is not flagged and keeps today's meaning; after it, only
leases claimed for sessions at 1.8 or later are flagged. In a fleet that mixes this build with the build of Architecture decision
0098, which the recorded requirement excludes, a hand-back that reaches the older build is an ordinary failure, and this build
honours none while the requirement is not recorded.

Rollback (`stream-completion.md`, "Rollback"): switch `SAGAT_LEASE_WITHDRAWAL` off on every engine, wait until no provisional lease
is left, remove the requirement, and roll the binary back with the switch set back to its value. The floor is enforced, not described,
once it is built: the first released build of Architecture decision 0098 that decodes the cause `NOT_ACKNOWLEDGED` and clamps the
holder's minor (the merged head has the clamp only; the released follow-up is a hard precondition of Task 006-057, so no mark, and
so no `NOT_ACKNOWLEDGED`, exists before the floor does), and every writer of the cause reads `lease-withdrawal-v1` under the lock its removal waits for, which cannot be removed once
a journal holds `step.lease_withdrawn` (Architecture decision 0098 decision 8), so the registry never admits a binary below that
build once any withdrawal exists.

## Evidence

`specs/006-sdk-multilanguage/research.md` Decision 006-040 to Decision 006-066, Gap 006-040 to Gap 006-048, Owner question 006-001
and Owner question 006-002; `specs/006-sdk-multilanguage/stream-completion.md`, "Delivery acknowledgement (protocol 1.8)";
`sdks/SPEC.md` §5, §6.1 to §6.5, §6.8, §9 and §10; Architecture decision 0098 and the build decisions on its two branches;
Architecture decision 0082; `docs/runbooks/disaster-recovery.md` sections 3 to 7; `tests/performance/reports/sdk-streaming.md` and
`tests/performance/reports/scalability-ha.md`; `tests/integration/claim-transition-efficiency/work_budget.rs`; the review rounds
under `specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/`.
