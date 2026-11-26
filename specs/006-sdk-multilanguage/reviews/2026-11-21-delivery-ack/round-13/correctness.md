# Correctness review of Architecture decision 0099 (delivery acknowledgement, worker protocol 1.8), round 13

## Verdict

My round-12 Major (the deciding attempt reaching an engine that has not recovered) is resolved for the scenario I gave. The new reason `ACKNOWLEDGEMENT_STORE_UNREACHABLE` covers it, because an acquisition stuck on a stale connection ends as a pool timeout. The base registry already classes a pool timeout as an unreachable database.

All four of my round-12 Minors are resolved or reduced to the residues listed below. One fix introduced a new Major: moving Task 006-044 to the waiting list leaves three tasks that still proceed but need its columns.

**Counts:** 0 Blockers, 1 Major, 4 Minors, 8 Nits.

Nothing was built or run. Claims about the code come from reading `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack`.

## Status of the round-12 findings

### Major 1 from my report (the deciding attempt reaching an engine that has not recovered): resolved

**What the documents now say:**
- The server answers `UNAVAILABLE` with `ACKNOWLEDGEMENT_STORE_UNREACHABLE` when its deadline passed before it held a connection, or when its connection failed.
- A deciding attempt that ends with that reason decides nothing: `sdks/SPEC.md:388-398`, `stream-completion.md:605-608` and `:779-789`, and the new failure row at `:899`.
- Task 006-049 and Task 006-058 test it.

**Checked against the code, for my scenario:**
- The acknowledgement's acquisition is bounded by the call's time left.
- With sqlx's ping before acquisition (`pool.rs:72-76`), a stale idle connection whose peer is gone hangs the ping until that bound, so the acquisition ends as a pool timeout or the call's own timeout. Either way it is "before it held a connection".
- The registry already counts `PoolTimedOut`, `Io`, `Tls`, `PoolClosed`, `WorkerCrashed`, class `08` and `57P01` to `57P03` as an unreachable database (`crates/infra-postgres/src/instances.rs:73-93`).
- So engine E2 answers the new reason, the host keeps its backoff, and a later attempt is answered `recovered`.

**What is left:**
- The classifier is not named, and two cases fall on the wrong side of it (Minor 3).
- The words "held a connection" do not fit `begin_safely` (Nit 6).
- An engine with a stale pool can stay unrecovered past the ordinary give-up, and Gap 006-047 does not say so (Nit 7).

### Minors from my report

| Round 12 | Status | Reason |
|---|---|---|
| Minor 1: the immediate attempt as a retry loop | Resolved | Attempts keep the backoff of §5 and any hint. The early send happens only once 6 s have passed with no shed, after up to 200 ms of jitter, at most once per lease per receive bound (`sdks/SPEC.md:394-396`, `stream-completion.md:785-787`). The SDK unit tests cover a shed burst and fast failures (`tasks.md`, Task 006-059). Residue: Nit 1. |
| Minor 2: whether acknowledgement attempts are "failed tries" | Resolved | The list is now explicit (heartbeat, report, hand-back, receive; never an `AcknowledgeTasks`), and so are the statuses. Adding "a receive" has an edge of its own (Minor 2 below). |
| Minor 3: Task 006-044 and the follow-up rewriting the same checks | Resolved as to the order | The migration sorts after the follow-up's and lists `lease-withdrawal-v2` (`tasks.md:144-146`). Moving the task to the waiting list creates Major 1. |
| Minor 4: a half-open acknowledgement connection | Resolved | The connection is dialled again after an attempt with no answer while the main connection answers. The means are named per SDK (`sdks/SPEC.md:366-370`). The redial has an edge of its own (Minor 1 below). |

### Nits from my report

- **Nit 1 (timing statements):** resolved.
  - The failure row (`stream-completion.md:881`) and Decision 006-065 (`research.md:941-942`) now say "one receive bound and its deciding attempt, more when heartbeats are the only evidence".
  - The proxy case reads "at the end of its deciding attempt".
  - The Bound row now has a task heartbeating.
