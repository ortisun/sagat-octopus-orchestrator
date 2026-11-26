# Design critique, round 6: delivery acknowledgement (Architecture decision 0099 and Architecture decision 0100, worker protocol 1.8)

**Review status: complete.** No target standard was named, so I reviewed against the persona's audit vectors and my round-5 report.

**What I read:**
- the persona and my round-5 report;
- the whole round-5-to-round-6 diff;
- `stream-completion.md`, lines 251 to 756;
- `sdks/SPEC.md`, lines 355 to 414;
- `disaster-recovery.md` in full;
- the current text of Architecture decision 0099 (decisions 2, 10 and 12, Consequences, Rollback) and Architecture decision 0100;
- `research.md`, Decision 006-062, Decision 006-064 and Gap 006-048;
- the task list of the phase.

**What I checked in the build of Architecture decision 0098:**
- `lease_withdrawal_required()` in `crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql`;
- how `SAGAT_LEASE_WITHDRAWAL` is read (`apps/execution-engine/src/settings.rs`, `docs/runbooks/lease-withdrawal.md`).

## Verdict

The core design has not changed for two rounds, and it is right:
- the row lock decides between the acknowledgement and the withdrawal;
- the deadline is the lease's own;
- the cap of four withdrawals is applied by the withdrawal command;
- the fallback is bounded;
- a mark that might have lost its acknowledgement does not count.

Every answer to round 5 was made in the text, including the runbook step, the list of restores and the durable commit.

Two things remain at the Major level:
- **Major 1.** The protection after a restore still depends on an operator remembering a new runbook step. A value PostgreSQL already provides would close most of Gap 006-048 automatically and make the design smaller. I should have proposed it in round 5.
- **Major 2.** The rollback's wait condition is undefined. Read literally, it never ends.

The new third trigger of the fallback is right in principle, but two details are wrong (Minors 1 and 2). One fix I gave in round 5 was wrong, and the design followed it (Minor 4).

No Blockers. 2 Majors, 6 Minors, 4 Nits. With both Majors fixed I would approve. Major 2 is a text fix; Major 1 replaces one function with a built-in.

---

## Part 1: The round-5 Major

**Major 1 (restores that keep the timeline): resolved.**

Every part of the round-5 fix is in the text:
- **The runbook step.** `control-plane end-lease-marks` is written into `docs/runbooks/disaster-recovery.md` section 7, step 5, after every restore of any kind and before the engines start.
- **The drill.** "To drill your own deployment" now tells the operator to read the timeline before and after their own restore and failover tools.
- **The gap.** Gap 006-048 names the snapshot, the base backup without `recovery.signal`, the logical dump, the managed restore and the logical-replication cut-over.
- **Durable commit.** It is a precondition both ways: the acknowledgement raises `synchronous_commit` to `local`, and the probe refuses to mark while `fsync` is off.
- **Tests.** There is a matrix row for each, and the drill asserts that a provisional mark held at the kill is not withdrawn after the promotion.

Three residues remain. None of them reopens the round-5 Major:
- **The cut-over case.** The failure-mode row says "the runbook's step applies before the engines move", but the runbook only speaks of restores (Minor 6).
- **"Before the engines start".** Nothing enforces it (Minor 6).
- **The operator dependency.** It stays the weakest link of the exactness, and it can be removed (new Major 1).

**Round-5 Minors and Nits:**
- **Minor 1:** resolved. There is one definition, and the parity rows cover a lower, a higher and a null timeline.
- **Minor 2:** resolved in intent: null instead of an error, evaluated once per statement, tested as a role without the right, and alerted. The guard I prescribed does not work in PostgreSQL (new Minor 4).
- **Minor 3:** resolved:
  - the deadline runs from arrival and includes the pool;
  - the answer at the deadline is `UNAVAILABLE` with `ACKNOWLEDGEMENT_DEADLINE`;
  - the refusal list is one closed list in both texts;
  - an exhausted pool is answered within 3.5 s.
- **Minor 4:** resolved:
  - `duplicate` and `recovered` follow the margin and the stop rule, and are `CANCELLED` without them;
  - a lease of another timeline is unmarked for every withdrawal;
  - a hand-back of a lease claimed on another timeline is an ordinary failure. That is the security reviewer's choice, and it is sound, because a capability record of that timeline may have been lost.
