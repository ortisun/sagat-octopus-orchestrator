**Review status: complete.** No target standard was supplied, so I reviewed against the persona's audit vectors, the questions in your request and my round-12 report.

**What I read:**
- the persona and my round-12 report;
- the round-12-to-13 diff in full;
- `sdks/SPEC.md` §5 (lines 266 to 299), §6.1 and §6.2 (lines 303 to 470), and §10;
- `stream-completion.md`:
  - "The server's acknowledgement", step 1 (lines 584 to 612);
  - "Why the deadline is the lease's own";
  - "The host at 1.8" (lines 738 to 820);
  - the failure table rows at lines 868 to 870;
  - the Observability section and the evidence table;
- `research.md`, Decision 006-059 (lines 800 to 862) and the passages the diff touched;
- `tasks.md`, precondition (a) and Task 006-043 to Task 006-052.

**What I checked in the code of the merged base:**
- `crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql`:
  - the guards, lines 98 to 168;
  - `platform_requirement_remove`, lines 357 to 378, which checks no dependency;
- `crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql`:
  - the census, lines 40 to 44, which counts live processes only (`last_seen` within 30 s);
  - the replay-floor guard, lines 52 to 65;
- `crates/infra-postgres/migrations/20261009000100_execution.up.sql`, lines 88 to 106: `run_events` has only its primary key `(tenant_id, run_id, seq)`;
- `crates/api-grpc/src/limits.rs`, lines 322 to 404: `CappedConnectInfo` holds only a watchdog, not a peer.

## Verdict

**Count: no Blockers, no Majors, 4 Minors, 6 Nits.**

- All six round-12 Minors are resolved as written.
- Two of the fixes created new problems:
  - Narrowing the removal guard of `lease-withdrawal-v2` (my round-12 Nit 3) reopens a way around the new trigger (Minor 1).
  - Moving Task 006-044 to the waiting list (my round-12 Minor 5) strands three tasks that need its columns (Minor 2).
- The new rules for the acknowledgement connection (merging frames, redialling) are underspecified in ways that six SDKs would build differently (Minors 3 and 4).

None of these makes the design wrong. Every path still ends in an acknowledgement, a withdrawal, an ordinary failure or a review, as stated.

---

## Part 1: The round-12 findings

### Minor 1 (the blocking condition referred to itself and could not be observed): resolved

- The condition now covers "another of its calls", lists a receive, excludes `AcknowledgeTasks` by name, and is stated as a host-visible status list, the same in `sdks/SPEC.md` §6.2 lines 391 to 394, `stream-completion.md` lines 780 to 783 and Decision 006-065.
- "A refusal the server answered (stale, not held) is not such an end" is kept.
- The case of two flagged calls failing together is in Task 006-059.
- The server side of the old observability problem is solved separately by `ACKNOWLEDGEMENT_STORE_UNREACHABLE`. The acknowledgement itself now says when the engine never reached the database.
- One wording gap remains: which evidence "since the evidence" refers to when several arrive (Nit 2).

### Minor 2 (dense evidence made a retry loop, and a promotion a herd): resolved

