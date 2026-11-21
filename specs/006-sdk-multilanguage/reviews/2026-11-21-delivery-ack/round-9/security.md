# Security review, round 9: delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)

**Verdict: no Blocker, 1 Major, 4 Minor, 5 Nit.**

My round-8 Minors 1, 2 and 4 are resolved; Minor 3 is partly resolved. The new rule (an acknowledged lease that is still `LEASED` is withdrawn when its host hands it back) gives a hostile host nothing it did not already have. A hostile host can simply never acknowledge (Gap 006-042). The problem is a buggy host: the evidence the rule relies on is weaker than the text claims, and part of it is missing (Minors 1 and 2). The Major is in request (7), which is the floor build's to make.

"Verified" means I read the cited code in `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack`, or in tonic 0.14.9, the version in `Cargo.lock`. "Design reading" means the defect is in the documents alone.

## Status of round 8

| Round-8 finding | Status | Reason |
|---|---|---|
| Minor 1: the attempt's flag could be cleared with the lease | **Resolved** | It is now attempt data with one allowed form, evaluated against the old row, and no other statement touches it. The pass's `NOT_ACKNOWLEDGED` no longer sets it, and the withdrawal that reaches the cap does. My scenario now ends with the fifth lease unflagged. Task 006-050 has the `NOT_SENT` cases. |
| Minor 2: no relief from server-wide trouble | **Resolved** | After one receive bound of answers carrying the envelope, the host gives the lease up at once and hands it back. A commit whose outcome is unknown is now safe too, because that hand-back is withdrawn. The cost is named in "Bad". |
| Minor 3: "no answer" counts toward a decline; a slow engine declines its hosts | **Partly resolved** | The probe is sound evidence. But the deadline "in the outermost layer, before the per-connection limit" cannot be built the way the server is assembled (Minor 3). |
| Minor 4: the engines were not stopped before a restore | **Resolved** | Section 7 step 5, the drill paragraph and Gap 006-048 now say so. Where the sentence sits is Nit 4. |
| Nit 1, stale "for engines" | Resolved | |
| Nit 2, "every status it makes" | **Partly resolved** | `stream-completion.md:691` still says it (Nit 1). |
| Nit 3, overclaim at the cap | Resolved | |
| Nit 4, `fsync` | Resolved | |
| Nit 5, a revoked `pg_postmaster_start_time()` | **Not resolved** | Carried as Nit 5. |
| Nit 6, request (8) and the migration | Answered on paper, now stale | The migration is merged (Minor 4). |

Others' round-8 findings in my domain:
- **Correctness Major 3 and design Minor 2** (withdraw an acknowledged `LEASED` lease on its hand-back): led to Minors 1 and 2.
- **Design Major 2** (the stop rule for `FORWARD` tasks only): Nit 3.
- **Correctness Minor 1** (the beat reads the epoch `FOR SHARE`): Major 1.
- **Correctness Major 1** (the probe): Nit 2.
- **Design Major 1** (throughput): resolved by the early give-up.

---

## Major

### Major 1. Request (7)'s `FOR SHARE` puts every process's beat behind any open owner transaction on the epoch; the runbook's own removal procedure then closes the gate of every engine and control plane (verified)

**Where.** `research.md:788` (Decision 006-059 (7): "a beat that reads the epoch `FOR SHARE`, as registration does"), and `ADR-0099` decision 10.