- **Nits 1 to 4:** resolved. The rate alert on `worker_registrations_declining_total` is in.

---

## Part 2: Findings

### Major 1: The protection after a restore still rests on an operator step; the server's start time would make it automatic and remove the timeline machinery

**Location:**
- Architecture decision 0099, decision 2 (line 56) and decision 12 (line 139);
- `stream-completion.md`, "The timeline" (lines 418 to 425), "The probe" (lines 435 to 436), "After any restore" (lines 437 to 442), and the failure rows at lines 701 and 702;
- Decision 006-062 (research.md line 650) and Gap 006-048 (line 703);
- `disaster-recovery.md` section 7, step 5 (lines 94 to 98).

**What is wrong.** Every restore that keeps the timeline depends on step 5 being run:
- a snapshot started as it is;
- a base backup started without `recovery.signal`;
- a managed service's restore;
- a cut-over to another cluster.

These are the most common restores, and the runbook offers the snapshot tool itself. Section 7 is read rarely and under pressure, and a missed step gives the worst outcome in the design: a lease whose acknowledgement the restore lost is withdrawn as "nothing ran" while its handler runs.

Every one of those restores starts a new PostgreSQL server process. `pg_postmaster_start_time()` therefore differs from the value stamped on the old server:
- **A promotion.** The standby is another server, with its own start time.
- **A point-in-time recovery.** It is a new start.
- **A snapshot or a base backup without `recovery.signal`.** It is a new start.
- **A managed restore.** It creates a new instance.
- **A logical-replication cut-over.** It is another cluster.
- **A switchover and back.** PostgreSQL cannot demote a primary without a restart, so the old primary comes back with a new start time.

The only restore that keeps both the timeline and the start time is a logical restore (`pg_restore`) into a server that keeps running.

The start time also simplifies the design:
- `pg_postmaster_start_time()` is executable by every role by default and does not fail on a standby. The `LANGUAGE sql` guard, the "role without the right" tests and most of `no_timeline` go away, and with them Minor 4.
- The cost is that a planned PostgreSQL restart ends the protection of the leases in flight. Those leases get today's meaning, the same cost the design already accepts for a planned switchover. Restarts are rare, and during a restart those leases mostly cannot be acknowledged anyway.

**Why it matters:**
- The owner is asked to accept Gap 006-048. Its largest part can be closed at no cost: one value read in the statements that already read the timeline.
- An exactness that depends on a mechanism is better than one that depends on an operator, and this design already made that argument when it rejected the operator fence (considered option 9).

**Fix:**
- Stamp the mark with the server's start time. Rename `lease_claim_timeline` to `lease_claim_incarnation` (a `timestamptz`), or keep the timeline and compare both.
- The mark counts only when the stamp is not null and equals `pg_postmaster_start_time()` of the current server. Everything else stays: one evaluation per statement, "another value or none does not count", and the parity test.
- Keep `control-plane end-lease-marks` for the one remaining case, a logical restore into a running server, and write Gap 006-048 as that case alone.
- On the pinned image, check whether a crash restart that keeps the postmaster running changes the start time. Either answer is safe, because the durable commit already covers a crash, but record the answer.
- Have `infra/dr/drill.sh` assert that the stamp changes for both of its disasters, and for a snapshot started as it is.

### Major 2: The rollback's wait condition is undefined and, read literally, never ends

**Location:**
- `stream-completion.md`, "Rollback" step 2 (lines 651 to 652) and step 4 (line 663);
- the gauges (lines 467 and 714);
- Architecture decision 0099, decision 10 (line 126) and Rollback (line 219);
- Decision 006-055 (research.md line 583);
- Task 006-044 (the `down` migration) and Task 006-066.

**What is wrong.** Step 2 waits until "no mark counts (`worker_leases_marked` is 0)". `worker_leases_marked` is defined nowhere. "A mark counts" is defined in decision 2 for every column of the marks, and both readings fail:
- **If it includes the hand-back reservation, the wait never ends.** The claim writes `lease_hand_back_generation` and the timeline stamp for every lease of a 1.8 session, whatever the switch (decision 2; marks table, line 409). After step 1, every new lease of a 1.8 worker carries a counting reservation, so the count stays above zero for as long as 1.8 workers work.
- **If it includes acknowledged leases, the wait is as long as the longest running 1.8 task.** An acknowledged lease's mark counts while the step is `RUNNING`, its expiry is renewed by heartbeats, and its attempt deadline may be unset. A rollback then waits for hours or days.

