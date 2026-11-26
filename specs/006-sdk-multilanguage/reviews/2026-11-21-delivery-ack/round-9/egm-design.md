# Design critique, round 9: delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)

**Review status: complete.** No target standard was named, so I reviewed against the persona's audit vectors, the questions in the request and my round-8 report.

**What I read:**
- the persona and my round-8 report;
- the round-8-to-9 diff in full;
- the current Architecture decision 0099;
- `stream-completion.md`, lines 251 to 734;
- `sdks/SPEC.md` §5, §6.1 to §6.5;
- `research.md`, Decision 006-059 and Decision 006-065 to 006-066, and the gaps;
- `tasks.md`: the preconditions, Task 006-043 to Task 006-046, Task 006-056 to Task 006-059.

**What I checked in the merged base:** whether the requests of Decision 006-059 are actually in it. Four of them are not, which is Major 1 below.

## Verdict

**Count: no Blockers, 1 Major, 5 Minors, 7 Nits.**

The mechanism is now right. The row lock, the lease's own deadline, the cap in the withdrawal command, the flag set once per attempt after a hand-back, and the probe hold up, and so do the four additions you asked about:
- the early give-up;
- the withdrawal of an acknowledged lease that is still `LEASED`;
- stops applied to `FORWARD` tasks only;
- the probe.

What is wrong as written is the ground the design stands on, not the design. The merged base does not contain the floor build the design declares as its hard precondition and rollback floor. That is a planning decision, not a redesign, but it must be made before the owner accepts.

---

## Part 1: The round-8 Majors

### Major 1 (throughput while the acknowledgement fails at the server): partly resolved

**Done:**
- The host rule is in §6.2 (lines 378 to 380), host item 4 (lines 685 to 686) and Decision 006-065.
- The failure row and the Consequences now state the cost.
- Task 006-050 and Task 006-059 test that every lease runs on its second lease.

**Not done:**
- **The gate scenario I asked for is missing.** Task 006-056 has no run that injects a sustained fault on the acknowledgement and reports 1.8 throughput and the time to recover. The cost is still a prediction.
- **Two texts still describe the old behaviour:**
  - considered option 8 still ends "and nothing else is needed", without naming the host rule that replaces the breaker for throughput;
  - host item 5 (lines 690 to 692) still says an answer with the envelope "is waited out; the bound of item 4 limits its cost", which contradicts item 4's "at once" (Minor 4).
- **The rule leaves a blind spot and a load shift.** It does not cover failures without the envelope (Minor 3). It moves load from the acknowledgement class into the lease class during a burst of shedding, and nothing measures that (Minor 5).

**What is left:** for one-second tasks, each flagged lease still holds a slot for about 6 s during the failure. That is about a sevenfold loss per slot instead of twenty-five-fold. It is acceptable with the bound, but it must be measured.

### Major 2 (`CANCELLED` for compensations): resolved

The `FORWARD` condition is in all the places it needs to be:
- decision 7;
- step 5 of the server's acknowledgement (lines 562 to 565);
- Decision 006-051;
- the failure rows;
- Task 006-049 and Task 006-050;
- the conformance list in Task 006-059.

The `stop` exclusion in the alert covers the rest. The step's kind is read in `lock_matched`. It is consistent with §6.4.

### The round-8 Minors and Nits

- **Minor 1:** resolved. The pass's `NOT_ACKNOWLEDGED` no longer sets the flag.
- **Minor 2:** resolved in substance by the acknowledged hand-back withdrawal. Overclaims remain (Nit 3).
- **Minor 3:** partly resolved.
  - The status is specified.
  - The behaviour of the SDKs inside a handler is not (Nit 7).
  - The Consequences still say "an engine of the floor build", although this build keeps the rule.
  - The merged base does not have the rule at all (Major 1).
