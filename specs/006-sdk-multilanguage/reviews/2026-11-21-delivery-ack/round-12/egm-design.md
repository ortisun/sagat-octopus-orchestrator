# Design critique, round 12: delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)

**Review status: complete.** No target standard was supplied. I reviewed against the persona's audit vectors, the questions in the request and my round-11 report.

**What I read:**
- the persona and my round-11 report;
- the round-11-to-12 diff in full;
- `sdks/SPEC.md` §5 (lines 266 to 295) and §6.2 (lines 355 to 425);
- `stream-completion.md`, "Negotiation and activation" (lines 392 to 421);
- `tasks.md`: precondition (a) and Task 006-043 to Task 006-049, Task 006-052, Task 006-056 and Task 006-057.

**What I checked in the code:**
- `crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql`, lines 180 to 309: registration refuses a missing requirement; the beat deletes rows silent for a day.
- `apps/control-plane/src/registry.rs`, lines 134 to 142: a plain connection failure never closes the gate.
- `apps/control-plane/src/capability_activation.rs` line 359 and `tests/integration/schedules-bulk-operations/lease_withdrawal_activation.rs` line 200: the merged head asserts that `lease-withdrawal-v2` is not activatable.

## Verdict

**Count: no Blockers, no Majors, 6 Minors, 6 Nits.**

- Round-11 Major 1 is **resolved**. The floor now belongs to the registry, whatever order the releases come in.
- The other round-11 answers are resolved, or resolved with small leftovers.
- The new defects sit in two places:
  - the early give-up's new "attempt after the evidence" rule;
  - the new acknowledgement channel and its permit of 4.

  None makes the design wrong as a whole, but two of them (Minor 1 and Minor 3) can quietly stop the early give-up from working on an ordinary busy host.

---

## Part 1: The round-11 findings

### Major 1 (the floor admitted a build listing `lease-withdrawal-v1` without decoding `NOT_ACKNOWLEDGED`): resolved

**Why it holds:**
- Request (9) adds `lease-withdrawal-v2` on both kinds. It means "replays the causes of 1.8 and applies (7)".
- The registry refuses to register a process that lacks a recorded requirement. I checked this: `instance_register` returns `CAPABILITY_MISSING`, lines 199 to 204.
- Marking requires `lease-withdrawal-v2` recorded for both kinds (decision 2, `stream-completion.md` line 436).
- Every withdrawal of a marked lease reads `lease-withdrawal-v2` under the lock its removal waits for, and its removal is refused while a journal holds `step.lease_withdrawn`.
- Together these give the chain the Rollback needs: a `NOT_ACKNOWLEDGED` event exists, so `lease-withdrawal-v2` is recorded and cannot be removed, so any binary without it is refused on both kinds.
  - The race between the first withdrawal and a removal is ordered by the same `FOR SHARE` / `FOR UPDATE` pair as in Architecture decision 0098.
  - Keeping `NOT_SENT` on `lease-withdrawal-v1` is correct, because that cause is decodable by the older build.
- The Rollback names `lease-withdrawal-v2` as the enforced floor, and the released-v0.2 scenario now includes a build of pull request #260 without its follow-up.

**Two edges remain.** They are below Major:
- The "live or lapsed" census misses a process whose row was deleted after a day, and the Consequences invite the operator to delete such rows (Minor 6).
- The forward step that activates `lease-withdrawal-v2` is named nowhere (Nit 3).

### The round-11 Minors

**Minor 1 (the early give-up):** resolved in substance.
- The decision is now taken again by an attempt sent after the evidence, so a failover that begins before that attempt is caught.
- The residual is stated in the failure row, Decision 006-065, the Consequences and Gap 006-047.
- The quiet-host case is reworded and pinned (slots full, or no work queued).
- But the new definition of a failed try refers to itself, and the "at once" rule creates a retry loop (Minors 1 and 2 below).