The text says the wait "takes the longest earlier deadline in flight plus the period of the lost-lease pass". That holds only for provisional leases. It also says marks end "acknowledged, withdrawn or reported", as if an acknowledgement ended a mark, which contradicts decision 2's *Acknowledged*.

The `down` migration's refusal "while a mark counts" inherits the same ambiguity.

**Why it matters.** Removing the requirement changes the outcome only for **provisional** leases:
- their withdrawal at a deadline;
- the hand-back of a provisional lease.

An acknowledged lease's hand-back is already an ordinary failure, and an unflagged 1.8 lease's hand-back is the documented cost of a removal. An operator who finds that the wait never ends will remove the requirement early (the safe direction, but the drain was the whole point of the new order) or abandon the rollback.

**Fix:**
- Step 2 waits for `worker_leases_provisional` to be 0: the step is `LEASED`, the offer counts, and `lease_acknowledged_at` is null.
- Define each gauge by the macro, in one sentence each.
- State the bound: the longest time-to-live or `timeoutMs` among provisional leases, plus the pass period and its backlog.
- The `down` refuses on the same condition: provisional marks, not every mark.
- Task 006-066's rollback case asserts that the wait ends while a 1.8 worker keeps working and a long acknowledged task is running.

### Minor 1: The third trigger makes the server's own shedding of the acknowledgement a fleet-wide fallback, within one give-up period

**Location:**
- `stream-completion.md`, host item 5 (lines 621 to 632), the failure rows at lines 690 and 691, and admission (lines 323 to 329);
- `sdks/SPEC.md` §6.2, lines 373 to 380;
- Decision 006-048;
- the acceptance row "Fallback" (line 753).

**What is wrong.**
1. **A saturated server does not fail the other calls too.**
   - The text says: "A database outage or a saturated server fails the other calls too and never counts." The design's own admission rule says the opposite: the class `Acknowledge` is admitted below the reserve kept for heartbeats and reports, so under saturation acknowledgements are shed first while heartbeats and streams are answered.
   - The gate requires exactly that ("no heartbeat is shed while acknowledgements are admitted").
   - The same holds for waits that only the acknowledgement takes in that form: a binding revocation holding the binding's advisory lock exclusively, or a long holder of the namespace's secrets lock, end in `lock_timeout` and `UNAVAILABLE` for acknowledgements alone.
   - Under the new rule each of these counts.
2. **"Three in a row" can happen within about 25 s.**
   - Each frame has its own call, and the calls run concurrently.
   - At the default 30 s time-to-live, a lease is given up about 23 s after it is received.
   - Three frames received within a second, whose calls all fail, are three calls given up at about the same moment.
   - A 25-second shed or lock wait affecting acknowledgements alone therefore declines the whole fleet for 8 to 12 minutes, doubling up to an hour if bursts repeat. Each host also registers twice, once at the fallback and again at the end of the period.
   - The stated reason, "waiting it out would hand every flagged lease back until its attempt is spent", needs four withdrawals of one attempt: about four give-up periods, roughly 100 s at the defaults. A fallback after 25 s is about four times earlier than that reason requires.

**Why it matters.**
- Before this round, a shed acknowledgement cost at most one withdrawal (line 329, still in the text). Now it can cost the fleet's protection for up to an hour, and fire two alerts, in exactly the bursts the gate measures.
- The registration waves add load to an overloaded server.
- This is bounded and alerted, so it is a Minor.

**Fix:**
- Count given-up calls only when the first and the third were given up at least four give-up periods apart, or at least 90 s apart. Before that, waiting costs no attempt.
- Replace the sentence about a saturated server with what is true: shedding of the class, and lock waits on the acknowledgement alone, count once they outlast the cap's time.
- Jitter the declining registration by up to one receive bound.
- Add to the gate's burst a shed of the class lasting less than that time: no fallback. Add to the scenario a longer one: fallback, no attempt spent.

### Minor 2: "Answered" is not defined, and the third trigger depends on it

**Location:**
- `stream-completion.md`, line 625 ("was answered during each, whatever status their attempts ended with");
- `sdks/SPEC.md`, lines 375 and 376.

