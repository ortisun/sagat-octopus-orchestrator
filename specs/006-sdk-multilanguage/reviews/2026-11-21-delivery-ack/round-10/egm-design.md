# Design critique, round 10: delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)

**Review status: complete.** No target standard was supplied, so I reviewed against the persona's audit vectors, the questions in the request and my round-9 report.

**What I read:**
- the persona and my round-9 report;
- the round-9-to-10 diff in full;
- Architecture decision 0099 (decisions 1 to 13, the Consequences and the Rollback);
- `stream-completion.md`, lines 320 to 439, and the changed parts of the marks, the withdrawals, the host items 4 and 5, the failure table, the signals and the evidence table;
- `sdks/SPEC.md` §5 and §6.2, lines 355 to 404;
- `research.md`, Decision 006-042, 006-045, 006-047, 006-051, 006-053, 006-059, 006-065 and 006-066, and the changed gaps;
- `tasks.md`: precondition (a), Task 006-044 to Task 006-047, Task 006-054, Task 006-056, Task 006-057 and the SDK host task.

**What I checked in the code:**
- whether Architecture decision 0098 defines `SagatLeaseAttemptCapped`. It does not, so "owned here" is right.
- how step rows are written. They use a plain `UPDATE`, with insert-or-update decided in code (`crates/infra-postgres/src/execution.rs`, lines 3821 to 3824). So the unqualified `status` and `attempt` in the flag's `CASE` refer to the old row, as the text says.
- what fields the contract carries for the deadlines.

## Verdict

**Count: no Blockers, 1 Major, 6 Minors, 10 Nits.**