- **Nit 2 (a frame counted by its arrival):** resolved by recording it in Gap 006-047 (`research.md`, the Gap 006-047 paragraph) and in the failure section.
- **Nit 3 (the reserve "from the activation"):** resolved. The text now says "from the build of Task 006-057".
- **Nit 4 (`lease-withdrawal-v2` one-way):** resolved. The guard is narrowed to `NOT_ACKNOWLEDGED`. The old wording remains in places (Nit 2 below).
- **Nit 5 (holding a lease in backoff):** resolved by a harness host setting for the first backoff step (Task 006-058).
- **Nit 6 (what the second refusal cannot see):** resolved. The claim is softened, Gap 006-044 records the gone row, and the runbook removes a silent row only once its process is confirmed stopped.

### Other round-12 reports, in my domain

| Finding | Status |
|---|---|
| Security Minor 1 (the beat's read order) | Resolved. A beat that gets the epoch row `FOR SHARE SKIP LOCKED` reads the latest committed version, and holds it until it commits, so no activation can commit in between. When an activation holds the row, the beat skips it and reports a change in progress. The skip path never waits, so no deadlock appears, even though the beat touches its own `serving_instances` row first. The stale sentence in Decision 006-059 is Nit 3. |
| Security Minor 2 and architecture Minor 1 (the trigger on `platform_requirements`) | Resolved. A registration that lacks `lease-withdrawal-v2` cannot slip past the trigger, because once `lease-withdrawal-v2` is recorded such a registration is refused (`instance_registry.up.sql:197-204`). Residue: Nit 8. |
| Architecture Minor 4 and security Nit 2 (the heartbeat's kind from the journal) | Resolved. The read can use `run_events_instance_idx` (tenant, run, instance, sequence; `20261027000000_scoped_instance_identity.up.sql:45-47`), as `lease_withdrawals` does (`execution.rs:7677-7695`). The generation is in the event's detail (`event_codec.rs:538-545`). The scan is bounded by one step's events. |
| Architecture Minor 2 and design Nit 3 (narrowing the guard) | Resolved. A writer of `NOT_ACKNOWLEDGED` reads `lease-withdrawal-v2` under its share lock, so a removal waits for it and its guard then sees the event, as for `lease-withdrawal-v1` (`20270208000000_lease_withdrawal.up.sql:52-77`). Residue: Nit 2. |
| Design Minors 3 and 4, architecture Minor 3 (connection per SDK, permit per attempt, merged frames) | Resolved. Residue: Minor 1 and Minor 4. |
| The third pinned phase of #260 | Partly resolved (Nit 4). |

## Blocker

None.

## Major

**Major 1. Task 006-044 now waits for the follow-up, but Task 006-048, Task 006-049 and Task 006-052 still "proceed", and every one of them needs Task 006-044's columns. The plan cannot be built as written. The obvious shortcut (landing Task 006-044's migration early) is the hazard that round-12 Minor 3 removed.**

Confidence: confirmed in the documents.

**Where:** `tasks.md:144-150`; Task 006-044 at `tasks.md:179-186`; Task 006-048 at `:240`; Task 006-049 at `:244-256`; Task 006-052 at `:328-330`.

**What each proceeding task needs:**
- **Task 006-048:** it stores `declines_acknowledgement` on the session. That column of `worker_sessions` is created by Task 006-044.
- **Task 006-049:** its adapter `acknowledge` writes `lease_acknowledged_at`. Its verdicts need the marks: `duplicate` needs `lease_acknowledged_at`; `recovered` needs `lease_claim_incarnation`; `withdrawn` and `unmarked` need `lease_offered_generation` and `withdrawn_lease_token_hash`. All are Task 006-044's columns on `steps`.
- **Task 006-052:** it writes `lease_capability_generation`, also Task 006-044's column.
- **The fixtures do not help:** "SQL fixtures until Task 006-047 and Task 006-050 land" can write rows, but not create columns.

**Why it matters:** an implementer following the list either stops (three proceeding tasks have nothing to run against) or lands Task 006-044 before the follow-up. Landing it early reintroduces the double rewrite of `platform_requirements_capability_known` and its two siblings (`20270208000000_lease_withdrawal.up.sql:12-29`). Whichever migration runs later drops the other's value, and Task 006-057's recording of `delivery-acknowledgement-v1` then fails.

**Why the review rounds missed it:** this round's change moved one task into the waiting list without re-reading what the proceeding tasks consume.

**Smallest fix (either):**
- **Split Task 006-044.**
  - Part one proceeds: the columns on `steps` and `worker_sessions`, the grants, the storage classification, and a `down` that refuses while a mark counts. It touches no closed list and names no capability.
  - Part two waits and sorts after the follow-up: `delivery_acknowledgement_required()`, the trigger, the three closed lists and `Capability::ALL`.
- **Or move Task 006-048, Task 006-049 and Task 006-052 into the waiting list**, and say that only Task 006-043 and the SDK tasks proceed.

## Minor

**Minor 1. The redial closes a connection that can carry up to three other attempts. Those attempts are cut by the host itself, but under the rules they end "with no answer". So they can give their leases up early, can set off a second redial, and behave differently in each SDK.**

Confidence: plausible. The documents do not say what happens to the attempts in flight.

**Where:** `sdks/SPEC.md:368-370`, `stream-completion.md:602-603`, `research.md` (the connection paragraph of Decision 006-065), Task 006-059.

**Scenario:**
1. **The trigger:** attempts A (lease a) and B (lease b) are on the acknowledgement connection. A was sent at 0 s, and its server path hangs past the server's own bound (a defect, or a mesh that holds the call). A heartbeat on the main connection, sent at 1 s, is answered at 1.05 s. At 6 s, A ends with `DEADLINE_EXCEEDED` (no answer), so the redial rule fires.
2. **The cut:** B was sent at 5.9 s, after evidence for b, and b's 6 s have passed. B would be answered `ACKNOWLEDGED` at 6.2 s.
   - Go (`ClientConn.Close`) and .NET (disposing the channel) cancel B. B ends with no answer and is neither a shed nor `ACKNOWLEDGEMENT_STORE_UNREACHABLE`, so lease b is given up and handed back. The step was acknowledged on the server, so that is a withdrawal of an acknowledged lease: a second claim and a delay that the outcome did not need.
   - Java (`shutdown()`) and grpc-js (close lets calls finish) answer B, and b runs.
   - So the same input gives an outcome that the conformance scenario can observe and that differs by language.
3. **The second redial:** B's cut is itself "an attempt on that connection that ended with no answer while the main connection answered". Read literally, it redials the new connection too.
4. **The counting:** an attempt cut by the host counts as "no answer" toward the three counted calls. The probe on the new connection usually saves the count, but the rule does not say that it must.

**Why the tests miss it:** the SDK unit case "a half-open acknowledgement connection dialled again with no fallback" has one attempt in flight.

**Fix:**
- Drain the old connection instead of cutting it: new attempts go to the new connection, and attempts in flight finish on the old one. Name the per-SDK means (Java `shutdown()` not `shutdownNow()`, a Go `ClientConn` closed once its calls have ended, and so on).
- Say that an attempt the host ended itself is not an attempt: it decides nothing, counts toward nothing and sets off no redial.
- Add an SDK unit case with two attempts in flight at the redial.

**Minor 2. "A receive" that ended with `DEADLINE_EXCEEDED` now blocks the early give-up. On a host whose configured call timeout is under 5 s, every stream window ends that way, so the early give-up never fires.**

Confidence: confirmed in the code and the documents.

**Where:** `sdks/SPEC.md:391-393` and `stream-completion.md:442-445` (the list now includes "a receive"); `sdks/SPEC.md:841-843` ("clients additionally bound receiving to six seconds or their shorter configured call timeout").

**Code:**
- The server's stream window is 5 s (`crates/api-grpc/src/service.rs:154`).
- The TypeScript SDK sets the stream deadline to `min(timeoutMs, 6000)` (`sdks/typescript/src/worker/grpc.ts:269`).
- With `timeoutMs = 3000`, every window ends with `DEADLINE_EXCEEDED` on the host before the server ends it.

**Scenario:** a host with a 3 s call timeout and a flagged lease whose acknowledgement fails alone. Heartbeats are answered (evidence). The deciding attempt fails. But some receive window ended with `DEADLINE_EXCEEDED` between the evidence and that attempt's end, which on that host is always true. So the lease waits for the ordinary give-up. This is the "acknowledgement failing alone" row, where the design promises the early give-up.

`PollTasks` does not have this problem, because the server answers it at once (no long poll, `service.rs:576-644`).

**Why the tests miss it:** every conformance and unit case uses the default timeout, and the stream bound is 6 s against a 5 s window.

**Fix (either):**
- Exclude a receive ended by the host's own bound when that bound is shorter than the server's window.
- Or, at 1.8, clamp the stream's receive bound to at least the window plus 1 s, whatever the configured timeout.

Add an SDK unit case with a 3 s call timeout.

**Minor 3. The design does not name the set of failures that are "unreachable store". Two cases fall on the wrong side: a server that answers read-only after a failover, and a deadline spent waiting in the concurrency layer.**

Confidence: plausible.

**Where:** `stream-completion.md:605-608`, `sdks/SPEC.md:389-391`, Task 006-049.

**Case (a), a read-only server after a failover:**
1. The demoted old primary restarts as a standby, and engine E2 dials it again, through a stale DNS answer or a pooler still pointing at it.
2. The ping succeeds, `matched` (a plain `SELECT`) succeeds, and the `UPDATE` fails with SQLSTATE `25006` (`read_only_sql_transaction`).
3. That is "held a connection" and "the connection did not fail". So the answer counts, the deciding attempt gives the lease up, and after the promotion the hand-back is an ordinary failure that spends the attempt.
4. This is the round-12 outcome through another door. The registry's classifier (`instances.rs:75-85`) does not list `25006` either.

**Case (b), the wait in the concurrency layer:**
1. "Before it held a connection" includes time spent queued in the platform's per-connection concurrency layer, behind a proxy that pools connections upstream.
2. There, the acknowledgement is failing alone, which is the case the early give-up exists for. The other classes have their own 60 places.
3. Classing it as unreachable suppresses the early give-up and counts it as `store`, which misleads the operator.

**Fix:**
- Name the classifier: the registry's set at `instances.rs:75-85` plus `25006`.
- Measure "before it held a connection" from the pool acquisition only. A call that reaches the handler with too little time left answers `ACKNOWLEDGEMENT_DEADLINE`.
- Add a server test for `25006` against a standby, or record the case in Gap 006-047.

**Minor 4. The SDK unit case "twelve single-lease frames with every attempt failing at 6 s each given up early within about two attempts' time" cannot pass under the permit and merge rules of this round.**

Confidence: confirmed by the arithmetic of the documents' rules.

**Where:** Task 006-059 (`tasks.md`, the unit test list).

**Scenario:** four permits; attempts fail at the host's 6 s bound; heartbeats answered every 100 ms.

| Time | What happens |
|---|---|
| 0 s | Calls C1 to C4 take the four permits. Frames 5 to 12 wait and are merged into one call M of 8 leases. |
| 6 s | C1 to C4 end. M takes a permit. C1 to C4 each have evidence and 6 s passed, so each is due an early attempt within 200 ms, but only three permits are free. Three go out by about 6.3 s; the fourth waits. |
| About 12.3 s | Three early attempts fail, and three leases are given up (two attempts after receipt). |
| 12 s | M fails. The fourth retry gets M's permit, is sent at 12 s and fails at 18 s: given up after three attempts. |
| About 18.2 s | M's early attempt, sent at about 12.2 s, fails. M's eight leases are given up 18.2 s after receipt. |

**Result:**
- Nine of the twelve leases take three attempts' time from receipt, whichever clock the test uses.
- Measured from each lease's own first attempt, C4 still takes three.
- A correct build fails the case, or the case gets loosened while it is being built.

**Fix:** state the expectation from the permit arithmetic. For example: "all twelve given up early, none by the ordinary give-up, the last within three attempts' time plus 1 s of the first frame".

## Nit

1. **Stale wording from round 11:**
   - Decision 006-065 still says "the first answer after a promotion leads to one more attempt, which is answered, and never to a hand-back" (`research.md:935-936`).
   - The Fallback row still says "that lease's next attempt sent at once" (`stream-completion.md:971`).
   - Under this round's rule the attempt goes out early only after 6 s with no shed, after up to 200 ms, and may be answered `ACKNOWLEDGEMENT_STORE_UNREACHABLE`.
   - The old clause "or waits for one in flight and then sends it" was removed. The text no longer says whether evidence that arrives while an attempt is in flight sends the next attempt early once that one ends, or leaves it to the backoff. Pick one, so that six SDKs time it alike.
2. **"Once any withdrawal exists" contradicts the narrowed guard.** It still appears at `ADR-0099…md:302` and `research.md:753` and `:857`. `lease-withdrawal-v2` is now removable until the first `NOT_ACKNOWLEDGED` withdrawal, so a build below the follow-up can be admitted while withdrawals of Architecture decision 0098 exist. That is safe, but these three places say otherwise. The released-v0.2 row already has it right ("while a `NOT_ACKNOWLEDGED` withdrawal exists").
3. **Decision 006-059 (7)** still says the beat "returns the committed epoch by a plain read, as the base's beat does, and reports whether an owner transaction holds the epoch row" (`research.md:819-821`). Eight lines later the same decision says the locked read comes first (`:828-831`).
4. **The third pinned phase** expects a binary of the #260 merge "registered before the recording, closes its gate at its next beat". But `platform_requirement_activate` refuses `lease-withdrawal-v2` while a live instance lacks it (`instance_registry.up.sql:333-340`). The process must be held silent past the 30 s window at the recording; say so in the case.
5. **The two-engine conformance case** does not name how the harness puts the acknowledgement connection on the engine held unreachable while the main connection lands on the other. The host has one address. Name the mechanism, for example a harness proxy that routes the host's second connection to the second engine.
6. **"Held a connection" does not fit `begin_safely`.** Every transaction in the code begins through it, which acquires and sends `BEGIN` in one spawned task (`pool.rs:99-132`). Splitting acquisition from `BEGIN` to know which one the deadline cut is what that comment warns is not cancel-safe. Define the reason as "the deadline passed before the transaction began".
7. **The stale-pool tail.** Each stale idle connection costs an acquisition its full bound, because a ping to a vanished peer waits for TCP retransmission. On a quiet engine with a pool of 10, recovery can outlast the ordinary give-up at a 30 s time-to-live, and the hand-back then spends the attempt. Record this in Gap 006-047 next to "a failover that outlasts a lease's give-up".
8. **The trigger, three details:**
   - Task 006-044's `down` "drops the function" but does not mention the trigger and its function. A later `up` would fail on `CREATE TRIGGER`.
   - "Under the epoch lock the insert already holds" is true only through `platform_requirement_activate`. A raw insert takes the lock only at its later `UPDATE` of the epoch. Having the trigger read the `lease-withdrawal-v2` row `FOR SHARE` makes the claim hold for every path. Today the outcome is only the safe "nothing is marked".
   - `stream-completion.md:425` still says "a trigger of its own was considered and rejected" a few lines after describing the trigger. Add "for the order".

## Evidence to ask for once built

- `SAGAT_SDK_SCENARIO=delivery-acknowledgement cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture`:
  - the two-engine case, with the routing named (Nit 5);
  - a redial with two attempts in flight, with the same outcome in six languages (Minor 1).
- The SDK unit tests:
  - a 3 s call timeout on a stream host with the acknowledgement failing alone, given up early (Minor 2);
  - the twelve-frame case against the arithmetic above (Minor 4).
- `cargo test -p sagat-tests --test integration_durable_execution`: the acknowledgement against a standby, answering `ACKNOWLEDGEMENT_STORE_UNREACHABLE` or recorded as a gap (Minor 3).
- `cargo test -p sagat-tests --test integration_platform_foundation`: the split migration of Task 006-044, with part one applied before the follow-up and part two after it (Major 1).

## Files consulted

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/round-12/correctness.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20261009000100_execution.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20261027000000_scoped_instance_identity.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/instances.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/pool.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/execution.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/event_codec.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/service.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/typescript/src/worker/grpc.ts
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/typescript/src/worker/host.ts
- /private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/delivery-ack-round12-to-13.diff