**Minor 2 (the 18 s case):** resolved. The case expects "an ordinary failure or a review" and reports which, and Gap 006-047 explains both outcomes.

**Minor 3 (the transport-level exclusion):** resolved in form.
- The rule is now one every SDK can observe: the SDK's own permit decides what was sent.
- The SDK unit test and the conformance case exist.
- Two problems remain:
  - "a channel of its own" does not guarantee a connection of its own in two of the six stacks (Minor 4);
  - the permit's queueing interacts badly with the give-up (Minor 3).

**Minor 4 (the reserve):** resolved.
- The reserve is keyed to the compiled `DELIVERY_ACKNOWLEDGEMENT_PROTOCOL_MINOR`.
- Task 006-057 names it.
- Task 006-049 and the gate (Task 006-056, "the minor raised in the test") run with the minor raised.
- There is a dormancy test with 64 places.
- The 60-place cost is in the Consequences, but dated "from the activation" (Nit 1).

### The round-11 Nits

- **Resolved:** 1 (the "undecided" clause), 2 (the tasks that wait are now listed), 3 (call ordering in the specification), 4 (one `mark` listing), 5 (Consequences "at least one receive bound"), 6 (the cap row after a rebuild), 7 (a function of its own name, which also settles the overload's meaning) and 8 (`instance_heartbeats_change_in_progress_total{kind}`).
- **Still open:** 9, the line wraps (Nit 6 below).

---

## Part 2: New findings

### Minor 1: "No try of a call to the database failed since the evidence" includes the acknowledgement's own attempts and cannot be observed by a host

**Location:**
- `sdks/SPEC.md` §6.2, lines 386 to 388;
- `stream-completion.md`, host item 4 ("no try of a call of the host to the database failed since the evidence (a failed try is one that ended without the database's answer ...)");
- `research.md`, Decision 006-065.

**What is wrong:**
1. **The rule does not exclude acknowledgements.**
   - The evidence is defined as "another of the host's calls" (a heartbeat, a report, a hand-back). The disqualifying condition says only "a try of a call to the database".
   - Read literally, the deciding attempt is itself a failed try of a call to the database after the evidence. So the condition can never hold.
   - Even if a reader excludes the lease's own attempt, the attempts of the host's other flagged calls still qualify. When the acknowledgement fails alone and two or more calls are in flight, each call's evidence is followed by another call's failed acknowledgement, so no lease ever gives up early.
   - That is the exact case the early give-up exists for: the failure row "The acknowledgement failing alone", and the gate's 5-minute statement-timeout injection.
   - Six SDKs will split between the strict and the lenient reading.
2. **"Without the database's answer" is not visible to a host.**
   - A host sees a gRPC status and the envelope reason. It cannot tell whether an `UNAVAILABLE` came from a pool timeout (no database answer) or from a statement timeout (which the database answered).
   - So the classification of a heartbeat's failure differs from SDK to SDK.

**Why it matters:** the early give-up becomes unreachable or nondeterministic across languages in its main use case, and the conformance row "two minutes of answers with the server's envelope" passes or fails depending on how many calls are in flight.

**Fix:**
- Write the condition as: "no try of another call of the host that reaches the database (a heartbeat, a report, a hand-back; never an `AcknowledgeTasks`) ended with a transport failure, `DEADLINE_EXCEEDED`, `UNAVAILABLE`, `RESOURCE_EXHAUSTED` or a retryable `INTERNAL` since the evidence". That is the host-visible list of §5.
- Keep "a refusal the server answered (stale, not held) is not one".
- Add an SDK unit case with two flagged calls failing together while heartbeats are answered: both are given up early.

### Minor 2: "At once, outside the backoff" makes dense evidence a retry loop for the first receive bound, and a herd at a promotion

**Location:**
- `sdks/SPEC.md` §6.2, lines 383 to 389;
- `stream-completion.md`, host item 4 ("On the evidence the host sends the lease's next attempt at once, outside the backoff");
- Decision 006-065.

**What is wrong:**
- Every failed attempt followed by any answered heartbeat, report or frame triggers an immediate attempt.
- Before one receive bound has passed since the first attempt, that attempt cannot give the lease up, so the host "goes on with the backoff". But the next answered heartbeat, sent after that attempt ended, is fresh evidence and triggers another immediate attempt.
- On a host with many running tasks, heartbeat answers arrive every few hundred milliseconds. Against a fast failure (a defect answering `INTERNAL` in milliseconds, a lock timeout of 1 s), each call is then retried at the heartbeat rate rather than at §5's exponential backoff.
- At a promotion, "the first answer after it sends every waiting lease one more attempt" at once. Every host in the fleet does this within the same second, against a primary that has just started.

**Why it matters:**
- The extra load falls on a server that is already in trouble, which the Consequences already warn about for the second claim and withdrawal.
- The gate does not measure it: Task 006-056 injects a statement timeout, but no fast-failing defect.

**Fix:**
- Before one receive bound has passed since the first attempt, record the evidence and keep the backoff.
- Send at once only the one deciding attempt, once the bound has passed and the evidence still stands.
- After a promotion, send the post-evidence attempt after a random delay of up to the backoff's first step.
- Add an SDK unit test: with heartbeats answered every 100 ms and attempts failing in 5 ms, the attempts in the first 6 s are no more than §5's count plus one.

### Minor 3: The permit of 4 does not say whether a call holds it across its backoff, and frames waiting for it never get an attempt, so they cannot give up early

**Location:**
- `sdks/SPEC.md` §6.2, lines 365 to 367;
- `stream-completion.md`, step 1 of the acknowledgement (line 585 onward) and host item 2;
- the failure row "The acknowledgement failing alone" ("about 6 s of delay");
- the SDK host task (Task 006-059: "a fifth acknowledgement waiting for the permit and not counted").

**What is wrong:**
1. **The permit's scope is undefined.** "At most 4 calls in flight" does not say whether the permit is taken per attempt (released during the backoff and between tries) or per call (held until a verdict or the give-up).
   - If it is per call, four failing calls hold the channel for up to the ordinary give-up (24 s at a 30 s time-to-live), and every other flagged frame waits.
2. **A lease waiting at the permit can only reach the ordinary give-up.** Even per attempt, the early give-up requires that an attempt of the lease has ended, so such a lease waits for the ordinary give-up.
   - Example: a host with `max_inflight` 100 that receives small frames (one or two leases each), with the acknowledgement failing at its deadline.
   - Each call needs at least two attempts (the failed one and the deciding one), so about 12 s per call at the 6 s bound.
   - With four permits, that is one call every 3 s. The tenth waiting frame is past the ordinary give-up before its first attempt.
   - So "about 6 s of delay" holds only for the first four calls.
3. **Nothing tests it.** The gate, which runs before the SDKs exist, never exercises the permit, so the throughput limit it sets is not measured.

**Fix:**
- State that the permit is taken per attempt.
- State that frames waiting for it merge into the next call sent, up to 64 leases. Merged calls are "one call" for the ordering and counting rules, as split halves are.
- Add an SDK unit case: twelve single-lease frames with every attempt failing at 6 s, each given up early within about two attempts' time.
- Alternatively, keep per-frame calls and state the bound on frames behind the permit in the failure row and in Gap 006-047.

### Minor 4: "A channel of its own" is not a connection of its own in the gRPC C-core and grpc-js stacks

**Location:**
- `sdks/SPEC.md` §6.2, line 365;
- `stream-completion.md`, step 1 ("so the host's other traffic can neither delay its acknowledgements on a shared connection nor make it decline") and host item 2;
- the Fallback evidence row ("the acknowledgements going on their own channel").

**What is wrong:**
- gRPC C-core (which the Python SDK uses) and grpc-js (TypeScript) keep a global subchannel pool by default. Two channels to the same target with the same arguments share one HTTP/2 connection.
- On a shared connection, an acknowledgement can again wait for stream credit beyond `max_concurrent_streams` (128) behind the host's transfers. That is the wait round 11 showed the host cannot observe.
- So the stated property ("neither delay ... nor make it decline") holds in Go, Java, C# with Grpc.Net.Client and Rust, but not in Python or TypeScript without an extra channel argument.

**Fix:**
- Say "a connection of its own".
- Name the setting each stack needs (`grpc.use_local_subchannel_pool` for C-core and grpc-js, or a distinguishing channel argument).
- Add a per-SDK unit test that the acknowledgement channel and the main channel do not share a transport.

### Minor 5: Precondition (a)'s list of tasks that proceed contradicts its own rule

**Location:** `tasks.md`, precondition (a), lines 143 to 146 ("Task 006-043, Task 006-044, Task 006-048, Task 006-049, Task 006-052 ... proceed").

**What is wrong:** the rule is that tasks which "read a requirement's kinds or run the server end to end wait". Three of the listed tasks cannot produce their evidence before the follow-up.
- **Task 006-044** creates `delivery_acknowledgement_required()`, which returns whether `lease-withdrawal-v2` is recorded for both kinds.
  - Its evidence, "the function's result for each recording (both kinds, engines only, none)", needs `lease-withdrawal-v2` rows. The closed capability checks refuse those until the follow-up extends them.
  - Both Task 006-044 and the follow-up rewrite the same three closed checks. Whichever migration runs second must carry the other's values, or it drops them.
- **Task 006-049**'s evidence needs provisional marks and withdrawn leases, which Task 006-047 and Task 006-050 build and which wait:
  - "a test per verdict and reason (acknowledged, ... withdrawn ...)";
  - "a probe answered `NOT_HELD` with the reason `withdrawn`".
- **Task 006-052**'s evidence ("refused before, served after the acknowledgement ... followed by a hand-back") needs a provisional lease and the hand-back's withdrawal service. That service reads `lease-withdrawal-v2`.

**Why it matters:** the server owner starts three tasks the plan says can proceed and finds their evidence unreachable. For Task 006-044 there is also a migration-order conflict.

**Fix:**
- Move Task 006-044 to the waiting list, with its closed checks including `lease-withdrawal-v2`.
- For Task 006-049 and Task 006-052, either move them too, or say their tests write the marks and the withdrawn state by SQL fixtures until Task 006-047 and Task 006-050 land, and repeat them afterwards.

### Minor 6: "Live or lapsed" misses a process whose row the registry deleted, and the Consequences invite the operator to delete such rows

**Location:**
- Architecture decision 0099, decision 10 and the Consequences, line 262 ("which can take the day the registry keeps a silent row, unless the operator removes it");
- `stream-completion.md`, lines 409 to 412;
- Decision 006-059 (9).

**What is wrong:**
- The second refusal of the `delivery-acknowledgement-v1` entry reads registered rows. `instance_heartbeat` deletes rows silent for more than a day (migration lines 263 to 268).
- In the merged head, a plain connection failure never closes the gate (`registry.rs` lines 134 to 142). So an engine of pull request #260 whose registry connection has failed for a day, while its request pool works, still serves capability calls, and the census no longer sees it.
- The Consequences go further and offer the operator a shortcut, "unless the operator removes it". Removing the row of a process that is still running does by hand exactly what the refusal exists to prevent.
- The claim "so no binary without the liveness rule holds an open gate when the first provisional lease is claimed" is therefore not absolute.
- The consequence stays within the trust assumption of decision 12: a host that keeps the protocol calls no capability before `ACKNOWLEDGED`. That is why I rate it Minor.

**Fix:**
- In the Consequences and in section 5 of `docs/runbooks/instance-registry.md`, allow removing a silent row only once the process is confirmed stopped (its host or pod gone). This is the same rule the census assumes.
- Add the deleted-row case to Gap 006-044, or bound it: "a process whose registry has been silent for more than a day".

### Nits

1. **The reserve's start is dated two ways.** The Consequences (Architecture decision 0099, line 260) say "60 places ... from the activation". The reserve is keyed to the compiled minor, so it applies from Task 006-057's build, whether or not anything is activated (`stream-completion.md` line 585). Write "from Task 006-057".
2. **"Which is answered" is an overclaim** (`sdks/SPEC.md` line 390). The failover that begins during that attempt is the stated residual. Also, "a failover gives no evidence before its promotion" does not cover a frame claimed just before the failure that arrives after a fast-failing attempt. Write "which a working primary answers", and name the frame case in Gap 006-047 or exclude it.
3. **The forward step for `lease-withdrawal-v2` is named nowhere.**
   - `capability-activate lease-withdrawal-v2` appears in no procedure:
     - the forward order (upgrade to the follow-up, activate `lease-withdrawal-v2`, upgrade to this build, activate `delivery-acknowledgement-v1`);
     - the runbook updates of Task 006-046;
     - the forward case of `provisional-lease-cutover`.
   - State also that recording `lease-withdrawal-v2` is irreversible as soon as any withdrawal of Architecture decision 0098 exists. Or key its removal guard to `NOT_ACKNOWLEDGED`, which is the cause the floor actually protects.
4. **The merged head asserts the opposite of request (9).** `apps/control-plane/src/capability_activation.rs` line 359 and `tests/integration/schedules-bulk-operations/lease_withdrawal_activation.rs` line 200 assert that `lease-withdrawal-v2` is not activatable. Name these tests in request (9) as tests to invert, so the follow-up's reviewer expects the change.
5. **Two indentation regressions:**
   - `stream-completion.md` line 289: `bool acknowledge_before_start = 23;` is now indented four spaces.
   - Lines 918 and 919: `SagatCallsAfterWithdrawal` and `SagatCapabilitiesRefusedRegistrySilent` are indented one level deeper, so Markdown nests them under `SagatLeaseDeliverySlow`.
6. **Lines still overrun the wrap** (round-11 Nit 9). Examples: the new lines of decision 2 and decision 10 in Architecture decision 0099, `sdks/SPEC.md` lines 365 to 367 and 379, and `tasks.md` lines 164, 229 and 256.

---

## Part 3: Is it complete enough to accept?

**Yes, for the owner's decision, with Minors 1 and 3 corrected in the same pass.**
- **The core is settled.** The floor, the last structural gap, is now enforced by the registry rather than described.
- **The mechanism has held for five rounds**, and this round did not weaken it:
  - the row lock and the lease's own deadline;
  - the cap and the attempt flag bound per instance;
  - the probe;
  - the no-epoch beat, now a function of its own that always applies a committed epoch;
  - the new floor capability.
- **Minors 1 and 3 should be fixed before acceptance.** They do not threaten correctness: every path still ends in a withdrawal, an ordinary failure or a review, as stated. But as written they can make the early give-up unreachable on an ordinary busy host, which is the main promise of the "acknowledgement failing alone" row and of the gate.
- **The rest can go into the tasks:**
  - Minor 2 needs a few sentences;
  - Minor 4 needs a channel setting per SDK;
  - Minor 5 is a correction to the task list;
  - Minor 6 is a runbook sentence;
  - the Nits.
- **The risks the owner accepts with it:** Gap 006-041, Gap 006-042, Gap 006-047 (now including the failover during the deciding attempt) and Gap 006-048.

**Files referenced:**
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/round-11/egm-design.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/apps/control-plane/src/registry.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/apps/control-plane/src/capability_activation.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/tests/integration/schedules-bulk-operations/lease_withdrawal_activation.rs
