# ADR-0099: A lease of worker protocol 1.8 is provisional until its host acknowledges it, and a provisional lease that expires is withdrawn

**Status:** Proposed
**Decided by:** agent, delegate (OD-2026-11-20-6), on the owner's answer "Yes, in 0.3.0" to delivery acknowledgement (owner decision OD-2026-11-20-12, which left the variant, the protocol minor and the deadline to this design); reviewed by the correctness, security, architecture and design reviewers on 2026-11-21 (`specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/`). Acceptance is the owner's.
**Refines / supersedes:** refines ADR-0029 (decision 2 said a task stream introduces no durable delivery acknowledgement; decision 5 left a lease lost on the wire to the reaper; decision 6 dispatched a received lease at once), ADR-0098 (adds the cause `NOT_ACKNOWLEDGED` to `step.lease_withdrawn` and a requirement that depends on `lease-withdrawal-v1`) and ADR-0015 (adds minor 1.8 under its rule; nothing in the rule changes); uses ADR-0082 (requirements are recorded by the operator only). Supersedes none.
**Enforced by:** to be built with the decision (`specs/006-sdk-multilanguage/tasks.md`, phase "Delivery acknowledgement"): the barrier tests of the acknowledgement against the withdrawal in both orders (`crates/application`), the claim and reaper tests of the provisional mark (`integration_durable_execution`), the budget lines `ACKNOWLEDGEMENT` and `FIRST_HEARTBEAT` and the unchanged `LEASE` line (`integration_claim_transition_efficiency`), the refusal of every capability call on a provisional lease (`contract_grpc_durable_execution`), the negotiation and registry gate tests, the conformance scenario `delivery-acknowledgement` in six languages with every earlier golden unchanged, and the released-v0.2 upgrade scenario `provisional-lease-cutover`.

## Context

The engine commits a lease before it knows the task reached a host. ADR-0098 withdraws the leases the server knows it never
handed to the transport, and a host hands back the leases it drops (`TASK_NOT_STARTED`). Two losses stay out of its reach,
gap G006-001 of `specs/006-sdk-multilanguage/research.md`: a frame of `StreamTasks` handed to the transport and lost on the
wire, and the answer of a unary `PollTasks` lost after its handler returned. Each lapses after the lease time-to-live and,
since feature 037, a lapsed lease of an `Activity` is an unknown outcome that goes to consultation or `step.needs_review`, for
work that never ran. The owner chose to close it in release 0.3.0 with a new protocol minor in all six SDKs.

What constrains the choice:

- **Exactness.** The claim to make is "a handler of a lease runs only after the server has committed that the host has the
  lease"; from it follows "a lease that the server never saw acknowledged was not run, by a host that keeps the protocol".
- **Cost.** Throughput follows round trips (`specs/038-claim-transition-efficiency/research.md`, finding 1). Today a task
  costs a lease (38 statements, 2 transactions, 5 rows written) and a completion (50 statements, 1 transaction, 11 rows),
  the lines of `tests/integration/claim-transition-efficiency/work_budget.rs`. A task shorter than its heartbeat interval
  (a third of the lease, 500 ms to 10 s) sends no heartbeat at all and writes no `step.started`.
- **The run lock.** Every write of a run's rows happens under that run's lock (`PgStore::begin`, which also takes the
  continuation-chain lock). The reaper and the withdrawal decide on what they read under that lock.
- **Older SDKs and released v0.2.** The server keeps serving every older minor exactly as it does (ADR-0015 decision 1); a
  release-0.2 binary does not register in the instance-capability registry, so no activation can see it.
- **Minors are cumulative.** A session at 1.8 also speaks 1.7 (step logs, design 041, held by OD-2026-11-15-8) and 1.6.
  The server does not serve 1.7 yet (`PROTOCOL_MINOR` is 6 on `develop`; task T041-7 raises it).
- **The conformance goldens.** Each scenario compares the journal of every run with a reviewed golden. A change that adds a
  journal event to every task would change most goldens.
- **Measured latencies.** Ready to handler entry, 99th percentile, 751 to 889 ms over the ten burst rows of five SDKs and
  both transports (`tests/performance/reports/sdk-streaming.md`), which bounds commit to handler entry from above; a
  completion, which does strictly more than an acknowledgement (it loads the run, applies the transition and routes),
  6 to 103 ms at the 99th percentile over the nine rows of 2026-11-12 (`tests/performance/reports/scalability-ha.md`).

