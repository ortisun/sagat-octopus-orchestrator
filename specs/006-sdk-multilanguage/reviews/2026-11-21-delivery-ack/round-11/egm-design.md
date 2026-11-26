# Design critique, round 11: delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)

**Review status: complete.** No target standard was supplied, so I reviewed against the persona's audit vectors, the questions in the request and my round-10 report.

**What I read:**
- the persona and my round-10 report;
- the round-10-to-11 diff in full;
- Architecture decision 0099: the header, decision 10, the Consequences and the Rollback;
- `stream-completion.md`: lines 392 to 451, the acknowledgement steps 5 to 7 (lines 590 to 629), host items 4 and 5, the failure table, the signals and the evidence table;
- `sdks/SPEC.md` §5 (lines 266 to 295) and §6.2 (lines 361 to 412);
- `research.md`: Decision 006-059, Decision 006-065, Decision 006-066 and Gap 006-044 to Gap 006-048;
- `tasks.md`: precondition (a), Task 006-045, Task 006-046, Task 006-049, Task 006-054, Task 006-056 and Task 006-057.

**What I checked in the code:**
- whether the merged head lists `lease-withdrawal-v1`. It does, on both kinds (`apps/control-plane/src/registry.rs` line 33, "always"; `apps/execution-engine/src/settings.rs` line 107), and it cannot decode `NOT_ACKNOWLEDGED`.
- what the per-connection layer is today: `crates/api-grpc/src/server.rs` lines 84 and 157, and `CappedConnectInfo` in `crates/api-grpc/src/limits.rs`.

## Verdict

**Count: no Blockers, 1 Major, 4 Minors, 9 Nits.**

Round-10 Major 1 is **partly resolved**:
- The fallback is gone everywhere it was stated.
- The hard precondition of Task 006-057 is written in the four places I asked for.
- But the precondition is "a released build with (2) exists", not "no admissible build lacks (2)". So the Rollback's claim that the registry enforces the floor is still false for one binary that already exists: the merged head (Major 1).

The other round-10 answers are mostly right. The leftovers are edges of the early give-up and of the new client and server concurrency rules.

---

## Part 1: The round-10 findings

### Major 1 (the fallback could neither be activated nor enforce its floor): partly resolved

**Done:**
- Decision 006-059 says "There is no fallback" and why.
- Precondition (a), Task 006-057's title, decision 10 ("which a released build must have before this design leaves dormancy"), Decision 006-047, Decision 006-055, `stream-completion.md` lines 397 and 398, and Rollback step 3 now all state the same plan.
- The dormant tasks proceed, and the feature cannot leave dormancy without the floor.
- `delivery-acknowledgement-v1` stays engines-only, so the activation problem (part 1 of the round-10 finding) is gone.

**Not done:** part 2 of the round-10 finding, "the floor is not enforced after a rollback", survives in a narrower form.
- The enforcement rests on `lease-withdrawal-v1` staying recorded. The merged head lists that capability too.
- The text now requires that a released build with request (2) exist. It does not forbid a released, or deployed, build without it.
- Precondition (a) also lost "before any build of it is released"; only Decision 006-059 still states it, as a plan.
- Details are in Major 1 below.

### The round-10 Minors and Nits

- **Minor 1:** resolved. The premise texts and Task 006-045, Task 006-046 and Task 006-057 are aligned, and the race test is replaced. One leftover sentence remains (Nit 1).
- **Minor 2:** largely resolved: at least one ended attempt, evidence after the latest failure with no failed call since, the quiet-host case, and background traffic in the evidence rows. Two things are not done:
  - The requested correction of Decision 006-065 and the failure row (the residual window) was not made.
  - The new "a frame with leases arrived" evidence contradicts the quiet-host claims (Minor 1 below).