The round-9 Major is partly resolved:
- The preferred plan (a follow-up of pull request #260) is now recorded consistently in the header, decision 10, the Rollback, Decision 006-059 and precondition (a).
- The fallback branch is new, and as written it can neither be activated nor enforce its floor (Major 1).
- Several texts still argue from the old premise (Minor 1).

The other round-9 changes are mostly right:
- the retitled decision 4 and its exactness paragraph;
- keeping the withdrawn token's hash on the hand-back's withdrawal;
- the precise early give-up window;
- excluding sheds;
- the attempt flag's fenced form.

The early give-up's new "another call answered `OK`" condition is the right idea, but it is under-specified. It does not keep out a failover, and it is often missing on a quiet host (Minor 2). One new gate expectation cannot be met under the design's own rules (Minor 3).

---

## Part 1: The round-9 Majors

### Major 1 (the merged base lacks the floor's preconditions): partly resolved

**Done:**
- Decision 006-059 now records the status of each request against the merged head: only (3) is present, and (1), (2), (5), (6), (7) and (8) are missing.
- The preferred plan is the follow-up of pull request #260 before any release, with the rollback floor defined as the first released build that has the cause.
- The header says "relies, once they are built".
- Decision 10 says "will decode", and the Rollback says "enforced, not described, once it is built".
- Precondition (a) states both branches.
- Requests (6) and (8) come as a new migration with `CREATE OR REPLACE FUNCTION` and a privilege test, if the old migration has been applied anywhere.

**Not done:**
- **The fallback branch is defective as written** (Major 1 below). Under the fallback, the requirement is recorded for both kinds, but no control plane lists the capability. And after the rollback's removal, nothing excludes the merged head.
- **Texts still state the old premise** (Minor 1): `stream-completion.md` lines 396 to 398, Decision 006-047, and Task 006-045, Task 006-046 and Task 006-057.

### The round-9 Minors and Nits

- **Minor 1:** resolved. The retitle, the exactness paragraph, Rollback step 2 and the new barrier case are all in.
- **Minor 2:** resolved:
  - the wording is now "not yet `RUNNING`";
  - `withdrawn_lease_token_hash` is written on every withdrawal of a marked lease;
  - decision 12's list is extended;
  - the Task 006-050 case is added;
  - the "covered by the promise alone" window is named.
- **Minor 3:** resolved in kind (any answer other than a shed). The runbook symptom is still wrong (Nit 5).
- **Minor 4:** resolved in both texts, apart from a vacuous-truth gap (Minor 2).
- **Minor 5:** resolved by excluding sheds and adding the three gate scenarios. A sustained shed is now unmeasured (Nit 5).
- **Nits:**
  - 1, 2, 3, 4 and 7 are resolved. Nit 7 is resolved only partly (Nit 6).
  - 5 (Architecture decision 0100's status) is not resolved: the title changed and the status did not (Nit 2).
  - 6 (line wraps) is still open (Nit 10).

---

## Part 2: New findings

### Major 1: The fallback of Decision 006-059 can neither be activated nor enforce its rollback floor

**Location:**
- `research.md`, Decision 006-059, the last sentences ("this design's dormant build then becomes the rollback floor ... recorded for both kinds");
- `tasks.md`, precondition (a), lines 145 to 147;
- Architecture decision 0099, decision 10 ("if that request cannot be built into the floor, it is recorded for both kinds") and the Rollback paragraph;
- Task 006-057 (lines 341 to 347).

**What is wrong:**

1. **The feature cannot be activated under the fallback.** Recording `delivery-acknowledgement-v1` for both kinds requires every live control plane to list it.
   - Decision 10 and Task 006-057 define the capability for engines only ("serves `AcknowledgeTasks` ... withdraws a marked provisional lease").
   - Task 006-057's evidence asserts that "the control plane's registry test does not" list it.
   - So the activation census would refuse on any fleet with a live control plane.
2. **The floor is not enforced after a rollback.** The Rollback removes `delivery-acknowledgement-v1` before the binary is rolled back.
   - After that removal, the only recorded requirement is `lease-withdrawal-v1`. The merged head of pull request #260 lists it, and that head cannot decode `NOT_ACKNOWLEDGED`.
   - So under the fallback the registry admits that head, on either kind, as soon as the rollback's removal commits.
   - "Deployed fleet-wide before Task 006-057" is a procedure, not an enforcement.
   - The Rollback paragraph's claim, "the registry never admits a binary below that build once any withdrawal exists", is false in this branch. The failure is the replay failure of round 9.

My round-9 alternative ("record it for both kinds") covered the forward path only. This finding corrects it too.

**Why it is Major:** in one of the two planned branches, the design is unbuildable (no activation) and its central rollback guarantee is wrong.

**Fix.** Choose one:
- **Preferred: drop the fallback.** Make a released floor build with requests (2) and (7) a hard precondition of Task 006-057, not of the first task. The dormant tasks can proceed, and the feature cannot leave dormancy until the floor exists. Say so in precondition (a), Decision 006-059, decision 10 and the Rollback.
- **Otherwise, give the dormant build a replay capability of its own.** It would mean "replays `NOT_ACKNOWLEDGED`", for example `lease-withdrawal-cause-v2`:
  - every process of the dormant build lists it, for both kinds, from its first deployment;
  - it is recorded for both kinds before Task 006-057, and the activation entry of `delivery-acknowledgement-v1` requires it;
  - its removal is refused while any journal holds the cause, like the replay-floor guard of `lease-withdrawal-v1` in Architecture decision 0098 decision 8;
  - `delivery-acknowledgement-v1` then stays engines-only in both branches, and the Rollback names the new capability as the enforced floor.

### Minor 1: Texts still argue from "the floor's control planes already decode the cause", and three tasks contradict the plan

**Location:**
- `stream-completion.md`, lines 396 to 398 ("the floor build's control planes already decode the cause (precondition (a) of the task phase)");
- `research.md`, Decision 006-047, line 646 (the same);
- Task 006-045, line 187 ("confirmed ... as the server build added it");
- Task 006-046, line 190 ("keeps request (7) ... as the floor build has it") and line 198 ("a beat racing an activation held between its census and its commit waits for the epoch");
- Task 006-057, line 342 ("required of engines only") and line 344 ("the control plane's registry test does not").

**What is wrong:**
- These are the normative restatements that a builder reads. Under both branches they state a precondition that the base does not have.
- Under the fallback, Task 006-045 and Task 006-046 build the cause and request (7), not "confirm" or "keep" them.
- Task 006-046's race test now contradicts `FOR SHARE SKIP LOCKED`: the beat no longer waits for the epoch (Minor 5).

**Why it matters:** two tasks could pass by asserting what was never built, which is exactly what round 9 warned about.

**Fix:**
- Rewrite these places to "will decode (request (2)), or per the fallback".
- Word Task 006-045 and Task 006-046 for both branches: confirmed if the follow-up landed, built here otherwise.
- Replace Task 006-046's race test as Minor 5 says.
- Align Task 006-057 with whichever fix Major 1 takes.

### Minor 2: The early give-up's "another call answered `OK`" does not keep a failover out, is often absent on a quiet host, and is vacuously satisfied when no attempt ended

**Location:**
- `sdks/SPEC.md` §6.2, lines 379 to 382;
- `stream-completion.md`, host item 4 (lines 706 to 712);
- Decision 006-065 ("The condition on another call keeps a database failover ... from turning into hand-backs");
- the failure row at line 788 ("so no host gives up early").

**What is wrong:**

1. **The evidence can come before the failure.** The condition is an `OK` "in that time", meaning any time since the lease's first attempt. Take this sequence:
   - a heartbeat is answered `OK` at 1 s;
   - a failover starts at 2 s, and the acknowledgement's attempts get deadlines and store errors until 6 s;
   - the lease is given up early;
   - its hand-back is repeated through the promotion and lands on a mark that no longer counts, so it is an ordinary failure, and the attempt is spent (with `maxAttempts = 1`, the step fails).

   So "A failover ... therefore waits for the ordinary give-up" claims too much. The rule narrows the race; it does not close it.
2. **The evidence is often absent.** It is limited to "a heartbeat, a report, a hand-back". A host whose only work is flagged leases sends none of those, so it never gives up early. Examples are a host that just started, a quiet host, or one with `max_inflight` equal to the frame size. A frame with leases is a committed claim, and it does not count either.
   - Two evidence rows assume the early give-up without arranging other traffic: the proxy that cuts at 1 s ("each lease given up after one receive bound") and "two minutes of answers with the envelope".
   - As written, they pass or fail depending on what else the harness happens to run.
3. **The condition can hold with nothing to support it.** With one attempt in flight and none ended, "every attempt that ended ... ended with an answer" is true with nothing behind it. The first attempt ends exactly at the window's edge, so silence can be early-given-up depending on the order in which an implementation evaluates the two events.

**Why it matters:** six SDKs will implement this edge. Part 1 spends attempts during exactly the event the condition was added to cover. Part 2 brings back the ordinary give-up of up to 120 s for the first wave.

**Fix:**
- Require that at least one attempt ended in the window.
- Make the evidence an `OK` answer to a call sent after the lease's most recent failed attempt ended, with no failed call of the host to the database since. A received frame with leases also counts.
- Correct Decision 006-065 and the failure row: a failover that starts within that short window still costs the attempt (Gap 006-041).
- State the background traffic in the evidence rows that expect an early give-up, and add a case for a host with flagged leases only.

### Minor 3: The 15 s failover case at an 18 s time-to-live expects "no attempt spent", which the design's own rules make impossible

**Location:**
- `stream-completion.md`, the Fallback evidence row (line 878, "a 15 s database failover ... at an 18 s and a 30 s time-to-live: no early give-up, no fallback, no review, no attempt spent, the leases run as `recovered`");
- the SDK task, line 371;
- Task 006-056, line 336;
- the failure row, line 788.

**What is wrong:**
- At an 18 s time-to-live, the ordinary give-up comes 12 s after receipt, inside a 15 s failover. The hand-back is repeated through the promotion, and the mark no longer counts (another incarnation), so it is an ordinary failure.
- A repeat that is answered after the promotion has, at best, about 3 s left. That is under the margin, so it is `CANCELLED` (`too_late`, on a `recovered` lease), and its hand-back is again an ordinary failure.
- The failure row's own "or `CANCELLED` without it" spends the attempt, but the row does not say so.

**Why it matters:** a gate case that cannot pass is either weakened during the build or blocks the build for the wrong reason.

**Fix:**
- At an 18 s time-to-live, expect an ordinary failure (the attempt spent, no review), or shorten the failover to under 6 s.
- Keep "no attempt spent, run as `recovered`" for the 30 s case only.
- Say in the failure row that `CANCELLED` after a promotion spends the attempt.

### Minor 4: The normative give-up still counts from the expiry alone; only the summary added the attempt deadline

**Location:**
- `stream-completion.md`, host item 4, lines 706 and 707 ("The give-up counts from the earlier of the expiry and, when the lease carries its timeout, the attempt deadline"), new this round;
- `sdks/SPEC.md` §6.2, lines 376 and 377 ("its time left at receipt (its expiry minus the host's clock at receipt)"), unchanged.

**What is wrong:** `stream-completion.md` itself says the normative host text is `sdks/SPEC.md`. A host built to that text, holding a lease whose `timeoutMs` is shorter than its time-to-live, plans its give-up after the attempt deadline. While the acknowledgement fails, this happens:
1. The pass withdraws the lease at the attempt deadline (`NOT_ACKNOWLEDGED`, which does not set the attempt flag).
2. The host's later hand-back is stale.
3. The next lease is flagged again, and the cycle repeats up to the cap.
4. A marked lease at the cap is then ended with `expire_attempt`, a review for an `Activity` whose work never ran.

**Fix:**
- Put the earlier of the two deadlines into §6.2, with where the host reads `timeoutMs` at receipt.
- Add a host unit test and a conformance case: `timeoutMs` 18 s with a 60 s time-to-live and a failing acknowledgement, given up at 12 s, no cycling.

### Minor 5: The effect of a beat that returns no epoch is undefined, and the race test contradicts it

**Location:**
- `stream-completion.md`, lines 431 to 434 ("returns no epoch, which the gate reads as 'an activation in progress', capability calls undecided");
- Decision 006-059 (7);
- Architecture decision 0099 decision 10;
- Task 006-046, line 198.

**What is wrong:** "undecided" has two readings, and only one is safe for the fleet:
- If an engine that serves turns to refusing on a beat with no epoch, an owner transaction left open closes every gate. The text promises the opposite ("never closes the fleet's gates").
- If the beat with no epoch counts as successful for the 25 s measure, the text must also say that it never reopens an engine that is refusing. "Until a beat and the gate's decision under the epoch it returned" implies this but does not state it.

Task 006-046 still tests a beat that "waits for the epoch and refuses after it", which `SKIP LOCKED` no longer does. Nothing counts or alerts an owner transaction held open, which now silently keeps refusing engines closed.

**Fix:** state three rules:
1. A beat that commits `last_seen` and returns no epoch keeps a decided gate's decision and resets its silence measure.
2. It never opens a refusing gate.
3. It is counted, with a runbook entry for an owner transaction held past a bound.

Replace the Task 006-046 case with both states during an activation held open: a serving engine keeps serving, a refusing engine keeps refusing, and both decide on the first beat after the commit. Safety follows from the census counting every engine that beats as live.

### Minor 6: "A connection with more than 128 calls does not fall back" rests on an SDK rule that is not normative, and the server's layer change is unscoped

**Location:**
- `stream-completion.md`, step 1 of the acknowledgement, lines 544 to 547 ("which is why an SDK does not put the acknowledgement behind a saturated connection of its own");
- the Fallback evidence row and Task 006-049 ("a connection with more than 128 calls ... answered within the bound");
- `sdks/SPEC.md` (no rule).

**What is wrong:**
- **The client side.** The server advertises `max_concurrent_streams` 128 (`crates/api-grpc/src/server.rs`, line 85).
  - On one HTTP/2 connection, the 129th stream waits in the client.
  - An acknowledgement queued there gets no answer within 6 s, and so does its probe on the same connection, so the call counts. Three such calls cause a fallback.
  - Only an SDK rule can prevent this, for example a separate channel for `AcknowledgeTasks`, or "an attempt that never got a stream does not count". `sdks/SPEC.md` has no such rule, so six SDKs cannot pass the case uniformly.
- **The server side.** Replacing tonic's `concurrency_limit_per_connection` changes the stack for every service of the engine, in the dormant build. A "reserve for the class `Acknowledge`" would lower every other class's per-connection concurrency even before activation.

**Fix:**
- Add the client rule to §6.2, with its conformance case.
- State that the reserve is borrowable, or that it applies only from Task 006-057.
- Name the task that builds the layer, and give it a test that per-connection concurrency is unchanged for older calls while dormant.

### Nits

1. **The "Enforced by" line opens ungrammatically.** It reads "to be built with the decision, the attempt flag's allowed set in the static check among it, (`specs/...`)".
2. **Architecture decision 0100 and its index check.** The title is now "Retired number", but the status still reads "Superseded by Architecture decision 0099", which a record never accepted cannot be; round-9 Nit 5 still applies. The new index check is placed in `scripts/check-agent-config.py`; it belongs in `scripts/check-design-docs.py`, which the same task already extends.
3. **The probe's trigger.** "Answered with the server's envelope (`OK`, or a stale refusal)": an `OK` carries no error envelope. Write "answered `OK`, or refused with the server's envelope", in `sdks/SPEC.md` line 390, host item 5 and Decision 006-065.
4. **"Shed" is not defined for the host.** It could mean only `RESOURCE_EXHAUSTED` with `ACKNOWLEDGEMENT_SHED`, or any `RESOURCE_EXHAUSTED`, such as a mesh's rate limit without the envelope. Define it in §6.2.
5. **A sustained shed is unmeasured, and the runbook symptom is wrong.**
   - Excluding sheds brings back the ordinary give-up (up to 120 s) for every flagged lease during a sustained shed of the acknowledgement class, and the gate measures only a 10 s burst. Add a 5-minute shed to Task 006-056 and state the cost in the Consequences.
   - The runbook symptom of `SagatAcknowledgementsFailing` (line 846, "waits one receive bound") is wrong for sheds, failovers and quiet hosts.
6. **The `REGISTRY_SILENT` row of §5:**
   - "Within the handler's deadline" is undefined when there is no `timeoutMs`.
   - The row is limited to 1.8 hosts, while request (7) refuses on engines of either build at every minor. Hosts below 1.8 fail the handler and spend the attempt, and the Consequences should say so.
   - The Consequences state the liveness rule twice (lines 251 and 270).
7. **The attempt flag at the cap after a rebuild.** "Losing it only means more flags, which the cap bounds and then sets it again" is false at the cap. A refused fifth withdrawal writes nothing, so after a projection rebuild an attempt at the cap stays flagged until it ends. The failure row's "exists only when ... by the floor build" should add "or after the projection is rebuilt".
8. **The fallback's trigger and scope.** It is triggered by "before #260 merges into `develop`", while the plan is keyed to "before any build of it is released". Requests (1) (the secret fence) and (5) (the cap's derivation) are assigned nowhere under the fallback.
9. **"The one bound flag the marks' rules allow"** (`stream-completion.md` line 468; Decision 006-066, line 893). The flag is not a mark (Decision 006-042). Write "the one bound boolean the static check allows, in the flag's own allowed set".
10. **Lines still overrun the wrap:**
    - decisions 2, 5 and 7 of Architecture decision 0099;
    - `stream-completion.md`, lines 458, 596 and 637;
    - `sdks/SPEC.md`, lines 392 and 393;
    - `tasks.md`, lines 338 and 343.

    Also, the gate's "reviews (none)" in a 15 s failover depends on how often running leases heartbeat. State the time-to-live and the heartbeat cadence, or report it rather than gate on it.

---

## Part 3: Is it complete enough to accept?

Not yet. The remaining blocker to acceptance is Major 1, and it is a planning choice, not a redesign:
- Dropping the fallback, and making a released floor a precondition of Task 006-057 rather than of the first task, resolves it in a few sentences.
- Minor 1 is the matching clean-up.
- Minors 2 to 5 correct normative text and test expectations, and belong in the same pass.
- Minor 6 and the Nits can be handled in the tasks.

The mechanism itself has held for three rounds, and this round's changes did not weaken it: the row lock, the lease's own deadline, the cap, the fenced attempt flag, the probe, and the hand-back's evidence. With Major 1 and Minors 1 to 5 fixed, I would accept it for the owner's decision, with Gap 006-041, Gap 006-042 and Gap 006-048 as the stated risks.

**Files referenced:**
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0100-withdrawn-a-fleet-requirement-may-require-another.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/server.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/execution.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/round-9/egm-design.md
