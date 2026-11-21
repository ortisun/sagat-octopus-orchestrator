# Architecture review, round 7: delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)

## Verdict

**No Blockers. One Major, and it blocks the merge of the floor build (the build of Architecture decision 0098), not only this design.** Counts: 0 Blockers, 1 Major, 8 Minors, 8 Nits.

- **Round 6 is answered.** My Major 1 is resolved, and so are all nine of my round-6 Minors. Minors 2 and 3 are resolved by removing what they were about. Of the round-6 findings by the other reviewers that fall in my domain, all are resolved except security Minor 3, which is only partly resolved; that remainder is the Major below.
- **Withdrawing Architecture decision 0100 is sound.** I confirm the design reviewer's argument (round 6, design Minor 5). Three things are now enforced weakly or not at all:
  - a wrong order applied by hand SQL is silent (Minor 2);
  - the read function checks `lease-withdrawal-v1` for the wrong process kinds (Minor 3);
  - the rule that replaced the trigger has no structural check (Minor 4).
- **The new liveness rule is the Major.**
  - It is recorded only in Architecture decision 0099, but the floor build has to carry it before it merges, and precondition (a) of the task phase leaves it out.
  - It changes the fail mode of the accepted Architecture decision 0082 for every deployment, including those that never activate the feature, and for more calls than it needs to.
  - It is measured from a point that leaves a window open.
- **The rest:**
  - the server's reasons for its transient answers are not one closed list across the three normative texts (Minor 1);
  - the census of `end-lease-marks` gives the wrong answer in the one case the command exists for (Minor 5);
  - `platform_operator_actions` has a generic name, columns specific to one action, and an unclear `down` (Minor 6);
  - the design does not say where the breaker lives (Minor 7);
  - the breaker can make the cost gate pass with the protection partly off (Minor 8).

**Limits of this review.**
- I had no shell, so I could not run `git branch -a` or `git worktree list`. A glob over the sibling directories timed out again.
- The server branch (`/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases`) has no `AcknowledgeTasks`, `acknowledge_before_start`, `lease_claim_incarnation`, `end_lease_marks` or `platform_operator_actions` under `crates/` or `apps/`. I found no duplicate of this work.
- I read the code of the floor build. It has none of requests (2), (3) or (7) of Decision 006-059 yet, as expected.

---

## Status of the round-6 findings

### My Major

| Round 6 | Status | Reason |
|---|---|---|
| Major 1: stamp the server's start time | **Resolved**, and more than I asked | <ul><li>`lease_claim_incarnation` from `pg_postmaster_start_time()` replaces the timeline outright. I had proposed both stamps. Dropping the timeline loses nothing: a promoted standby, a point-in-time recovery, a returning old primary and every restore that starts a server all have another start time.</li><li>It removes the guarded function (my round-6 Nit 12) and the probe.</li><li>Gap 006-048 shrinks to a logical restore into a running server, which `end-lease-marks` covers.</li><li>A drill case (a snapshot-style restart) and the parity cases are added.</li><li>One residue for Gap 006-048 is Nit 7: a server resumed from a memory snapshot.</li></ul> |

### My Minors and Nits

- **Minor 1: resolved.** The command now has:
  - a table for its record;
  - `--expect-live 0` and `--dry-run`;
  - an update limited to `LEASED` and `RUNNING`.

  The criterion of its census is wrong for a logical restore (Minor 5), and the table has issues of its own (Minor 6).
- **Minors 2 and 3: resolved by removal.** The trigger and `RequiredBy` are gone.
- **Minor 4: resolved.** The drain and the `down` wait for `worker_leases_provisional`. The rollback case of Task 006-066 has a long acknowledged task, and the switch is set back at step 4.
- **Minor 5: resolved.** The capability record raises `synchronous_commit`. The test is in Task 006-052.
- **Minor 6: resolved.** At the deadline the transaction is rolled back and the connection returned; the connection is discarded only after 100 ms. Task 006-049 asserts "no connection discarded".
- **Minor 7: resolved.** I found no remaining stale text. The only mention of Architecture decision 0100 outside the reviews is the historical sentence in Decision 006-047.
- **Minor 8: resolved.** The test names "an engine of the floor build".
- **Minor 9: resolved.** The step is unconditional and marked "(pending Task 006-054)", and a runbook test comes in Task 006-054. Two small gaps remain (Nit 4).
- **Nits:**
  - Nits 1 to 3 are moot.
  - Nits 4, 5, 7, 9, 10 and 11 are resolved.
  - Nit 6 is resolved. The incarnation also covers a loss caused by `fsync = off`, because that loss needs an operating-system crash, which restarts the server.
  - Nit 8 turned into the liveness rule (Major 1).
  - Nit 12 is moot for the timeline, but the same pattern returns with `fsync` (Nit 3).