- **Minor 4:** resolved: attempt data, one allowed form, negative tests, the reset case.
- **Minor 5:** resolved, except that the `stop` label is not defined (Nit 1).
- **Nits:**
  - 1, 2, 4 and 5 are resolved.
  - 3 (the drain's bound) is unchanged; it is correct as written.
  - 6 (line wrap) is still open (Nit 6).

---

## Part 2: Findings

### Major 1: The merged base lacks four of the floor build's hard preconditions, so the stated rollback floor and the "engines only" requirement are false today

**Location:**
- `tasks.md`, "Before the first task", (a), lines 139 to 145;
- Architecture decision 0099:
  - the "Decided by" header ("accepted and merged first");
  - decision 10 ("the floor build's control planes already decode the cause");
  - the Rollback paragraph ("The floor is enforced, not described: the build of Architecture decision 0098 decodes the cause `NOT_ACKNOWLEDGED`");
- Decision 006-059 (2), (6), (7) and (8);
- Task 006-045 ("confirmed ... as the server build added it");
- Task 006-046 ("keeps request (7) ... as the floor build has it").

**What is wrong.** In the merged base:

| Request of Decision 006-059 | What the base has |
|---|---|
| (2) the floor decodes `NOT_ACKNOWLEDGED` | `WithdrawalCause` is `NotSent` and `NotStarted` only (`crates/domain/src/execution/event.rs`, lines 438 to 458); no code in the repository knows the cause |
| (6) the guard's message says "of unmarked leases" | Line 57 of `20270208000000_lease_withdrawal.up.sql` still says "set SAGAT_LEASE_WITHDRAWAL=off on the engines to stop new withdrawals" |
| (7) a silent engine serves no capability call | Nothing implements it; `docs/runbooks/instance-registry.md` line 221 still says "a lost connection leaves the process serving" |
| (8) the requirement read counts only when recorded for both kinds | `lease_withdrawal_required()` in the same migration (lines 71 to 76) returns `FOUND` for any recording |
| (3) the holder's minor clamped to the build's ceiling | Present (`crates/application/src/worker/withdrawal.rs`, lines 229 to 237) |

Also, `ADR-0098` is still `Proposed` (line 3). I did not check request (1).

**Why it matters:**
- **Rollback breaks.** A process that lists `lease-withdrawal-v1` and cannot decode `NOT_ACKNOWLEDGED` is admitted by every requirement this design records. That includes an engine after the rollback's removal of `delivery-acknowledgement-v1`, and any control plane at any time. It then fails to replay every run that holds such a withdrawal.
- **Decision 10 rests on (2).** Recording the requirement "for engines only" is justified solely by the floor's control planes already decoding the cause.
- **Request (7) carries more weight in this round.** Its completeness matters for the capability record, which is now the server's only evidence for an acknowledged lease's hand-back (Minor 2).
- **Requests (6) and (8) may no longer be possible as specified.** They say "changed in its migration before that is applied anywhere". The migration is now on the integration branch.
- **Two tasks would pass falsely.** Task 006-045 would "confirm" what does not exist, and Task 006-046 would "keep" what was never built.

**Fix.** Choose one of these and write it into precondition (a), Decision 006-059 and the header:
- **Preferred: a floor follow-up.** Before any build of the floor is released or deployed, add:
  - the cause and its codec (2);
  - the capability refusal with `REGISTRY_SILENT` (7);
  - for (6) and (8), a new migration if `20270208000000` has been applied anywhere, since an applied migration cannot be edited.

  Say that the rollback floor is the first released build that has all of them.
- **Otherwise, move them into this design.**
  - Task 006-045 adds the cause, and Task 006-046 builds (7).
  - The dormant build of this design becomes the rollback floor. It must be deployed fleet-wide before Task 006-057.
  - The merged floor build cannot be excluded by any recorded requirement. So either record `delivery-acknowledgement-v1` for both kinds and drop the "engines only" argument, or add a removal guard on it while any journal holds the cause.
  - Rewrite the Rollback paragraph to match.

### Minor 1: The exactness argument still says "exactly one of the acknowledgement and a withdrawal commits", which the acknowledged hand-back withdrawal made false

**Location:**
- Architecture decision 0099, the title of decision 4;
- `stream-completion.md`, lines 611 to 621 ("for a hand-back, not acknowledged"; "If the acknowledgement holds the row first, the withdrawal reads `lease_acknowledged_at` set and does nothing");
- Rollback step 2, line 728 ("an acknowledged lease is an attempt either way").

**What is wrong:**
- Decision 4's body, decision 5 and the withdrawals table (line 597) now withdraw a hand-back that comes after an acknowledgement. The paragraph that proves exactness, which a builder reads first, specifies the opposite predicate for the hand-back.
- The rollback text is wrong too: an acknowledged `LEASED` lease's hand-back does change meaning when the requirement goes. It becomes an ordinary failure, which is the safe direction.

**Why it matters:**
- Two normative texts give the hand-back two different decisions.
- More importantly, the safety of an acknowledged-then-withdrawn lease no longer comes from the row lock. It rests only on the host's promise in decision 12, and the core argument should say so.

**Fix:**
- Retitle decision 4: "Exactly one of the acknowledgement and the pass's withdrawal commits".
- In lines 611 to 621, say the following: the row lock orders the acknowledgement against the pass; a hand-back can follow an acknowledgement, and its "nothing ran" rests on the host's promise (decision 12) and the capability record.
- Correct Rollback step 2.
- Have Task 006-051's "the same with the hand-back's withdrawal" assert the new outcome.

### Minor 2: "`LEASED`, no heartbeat" is not evidence that a handler did not start, and a report after such a withdrawal leaves no trace

**Location:**
- Decision 006-045 ("the capability record and the heartbeat are the evidence round 1 lacked");
- `sdks/SPEC.md` §6.2, line 381 ("a lease that has not run (no heartbeat, no capability)");
- the marks table, line 445 (`withdrawn_lease_token_hash` is written only for `NOT_ACKNOWLEDGED`);
- Architecture decision 0099, decision 12's list of what the server sees.

**What is wrong:**
- At 1.8 the first beat comes at the earlier of the interval (never over 10 s) and a third of the time left. At the default time-to-live that is about 8 s.
- Every task shorter than that completes from `LEASED` without ever beating. So for most tasks the step status says nothing, and the capability record is the only evidence.
- Suppose an SDK defect hands back an acknowledged lease whose handler did start. Examples in the new code: an early give-up taken while an attempt was in flight, then the late `ACKNOWLEDGED` also run; or a confusion on the "too little time" path. Then:
  - the step is withdrawn and run again with the same attempt;
  - the journal says nothing ran;
  - the first handler's `CompleteStep` is refused as an ordinary stale report, uncounted, because the token's hash was not kept.

**Why it matters.** The new rule moves a known-outcome claim ("nothing ran") onto six new host implementations. The one cheap detective control the design already has, the stale token after a withdrawal, does not cover it.

**Fix:**
- Correct Decision 006-045 and §6.2: "not yet `RUNNING` and served no capability", not "has not run".
- Write `withdrawn_lease_token_hash` also on the `NOT_STARTED` withdrawal of an acknowledged lease. It is the same update of the same row, at no extra statement.
- Add "a report, heartbeat or capability call carrying the token of an acknowledged lease withdrawn on its hand-back" to decision 12's list. It is counted, audited and alerted like the `NOT_ACKNOWLEDGED` case.
- Add a case to Task 006-050.

### Minor 3: The probe is answered by the fast no-match path, so it cannot see a path that cuts calls by duration, and that case gets neither the early give-up nor the fallback

**Location:**
- `sdks/SPEC.md` §6.2, lines 386 to 388;
- `stream-completion.md`, host item 5 (lines 695 to 698) and the failure row at line 751;
- the runbook symptom of `SagatAcknowledgementsFailing` (line 811).

**What is wrong.**
1. Take a mesh or gateway whose route timeout is shorter than the acknowledgement's duration. That duration is up to 4 s while the database is slow.
2. Every attempt ends with a status without the envelope, so the host waits for the full give-up: about 24 s at the default, up to 120 s.
3. The hand-back is answered `OK`. The probe's token was cleared by the withdrawal, so it is answered `NOT_HELD` at once, with no lock.
4. The call does not count, and the host never falls back.
5. The acknowledgement probably committed. Thanks to the new rule the hand-back is still withdrawn, so no attempt is spent. But the first lease of every new step waits the full give-up.

The runbook symptom says "waits one receive bound", which is wrong for this case.

**Why it matters.** This is the round-8 throughput outage again, with a cause on the host's path that the fallback was built to catch. "A mesh's timeout" is named in the design itself.

**Fix.** Make the early give-up apply to any answered transient failure, with or without the envelope: "every attempt that ended within one receive bound ended with an answer". Wait the full give-up only for silence.
- This is safe: a give-up only hands back, and the hand-back of a `LEASED` lease is withdrawn.
- The envelope then matters only for counting toward a decline, which is simpler.
- Add a conformance case: a proxy that cuts `AcknowledgeTasks` at 1 s while the server takes 2 s. Expect each lease given up after one receive bound, no fallback, and no attempt spent.
- Correct the runbook symptom.

### Minor 4: The early give-up's window is ambiguous, contradicted by item 5, and silent about an attempt in flight

**Location:**
- `sdks/SPEC.md` §6.2, lines 378 to 380 ("during one 6 s receive bound");
- `stream-completion.md` item 4, lines 685 to 686 ("since one receive bound ago"), and item 5, lines 690 to 692 ("is waited out").

**What is wrong:**
- **The window.** Read as a sliding window, "every attempt since one receive bound ago ended with the envelope" is true after the very first answer with the envelope. A lease could then be given up in 200 ms. The two texts word the window differently.
- **Item 5 contradicts item 4:** it still says such answers are waited out.
- **An attempt in flight.** The ordinary give-up never happens while an attempt is in flight. The early one says "at once" and does not say whether it waits.
- **The interaction with `RETRY` answers and with split halves** is not stated.

**Why it matters.** Six SDKs will implement six readings. The in-flight question is exactly the defect Minor 2 worries about.

**Fix.** One sentence in both texts: "when the lease has waited at least one receive bound since its first attempt was sent, and every attempt that ended in that time ended with a transient answer, the host starts no further attempt, waits for one in flight as the ordinary give-up does (and runs a late `ACKNOWLEDGED`), and otherwise gives the lease up". Add:
- a `RETRY` answer restarts the window;
- the halves of a split call share the window.

Then delete "is waited out" from item 5.

### Minor 5: The early give-up turns a medium burst of shedding into hand-backs in the lease class, and nothing measures it

**Location:**
- `stream-completion.md`, "Admission" (lines 332 to 338), and item 4;
- Task 006-056 (the burst run);
- the Consequences ("the second claim and withdrawal ... fall on a server already in trouble").

**What is wrong.**
- Admission sheds the acknowledgement class precisely to protect the lease class, where "a shed heartbeat can cost a review".
- Under round 8's rule a shedding burst of 6 to 24 s healed by itself.
- Now every flagged lease received in such a burst becomes the following work:
  - a hand-back, which is a run transaction with a journal event and a settle;
  - up to eight hand-backs per host in flight at once, in the lease class;
  - a second claim of 38 statements.
- So the extra work lands in the very class the admission rule reserves, while the engine is saturated. The gate checks that no heartbeat is shed "while acknowledgements are admitted". It does not check what happens after they are shed.

**Why it matters.** It is a feedback loop. It is bounded at once per attempt, but it is unmeasured, and its failure mode is reviews of running tasks, not merely delay.

**Fix:**
- Add the scenario I asked for in round 8 to Task 006-056:
  - 5 minutes of a statement timeout injected on the acknowledgement alone;
  - a 10 s burst of `ACKNOWLEDGEMENT_SHED` while heartbeats run.

  Report the hand-backs, the second claims, the lease-class sheds, the heartbeat's 99th percentile, the throughput, and the time to recover.
- State a decision rule in Decision 006-065. If heartbeats are shed in the burst, exclude `RESOURCE_EXHAUSTED` with its hint from the early give-up, keeping it for the deadline, the lock and statement timeouts and the store errors, which signal a sick path rather than a busy engine.

### Nits

1. **The `stop` label is not defined.** The alert excludes withdrawals "labelled `stop`" (line 810), but the counter definitions (lines 764 to 766) give `worker_leases_undelivered_total` no such label or outcome. Define it there.
2. **Probes and forged tokens share a counter.** Every probe is counted as `not_held{no_match}`, the same series as forged tokens. Say so, and keep any alert off that series.
3. **Overclaims remain:**
   - "Enforced by": "no attempt spent by any failure of the acknowledgement path";
   - host item 4: "never an attempt";
   - the Consequences: "never charged an attempt ... while it is `LEASED`".

   Each should be qualified: a mark that counts, under the cap, the requirement recorded. A first heartbeat sent before the handler that commits but whose answer is lost makes the lease `RUNNING`, and its hand-back then spends the attempt.
4. **A run-on sentence.** The entry in `docs/adr/README.md` reads "written on a lease that has not run is withdrawn". A semicolon is missing after "written on".
5. **The status of Architecture decision 0100.** It is now "Superseded by Architecture decision 0099". A record never accepted cannot be superseded; "Withdrawn" was right.
6. **Layout:**
   - stray blank lines in `stream-completion.md` (lines 254 to 258 and 459 to 460);
   - lines that overrun the wrap: lines 640, 670 and 688, `sdks/SPEC.md` line 389, and decision 13 of Architecture decision 0099.
7. **`REGISTRY_SILENT` is incomplete:**
   - §5 has no row for `ResolveSecrets`, the blob calls or `AppendStepLogs`, so "a retry hint" binds no SDK, and a handler that needs a secret still fails and spends its attempt;
   - the Consequences still limit the rule to the floor build.

---

## Part 3: Answers

**Is it complete enough to accept?**

Not until Major 1 is decided. That is a planning decision about where four small floor changes are built and which build is the rollback floor; it is not a redesign. Minors 1, 2 and 4 correct normative text and belong in the same pass. Minors 3 and 5 are a widened host rule and a gate scenario, and they can go into the tasks.

**Are the latest additions right and minimal?**

- **The probe:** right in kind. It overlaps the swallowing failure and tells it apart from an outage that healed, at the cost of one call per candidate call. It is blind to failures that depend on duration (Minor 3).
- **The early give-up:** right. It needs a precise window (Minor 4), coverage of answers without the envelope (Minor 3), and a measurement (Minor 5).
- **The withdrawal of an acknowledged lease that is still `LEASED`:** the right trade, and consistent with the identical hand-back of an unmarked 1.8 lease. But it moves the hand-back's safety from the row lock to the host's word. The text must say so (Minor 1) and keep the evidence for it (Minor 2). The owner's confirmation is correctly flagged.
- **Stops for `FORWARD` tasks only:** right and minimal. Resolved.

**What should still be cut:** very little. The design is near minimal; it now grows mostly in tests.
- `worker_leases_provisional_overdue`, optionally, as before.
- The `synchronous_commit` raise, if Task 006-044 finds that a crash restart changes the start time.
- I checked the cap clause of the attempt flag and would keep it. Without it, a flagged lease at the cap whose acknowledgement fails would be handed back as an ordinary failure, and the acknowledgement's path would spend an attempt.

**What the design adds since round 8:**

| Area | Added or changed | Removed |
|---|---|---|
| Host rules | the early give-up, the probe | — |
| Server | the hand-back predicate (`LEASED`, no capability), the step's kind read in `lock_matched` | — |
| Signals | the skip reason renamed (`flagged_before`) | one alert clause (7 alerts become 6) |
| Checks | one design-document check | — |
| Wire and schema | nothing new | — |

**What fails first in production:**
1. **The first replay of a run that holds `NOT_ACKNOWLEDGED`** by a process of the merged floor build. That is a control plane during the rolling upgrade, or an engine after a rollback. Replay fails for that run. This one is deterministic once any frame is lost (Major 1).
2. **A mesh or gateway route timeout shorter than the acknowledgement**, while the database is slow. Every new step's first lease waits the full give-up, and no host falls back (Minor 3).
3. **A shedding burst of 6 to 24 s on a saturated engine.** It produces a wave of hand-backs in the lease class and heartbeats are shed, which costs reviews of running tasks (Minor 5).
4. **An SDK defect in one of six languages** that hands back an acknowledged lease whose handler started. The step runs again, the journal says nothing ran, and nothing is counted (Minor 2).
5. **A failure of the registry's connection beyond 25 s**, once request (7) exists. Handlers that need secrets or blobs fail and spend attempts, because no SDK repeats on `REGISTRY_SILENT` (Nit 7).

## Files referenced

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0098-a-lease-the-server-never-handed-to-the-transport-is-withdrawn-behind-a-fleet-requirement.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0100-withdrawn-a-fleet-requirement-may-require-another.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/README.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/instance-registry.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/domain/src/execution/event.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker/withdrawal.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/round-8/egm-design.md