- **Minor 3:** resolved in form, since the 18 s case now expects the outcome of Gap 006-047. Its "no review" is not assured by the rules (Minor 2 below).
- **Minor 4:** resolved. §6.2 now has the earlier deadline, and the SDK task tests it.
- **Minor 5:** resolved: the latch rules are stated, the Task 006-046 test is replaced, and the alert is added. The no-epoch beat itself is not counted (Nit 8).
- **Minor 6:** partly resolved. The client rule and the dormancy of the reserve are written, but the client rule is not observable uniformly in six gRPC stacks and has no SDK test (Minor 3 below). The reserve's switch is unkeyed and contradicts two dormant tasks (Minor 4 below).
- **Nits:**
  - Resolved: the Enforced-by line, the status of Architecture decision 0100 and the template, the index check's script, the probe wording, "shed" defined, the 5-minute shed in the gate, the runbook symptom, the `REGISTRY_SILENT` deadline, the cap-after-rebuild sentence in the marks, and "bound boolean".
  - Still open: the Consequences' "waits one receive bound" (Nit 5), the cap row of the failure table (Nit 6) and the line wraps (Nit 9).

---

## Part 2: New findings

### Major 1: The floor's enforcement still admits any build that lists `lease-withdrawal-v1` without decoding `NOT_ACKNOWLEDGED`, and the merged head is such a build

**Location:**
- Architecture decision 0099, the Rollback paragraph (lines 272 to 277: "so the registry never admits a binary below that build once any withdrawal exists");
- `stream-completion.md`, Rollback step 3 (around line 794);
- `tasks.md`, precondition (a) (lines 142 to 147) and Task 006-057 (line 350);
- `research.md`, Decision 006-055 and Decision 006-059.

**What is wrong:**
- The registry excludes a binary only if it does not list a recorded requirement. After the Rollback's removal of `delivery-acknowledgement-v1`, the only recorded requirement is `lease-withdrawal-v1`.
- The merged head of pull request #260 lists `lease-withdrawal-v1` on both kinds (`apps/control-plane/src/registry.rs` line 33) and cannot replay `NOT_ACKNOWLEDGED`.
- Task 006-057 now requires that some released build with requests (2) and (7) exists. It does not require that no earlier release, or deployed snapshot of `develop`, lacks (2).
- Precondition (a) no longer says the follow-up lands "before any build of it is released"; only Decision 006-059 still says so, as a plan.
- So here is a plausible sequence: 0.3.0 ships pull request #260 without the follow-up, 0.3.1 ships the follow-up, and Task 006-057 proceeds. An operator who later rolls back to 0.3.0 is admitted, and that build fails to replay every journal holding the cause. This is the round-9 replay failure.
- "No mark, and so no `NOT_ACKNOWLEDGED`, exists before the floor does" is true, but it does not help: the hazard is a rollback after marks exist.

**Why it is Major:** the design's central rollback guarantee, the floor "enforced, not described", is false as written whenever such a build exists. The text chooses a procedure again, which round 10 rejected.