## Decision

1. **Minor 1.8 adds `AcknowledgeTasks` and one host rule.** A host accepted at 1.8 MUST NOT start a handler of a lease, and
   MUST NOT call `Heartbeat`, `ResolveSecrets`, `ResolveDatabase`, `WriteBlob`, `ReadBlob` or `AppendStepLogs` for it, before
   the server answered `ACKNOWLEDGEMENT_VERDICT_ACKNOWLEDGED` for that lease. It acknowledges every lease of a frame, or of a
   `PollTasks` answer, in one call sent as soon as the frame is read (the wire and the host procedure are
   `specs/006-sdk-multilanguage/stream-completion.md`, "Delivery acknowledgement (protocol 1.8)"). Minor 1.8 is registered in
   `sdks/SPEC.md` §9; it is cumulative, so it is served only once 1.7 is.
2. **The provisional mark is lease data, not a step status and not a journal event.** A claim for a session accepted at 1.8,
   made while the requirement `delivery-acknowledgement-v1` is recorded, MUST mark the lease provisional
   (`steps.lease_offered_at` set to the commit time, `steps.lease_acknowledged_at` empty) in the statement that already writes
   the lease. Nothing else about the lease changes: the token, the generation, the attempt, the expiry (commit plus the lease
   time-to-live) and the credit it spends. Every write that clears or replaces a lease clears both columns.
3. **The acknowledgement is one transaction per run under the run lock, and it loads no run state.** For each run of the
   call, in request order, the server opens the run as `Heartbeat` does (`begin` with `Lock::Wait`) and, for each task of that
   run, applies the checks of `Heartbeat` (the step identity, the lease-token hash on a step `LEASED` or `RUNNING`, the
   binding) and then writes the step row: the acknowledgement time if it was empty, the attempt deadline moved later by the
   time the lease waited (decision 6), and the lease renewed as a heartbeat renews it. It MUST NOT load the run's state, apply
   a transition, append a journal event or write the outbox. A task whose checks fail is answered
   `ACKNOWLEDGEMENT_VERDICT_NOT_HELD` and nothing of it is written; a run of another tenant is indistinguishable from a
   missing one and is audited as `Heartbeat` audits it. A second acknowledgement of the same lease answers `ACKNOWLEDGED` again
   and changes nothing but the renewal.
4. **A provisional lease that expires is withdrawn, never lapsed.** In the lost-lease pass, re-checked under the run lock, a
   lease that is still `LEASED`, still provisional and still unacknowledged at its expiry MUST be withdrawn as ADR-0098
   decision 3 withdraws (one fenced transaction, `step.lease_withdrawn` with the cause `NOT_ACKNOWLEDGED`, the step back to
   `READY`, the attempt not consumed, no review), while the requirement is recorded. An acknowledged lease that expires keeps
   today's meaning: an unknown outcome, consultation or review. The acknowledgement and the withdrawal both hold the run lock
   and both fence on the lease, so exactly one of them commits for a lease, and the host runs the handler only on the verdict
   of the acknowledgement: this is the whole exactness argument, and no clock enters it.
5. **The acknowledgement deadline is the lease's own expiry.** No other deadline is added. The derivation is written in
   `stream-completion.md` ("Why the deadline is the lease expiry"); in short: a host of 1.8 reads every lease it will ever read
   within the receive bound R = 6 s of its commit (`sdks/SPEC.md` §10 for a stream; this decision extends the same bound to a
   unary `PollTasks` at 1.8), and one acknowledgement takes A, measured at no more than 0.103 s at the 99th percentile; any
   deadline from R + A = 6.103 s up to the time-to-live is equally safe, because safety comes from decision 4, and the
   deadline only trades how long a lost lease holds its step against how many healthy deliveries are withdrawn under load. The
   expiry gives a margin of 23.897 s at the default 30 s, adds no parameter, and keeps the bound a host already has to meet
   with its first heartbeat. The server MUST NOT offer 1.8 when its lease time-to-live is under R + ⌈A⌉ = 7 s.
6. **The attempt deadline of a provisional lease counts from its acknowledgement.** The lost-lease pass MUST NOT time out a
   lease that is provisional and unacknowledged (its expiry, decision 4, applies). The acknowledgement moves the attempt
   deadline later by the time between the claim's commit and the acknowledgement, so a step's `timeoutMs` measures the attempt
   from the moment its host has the task. Leases of older minors keep the attempt deadline of today.