**What is wrong.** "Whatever status their attempts ended with" describes the given-up calls, but nothing says what counts as an answer to the *other* call:
- If an error answer counts, a database outage in which heartbeats come back `UNAVAILABLE` quickly is "answered", and the 30 s outage test should fall back.
- If only success counts, it is unclear what counts for a stream:
  - a frame;
  - a keepalive (an HTTP/2 PING);
  - a stream that is merely open.
- A host whose only other call is an idle stream may never count. Six SDKs will each pick one of these readings.

**Fix.** Define it once in both texts. Another call was answered when, after the given-up call's first attempt was sent and before its give-up, the host received:
- an `OK` answer to a unary call;
- a message on its stream (a protocol keepalive is not one);
- or a `PollTasks` answer that ended normally.

Add a unit test in each SDK for the idle-stream host.

### Minor 3: The rollback leaves the protection of Architecture decision 0098 switched off, and gives no way to confirm step 1

**Location:**
- `stream-completion.md`, "Rollback" steps 1 and 3 (lines 648 to 662);
- Architecture decision 0099, line 219;
- `docs/runbooks/lease-withdrawal.md` on the branch of Architecture decision 0098 ("set `SAGAT_LEASE_WITHDRAWAL=off` on the execution engines and restart them").

**What is wrong:**
- **The switch reaches more than this feature.** It needs a restart of every engine, and it also stops Architecture decision 0098's withdrawals of unmarked leases, which are not what is being rolled back.
- **Nothing turns it back on.** No step says to turn it on again after the binary is back on the floor, so the floor build of Architecture decision 0098 keeps running with its own protection off. It logs this once at start, and each lapse is counted `disabled`.
- **Step 1 cannot be confirmed.** "Once every engine runs with it" names no evidence. An engine that misses the change keeps marking: for example, a replica created by an autoscaler from the old values. Then the wait of Major 2 never ends.

**Fix:**
- Add a last step: set `SAGAT_LEASE_WITHDRAWAL` back to its value before the rollback, on the floor build.
- Name the evidence for step 1: the start log line of every live engine in the registry; `worker_lease_marks_skipped_total{reason="switch_off"}` rising on each engine that claims; and `worker_leases_provisional` falling and not rising again.
- Task 006-066's rollback case asserts that the floor withdraws an unmarked lease after the rollback.

### Minor 4: The guard of `sagat_database_timeline()` does not prevent the error it exists for; my round-5 fix was wrong

**Location:**
- `stream-completion.md`, lines 418 to 425;
- Decision 006-062;
- Task 006-044.

**What is wrong.**
- PostgreSQL checks the execute right on a function when it initialises the expression that calls it, not when the branch is taken. This is `ExecInitFunc`, through `object_aclcheck`.
- A `LANGUAGE sql` body `CASE WHEN has_function_privilege(...) AND NOT pg_is_in_recovery() THEN pg_walfile_name(pg_current_wal_lsn()) END` is inlined or planned as a whole, so a role without the right gets "permission denied for function `pg_walfile_name`" whichever way the condition evaluates.
- The recovery half of the guard does work, because the recovery error is raised at execution.
- I prescribed this form in round 5. The test of Task 006-044 ("a role without the right gets null and every statement still works") would catch it at build time, so it is a Minor.

**Fix.** Either of these works:
- `LANGUAGE plpgsql` with `IF ... THEN RETURN ...; END IF; RETURN NULL;` and no exception block, so no subtransaction. PL/pgSQL plans a statement only when it reaches it.
- `SECURITY DEFINER` with a fixed `search_path`, as `lease_withdrawal_required()` already is in the build of Architecture decision 0098, so the right checked is the owner's.

Under Major 1 the question disappears.

### Minor 5: Architecture decision 0100 is no longer needed; cut it

**Location:**
- Architecture decision 0100 as a whole;
- `stream-completion.md`, lines 384 to 391;
- Decision 006-047;
- Task 006-044 (the trigger) and Task 006-046 (`Capability::requires()`, the static check, the race tests).

**What is wrong.** The decision protects against two things:
- **Recording `delivery-acknowledgement-v1` without `lease-withdrawal-v1`,** which it says would admit a binary that cannot replay the event. It would not. Once `delivery-acknowledgement-v1` is recorded, the registry refuses every process that does not list it, and only this build or a later one lists it. Every such build replays the cause.
- **Removing `lease-withdrawal-v1` while `delivery-acknowledgement-v1` is recorded.** That is safe since this round: every withdrawal and every 1.8 hand-back reads **both** requirements under their locks (Decision 006-064), so with `lease-withdrawal-v1` removed no event is written. The replay-floor guard of Architecture decision 0098 already refuses that removal once a journal holds an event.