**Fix.** Choose one; either is a few sentences:
- **Checked precondition.** Make Task 006-057's precondition "every released build that lists `lease-withdrawal-v1` decodes `NOT_ACKNOWLEDGED`", checked against the release tags in its evidence. Restore "before any build of it is released" in precondition (a) as hard. Then the Rollback's sentence holds for released builds, and the runbook names the exclusion of unreleased snapshots.
- **Enforced by the registry (preferred if the follow-up's release may trail pull request #260).** The follow-up adds a capability meaning "replays the causes of 1.8", for example `lease-withdrawal-cause-v2`:
  - both kinds of the floor list it;
  - the activation entry of `delivery-acknowledgement-v1` requires it recorded for both kinds;
  - its removal is refused while a journal holds `NOT_ACKNOWLEDGED`, as decision 8 of Architecture decision 0098 refuses for `lease-withdrawal-v1`.

  The Rollback then names this capability as the enforced floor.

### Minor 1: The early give-up still has a failover window, "failed call" is undefined for retried calls, and the new frame evidence contradicts the quiet-host claims

**Location:**
- `sdks/SPEC.md` §6.2, lines 380 to 385 ("A failover, a busy engine and a host with no other call therefore wait for the ordinary give-up");
- `stream-completion.md`, host item 4 (lines 730 to 740), the failure row at line 816 ("so no host gives up early") and the Fallback evidence row at line 911 ("on a host whose only work is flagged leases, the same answers wait for the ordinary give-up");
- `research.md`, Decision 006-065 ("keeps a database failover ... from turning into hand-backs").

**What is wrong:**
1. **The decision is not taken again once it is made.**
   - Once the condition holds, the host "starts no further attempt, waits for one in flight ... and otherwise gives the lease up".
   - If a failover starts between the qualifying `OK` and the end of the in-flight attempt, that attempt fails during the failover. The lease is still given up, and its hand-back is repeated through the promotion, where the mark no longer counts. The result is an ordinary failure, with the attempt spent.
   - A hand-back already in flight when a failover starts has the same fate.
   - Round 10 asked for the failure row and Decision 006-065 to state this residual; they still say "no host gives up early".
2. **"No failed call to the database since" does not say whether one failed try counts.**
   - A heartbeat being retried under §5 has not "failed" until its tries are used up.
   - Six SDKs will read this differently, and the stricter reading is the one that keeps a failover out.
3. **"A frame with leases arrived" makes the quiet-host claim false.**
   - A host whose only work is flagged leases still receives frames while it has free slots and the queue has work.
   - Under two minutes of statement timeouts, each such frame satisfies the evidence, so the lease is given up early. That is the desired behaviour, but it contradicts §6.2's sentence and the evidence row, which expects the ordinary give-up.
   - The row passes or fails depending on `max_inflight` and the queue.

**Fix:**
- When the in-flight attempt ends with a failure and no qualifying evidence follows it, withdraw the early give-up and resume attempts until the ordinary give-up.
- Define a failed call as any failed try of a call that reaches the database.
- Reword §6.2 and the row as "a host with no other call and no further frame", and pin the row's setup: `max_inflight` equal to the frame size, or no work queued.
- State in the failure row and in Decision 006-065 that a failover beginning after the decision, or during a hand-back, costs the attempt (Gap 006-047, "a mark across another server's takeover").

### Minor 2: The 18 s failover case expects "no review", which depends on the hand-back's retries outlasting the failover

**Location:**
- `stream-completion.md`, the failure row at line 816 and the Fallback evidence row at line 911 ("the outcome of Gap 006-047 (an ordinary failure, no review)");
- the SDK conformance task ("at an 18 s time-to-live (the outcome of Gap 006-047)");
- `sdks/SPEC.md` §5, line 281 (`CompleteStep`, `FailStep`: "at most 5 times").

**What is wrong:**
- At an 18 s time-to-live, the ordinary give-up is at about 12 s, so the hand-back starts about 3 s before a 15 s promotion.
- The hand-back is a `FailStep` with at most 5 tries. With the default backoff (200 ms doubling, scaled by 0.5 to 1.0), the waits total about 1.5 to 3 s, plus each try's own failure time.
- If the tries are used up before the promotion, the lease lapses at 18 s. The pass then finds a mark of another incarnation and ends it with `expire_attempt`: an unknown outcome, and a review for an `Activity`.
- So "no review" is a property of the timing, not of the rules.

**Fix (choose one):**
- Bound the retries of a `TASK_NOT_STARTED` hand-back by the lease's expiry rather than by a count (as `AcknowledgeTasks` is bounded by its give-up), and say so in §5.
- Or expect "an ordinary failure, or a review if the hand-back's retries end before the promotion" and report which occurred.

### Minor 3: "An attempt that the host's transport never started on a stream does not count" cannot be observed uniformly, and has no SDK test

**Location:**
- `sdks/SPEC.md` §6.2, line 393;
- `stream-completion.md`, line 568 and host item 5 (line 749);
- the Fallback evidence row ("a connection with more than 128 calls ... does not fall back");
- the SDK host task (no case).

**What is wrong:**
- Mainstream gRPC clients do not expose whether a call obtained an HTTP/2 stream before its deadline. This holds for the C-core stacks (Python and, through them, others), grpc-go, grpc-js, Grpc.Net.Client and tonic.
- A call that waits for stream credit ends with the same `DEADLINE_EXCEEDED` as a call swallowed on the path.
- So the rule is implementable in some SDKs and not in others, and the evidence case cannot pass uniformly.
- No SDK unit test or conformance case names the rule.

**Fix:**
- Make the rule observable: each SDK bounds its own concurrent calls per channel below the server's advertised `max_concurrent_streams` (128), and an attempt still queued in the SDK's own limiter at its deadline does not count. Or give `AcknowledgeTasks` and its probe a dedicated channel.
- Add the case to the SDK host task and to the conformance list.

### Minor 4: The reserve's "applies only from Task 006-057" has no switch, contradicts two dormant tasks, and its cost at activation is unstated

**Location:**
- `stream-completion.md`, step 1 of the acknowledgement (lines 563 to 568, "the reserve is 4 of those places for the class `Acknowledge`, and applies only from Task 006-057");
- Task 006-049, lines 243 and 244 ("one with 64 transfers in flight answered within the bound");
- Task 006-056, line 340 (the gate);
- Task 006-057 (no mention).

**What is wrong:**
- **No key is named.** The text does not say what turns the reserve on: the compiled minor, the recorded requirement, or a code change in Task 006-057. Task 006-057 does not list it.
- **Task 006-049 cannot pass while dormant.** With `concurrency_per_connection` at 64 and 64 transfers in flight, an acknowledgement on that connection waits unless the reserve exists. That dormant task's evidence therefore cannot pass as written.
- **The gate measures something else.** The gate, which runs before Task 006-057, would measure a configuration other than the one shipped.
- **The cost is unstated.** From activation, every other class is limited to 60 places per connection. That change for every service is not stated in the Consequences.

**Fix:**
- Key the reserve to the compiled `DELIVERY_ACKNOWLEDGEMENT_PROTOCOL_MINOR`, which tests can raise as the ceiling tests do.
- Name it in Task 006-057.
- Run Task 006-049's case and the gate with the minor raised.
- Add a dormancy test: per-connection concurrency of 64 for every class.
- State the 60-place limit in the Consequences, or make the reserve borrowable.

### Nits

1. **A stale premise in Decision 006-059 (7).** `research.md` lines 805 and 806 still say "which the gate reads as capability calls undecided ... never closes the fleet's gates" before the latch sentence refines it. Delete the earlier clause.
2. **Two texts disagree on which tasks wait.** Decision 006-059 says "the dormant tasks proceed without it", while precondition (a) says Task 006-045 and Task 006-046 wait for the follow-up to merge. Write "the other dormant tasks" in Decision 006-059.
3. **Call ordering differs.** `sdks/SPEC.md` line 396 says calls are "ordered by their give-ups"; `stream-completion.md` and Decision 006-065 say "by the give-up of the last of their leases". The normative text should carry the precise rule.
4. **`worker_leases_expired_total{mark}` is defined twice with different values** (`stream-completion.md`, lines 858 and 860): "the same `mark` values" (four) against "(`acknowledged`, `recovered`, `unmarked`)" (three). Keep one definition.
5. **The Consequences still understate the cost.** Architecture decision 0099, line 255, says each flagged lease "waits one receive bound". During sheds, a failover or on a quiet host it waits until the ordinary give-up, up to 120 s, as the runbook symptom now says.
6. **The cap row of the failure table** (`stream-completion.md`, line 832): "exists only when earlier withdrawals were written without the flag (by the floor build)" should add "or after a projection rebuild", as the marks text now says.
7. **The overload's third argument is not described.** Neither `stream-completion.md` line 434 nor Decision 006-059 says what the boolean of `instance_heartbeat(uuid, bytea, boolean)` means.
8. **An owner transaction held open with no engine lapsed raises no signal.** `SagatCapabilitiesRefusedRegistrySilent` fires only on refusals. A counter of beats that returned no epoch would show the held transaction before any engine lapses.
9. **Lines still overrun the wrap:**
   - `sdks/SPEC.md`, lines 396 and 397;
   - `stream-completion.md`, lines 424, 437 and 911;
   - Architecture decision 0099, line 275;
   - `tasks.md`, lines 346, 347 and 353.

---

## Part 3: Is it complete enough to accept?

Not yet, but it is one precondition sentence away:
- Major 1 is fixed either by making "no admissible build lacks request (2)" the checked precondition of Task 006-057, or by having the follow-up add a replay capability that the activation requires.
- Minors 1 and 2 correct claims and test expectations in normative text, and belong in the same pass.
- Minors 3 and 4 and the Nits can go into the tasks.

The mechanism has held for four rounds, and this round's changes did not weaken it:
- the row lock and the lease's own deadline;
- the cap and the fenced attempt flag, now bound per node;
- the probe and the hand-back's evidence;
- the latch of the no-epoch beat, which I checked against the census and found safe.

With Major 1 fixed, I would accept it for the owner's decision, with Gap 006-041, Gap 006-042, Gap 006-047 and Gap 006-048 as the stated risks.

**Files referenced:**
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/apps/control-plane/src/registry.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/apps/execution-engine/src/settings.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/server.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/limits.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/round-10/egm-design.md