- Attempts keep the backoff of §5 and any retry hint. An early attempt is sent only once a receive bound has passed with no shed, after up to 200 ms of jitter, and at most once per lease per receive bound.
- The unit test (attempts failing in 5 ms, heartbeats every 100 ms, no more than §5's count plus one) is in Task 006-059.
- 200 ms is a narrower jitter than I proposed. It is enough here, because:
  - each host sends at most 4 calls of at most 64 leases;
  - the hosts' first answers after a promotion are already spread by their heartbeat intervals;
  - a herd beyond that is shed, and a shed decides nothing.
- Two passages still say "at once" (Nit 1).

### Minor 3 (the permit's scope, and frames waiting behind it): resolved in rule

- The permit is taken per attempt.
- Waiting frames are merged into the next call, up to 64 leases, and the twelve-frame unit test exists.
- But how a merged call's leases share the window is ambiguous in a way that changes behaviour (new Minor 3).

### Minor 4 (a channel of its own is not a connection of its own): resolved

- `sdks/SPEC.md` line 366 names a means for each of the six stacks, and I checked each one:
  - `grpc.use_local_subchannel_pool` exists in both grpc-js and grpcio;
  - a Go `ClientConn`, a Java `ManagedChannel`, a .NET `GrpcChannel` on its own handler and a Rust `Endpoint` connected separately each own their transports.
- There is a per-SDK test ("the two channels on different transports") and a server-side assertion in Task 006-058.
- Small leftover: the assertion relies on a peer that `CappedConnectInfo` does not hold today (Nit 3).
- The new redial rule has defects of its own (new Minor 4).

### Minor 5 (precondition (a) contradicted its own rule): resolved as asked, but the fix strands three tasks

- Task 006-044 now waits.
- Its migration sorts after the follow-up's and carries `lease-withdrawal-v2` in the closed lists.
- Task 006-049 and Task 006-052 use SQL fixtures until Task 006-047 and Task 006-050 land.
- But Task 006-044 also creates the columns those tasks write (new Minor 2).

### Minor 6 (the census missed deleted rows, and the operator was invited to delete them): resolved

- The Consequences, the runbook rule (in Task 006-046's evidence) and Gap 006-044 now say that a silent row is removed only once its process is confirmed stopped.
- The residue is named: a row deleted after a day, one pushed out of a full table, or one never written.
- The claim is softened to "no registered binary".

### The round-12 Nits

- **Nit 1:** resolved ("from the build of Task 006-057").
- **Nit 2:** resolved. "Answered by a working primary" is used, and the stream-frame residue is in Gap 006-047 and in host item 4.
- **Nit 3:** resolved. The forward order is in `stream-completion.md` and in Decision 006-059 (9), and the guard is keyed to `NOT_ACKNOWLEDGED`. But the narrowing opens new Minor 1, and three sentences still describe the old guard.
- **Nit 4:** resolved. The two tests to invert are named.
- **Nit 5:** resolved. I checked line 289 and the alert list (lines 944 to 950).
- **Nit 6 (line wraps):** not done, by choice. I do not raise it again.

---

## Part 2: New findings

### Minor 1: With the narrowed guard, `lease-withdrawal-v2` can be removed while `delivery-acknowledgement-v1` is recorded, and recording it again skips the "live or not" census

**Location:**
- Decision 006-059 (9), `research.md` lines 849 to 851;
- `stream-completion.md` lines 411 to 420 (the new trigger) and the failure row "`delivery-acknowledgement-v1` recorded by hand without `lease-withdrawal-v2` ...";
- Architecture decision 0099, decision 10 and the Rollback, line 302;
- `research.md` lines 753 and 857.

**What is wrong:**
1. **The new trigger guards one direction only.**
   - It is a `BEFORE INSERT` trigger on the row of `delivery-acknowledgement-v1`.
   - Removing `lease-withdrawal-v2` while `delivery-acknowledgement-v1` stays recorded is refused only once a journal holds `NOT_ACKNOWLEDGED`. That condition is new in this round; before, any `step.lease_withdrawn` blocked the removal.
   - `platform_requirement_remove` (registry migration, lines 357 to 378) checks no dependency.
2. **The resulting sequence reopens what round-11 Major 1 closed.** It is reachable before the first withdrawal of protocol 1.8, for example shortly after an activation or on a quiet fleet, by an operator who runs the rollback steps in the wrong order:
   1. `lease-withdrawal-v2` is removed.
   2. A build of pull request #260 without its follow-up registers. It is no longer refused, and its gate is open because it lacks nothing that is recorded.
   3. Its registry connection fails while its request pool works. That build has no liveness rule, so its gate stays open.
   4. `lease-withdrawal-v2` is recorded again. The generic command's census counts live processes only (`20270208000000_lease_withdrawal.up.sql` line 44), so the silent process is not seen. The trigger does not fire, because no `delivery-acknowledgement-v1` row is inserted.
   5. Engines of this build see both requirements and mark again.
   6. A binary without the liveness rule now holds an open gate when a provisional lease is claimed. This is exactly what the second refusal of decision 10 exists to prevent.
3. **The design's own Gate case reaches this state.** "`lease-withdrawal-v2` ... recorded by hand for engines only" while `delivery-acknowledgement-v1` is recorded can now only be reached by this removal path (or with the trigger disabled). So the failure row "Refused by the trigger" is incomplete.
4. **Three sentences now overstate the guard.** They still say the registry "never admits a binary below that build once any withdrawal exists": Architecture decision 0099 line 302, and `research.md` lines 753 and 857. With the narrowed guard, that holds only once a `NOT_ACKNOWLEDGED` withdrawal exists.

**Why it is Minor:** it needs a wrong order of operator steps, and it stays within the trust assumption of decision 12: a host that keeps the protocol calls no capability before `ACKNOWLEDGED`. That is the same rating as round-12 Minor 6.

**Fix:**
- Extend the new trigger to `BEFORE DELETE`: refuse removing `lease-withdrawal-v2` while `delivery-acknowledgement-v1` is recorded. The rollback already removes `delivery-acknowledgement-v1` first and keeps `lease-withdrawal-v2`, so no procedure changes.
- Add that refusal to Task 006-044's evidence.
- Reach the Gate's "engines only" case by a fixture with the trigger disabled.
- Correct the three sentences to "once any `NOT_ACKNOWLEDGED` withdrawal exists".

### Minor 2: Task 006-044 now waits, but Task 006-048, Task 006-049 and Task 006-052 "proceed" while needing its columns

**Location:** `tasks.md`, precondition (a), lines 144 to 150; Task 006-044, lines 179 to 194; Task 006-048, line 240; Task 006-049; Task 006-052.

**What is wrong:**
- Task 006-044 creates every new column:
  - on `steps`: `lease_hand_back_generation`, `lease_offered_generation`, `lease_capability_generation`, `lease_claimed_at`, `lease_claim_incarnation`, `lease_acknowledged_at`, `provisional_withdrawn_attempt` and `withdrawn_lease_token_hash`;
  - on `worker_sessions`: `declines_acknowledgement`.
- The tasks listed as proceeding write or read those columns:
  - **Task 006-048** stores `declines_acknowledgement` on the session.
  - **Task 006-049** writes `lease_acknowledged_at`, and its "SQL fixtures" write marks into the columns of Task 006-044.
  - **Task 006-052** writes `lease_capability_generation`.
- Only the parts of Task 006-044 that read `lease-withdrawal-v2` need to wait: the requirements function, the trigger and the closed checks.

**Why it matters:** the server owner again starts tasks the plan says can proceed, and finds the schema missing.

**Fix:** split Task 006-044 into two tasks:
- **(a) proceeds:** the columns, the session flag, the grants, the start-time check, and a `down` for those parts;
- **(b) waits:** `delivery_acknowledgement_required()`, the trigger and the closed checks with `lease-withdrawal-v2`, sorted after the follow-up's migration.

Or list Task 006-048, Task 006-049 and Task 006-052 as waiting.

### Minor 3: A merged call's leases "share" a window, which either gives a fresh lease up after its first try or keeps restarting an old lease's window

**Location:**
- `stream-completion.md` line 786 ("the halves of a split call, like a merged call, share it");
- `sdks/SPEC.md` lines 367 and 368 (no mention of the window);
- Decision 006-065 (says the split halves share it, and nothing about merged calls);
- Task 006-059's twelve-frame case.

**What is wrong:**
- The early give-up is defined per lease: "after an attempt of the lease ended with no verdict", then the evidence, then a deciding attempt, with "one receive bound since its first attempt".
- A merged call mixes leases with different histories. "Merged into the next call sent" does not say whether that call is a fresh call or the retry of a call already in its backoff, which also needs a permit.
- If frames merge into a retry and share its window, a lease that has never been sent can be given up at the end of its first attempt. That is what the twelve-frame case's "within about two attempts' time" seems to assume.
- If instead the window starts at the merge, a host that keeps receiving frames keeps restarting its old leases' windows, and pushes them toward the ordinary give-up.
- The documents also disagree with each other: the Specification says nothing, and research names the split halves only.

**Why it matters:**
- This is host behaviour in six languages, decided by a sentence that admits two readings.
- The conformance golden and the timings of the twelve-frame test depend on the reading.

**Fix:**
- State that the window, the evidence and the give-up stay per lease, and that a merged call is "one call" only for the ordering and counting of the fallback.
- Say whether waiting frames join a retry or form a new call, and whether retries take a free permit before new frames.
- Restate the twelve-frame expectation under that rule.
- Align `sdks/SPEC.md` §6.2 and Decision 006-065.

### Minor 4: The redial rule cuts its own attempts, leaves "no answer" undefined, and never leaves an engine that answers `ACKNOWLEDGEMENT_STORE_UNREACHABLE`

**Location:**
- `sdks/SPEC.md` lines 368 to 370;
- `stream-completion.md` lines 602 and 603;
- the new failure row "The engine that answers the acknowledgement has not reached the database ...";
- Task 006-059.

**What is wrong:**
1. **Closing the connection fails up to three other attempts in flight on it.**
   - Those attempts then end with no answer, which the host itself caused. They then:
     - count toward the fallback's "every attempt ended with no answer";
     - can trigger a further redial.
   - The stacks also close differently:
     - grpc-go `ClientConn.Close` cancels the calls in flight;
     - grpc-java `shutdown` lets them finish and `shutdownNow` cancels them;
     - grpc-js `close` lets them finish.
   - So the six SDKs will diverge on the counting.
2. **"No answer" is not defined.**
   - It is unclear whether a mesh's 503 without the envelope is an answer.
   - In the conformance case "two minutes of a 503", one reading redials and makes a TLS handshake on every attempt; the other never redials.
3. **Separate connections let the acknowledgement reach a different engine than the stream and the heartbeats.**
   - An engine that keeps answering `ACKNOWLEDGEMENT_STORE_UNREACHABLE` is answering, so the host never redials. Examples: a pool broken until a restart, or an address still resolving to the old primary after a failover.
   - Kubernetes does not cut an established HTTP/2 connection when a pod leaves readiness.
   - Every flagged lease of that host then waits for the ordinary give-up (about 24 s at a 30 s time-to-live), with no end and no fallback, because the answer carries the envelope.
   - The new failure row's "until an engine that reached the database answers" can then only mean the same engine recovering.

**Fix:**
- On redial, open the new connection for new attempts and let the old one's attempts end within their own 6 s bound before closing it. An attempt ended by the host's own close is not an attempt and counts toward nothing.
- Define "no answer" as a transport failure, or the host's own deadline with no status received.
- Also redial after two consecutive `ACKNOWLEDGEMENT_STORE_UNREACHABLE` answers while a call on the main connection that reaches the database was answered `OK` after them.
- Add unit cases for both, and a conformance case where one engine stays unreachable for longer than a time-to-live.

### Nits

1. **Two passages still say "at once".**
   - The failure row "A database failover while flagged leases wait" (`stream-completion.md` line 870) says "sends every lease waiting in its backoff one more attempt at once".
   - The Fallback evidence row (line 971) says "that lease's next attempt sent at once".
   - Both contradict the new rule (the backoff kept, an early attempt only after one receive bound and after up to 200 ms of jitter). Write "sent early, within 200 ms" in both.
2. **"Since the evidence" does not say which evidence.**
   - When a failure falls between two pieces of evidence, it is unclear whether the later evidence resets the condition.
   - Resetting is safe, because the deciding attempt must be sent after the evidence it relies on.
   - Write: "since the latest evidence before the deciding attempt was sent, until that attempt's end".
3. **The server-side peer assertion needs a field that does not exist.**
   - `CappedConnectInfo` (`limits.rs` lines 327 to 329) holds only the watchdog.
   - Task 006-058's "the peer held in `CappedConnectInfo`" needs a connection identity added, either the remote address or a per-connection counter.
   - Name that addition in Task 006-049, which already changes `CappedConnectInfo` for the semaphore.
4. **"One indexed read of the run's journal" is not bounded.**
   - The stale path's lookup of the latest `step.lease_withdrawn` for a generation can only use the primary key `(tenant_id, run_id, seq)` for a root step.
   - It walks back past every later event of the run, which can be many thousands in a collection run, once for each stale call with a withdrawn token.
   - Add a partial index on `run_events (tenant_id, run_id, node, seq) WHERE event_type = 'step.lease_withdrawn'` (withdrawals are rare, so it is small), or bound the scan.
5. **Task 006-044's `down` "drops the function" but not the new trigger and its function.** Its restored closed checks must also keep the follow-up's `lease-withdrawal-v2`, or a `down` drops them. Name both.
6. **The trigger's "under the epoch lock the insert already holds" is true only for `platform_requirement_activate`.** That function locks the epoch at line 323 before its insert at line 343. Hand SQL may insert before it moves the epoch. Nothing depends on the ordering, because the registration refusal already covers the race, so either drop the clause or have the trigger take the epoch row itself.

---

## Part 3: Is it complete enough to accept?

**Yes, for the owner's decision, with Minors 1 and 2 corrected in the same pass and Minors 3 and 4 before the SDK tasks start.**

- **The core is unchanged and holds:**
  - the row lock and the lease's own deadline;
  - the cap and the attempt flag;
  - the probe;
  - the floor owned by the registry;
  - the beat, whose new order (the locked read first) closes the window round 12 found;
  - the census, now enforced in the database for the direction it covers.
- **Minor 1 and Minor 2 are a trigger clause and a task split.** Minor 1 restores the guarantee round 11 bought. Minor 2 keeps the server plan buildable in its stated order.
- **Minors 3 and 4 are a few normative sentences in `sdks/SPEC.md` §6.2 and their unit cases.** They must land before Task 006-059 to Task 006-064, or the six hosts will differ.
- **The Nits can go into the tasks.**
- **The risks the owner accepts with it:** Gap 006-041, Gap 006-042, Gap 006-044 (now including deleted rows), Gap 006-047 (now including the stream-frame case) and Gap 006-048.
- **No further full round is needed.** A targeted check of these four answers is enough.

**Files referenced:**
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0100-withdrawn-a-fleet-requirement-may-require-another.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/round-12/egm-design.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20261009000100_execution.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/limits.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/apps/control-plane/src/capability_activation.rs