**What the code has.**
- Today `instance_heartbeat` reads the epoch with no lock (`crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql:269`), after it has updated its own row (`:260`).
- The file's lock-order rule says the epoch comes first, and "no function waits for a lock while it holds a later one" (`:11-13`).
- A beat that fails for any reason other than a lost connection for longer than the liveness window closes the process's whole gate (`crates/application/src/instances.rs:31-33`).
- The removal by hand SQL holds the epoch `FOR UPDATE` in an open owner transaction, and the feature's own check is meant to sit inside it: `BEGIN; SELECT platform_requirement_lock(); -- (the feature's own check …) …; COMMIT;` (`docs/runbooks/instance-registry.md:157-161`). This design names that hand SQL as a way to remove `delivery-acknowledgement-v1` (`stream-completion.md:398`). Its rollback drains the provisional leases before the removal (Decision 006-055).

**Scenario.**
1. An operator rolls back. They open the hand-SQL transaction, then wait inside it for the drain, or leave the `psql` session idle in the transaction.
2. Every beat of every engine and control plane now waits on the epoch row until its 5 s statement timeout.
3. After 30 s every gate closes and the whole fleet stops serving: heartbeats, reports and claims included.

Today the same open transaction blocks only new registrations. This breaks (7)'s own promise ("heartbeats, reports, hand-backs and claims keep flowing") and decision 5 of Architecture decision 0082 ("never fatal"). The command path is bounded to about 8 s and does not trigger it; the hand SQL is not bounded.

**Fix (in the floor build's follow-up migration).**
- Use `CREATE OR REPLACE FUNCTION instance_heartbeat` and keep `SECURITY DEFINER SET search_path = pg_catalog, pg_temp`. The `FOR SHARE` must stay inside the function: `sagat_app` has only `SELECT` on the epoch table (`:404`).
- Read the epoch first, with `FOR SHARE SKIP LOCKED` and not `STRICT`.
- When the row is skipped, still commit `last_seen` and return `out_epoch = NULL`, meaning an activation is in progress.
- The gate treats that as "capability calls undecided" (refused with `REGISTRY_SILENT`), never as a registry failure.
- Test: an owner transaction holds `platform_requirement_lock()` for 60 s. No gate closes, heartbeats and reports flow, capability calls are refused with `REGISTRY_SILENT`, and they are served again after the commit.

---

## Minor

### Minor 1. Withdrawing an acknowledged `LEASED` lease rests on evidence that a short handler never produces, and nothing records it afterwards (verified against the code; design reading for the rule)

**What the code has.**
- Only the first heartbeat moves a step from `LEASED` to `RUNNING` (`crates/domain/src/execution/command.rs:508-515`).
- `CompleteStep` is accepted from `LEASED` (`crates/domain/src/execution/state.rs:1544`).
- The first beat comes at the earlier of the interval (at most 10 s) and a third of the time left (`sdks/SPEC.md:474-481`).

So a handler shorter than that never heartbeats, and one that uses no capability leaves no record at all. Decision 006-045's claim, "the capability record and the heartbeat are the evidence round 1 lacked", is untrue for the common case.

**What the design has.**
- `withdrawn_lease_token_hash` is kept only for `NOT_ACKNOWLEDGED` (marks table, `stream-completion.md`).
- `step.lease_withdrawn` carries no record of the acknowledgement (`crates/domain/src/execution/event.rs:699-704`), and the withdrawal clears the marks.

**Scenario.**
1. One of six SDKs has a defect: a give-up timer races a late `ACKNOWLEDGED`, so the lease is handed back after its handler started.
2. The handler charges a card through an external API in 3 s and calls no capability.
3. The hand-back lands first and is withdrawn. The journal says nothing ran.
4. The next lease runs again with the same attempt number and idempotency key. With `maxAttempts = 1` the step used to fail instead.
5. The handler's late `CompleteStep` gets the ordinary stale answer, which is not counted.

**Fix.**
- (a) Set `withdrawn_lease_token_hash` on every withdrawal of a lease whose mark counts, `NOT_STARTED` included. A later heartbeat, report or capability call with that token is then counted, audited and alerted, as for `NOT_ACKNOWLEDGED`.
- (b) Add the label `mark` (`provisional`, `acknowledged`) to `worker_leases_undelivered_total` for `NOT_STARTED`, with an alert clause for acknowledged hand-backs.
- (c) Correct Decision 006-045. In decision 12 and Gap 006-042, say that the trust assumption now covers the window from `ACKNOWLEDGED` to the first heartbeat.
- (d) For the owner to choose: send the first heartbeat before every flagged handler. A conforming host's `LEASED` would then mean "not dispatched", at the cost of one transaction per short task, measured by the gate.

### Minor 2. The documents disagree on whether a `recovered` lease's hand-back is withdrawn, and the wrong reading trusts a capability record that may have been lost (design reading)

**Where.**
- The documents that say "withdrawn":
  - ADR decision 7 (`ADR-0099-…md:121-123`): of "a lease whose mark does not count … the host hands the lease back, which is withdrawn (decision 5)".
  - Decision 006-051: "its hand-back is withdrawn while the lease is `LEASED`".
- The documents that say "ordinary failure":
  - Decision 9 (`:138`): "a lease whose reservation does not count is an ordinary retryable failure".
  - The table row "A mark that does not count" in `stream-completion.md`.

**Why it matters.** A mark that does not count means one of two things. Either another server incarnation took over, or the lease was claimed before the current recording. In either case `lease_capability_generation` can be missing:
- A failover to an asynchronous standby loses the record of secrets already served.
- A floor engine never records them.

A host then hands the lease back, and "no capability served" withdraws it as "nothing ran".

**Fix.**
- Decision 7 and Decision 006-051: the hand-back is withdrawn only when the mark counts; a `recovered` lease's hand-back is an ordinary failure.
- Add that case to the unit test of the pure function.

### Minor 3. The deadline cannot start before the per-connection limit as the server is built; a host saturated with transfers declines (verified)

**What the code has.**
- tonic 0.14.9 puts the layers added with `Server::builder().layer(...)` inside `inner` (`transport/server/mod.rs:812`).
- It then wraps `inner`, per connection, in `ConcurrencyLimitLayer` and `GrpcTimeout` (`:1277-1282`).
- The platform uses `concurrency_limit_per_connection(64)` and `.layer(admission)` (`crates/api-grpc/src/server.rs:157`, `:164`). So any deadline layer sits inside the limit.
- Streams beyond `max_concurrent_streams` (128) wait at the client, outside every server bound.

**Scenario.**
1. A host has 64 `WriteBlob` transfers in flight on one connection. A transfer may last up to 600 s (`server.rs:83`).
2. Each `AcknowledgeTasks`, and its probe, queues behind them: no answer within 6 s, and no envelope.
3. After three counted calls the host declines for 10 minutes, doubling up to an hour.

This fails toward today's behaviour, hence Minor.

**Fix.**
- Drop tonic's per-connection limit. Enforce it in the platform's own layer, under the deadline layer, keyed by connection (`Incoming` already wraps each one), with a reserve for the acknowledgement class.
- Host side: a call made while the host's own connection had the server's per-connection limit in flight does not count. Alternatively, `AcknowledgeTasks` uses a connection of its own.
- The "saturated connection" test of Task 006-049 must saturate the connection with long transfers.

### Minor 4. The merged floor build does not answer requests (2), (6), (7) and (8), and (6) and (8) still say "edit the migration before it is applied anywhere" (verified)

**What the code has.**
- The guard text has not been changed: `20270208000000_lease_withdrawal.up.sql:57`.
- `lease_withdrawal_required()` still ignores `kinds` (`:71-76`).
- `WithdrawalCause` has only `NotSent` and `NotStarted` (`event.rs:438-443`).
- `REGISTRY_SILENT` appears nowhere in `crates/`.

Editing a migration that has been applied breaks sqlx's checksum on every database that applied it.

**Fix.**
- Write a new migration that uses `CREATE OR REPLACE` for both functions and repeats `SECURITY DEFINER SET search_path = pg_catalog, pg_temp`. Without them the function silently becomes invoker-rights with an unpinned path, its `FOR SHARE` fails for `sagat_app`, and every withdrawal fails.
- Add a privilege test that asserts `prosecdef` and `proconfig`.
- Update the wording of (6) and (8) and precondition (a) to name the follow-up floor pull request.

To check whether the migration has been applied anywhere:
- `git tag --contains $(git log -1 --format=%H -- crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql)`
- `SELECT version, checksum FROM _sqlx_migrations WHERE version = 20270208000000;` on each environment.

---

## Nit

1. **Stale lines.**
   - `stream-completion.md:688-689` says "flagged at most once … at most one give-up". Silence no longer sets the flag, so a lease can be withdrawn up to four times before the cap sets it.
   - `:690-692` says "waited out", which contradicts `:685-686`, and "every status it makes", which contradicts the narrowed claim.
2. **The probe's answer is not defined.**
   - Say that the host never dispatches on a probe's answer. Any verdict other than `NOT_HELD` means the hand-back did not land: the host hands back again (withdrawn under decision 5) and does not count the call.
   - Today probes also fall into `not_held{no_match}` beside forged tokens; Minor 1 (a) separates them.
3. **The rule for `FORWARD` tasks only rests on a property nothing enforces.**
   - The heartbeat returns `step.cancel_requested` whatever the step's kind (`command.rs:507`, `crates/application/src/worker.rs:1908-1938`).
   - `sdks/SPEC.md:492` ("never cancelled by the server") holds only if the planner never sets the flag on a compensation or reconciliation. The planner sets it on `LEASED` and `RUNNING` steps after its skips (`plan.rs:430`, `:696`, `sagas.rs:159`).
   - Fix: a domain property test that `StepCancelRequested` is never planned for a compensation or reconciliation step.
4. **The instruction to stop the engines sits in section 7, "After a restore"** (`disaster-recovery.md:92`). Add one line to section 4, or to a short section that comes before any restore.
5. **Round-8 Nit 5 is carried:** a start-up `has_function_privilege` probe for `pg_postmaster_start_time()`.

---

## Classes checked with no new finding

- **What a hostile host gains from the new rule.** Nothing beyond Gap 006-042.
  - A served capability blocks the withdrawal.
  - A capability call and a hand-back are ordered by the run or path lock.
  - The secret check runs before the hand-back (`worker.rs:2451-2480`).
- **Abuse of the probe.** None found.
  - The probe carries the host's own token and matches by token hash.
  - It writes no audit record and leaks nothing new.
  - The next lease has a new token and generation.
- **The early give-up.**
  - A forged envelope hurts only the host behind that path.
  - Admission is per engine, not per tenant (`limits.rs:489-551`; tenant budgets "belong to the bearer pipeline", `:4`). So one tenant's flood costs other tenants about 6 s and one claim per flagged lease. This is bounded and named in "Bad".
- **Tenant boundary, audit and logs.**
  - The new labels (`stop`, `conflict`, `flagged_before`) are closed sets.
  - The step's kind is read, never returned.
  - `REGISTRY_SILENT` carries no tenant data.
- **Least privilege of the new column.** Covered by the existing table grants, and its one form is evaluated under the row lock.
- **Injection.** No dynamic SQL and no new unbounded input.

## Commands to run once the build exists

- Major 1: hold `BEGIN; SELECT platform_requirement_lock();` open for 60 s as the owner, then run `SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test -p sagat-tests --test integration_platform_foundation`.
- Minor 3: `SAGAT_SDK_SCENARIO=delivery-acknowledgement cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture`, with 64 transfers in flight on the host's connection. It must not fall back.
- Minor 1: the Task 006-050 matrix with a `CompleteStep` sent after a `NOT_STARTED` withdrawal of an acknowledged lease. It must be counted and audited.

## Files

- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/disaster-recovery.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/instance-registry.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/instances.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/domain/src/execution/command.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/domain/src/execution/state.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/domain/src/execution/event.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/domain/src/execution/plan.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/domain/src/execution/sagas.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/server.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/limits.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/workers.rs`
- `/Users/marcelo/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/tonic-0.14.9/src/transport/server/mod.rs`