7. **A provisional lease receives no capability.** `ResolveSecrets`, `ResolveDatabase`, `WriteBlob`, `ReadBlob` and
   `AppendStepLogs` for a lease that is provisional and unacknowledged MUST be refused with `FAILED_PRECONDITION` and the
   stable reason `TASK_NOT_ACKNOWLEDGED`, in the query that already checks the lease. `Heartbeat`, `CompleteStep` and `FailStep`
   are served as today: each takes the step beyond the reach of decision 4 (`RUNNING`, or a reported attempt), which is the
   safe direction for a host that broke rule 1.
8. **A withdrawn lease of a stopping run ends cancelled.** The withdrawal runs the engine's ordinary settle in its
   transaction, so the planner cancels the step it returned to `READY` when the run is stopping (`plan.rs`: a `READY` step of a
   stopping run is `step.cancelled`). A host that is answered `ACKNOWLEDGED` with `cancel_requested = true` MUST NOT start the
   handler and hands the lease back with `TASK_NOT_STARTED`, which ADR-0098 decision 5 withdraws and the settle cancels.
9. **The fleet decides, the operator records.** Every control plane and engine of the build lists the capability
   `delivery-acknowledgement-v1`. The operator records the requirement with `control-plane delivery-acknowledgement-activate`
   (owner role, one bounded transaction, `--operator`, `--expect-live N`, `--dry-run`), which MUST refuse while
   `lease-withdrawal-v1` is not recorded; the removal of `lease-withdrawal-v1` MUST refuse while
   `delivery-acknowledgement-v1` is recorded. An engine offers 1.8, marks leases provisional and withdraws with
   `NOT_ACKNOWLEDGED` only while it knows the requirement is recorded (it learns within one registry heartbeat, ADR-0082
   decision 4). The store proves it can (`WorkerStore::supports_delivery_acknowledgement`, true once the migration is applied)
   beside the proofs of 1.5, 1.6 and 1.7.
10. **Older minors are served exactly as today.** A lease of a session below 1.8 is never provisional; it lapses, or is
    withdrawn by ADR-0098, as it would be without this decision. The database runner keeps 1.6 and the sandbox runner keeps 1;
    the internal runners are not moved by this decision (gap G006-007).
11. **What is counted.** `worker_task_acknowledgements_total{verdict}` (`acknowledged`, `duplicate`, `not_held`);
    `worker_leases_undelivered_total{cause="not_acknowledged", outcome}` (ADR-0098's counter: `withdrawn`, `fence_missed`,
    `lapsed`); the histogram `worker_task_acknowledgement_delay_seconds` (the database clock, acknowledgement time minus offer
    time), which is the measured input of decision 5; the gauges `worker_leases_provisional` and
    `worker_leases_provisional_expired` from the operations statistics query; `worker_capability_calls_refused_total{method,
    reason="TASK_NOT_ACKNOWLEDGED"}`; and two alerts (`stream-completion.md`).

## Considered options

1. **A provisional lease marked on the step row, acknowledged by a batched call under the run lock (chosen).** Exact by the
   run lock (decision 4). Per frame of n leases over r runs it adds one call and r transactions that load no run state and
   append nothing; the claim keeps its statements; the happy path writes no journal event, so no conformance golden of an
   existing scenario changes. A host can still hand back an acknowledged lease that it did not start (`TASK_NOT_STARTED` acts
   on `LEASED`).
2. **Variant 1 as sketched in owner question O006-001: a new step status `OFFERED` and an acknowledgement event.** Equally
   exact, but a new status reaches every reader of `steps.status` (the claim's counts of a session's leases, the reaper, the
   cancellation planner, reset, Studio, the operations statistics, the projections and the replay of every control plane and
   engine), and an event per task adds a journal append per task and changes every golden. It buys nothing the lease columns
   do not give: the provisional state is a property of the lease, like its expiry and its token, which are not journaled
   either.