The only remaining effect of the wrong order is useless marks: they are written, and then every withdrawal lapses.

**Why it matters.** The decision costs:
- an architecture decision of its own;
- a trigger, and a function body rewritten in every migration that adds a dependency, with its `down`;
- a static check that parses that body;
- two typed refusals;
- race tests with an activation and a removal held open.

All of that buys an ordering rule the design now gets for free.

**Fix:**
- The engine marks only while it knows **both** requirements are recorded. It already reads the requirement set on each registry heartbeat.
- Keep a typed refusal in the activation command, without a trigger, for the operator's convenience. Hand SQL that bypasses it is now harmless.
- Drop Architecture decision 0100, or keep it as a considered option of Architecture decision 0099. Ask the architecture reviewer to confirm. A caveat for that review: the two read functions must check the process kinds the event needs, which is Architecture decision 0098's existing responsibility.

### Minor 6: The runbook step has gaps that Major 1 does not close

**Location:**
- `disaster-recovery.md` section 7, step 5 (lines 94 to 98);
- `stream-completion.md`, lines 437 to 442 and the failure row at line 702.

**What is wrong:**
1. **The cut-over is not in the runbook.** The failure row says the runbook's step applies to a logical-replication cut-over "before the engines move", but section 7 speaks only of "every restore of any kind", and a cut-over is not a restore.
2. **"Before the engines start" is not enforced.** An engine that is still connected can run the pass before the command commits: for example, pods that were not scaled down for an in-place snapshot restore.
3. **"Only where the requirement is recorded" does not say in which database.** It should be the restored database, not the one that was damaged.
4. **The update may scan every row of `steps`.** "Clear them with the lease" (line 455) does not say whether finished steps keep the stamp. If they do, `end_lease_marks()` updates every historical row, inside one "bounded" transaction, during the time allowed for the restore.

**Fix:**
- The command refuses, unless given `--force`, when the registry holds a heartbeat newer than the server's start time.
- Its update is restricted to `status IN ('LEASED', 'RUNNING')`.
- Run it whenever the binary has the command. It is idempotent and does nothing where no mark counts, so the operator has no condition to judge.
- Add the cut-over to section 7.

### Nits

1. Architecture decision 0099, Consequences (line 206), says "two functions (the timeline and the end of the marks)". There are three: `delivery_acknowledgement_required()` too.
2. In `disaster-recovery.md` section 7, step 1 ("after the engines are up it is 64") now comes before step 5 ("before the engines start") and step 6 (which starts them). Move step 1 after step 6, or say it is checked then.
3. "The connection is discarded" (`stream-completion.md`, line 477). The statement timeout is already set to the time left, so the server ends a statement before the call's deadline. Say that the transaction is rolled back and the connection returned, and that a connection is discarded only when a statement or a commit outlives its timeout. Otherwise a slow database also gets a wave of new connections.
4. The `fsync` probe can be up to one registry heartbeat old, and `fsync` can be changed by a reload. Either say so, or read `current_setting('fsync')` as a column of the claim statement, which costs nothing.

---

## Part 3: Answers to the questions

**Is it now the right design, and complete enough to accept?**

The right design, yes. Its mechanisms have been stable since round 4, and each later addition answers a named failure.

Complete enough to accept after Major 2, which is a text fix. Before acceptance I would also make Major 1: it turns the largest part of Gap 006-048 from an operator's duty into a property of the database, and it makes the design smaller. The Minors can be made in the text before acceptance or carried into the tasks.

**Is the fallback's new third trigger right?**

In principle, yes. A failure of this one call alone must end, or every flagged lease is handed back until its attempt is spent; the correctness reviewer was right. Using "other calls answered" to tell that apart from an outage is the right discriminator.

As written, it is wrong in two ways:
- it counts the server's own designed shedding, and lock waits on the acknowledgement alone, within about 25 s, instead of after the roughly 100 s the cap actually needs (Minor 1);
- "answered" is not defined (Minor 2).

With a time floor and the definition, it is right.