### Other reviewers' round-6 findings in my domain

| Finding | Status |
|---|---|
| Security Major 1 (a tenant's burst declines every host on an engine) | **Resolved** by Decision 006-065: the server marks its transient answers with a reason, the engine has a breaker, and the host never counts the server's own trouble. The list of reasons is not closed across the texts (Minor 1). The breaker's effect across tenants is an owner item. |
| Correctness Major 1 (a host with full slots never falls back) | **Resolved.** Hand-backs count as answered calls, give-ups are ordered, 60 s separate the first and third, and the `max_inflight` case is added. |
| Security Minor 3 (a floor engine silent at the census) | **Partly resolved.** The rule exists, but its record, precondition, scope and measurement point are the Major. |
| Security Minor 4 and design Minor 6 (`end-lease-marks` record, and before the engines start) | **Resolved**, except for the census criterion (Minor 5). |
| Design Major 1 (start time) | **Resolved** (see my Major 1 above). |
| Design Major 2 (the rollback's wait never ends) | **Resolved.** |
| Design Minor 5 (cut Architecture decision 0100) | **Resolved.** Its caveat ("the two read functions must check the process kinds the event needs") is half answered (Minor 3). |

---

## Withdrawing Architecture decision 0100: is the reasoning sound?

**Yes.**
- The event `step.lease_withdrawn` is written only by a withdrawal. Every withdrawal of a marked lease reads both requirements under the share locks their removals wait for, and writes nothing when either is missing.
- `lease-withdrawal-v1` cannot be removed once a journal holds the event (`20270208000000_lease_withdrawal.up.sql:52-65`).
- While `delivery-acknowledgement-v1` is recorded for engines, no engine below this build is admitted.

So no order of recordings and removals can leave an event without its replay floor. A wrong order costs only marks that are never honoured.

What the trigger used to give, and what replaces it now:
- **The guarantee.** It now rests entirely on every withdrawal path calling `delivery_acknowledgement_required()`. Only tests of today's paths check that (Minor 4).
- **The operator's signal.** A hand-SQL recording of `delivery-acknowledgement-v1` without `lease-withdrawal-v1` now silently turns the protection off, and no alert names it (Minor 2).
- **The number 0100.** The record is deleted, but the reviews of rounds 5 and 6 cite it, and ADR numbers are shared across branches (Nit 1).

---

## Major

### Major 1. The liveness rule is recorded in the wrong place, is missing from the floor build's precondition, narrows an accepted record's availability for every deployment more than it needs to, and leaves a window at its boundary

**Where:**
- Architecture decision 0099, decision 10, last sentence, and the "Refines" line;
- `stream-completion.md:411-414` ("Liveness");
- Decision 006-059 (7) (`research.md:647`) and Decision 006-064 (`research.md:686`);
- `tasks.md:139-144` (precondition (a): "requests (1) and (3) to (6) are answered");
- Task 006-046 (`tasks.md:188`);
- Architecture decision 0082, decision 5;
- the gate code on the server branch, `crates/application/src/instances.rs:27-37` and `:411-427`.

**What is wrong.**

1. **The record.**
   - The rule must hold in the floor build. That build is where an engine serves a capability without recording it, and it merges before this design.
   - Yet the rule lives only in Architecture decision 0099, which the owner accepts later, and in request (7). Precondition (a) lists requests "(1) and (3) to (6)", so the floor build can merge without (7).
   - Decision 006-064's argument ("an engine silent at the census serves no worker call after it reconnects") then has no gate. The omission would first show at Task 006-066, the integrator's released-upgrade test, after all six SDKs are built.
   - The rule also refines the accepted Architecture decision 0082 from inside a record about leases. Architecture decision 0082 already has a pattern for this: its "Amendments" section lists Architecture decision 0083 narrowing its decision 2.
2. **The scope.**
   - Architecture decision 0082 decision 5 keeps a process serving through a plain connection failure, "stopping the fleet would make an outage longer". The registry has a connection of its own (decision 8). A broken registry connection while the request pool works is a realistic case: after a failover the pool reconnects, while one connection hangs on a dead socket.
   - Under the new rule, every worker call is refused after 30 s of registry silence: heartbeats, completions and stream messages too. Leases then lapse into unknown outcomes and reviews for tasks that are running fine. This happens on every deployment of the floor build and of this build, whether or not delivery acknowledgement is ever activated.
   - The hazard is narrower than that. Only a capability served unrecorded matters: the calls of decision 8, which are every lease-token call except `Heartbeat`, `CompleteStep`, `FailStep` and `AcknowledgeTasks`. Heartbeats, reports, hand-backs and claims of a floor engine are harmless here.
     - A floor engine's claim never writes `lease_hand_back_generation`, so its lease never counts.
     - A floor engine's hand-back is clamped to an ordinary failure.
   - Readiness already fails after the window (decision 5, last sentence), so new connections already move away. Refusing capability calls on existing connections is the smallest addition.
3. **The boundary.**
   - The gate counts silence "from its last answer" (`instances.rs:411-427`, `heard_at` after the answer returns). The database counts liveness from `last_seen = clock_timestamp()` inside the beat's `UPDATE`, before the commit.
   - So the engine believes it is live for the commit time plus the answer's transit longer than the database does. An activation in that interval passes its census while the engine still serves.
   - A capability call admitted just before the cut-off and served just after it widens the window further.
   - Task 006-046's test ("held silent past the window … refuses after reconnecting") does not exercise this boundary.
4. **The runbook.** `docs/runbooks/instance-registry.md` section 3 ("While it runs") documents the fail mode this rule changes. Task 006-046 updates only section 5.

**Smallest correction:**
- **Record the rule as an amendment of Architecture decision 0098 on its branch** (decision text, "Refines Architecture decision 0082 decision 5", "Enforced by"). Add an entry to the "Amendments" section of Architecture decision 0082 at acceptance, as Architecture decision 0083 did. Architecture decision 0099 decision 10 then cites it, and its "Refines" line drops Architecture decision 0082. If the owner prefers Architecture decision 0098 unchanged, use a small record of its own instead; either way, not Architecture decision 0099.
- **Add (7) to precondition (a).**
- **Narrow the rule** to the capability calls of decision 8. Keep heartbeats, reports, hand-backs and streams flowing.
- **Measure from the start of the last successful beat, with a margin.** Stop at the liveness window less the beat call's bound. Check the gate again at the point of hand-over (after the capability record commits, before the answer is sent), so an admitted call cannot serve past the cut-off.
- **Add a boundary test.** Hold the beat's answer with a barrier so the engine's last answer lags `last_seen`, activate at the database's 30 s mark, and assert the engine already refuses `ResolveSecrets`.
- **Update section 3 of `docs/runbooks/instance-registry.md`.**
- **Add the availability cost to the Consequences** ("Bad"): a broken registry connection stops capability calls on that engine after 30 s.

**Blocking:** yes, before the floor build merges (it is a request of Decision 006-059) and before Task 006-046.

**Own decision or amendment?** The rule is a refinement of Architecture decision 0082, not a decision of delivery acknowledgement. Under the template's one-decision-per-record rule it belongs to the build that must carry it first (Architecture decision 0098), recorded as an amendment of Architecture decision 0082 in that record's Amendments section. The rationale ("the gate reopens one round trip after the database returns") is sound for capability calls. It is not sound for heartbeats, whose refusal costs reviews.

---

## Minors

### Minor 1. The server's reasons for its transient answers are not one closed list: a lock timeout still counts toward the host's decline

**Where:**
- `sdks/SPEC.md:373-377` (normative for hosts): only `ACKNOWLEDGEMENT_SHED` and `ACKNOWLEDGEMENT_DEADLINE`;
- `stream-completion.md:651-653` (host item 5): also "the store unavailable";
- `stream-completion.md:404-407` (the breaker): also "a lock or statement timeout, or the store unavailable";
- Architecture decision 0099 decision 10: "(shed, deadline)";
- `stream-completion.md:537-539` (server item 7): a deadlock, a serialization failure, a lock or statement timeout and an unavailable store answer `UNAVAILABLE` with no reason named.

**What is wrong.**
- Per the normative host text, an `UNAVAILABLE` without a reason counts toward the third trigger.
- A revocation holding the binding's lock is listed as a cause the breaker absorbs. It ends acknowledgement calls with lock timeouts, while heartbeats and the stream are answered. So after 60 s every affected host declines: round-6 security Major 1 in a narrower form.
- Six SDKs will implement whichever text they read.

**Fix:**
- One closed set of reasons, carried in the error details of every answer the server makes for its own trouble. For example `ACKNOWLEDGEMENT_SHED`, `ACKNOWLEDGEMENT_DEADLINE` and `ACKNOWLEDGEMENT_BUSY` (lock or statement timeout, deadlock, serialization failure), plus `STORE_UNAVAILABLE`.
- The same list in `sdks/SPEC.md` §6.2, host item 5, the breaker's inputs and decision 10.
- In the worker contract's error comment (Task 006-043).
- A test in Task 006-049 that every `UNAVAILABLE` and `RESOURCE_EXHAUSTED` the handler or its admission returns carries one of them.

**Blocking:** before Task 006-043.

### Minor 2. With the trigger gone, `delivery-acknowledgement-v1` recorded by hand without `lease-withdrawal-v1` silently turns the protection off

**Where:** `stream-completion.md:388-394` and the skip reasons at line 1053 of the diff (the order `not_activated`, `switch_off`, …); `SagatDeliveryAcknowledgementInactive`.

**What is wrong.**
- In that state every engine skips with `not_activated`, the same reason as "nothing is activated".
- The inactive alert watches only `no_durability` and the breaker. Its own wording says it exists for "the operator believes the protection is on and it is not".
- The command refuses this order, but the hand SQL of `docs/runbooks/instance-registry.md` section 5 does not.

**Fix:**
- A distinct skip reason, `requirement_missing`, counted when `delivery-acknowledgement-v1` is recorded and `lease-withdrawal-v1` is not, placed right after `not_activated`.
- Add it to `SagatDeliveryAcknowledgementInactive`, with a `promtool` case (Task 006-054).
- One row in the runbook.

### Minor 3. `delivery_acknowledgement_required()` checks `lease-withdrawal-v1` "for engines", but the event's replay floor needs control planes too

**Where:** `stream-completion.md:396-400`; Task 006-044; round 6, design Minor 5's caveat; `lease_withdrawal_required()` on the server branch (`20270208000000_lease_withdrawal.up.sql:71-77`, which checks only that the row exists).

**What is wrong.**
- Control planes replay journals: the event reads and the Studio. A control plane below the floor is excluded only by `lease-withdrawal-v1` recorded for `CONTROL_PLANE`.
- The generic command records it for both kinds (`capability_activation.rs:42-44`). But `platform_requirement_activate` accepts any kinds by hand, and the new function's stated test, "for engines", would accept a recording for engines only.

**Fix:**
- The function returns `lease-withdrawal-v1` as recorded only when its `kinds` contain both kinds, and `delivery-acknowledgement-v1` only when its `kinds` contain `EXECUTION_ENGINE`.
- Add a case with `lease-withdrawal-v1` recorded for engines only to Task 006-050.
- Ask the floor build to apply the same check to `lease_withdrawal_required()`, as request (8) of Decision 006-059, or record it as a gap of that build.

### Minor 4. The rule that replaced the trigger, "every withdrawal reads both requirements", has no structural check

**Where:** Decision 006-047 (`research.md:505`); Task 006-050; the server branch's `crates/application/src/worker/withdrawal.rs:224-230` and `:257-263`, where the decision function takes `required: bool`.

**What is wrong.**
- The guarantee now rests on every present and future withdrawal path calling the read before it decides.
- The decision function takes a plain boolean, so a new path (feature 022 will add activation entries, and later minors may add causes) can pass `true` and compile.
- Today's paths are tested. A future path is not.

**Fix (the smallest fitness function):**
- The port's read returns a witness type, for example `RequirementsHeld { lease_withdrawal, delivery_acknowledgement, recorded_since }`, with a private constructor in the adapter's crate.
- The pure decision function and the withdrawal command take that type instead of booleans. A path that did not read cannot be written.
- Name it in the fitness table of `docs/engineering/architecture-principles.md`.

### Minor 5. The census of `end-lease-marks` gives the wrong answer in the one case the command is kept for, and is not taken under the epoch lock

**Where:** `stream-completion.md:452-462` ("refuses while any engine's registry beat is newer than the server's start time"); Task 006-054; `docs/runbooks/disaster-recovery.md` section 7, step 5.

**What is wrong.**
- The command exists for a logical restore into a server that keeps running.
  - There the server's start time is old (weeks), and the restored `serving_instances` rows carry `last_seen` from the moment of the dump, which is newer than that start time.
  - So the command refuses after every such restore. The operator learns to pass `--expect-live 0` every time, and the check becomes a ritual.
- After a restore that started another server, the criterion works, but there the command has nothing to do.
- The census is described apart from the update. An engine starting between them registers and starts its pass.

**Fix:**
- Count the engines that are live as the registry defines it: `last_seen` within 30 s on the database's clock (Architecture decision 0082 decision 2), and also later than the server's start time.
- Take the census and the update in one transaction after `platform_requirement_lock()`, as the activation does, so no engine registers in between.
- Test both kinds of restore in Task 006-054: rows restored from a dump are not counted, and a live engine is.

### Minor 6. `platform_operator_actions` has a generic name and columns specific to one action, no closed list of actions, and a `down` that is unclear

**Where:** `stream-completion.md:457-462`; Task 006-044 (`tasks.md:169`); Architecture decision 0099, Consequences.

**What is wrong:**
- **Name and columns disagree.** The name invites the next deployment-level command to reuse the table. Its columns (the count, the server's start time, the live engines found) belong to one action, and there is no `action` column with a `CHECK`, as `platform_requirement_history` has (`20270113000000_instance_registry.up.sql:72-87`).
  - The precedent for commands of one feature is a table of their own (`schedule_trigger_handover`).
  - A general deployment-level operator audit would be a decision about the platform, not a column of this feature.
- **The `down` is ambiguous.** "Drops the functions and the table only when it is empty" could mean it refuses or it keeps the table.
  - The runbook step runs on every restore, even where the feature was never activated. So the table fills in deployments that never used the feature.
  - Architecture decision 0082's rule is that the `down` refuses once evidence exists. Dropping evidence silently is not allowed.
- **The guards are not tested.** The evidence lists the privilege audit but no test that `UPDATE`, `DELETE` and `TRUNCATE` are refused.

**Fix:**
- Either name it for its action (`lease_mark_endings`), or keep the generic name with `action text NOT NULL CHECK (action IN ('END_LEASE_MARKS'))` and say in Decision 006-062 that it is the deployment-level home for later operator commands.
- State that the `down` refuses while the table holds a row, as Architecture decision 0082's history does, and list that refusal in "Rollback" step 5.
- Add the guard tests (as owner and as the runtime role) to Task 006-044.

### Minor 7. The breaker's home, its inputs and its state are not stated, and two layers must feed it

**Where:** Architecture decision 0099 decision 10; `stream-completion.md:404-410`; Decision 006-065; Task 006-047 (`tasks.md:195`, "the breaker … and its gauge"); Task 006-049.

**What is wrong:**
- **Two layers feed it.**
  - A shed is decided by the gRPC admission (`crates/api-grpc/src/limits.rs`), before the application service runs.
  - A deadline, a lock timeout and an unavailable store are decided in the application service.
  - The claim that reads the breaker is in the application.
- **The obvious placement violates the layers.** An implementer who puts the breaker next to the admission makes the claim path depend on an adapter. Nothing says it is per process, in memory and unpersisted.
- **The word is taken.** The codebase already has a durable breaker of compensations (`BreakerPolicy` in the application, `crates/infra-postgres/src/breaker.rs`), which is a different kind.
- **The tasks do not wire it.** Task 006-047 builds the breaker. Task 006-049, which produces its inputs, does not mention feeding it. The server-side evidence row "Breaker" has no server task.

**Fix:**
- State the shape.
  - A pure type in `crates/application` (window, thresholds, an injected clock; one instance per engine, built in `apps/execution-engine`).
  - Fed through one narrow trait by the admission layer and by the acknowledgement service.
  - Read by the claim service, which passes a boolean to the statement, as the switch is passed.
  - Named `AcknowledgementBreaker`, to keep it apart from the compensation breaker.
- Give `worker_acknowledgement_breaker_open` no label, since the scrape target names the engine.
- Add to Task 006-049 an integration test: a burst shed at admission opens the breaker, and the next claims are unmarked with `acknowledgement_failing`.

### Minor 8. The breaker can make the cost gate pass with the protection partly off

**Where:** Architecture decision 0099 decision 13; Decision 006-057; Task 006-056 (`tasks.md:298`, "the breaker's behaviour in the burst … reported").

**What is wrong.**
- Under load the breaker unflags leases, and unflagged leases cost no acknowledgement.
- If it opens during the interleaved throughput runs, the "no more than 15% below 1.7" criterion measures 1.8 with some of the protection off.
- The criterion "fewer than 0.1% answered other than `OK`" is then met partly because fewer calls are made.

**Fix:**
- In the throughput runs, require that the breaker stayed closed (`worker_lease_marks_skipped_total{reason="acknowledgement_failing"}` at 0, no `to="open"` transition). Otherwise the run is invalid.
- Report the breaker only for the burst run.
- One sentence in Decision 006-057.

---

## Nits

1. **The number 0100.**
   - Rule 1 of `docs/adr/README.md` says numbers are shared by every branch. The reviews of rounds 5 and 6 cite "Architecture decision 0100".
   - Keep a stub, `ADR-0100-…md`, with `Status: Withdrawn (2026-11-21, folded into Architecture decision 0099 as considered option 10)`. Or add one README line saying the number is retired, so another branch does not take it.
   - Check that `scripts/check-agent-config.py` accepts the status.
2. **"Enforced by" and wording of Architecture decision 0099.**
   - The "Enforced by" line lacks the breaker's tests, the liveness rule's test (if it stays there; see the Major), the refusal of `end-lease-marks` with a live engine, and the guards of its table.
   - Decision 10 states its four rules in prose. State the breaker and the marking condition as MUST, as decisions 5 and 8 do.
3. **`fsync` "unreadable counts as off".**
   - `current_setting('fsync')` raises an error for a role without the right; it does not return null. In the claim statement, that error would fail every claim, not only the marks.
   - Read `(SELECT setting FROM pg_catalog.pg_settings WHERE name = 'fsync') = 'on'`. The view omits a parameter the role may not see, so the subquery is null and counted as off.
   - Or drop the clause and rely on Task 006-044's verification.
4. **Runbook marker and test.**
   - Nothing fails when "(pending Task 006-054" stays after that task is ticked. Extend `scripts/check-design-docs.py`, or the new runbook test, to fail on a pending marker whose task is ticked.
   - Make the runbook test parse the whole invocation with `cli::parse`, flags included (`--operator NAME`, `--expect-live 0`, `--dry-run`), so a renamed flag also fails.
5. **"Where that is read".**
   - Say which statements read `required_since`: the withdrawals and the hand-back do. The claim, the acknowledgement, the gauges and the `down` do not.
   - Give the macro's two forms names, so the parity test and the static check know which one each statement uses.
   - An acknowledgement that does not read it moves a lease toward "an attempt", which is safe. Say so.
6. **The convenience refusal of the activation entry.** Say how the generic command of Architecture decision 0098 decision 4 expresses it: a `requires: &'static [Capability]` field of `Activatable`, checked after the epoch lock. Feature 022 will use the same mechanism.
7. **Gap 006-048.** A server resumed from a memory snapshot keeps its start time and loses the commits since the snapshot. Examples are a virtual machine snapshot with memory and a checkpoint-and-restore of the process. List it beside the logical restore, since the owner accepts the gap as written.
8. **Cross-tenant effect.** One sentence in "Failure modes": a burst of one tenant on an engine opens that engine's breaker, so every tenant's leases on it go unflagged until 60 s after the burst. This is the safe direction, and it is alerted after 10 minutes.

---

## The liveness rule against Architecture decision 0082 decision 5

- **The direction is consistent with that decision.** Its last sentence already fails readiness after the window. Refusing capability calls on connections that are already open is the matching step.
- **Its rationale is only half answered.**
  - For capability calls, "the gate reopens one round trip after the database returns" is true and sufficient.
  - For heartbeats it is false. A registry connection can fail alone, and a refused heartbeat costs a review.
- **Narrowing to capability calls keeps decision 5's reason intact** and closes the hazard of Decision 006-064 completely.
- **Placement and measurement:** see the Major.

## The breaker's layering

- **Domain:** untouched.
- **Application:** the policy and its window, as a pure type with an injected clock, plus a narrow trait that both producers call. This needs stating (Minor 7).
- **Adapters:**
  - the gRPC admission and the acknowledgement handler report outcomes;
  - the PostgreSQL claim statement receives one boolean, as the switch is received;
  - nothing is persisted.
- **Composition root:** one instance per engine, in `apps/execution-engine`.
- **State per process:** yes, by design. With a balancer that spreads calls, each engine sees its share. That is stated in the text, and it is acceptable.

## `platform_operator_actions` against Architecture decision 0082 and `platform_requirement_history`

- **What the design applies correctly:**
  - the operator's act only (the function is revoked from `PUBLIC` and granted to no role);
  - `ENABLE ALWAYS` append-only guards;
  - not readable by the runtime role;
  - the operator text checked as the registry checks it;
  - `session_user` recorded beside it;
  - the record written in the function's own transaction.

  This is the deployment-level evidence pattern that `platform_requirement_history` set, because `audit_records` is tenant-scoped.
- **Kept apart from the history.** That is right. The history's `action`, `capability`, `kinds` and `epoch` are about requirements, and extending it would weaken its checks.
- **What is missing:** the closed list of actions or a specific name, the `down` rule, and the guard tests (Minor 6).

## Recording the requirement for engines only

- **Architecture decision 0082 decision 7:** consistent. Nothing is listed that the process will not do, and control planes of this build do not list it (Task 006-057's evidence checks both).
- **Architecture decision 0098 decision 4:** consistent. Entries carry their kinds, and `KINDS_DIFFER` protects a recording for other kinds.
- **The replay floor for control planes** rests on `lease-withdrawal-v1` covering control planes, which the read function should check (Minor 3).
- **The cost saved is real:** a rollback no longer restarts control planes for this feature.

## The build plan's order

- Task 006-044 (the migration, the read function, the table), then Task 006-046 (dormant registry, liveness), then Task 006-047 (marks and breaker), then Task 006-049 (the acknowledgement), then Task 006-050 (withdrawals), then Task 006-054 (the command, the runbook test), then Task 006-056 (the gate), then Task 006-057 (leaving dormancy). The order works.
- Three adjustments:
  - request (7), narrowed, joins precondition (a) (Major);
  - Task 006-049 wires the breaker (Minor 7);
  - Task 006-056 invalidates a throughput run in which the breaker opened (Minor 8).
- The reason list must be settled before Task 006-043 (Minor 1).

## Fitness functions for the rules this change relies on

| Rule | Enforced by | Gap and smallest addition |
|---|---|---|
| No event without its replay floor, in any order of recordings | The barrier tests of a removal between a withdrawal's read and its commit, in both orders, and in one hand-SQL transaction (Task 006-051); the matrix with each requirement recorded or not (Task 006-047) | A witness type for the read (Minor 4); the kinds checked (Minor 3) |
| The wrong order is visible | none | A skip reason `requirement_missing` and its alert case (Minor 2) |
| A mark counts only on its server incarnation | The parity test; the matrix; the drill, with a snapshot-style restart | none |
| The capability record survives | The `synchronous_commit = off` test (Task 006-052) | none |
| An engine silent at the census serves no capability unrecorded | Task 006-046 and Task 006-066 | The boundary test with a held beat answer; precondition (a) (Major) |
| The server's trouble never declines a host | The conformance cases of the shed and of the 503 | A closed list of reasons and a test over the handler's error mapping (Minor 1) |
| The breaker opens and closes | Its unit test | A server integration test from the admission's shed to an unmarked claim (Minor 7) |
| The gate measures the protection on | none | The breaker closed throughout the throughput runs (Minor 8) |
| `end-lease-marks` runs only with no engine live | The command's test | Both kinds of restore, and the census under the epoch lock (Minor 5) |
| The operator record is append-only | The privilege audit | Guard tests for `UPDATE`, `DELETE` and `TRUNCATE` (Minor 6) |
| A documented command exists | The runbook test (Task 006-054) | Flags parsed; a check that the pending marker is gone (Nit 4) |
| The drain ends | `worker_leases_provisional`; the long acknowledged task (Task 006-066) | none |

## Identifiers (constitution 4.7.0)

- The added prose uses the full forms: "Architecture decision 0082", "Decision 006-065", "Task 006-054", "Gap 006-048", "Owner question 006-002", "Requirement 032-009".
- The short forms appear only where a tool reads them: register headings, task lines and file names.
- `stream-completion.md:454` ("decisions D006-020 to D006-023") is older context, not an added line.
- I found no abbreviation from the refused list among the added lines.

## Items for the owner

1. **Accept Architecture decision 0098 first, as amended on its branch, with requests (1) to (7) of Decision 006-059 in its code before it merges.** Request (7), the liveness rule narrowed to capability calls, changes the fail mode of the accepted Architecture decision 0082 decision 5 for every deployment. It should be accepted as an amendment there (Major).
2. **Accept Architecture decision 0099 with both assumptions of decision 12.** What remains is a logical restore into a running server, covered by the runbook's step, and a server resumed from a memory snapshot (Nit 7).
3. **A planned restart of PostgreSQL ends the protection of the leases in flight.** That is the cost of the incarnation.
4. **Answer Owner question 006-002,** the reservation of 1.8.
5. **The delegate's thresholds.**
   - The 15% bound and "fewer than 0.1% answered other than `OK`".
   - The breaker's 20% of at least 20 calls over 60 s, closing after 60 s.
   - The host's 60 s minimum between the first and the third give-up.
   - The inactive alert after 10 minutes of an open breaker.
6. **The breaker is per engine and shared by all tenants.** One tenant's burst unflags every tenant's leases on that engine for its duration plus 60 s. This is the safe direction.
7. **Retire the number 0100** (Nit 1).
8. **Choose between a general deployment-level operator record (`platform_operator_actions`) and a table for this command only** (Minor 6).
9. **The constraints that stay.**
   - Before the activation, and after a removal, a 1.8 hand-back spends the attempt.
   - There is no emergency stop for the provisional leases in flight.
   - The rollback restarts every engine with `SAGAT_LEASE_WITHDRAWAL=off`.
10. **Release coupling for 0.3.0.**
    - Task 041-3, Task 041-7, and Task 041-15 to Task 041-20;
    - both builds of Architecture decision 0098;
    - the gate.

## What blocks what

- **Major 1:** before the floor build merges, and before Task 006-046.
- **Minor 1:** before Task 006-043, which carries the contract.
- **Minors 3, 5 and 6:** before Task 006-044, because they settle schema.
- **Minors 2 and 7:** before Task 006-047 and Task 006-049.
- **Minor 4:** before Task 006-050.
- **Minor 8:** before Task 006-056.
- **The Nits:** during the build. Nit 1 is before this design merges.

## Files relevant to this review

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md (header lines 3-5; decision 10 at lines 133-148; decision 13; Consequences)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/README.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0082-the-instance-registry-records-requirements-only-for-the-operator-and-a-refused-process-stays-up-with-a-closed-gate.md (decisions 2, 5 and 8; Amendments)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md (lines 382-414, 447-462, 537-546, 651-664)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md (Decision 006-047 at line 505, Decision 006-059 at 632-650, Decision 006-064 at 686, Decision 006-065 at 699, Gap 006-048 at 734)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md (precondition (a) at lines 139-144; Task 006-044 at 169, Task 006-046 at 188, Task 006-047 at 195, Task 006-049 at 212, Task 006-054 at 280, Task 006-056 at 298, Task 006-066 at 367)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md (lines 370-381)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/disaster-recovery.md (line 82; section 7, step 5 at line 92)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/instance-registry.md (sections 2, 3 and 5)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/instances.rs (lines 21-38, 411-427, 949-969)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/worker/withdrawal.rs (lines 224-230, 257-263)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql (lines 49-87, 147-168, 247-277, 312-378)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql (lines 31-85)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/apps/control-plane/src/capability_activation.rs (lines 31-44)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/src/breaker.rs (the existing compensation breaker, for the naming)