3. **Variant 2: the first heartbeat before the handler.** Equally exact (the first heartbeat writes `step.started` under the
   run lock), and it reuses `Heartbeat` and `step.started`. It costs one call and one engine transaction per task, not per
   frame: that transaction loads the run's state, applies a transition and appends `step.started`, a strict superset of the
   statements of option 1 (the build pins both lines and asserts it). It adds one journal event to every task shorter than its
   heartbeat interval, which is most tasks, so every such golden changes. The server must still mark which leases carry the
   contract, so it saves no column. Once the step is `RUNNING` a host can no longer hand back a lease it did not start.
4. **A shorter deadline of its own (for example R + A, about 7 s).** Recovers a lost lease about 23 s sooner. Lost leases are
   rare (three observed cases since feature 037: two CI jobs and one play of feature 022), while a deadline close to the healthy delivery time withdraws healthy deliveries
   when the database is slowest, and each such withdrawal claims and delivers the same task again: load added exactly when the
   system is overloaded. It also adds a parameter to derive, configure and test. Rejected; an operator who wants faster
   recovery lowers the lease time-to-live, down to 7 s, and gets it for every lease.
5. **One acknowledgement statement with no run lock** (an `UPDATE` over the step rows of the call). One statement per call,
   but the reaper and the withdrawal decide from a read made under the run lock and then write the row; a write that does not
   hold the run lock would let a withdrawal overwrite an acknowledgement it did not see. Rejected: it breaks the invariant that
   makes decision 4 exact.
6. **The acknowledgement in a bidirectional stream.** Needs a bidirectional stream in six SDKs, gives nothing for a unary
   poll, and breaks the finite receive window of ADR-0029. Rejected.
7. **The acknowledgement carried by the next poll.** The handler would wait for the next receive call. Rejected.
8. **Do nothing beyond ADR-0098.** Leaves G006-001: a review for work that never ran after each loss on the wire. Rejected by
   the owner (OD-2026-11-20-12).

## Consequences

Good: a frame lost on the wire and a lost `PollTasks` answer go back to `READY` at the lease's expiry instead of to an
operator, with the attempt number, and so the idempotency key, unchanged; a provisional lease never receives a secret, a
database credential, a blob or a log stream; cancellation of a task that has not started is known and immediate; the attempt
deadline measures the attempt, not the delivery; the claim, the completion and every existing golden are unchanged.

Bad: one more call per frame and one more transaction per run in the frame, whose statements the build pins; the throughput of
the transitions scenario at 1.8 is not measured yet and must be before the release (task T006-064); the handler of every task
starts one acknowledgement later; one more capability, one more operator command and one more migration on `steps`; 1.8 cannot
be served before 1.7 is; a host that stops between the verdict and the handler, or loses the acknowledgement's answer and the
network for a whole lease, still leaves an acknowledged lease to lapse into a review (gaps G006-004 and G006-005). The
guarantee binds hosts that keep the protocol: a hand-written client that runs a handler without acknowledging it, uses no
capability and dies before any report, has its work run again with the same attempt number and idempotency key (gap G006-006;
external effects stay at least once, constitution principle III).

Compatibility with runs in flight: a lease issued before the activation is not provisional and keeps today's meaning; after it,
only leases of sessions accepted at 1.8 are provisional, and a session is accepted at 1.8 only after the activation.

Rollback: remove the requirement first; within one registry heartbeat no engine offers 1.8, marks a lease provisional or writes
`NOT_ACKNOWLEDGED`, and a provisional lease that then expires lapses as before. Sessions already accepted at 1.8 keep acknowledging
(harmless: an acknowledgement of a lease that is not provisional is a renewal) until they register again. Then roll the binary
back. A journal that holds `step.lease_withdrawn` with the cause `NOT_ACKNOWLEDGED` cannot be replayed by a binary older than
this decision, which is why the cause is written only after the activation; the `down` migration refuses while a provisional
mark is present.

## Evidence

`specs/006-sdk-multilanguage/research.md` decisions D006-011 to D006-024, gaps G006-004 to G006-008 and owner question
O006-001; `specs/006-sdk-multilanguage/stream-completion.md`, "Delivery acknowledgement (protocol 1.8)";
`sdks/SPEC.md` §6.2 to §6.4, §9 and §10; ADR-0098 and its evidence (CI jobs 136567175914 and 136453650156, the C# play of
`flow-rate-limit` on PR #251); `tests/performance/reports/sdk-streaming.md` and `tests/performance/reports/scalability-ha.md`
for the measured latencies; `tests/integration/claim-transition-efficiency/work_budget.rs` for the budget lines.