**Is the rollback order right?**

Yes:
1. switch marking off;
2. drain;
3. remove the requirement;
4. roll the binary back.

Removing the requirement first would make the marks in flight lapse, which is safe but wastes the protection, and the design now says so.

Two things are wrong around the order:
- the drain waits on an undefined count that, read literally, never reaches zero; it must wait on provisional leases only (Major 2);
- the switch also stops Architecture decision 0098's own withdrawals, and nothing turns it back on after the rollback (Minor 3).

**Is the design now too large? What it adds, counted:**

| Area | The whole design | Since round 5 |
|---|---|---|
| Wire | 1 call, 4 messages, 1 enumeration (4 verdicts), 2 fields, 2 stable reasons (`TASK_NOT_ACKNOWLEDGED`, `ACKNOWLEDGEMENT_DEADLINE`), 1 withdrawal cause | +1 reason |
| Schema | 7 columns on `steps`, 1 on `worker_sessions`, 3 functions, 1 trigger with its function | +1 function (`delivery_acknowledgement_required()`) |
| Server | 1 service over a 5-method port, 1 admission class, 2 fence variants, changes to the 3 lease statements, the pass, the hand-back and 5 capability paths, the probe, the call deadline, 4 audit actions | +4 rules: the pass ends what it does not withdraw; the hand-back gated by the requirement; another timeline's hand-back is an ordinary failure; `CANCELLED` for `duplicate` and `recovered` without the margin |
| Fleet and operations | 1 capability, 1 activation entry, `Capability::requires()`, 2 typed refusals, 2 static checks, 1 command, 1 runbook step, 1 drill extension, a 4-step rollback | +1 command (it was a bare function), +1 architecture decision (split out, no new mechanism) |
| Signals | 6 new counters and 1 extended, 2 histograms, 4 gauges, 7 alerts (14 `promtool` cases), 4 SDK hooks, 6 skip reasons | +2 alerts, +1 skip reason, +3 labels |
| Host | about 16 rules | the third trigger rewritten |
| Documents | 2 architecture decisions, 25 decisions, 9 gaps, 25 tasks (6 of them per SDK) | +1 decision |

Nothing was removed this round.

**What I would cut:**
- **Architecture decision 0100 and its machinery** (Minor 5): a trigger, a function body maintained per migration, a static check that parses it, and the race tests.
- **`sagat_database_timeline()` with its guard, its right and most of `no_timeline`** (Major 1), replaced by a built-in value that every role can read.
- **Optional:** `worker_leases_provisional_overdue`, which `worker_leases_provisional` and the not-acknowledged alert already cover. `withdrawn_lease_token_hash` stays marginal, but it is the only evidence for Gap 006-042, so I would keep it.

Together those cuts remove about one architecture decision, one trigger, one function and one static check. They add one column comparison.

**What fails first in production:**
1. **The first burst that sheds the class `Acknowledge`, or a binding revocation under load.** The fleet falls back within about 25 s, registers twice, and fires two alerts. If bursts repeat, the decline doubles up to an hour (Minor 1).
2. **The first rollback.** The wait on `worker_leases_marked` does not end (Major 2). The operator then removes the requirement early, or finishes the rollback with `SAGAT_LEASE_WITHDRAWAL` still off on the floor (Minor 3).
3. **The first snapshot or managed restore done without step 5.** A lost acknowledgement becomes a false "nothing ran" while the handler runs. This is rare, but it is still the worst outcome in the design, and Major 1 removes it.
4. **A hardened database role.** Every statement that reads the timeline fails, if the guard is built as written and the test of Task 006-044 is skipped (Minor 4).

## Files referenced

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md (decisions 2, 10 and 12; Consequences, line 206; Rollback, line 219)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0100-a-fleet-requirement-may-require-another-and-the-registry-refuses-either-order.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md (lines 251 to 756)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md (lines 355 to 414)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md (Decision 006-055 at line 583, Decision 006-062 at line 650, Decision 006-064 at line 675, Gap 006-048 at line 703)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md (Task 006-043 to Task 006-067)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/disaster-recovery.md (lines 82 to 100)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/round-5/egm-design.md
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql (lines 71 to 85, `lease_withdrawal_required()`)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/docs/runbooks/lease-withdrawal.md (lines 76 and 92)
- /private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/delivery-ack-round5-to-6.diff
