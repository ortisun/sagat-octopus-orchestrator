# Design critique, round 7: delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)

**Review status: complete.** No target standard was named. I reviewed against the persona's audit vectors and my round-6 report.

**What I read:**
- the persona and my round-6 report;
- both diffs in full;
- `stream-completion.md`, lines 251 to 800;
- `sdks/SPEC.md`, lines 266 to 440;
- `disaster-recovery.md` in full;
- the current Architecture decision 0099;
- the decisions and gaps of `research.md` that this round touched;
- the task list of the phase.

**What I checked in the code:**
- `required_since` and the activation function in the build of Architecture decision 0098 (`20270113000000_instance_registry.up.sql`);
- decisions 3, 5 and 8 of Architecture decision 0082;
- the gRPC error mapping in `crates/api-grpc/src/status.rs`.

## Verdict

The core has not changed since round 4, and it is right: the row lock decides, the deadline is the lease's own, and the cap is applied by the withdrawal command.

This round made the design smaller where it was too large:
- Architecture decision 0100 is gone, with its trigger, its static check and its typed refusals.
- The timeline function and its probe are gone.
- The control-plane requirement is gone.
- The server's start time closes most of Gap 006-048 without an operator.

It also added three mechanisms:
- the breaker;
- the liveness rule;
- the server's reasons on its own answers.

Two of them carry this round's Majors, and the host's revised third trigger carries the third.

**Count: no Blockers, 3 Majors, 6 Minors, 5 Nits.** Every Major has a fix in the text. With them made I would approve, and Minors 1 and 2 should be fixed in the same pass, because the six SDK owners implement from that text.

---

## Part 1: The round-6 Majors

### Major 1 (the protection after a restore rested on an operator step): resolved

Every part of the fix is in the text:
- **The stamp.** The mark is stamped with `pg_postmaster_start_time()` (decision 2, "The incarnation"), and a mark of another incarnation, or none, does not count.
- **What was removed.** The timeline function, its guard, the probe and `no_timeline` are gone. Considered option 9 records why.
- **The gap.** Gap 006-048 is narrowed to a logical restore into a running server.
- **The command.** `end-lease-marks` is kept, run after every restore, restricted to `LEASED` and `RUNNING` rows.
- **The drill.** It reads the start time across both disasters and across a restart in the style of a snapshot.
- **The open question.** Task 006-044 records whether a crash restart changes the start time.

Three residues remain. None reopens the Major:
- the claim that every loss of committed transactions starts another server is overstated (Minor 6);
- the command's refusal, which I prescribed, is wrong (Minor 4);
- the new stamp makes the `fsync` condition redundant (Minor 5).

### Major 2 (the rollback's wait condition): resolved

- The drain waits for `worker_leases_provisional` alone, and each gauge is defined by the macro.
- The bound is stated.
- The `down` migration refuses only on provisional marks.
- Task 006-066 asserts that the drain ends while a long acknowledged task runs and a 1.8 worker keeps working.
- A heartbeat on a provisional lease moves the step to `RUNNING`, so nothing can hold the drain open.

### The round-6 Minors and Nits

- **Minor 1:** resolved in its cause. The server's answers carry reasons, there is a 60 s floor, and the declining registration is jittered. The new rule has its own defects (Major 1, Minor 1).
- **Minor 2:** resolved as a definition. The hand-back clause leaves a hole for the first call (Major 1).
- **Minor 3:** resolved. The switch is confirmed, set back after the rollback, and tested.
- **Minor 4:** gone with the timeline.
- **Minor 5:** resolved by withdrawing Architecture decision 0100.
- **Minor 6:** resolved, except for the refusal condition (Minor 4 below).
- **Nits 1 to 4:** resolved.

---

## Part 2: Findings

### Major 1: The host's third trigger does not stay within the cap of four, and "in a row" has two readings with opposite results

**Location:**
- `stream-completion.md`, "The host at 1.8", item 5 (lines 651 to 665);
- `sdks/SPEC.md` §6.2, lines 375 to 379;
- Decision 006-065 (`research.md` line 699);
- the acceptance row "Fallback" (line 800);
- Task 006-058 and Task 006-059.

**What is wrong.** Three separate defects:

1. **The 60 s floor is compared with durations, but the trigger fires only at give-ups.** The text says "60 s is shorter than four give-ups of one attempt at the default time-to-live (about 92 s)". Take one step that the same host reclaims each time. Its give-ups come at about 23, 46, 69 and 92 s:
   - The third give-up is 46 s after the first, so the trigger does not fire there.
   - It fires at the fourth, 92 s, which is the last withdrawal under the cap.
   - The margin is zero, not 32 s.

2. **In the full-slot case the design added, the first call never counts.** A host whose slots are all held by waiting leases has nothing answered during its first call: there are no earlier hand-backs, and an idle stream sends no message. Counting therefore starts at the second give-up:
   - the counted give-ups are 46, 69, 92 and 115 s;
   - the trigger fires at the fifth give-up;
   - that hand-back is the fifth of the attempt, so it is an ordinary failure and the attempt is spent.

   The acceptance row for this exact case ("also with `max_inflight` equal to the frame size") asserts that no attempt is spent.

3. **Short time-to-live leases spend an attempt even when the first call counts.** The design marks leases down to an 18 s earlier deadline:
   - at a 20 s time-to-live, the give-up period is about 14 s, the fifth give-up at about 70 s spends the attempt, and the trigger fires at the sixth;
   - at 18 s, the fifth (about 60 s) spends it.

   Every time-to-live under about 26 s spends one attempt per lease before the fallback.

**"In a row" with "the first and the third".** This can be read two ways:
- **A sliding triple.** On a busy host, give-ups of different frames come milliseconds apart. No three consecutive give-ups then ever span 60 s, so the host never falls back. Every flagged lease goes through four withdrawals and then spends its attempt. That is round 5's correctness Major 2, back again.
- **A run since the last `ACKNOWLEDGED`.** The trigger fires about 60 s after the run began.

Six SDKs will split between these readings. The text also does not say whether a call given up with a server reason, or one answered `NOT_HELD` or `CANCELLED`, breaks the run.

**Why it matters.** This is the normative rule of six SDKs. Its stated invariant (fall back before the cap) is false at the default time-to-live in the design's own full-slot case, and false at every short time-to-live the design marks. With `maxAttempts` at 1, a spent attempt fails the step.

**Fix:**
- Count a call when its own hand-backs, or another call during it, were answered `OK`. A hand-back is answered after its call's give-up, so the first call counts too.
- Define the run as the calls given up since the last `ACKNOWLEDGED`. A call given up with a server reason, or one that ended with any verdict, neither counts nor breaks the run.
- Replace the 60 s floor with one measured in give-ups:
  - **Preferred:** no time floor, firing at the third counted call. The server's reasons now cover the bursts that motivated the floor in round 6.
  - **Alternative:** at least two give-up periods (of the shortest lease involved) between the first and the third.

  Either way the trigger fires by the third withdrawal of an attempt.
- Add conformance cases:
  - the full-slot case at an 18 s and a 30 s time-to-live, asserting no attempt spent;
  - a host that receives frames faster than one a second, asserting that the fallback happens.

### Major 2: The breaker has no rule for the state it creates; it flaps or latches, its alert is blind to flapping, and its minimum volume is on the wrong side

**Location:**
- `stream-completion.md`, "The breaker" (lines 404 to 410), the failure row at line 731, the alert at lines 773 to 777, and the acceptance row "Breaker" (line 791);
- Architecture decision 0099, decision 10 (lines 133 to 148);
- Decision 006-065;
- Task 006-047.

**What is wrong:**

1. **Opening the breaker starves it of its own signal.** While it is open, no new lease is flagged. Once the leases flagged before it opened are decided or given up (one give-up period, about 23 s), the engine answers no acknowledgement calls. "Closes after 60 s below the threshold" with fewer than the 20-call minimum is undefined:
   - **If too few calls counts as below,** the breaker closes after a fixed 60 s whatever the trouble, flags a wave of leases, and reopens within seconds. Under sustained trouble it flaps with a period of roughly 65 to 85 s, and each wave of flagged leases is handed back.
   - **If too few calls counts as no evidence,** the breaker never closes until the engine restarts.

   The claim "the effect ends when the trouble does" holds under neither reading: the breaker cannot see the trouble while it is open.

2. **The alert misses the flapping case.** `SagatDeliveryAcknowledgementInactive` fires on "breaker open for more than 10 minutes". Under the first reading the gauge flaps, so under sustained trouble the operator is never told that the protection is mostly off.

3. **The minimum of 20 calls protects the cheap side.**
   - A false opening costs a minute of leases with today's meaning.
   - A missed opening costs the very cycle the breaker exists to prevent.

   On an engine with fewer than about 20 attempts a minute, the breaker never opens, and since this round the host never declines on a server reason. So a persistent failure of the acknowledgement path alone sends every flagged lease through four withdrawals, then spends its attempt, at five cycles per attempt. Examples of such a failure:
   - the triggers of `steps` running past the 1 s statement timeout;
   - a binding lock held for a long time;
   - a defect that answers "store unavailable".

   The claim that no lease "cycles through hand-backs to its cap" fails here.

4. **One tenant can still switch the protection off.** Admission is per process and shared, so one tenant's flood of `AcknowledgeTasks` sheds every tenant's calls and opens the breaker for all of them. Considered option 8 rejected the round-6 rule because "any tenant [could] switch the protection off for every host on an engine". The chosen breaker does the same, for the length of the flood plus 60 s. That is smaller, but the record should say it.

**Fix.** Write the breaker as an explicit three-state machine:
- **Closed to open:** more than 20% of at least 5 calls ended with a server-made answer, in each of two consecutive 10 s buckets. Short spikes keep the protection, and low-volume engines can still open.
- **Open:** 60 s, doubling on each reopening within 10 minutes, up to 10 minutes.
- **Half-open:** flag a fixed share of claims (10%) for 30 s. Close when their server-made failures stay under 5% with at least one call, stay half-open when there were none, and reopen otherwise.

Also:
- Alert on the share of time open, `avg_over_time(worker_acknowledgement_breaker_open[15m]) > 0.5`, or on more than a few transitions in 15 minutes.
- Unit-test every transition, including zero traffic while open.
- State the cross-tenant effect in considered option 8.

### Major 3: The liveness rule stops heartbeats and reports on a failure of the registry's own connection, which Architecture decision 0082 deliberately left serving

**Location:**
- `stream-completion.md`, "Liveness" (lines 411 to 414) and the failure row at line 738;
- Architecture decision 0099, decision 10 (last sentence) and its "Refines" line;
- Decision 006-059 (7);
- Task 006-046;
- Architecture decision 0082, decisions 5 and 8.

**What is wrong.** The rule says an engine whose registry beat has failed for more than 30 s "serves no worker call until a beat succeeds". That includes `Heartbeat`, `CompleteStep` and `FailStep`, and the floor build adopts it too. Architecture decision 0082 decision 5 chose the opposite for a connection failure: "stopping the fleet would make an outage longer".

Decision 8 gives the registry a connection of its own, so the registry can fail while the request pool keeps working. That happens with the usual partial failures that break new connections and leave established ones working:
- a DNS outage;
- `max_connections` or a pooler's client limit reached;
- an expired authentication token for new connections (the IAM authentication of a managed database);
- a certificate rotation problem.

After 30 s of such a failure:
- every engine refuses heartbeats and reports;
- the lost-lease pass, which is not a worker call, keeps running and expires the leases;
- acknowledged leases become unknown outcomes, and for an `Activity` a review.

All of this happens fleet-wide, to close a narrow race.

**Why it matters.** The race it closes is a floor engine, silent at the census, serving a capability unrecorded before its first beat. That race only needs capability calls blocked. Blocking reports turns a registry hiccup into reviews across the fleet.

**Fix:**
- Narrow the rule to the calls that hand something over: `ResolveSecrets`, `ResolveDatabase`, the blob transfers, `AppendStepLogs`, and optionally the claim.
- Keep serving `Heartbeat`, `CompleteStep`, `FailStep` and `AcknowledgeTasks`. None of them hands anything over, and a floor engine treats a 1.8 hand-back as an ordinary failure.
- State in the "Refined by" pointer for Architecture decision 0082 that decision 5 is otherwise unchanged.
- Add a test in which the registry's connection fails for 60 s while the pool works: heartbeats are served and capabilities are refused.

### Minor 1: The set of the server's reasons is not closed, and the two texts disagree

**Location:**
- `sdks/SPEC.md`, lines 373 and 374 (two reasons);
- `stream-completion.md`, line 652 (two reasons and "the store unavailable") and the server's step 7 (line 537);
- the breaker (line 406).

**What is wrong.** The breaker counts lock and statement timeouts and the store being unavailable as the server's own trouble, but no reason is assigned to them. A lock or statement timeout that fires before the call's deadline is answered `UNAVAILABLE` with no named reason. A host that follows `sdks/SPEC.md` counts it as a failure of its own path. A slow database that trips the 1 s statement timeout on the acknowledgement alone, for longer than the floor, then makes the fleet decline while the breaker is also open: round 6's Minor 1 again, for this case.

**Fix.** Define "the server's answer" by its envelope, not by a list:
- An answer that carries `x-sagat-error` is the server's. As far as I can see, `crates/api-grpc/src/status.rs` already attaches it to every status the platform makes.
- A gateway's 503 has no `x-sagat-error`.

Say this in both texts, keep the named reasons for logs and counters, and test that a statement timeout answered before the deadline is waited out.

### Minor 2: `sdks/SPEC.md` now says that a late `ACKNOWLEDGED` triggers the fallback

**Location:** `sdks/SPEC.md`, lines 379 and 380.

**What is wrong.** The new sentence "When `lease_time_left_ms` less the attempt's duration is under 2 s, the host treats `ACKNOWLEDGED` as `CANCELLED`" was inserted just before "It then waits a random delay of up to 6 s, closes its receive, … registers again … with `declines_acknowledgement = true`". Read literally, one late answer makes the host decline for 10 minutes.

**Fix:**
- Move the 2 s sentence to the verdicts, next to `CANCELLED` (line 368).
- Begin the fallback sentence with "On fallback, the host waits …".

### Minor 3: The 2 s rule spends an attempt for a deadline a heartbeat can move, and does nothing for the one it cannot

**Location:**
- `stream-completion.md`, "The host at 1.8", item 3 (lines 633 to 636), and "Why the deadline is the lease's own", item 3 (lines 604 to 606);
- `sdks/SPEC.md` line 379;
- the scenario in Task 006-058.

**What is wrong:**
- **The expiry.** `lease_time_left_ms` measures the time to the expiry, which a heartbeat renews. Treating `ACKNOWLEDGED` as `CANCELLED` hands back an acknowledged lease, which is an ordinary failure, so the attempt is spent and a step with `maxAttempts` at 1 fails. Sending the first heartbeat at once would usually succeed: the gate requires a heartbeat 99th percentile under 2 s, and most heartbeats take far less.
- **The attempt deadline.** It suffers from the same unbounded `p`, but the host cannot see it, because the answer reports only the expiry. A handler can start with about 0 s of its `timeoutMs` left.
- **When it happens.** Both cases arise together: when the database is slow, many leases at once.

**Fix:**
- For the expiry: when under 2 s, send the first heartbeat before dispatch, run on its success, and hand back only on its failure.
- Report the time left before the attempt deadline as well (or the earlier of the two), and apply `CANCELLED` to that one.

### Minor 4: The refusal of `end-lease-marks` always fires in the one case the command exists for (my round-6 fix was wrong); a restart of PostgreSQL would replace the command

**Location:**
- `stream-completion.md`, "After any restore" (lines 452 to 462);
- `disaster-recovery.md` section 7, step 5;
- Task 006-044 (`platform_operator_actions`) and Task 006-054.

**What is wrong.** The command refuses while any engine's beat is newer than the server's start time. I prescribed that rule in round 6, and it is wrong:
- **A logical restore into a running server.** The server started before the dump, so every restored registry row carries a beat newer than the start time. The command refuses every time, until `--expect-live 0`.
- **A restore that starts another server.** The command never refuses, and it has nothing to do.

So the refusal fires exactly when it should not, and operators will learn to pass the override every time.

**Fix (preferred).** Replace the command with a step:
- After any restore that did not start the server (a logical restore into the same running server, a virtual-machine snapshot reverted with its memory, Minor 6), restart PostgreSQL before the engines start, and check that `pg_postmaster_start_time()` changed.
- This removes `control-plane end-lease-marks`, `end_lease_marks()`, the table `platform_operator_actions` with its guards, two flags, the refusal logic and their tests.

**Fix (if the command is kept).** It reads the beats, waits one liveness window plus one heartbeat interval, and refuses only if a beat moved.

### Minor 5: The `fsync` condition is redundant under the start time; cut it

**Location:**
- Architecture decision 0099, decisions 2 and 12;
- `stream-completion.md`, "Durable commit" (lines 447 to 451), the failure rows at lines 742 and 745, and the alert at line 773;
- Gap 006-048 (the reload window);
- Task 006-044 and Task 006-047.

**What is wrong.**
- **`fsync` off loses data only when the operating system loses it.** With `fsync` off, a flushed commit is still written to the kernel, so it survives a crash of PostgreSQL, a crash restart and a restart of the container. It is lost only on a crash of the operating system or a power loss, and those always start a new postmaster, whose new start time ends every mark.
- **The design therefore carries a condition the stamp already covers.** That is a column read in the claim, the skip reason `no_durability`, an alert clause, a row of the failure table, a gap window and a verification task.
- **The raise of `synchronous_commit` is different.** It is needed only if a crash restart keeps the start time, because the restart discards WAL buffers that were never written. I believe it does keep it, since `PgStartTime` is set once, in `PostmasterMain`. Task 006-044 will settle it.

**Fix:**
- Cut the `fsync` condition and everything listed with it.
- Make Task 006-044's recorded answer decide the raise: if a crash restart changes the start time, cut the raise too.

### Minor 6: "Every event that can lose committed transactions and keep running starts another server" is overstated, and one assumption is unstated

**Location:**
- Architecture decision 0099, decision 12 (lines 158 to 163), and Consequences;
- `stream-completion.md`, "The incarnation" (lines 433 to 442);
- Gap 006-048 (line 734);
- `disaster-recovery.md`, "To drill your own deployment" and section 7, step 5.

**What is wrong.** These cases are missing:
- **Snapshots that keep the running process.** A virtual-machine snapshot reverted with its memory, or a storage snapshot rolled back under a running server and followed by a crash restart, both lose commits and keep the start time. Operators of virtual machines on their own premises do use the first as a quick restore.
- **Several writable servers.** Bidirectional replication, a distributed PostgreSQL, or a pooler that spreads writes give each node its own start time. Marks then silently stop counting across nodes, and the row-lock argument of decision 4 already fails there.
- **The gauges.** A middleware that sends plain reads to a standby (the load balancing of pgpool) would compute `worker_leases_provisional` with the standby's start time and end the rollback's drain at once.

**Fix:**
- Add the snapshot-revert cases to Gap 006-048 and to the runbook step. They are covered by the restart of Minor 4.
- State "one writable server" as an assumption of decision 12.
- Note that the gauges must be read on the primary.

### Nits

1. **A clock comparison remains.** "The incarnation" says it "involves no comparison of clocks", but the hand-back's rule `lease_claimed_at > required_since` is one. `required_since` is `clock_timestamp()` inside the activation function. A backward step of the database clock, between a claim and a re-activation, can make a lease claimed before the re-activation count. Either say so, or compare the requirement epoch, which the engine knows from its heartbeat and can bind on the claim.
2. **Gap 006-048 is broader than the danger.** "A logical restore into a server that keeps running" is only dangerous into the same server the dump came from, without a restart since. A dump restored into another server is already covered.
3. **The gate's criteria name no run.** The gate ("fewer than 0.1% of calls answered other than `OK`") does not say which run it applies to. The burst run sheds acknowledgements by design ("no heartbeat is shed while acknowledgements are admitted"), so the criterion would fail there by construction. Name the run each criterion applies to.
4. **The drain's bound can be shorter.** "The longest earlier deadline among them (a time-to-live or a `timeoutMs`)" is at most the engines' time-to-live, because a provisional lease is never renewed. Say "at most the time-to-live plus the pass's period".
5. **Sheds have no reason in the counter.** `worker_task_acknowledgements_total` has `deadline` but no `shed`, while the breaker counts both. Give the breaker's input one counter, with both reasons.

---

## Part 3: Answers to the questions

**Is it now complete enough to accept?**

After Majors 1 to 3, yes. Each is a change to the text:
- a counting rule;
- the breaker's states;
- a narrower set of refused calls.

Minors 1 and 2 belong in the same pass, because they are normative host text. The rest can go into the tasks. The design's mechanisms are stable, and this round's subtractions were real.

**Is the breaker right?**

- **The principle is right.** The server's own trouble should stop new flags at the engine, not make hosts decline.
- **The thresholds.** The 20% ratio is reasonable. The minimum of 20 calls is on the wrong side, and a 60 s window with no rule for the state the breaker creates is not a breaker.
- **What it costs.** Little in compute. In protection it costs exactly when frames are most likely lost (overload), so it should open on sustained trouble, not on a spike.
- **How it fails.** It flaps or latches, and its alert misses the flapping. One tenant can trip it for every tenant on the engine. All of this is Major 2.

**Is the host's rule now right?**

The structure is right:
- the server's reasons are waited out;
- "answered" is defined;
- the hand-backs count;
- the declining registration is jittered.

Three things are wrong:
- the floor (Major 1);
- the closure of the set of reasons (Minor 1);
- the 2 s rule (Minor 3).

There is also the misplaced sentence in `sdks/SPEC.md` (Minor 2).

**Does `pg_postmaster_start_time()` hold up?**

| Case | Holds? |
|---|---|
| Promotion, synchronous or asynchronous | Yes |
| Point-in-time recovery | Yes |
| A snapshot or base backup started as a server | Yes |
| A managed restore (new instance, or a restore in place that restarts) | Yes |
| Managed high-availability failover (a block-replicated standby started at failover, a reader promoted) | Yes |
| The return of an old primary | Yes |
| A blue/green or logical cut-over | Yes |
| A crash restart that keeps the postmaster | Most likely the value is unchanged, and the durable commit covers it |
| Planned restarts, minor-version patches, maintenance windows, computes that scale to zero | The value changes, in the safe direction, at the cost of the protection in flight |
| Poolers (a bouncer, or a proxy with failover) | Yes: the function runs on the backend, and the backend changes with a failover |
| A logical restore into the same running server | No: the runbook step covers it |
| A virtual-machine snapshot reverted with memory, or storage rolled back under a running server | No: add to the gap (Minor 6) |
| Several writable servers | No: state the assumption (Minor 6) |

**What the design adds, counted:**

| Area | The whole design | Since round 6 |
|---|---|---|
| Wire | 1 call, 4 messages, 1 enumeration (4 verdicts), 2 fields, 3 stable reasons, 1 withdrawal cause | +1 reason (`ACKNOWLEDGEMENT_SHED`) |
| Schema | 7 columns on `steps`, 1 on `worker_sessions`, 2 functions, 1 append-only table with guards | −1 function (the timeline), −1 trigger and its function, +1 table |
| Server | 1 service over a 5-method port; 1 admission class; 2 fence variants; the 3 lease statements, the pass, the hand-back and the capability paths; the call deadline; the breaker; the liveness rule (also in the floor build) | −the probe; +the breaker; +the liveness rule; +4 rules (claimed after the recording, the `NOT_SENT` exception, the acknowledged hand-back in two fences, the durable capability record) |
| Fleet and operations | 1 capability (engines only), 1 activation entry with a refusal, 1 command with 3 flags, 1 runbook step, 1 drill extension, a 5-step rollback, 1 runbook test | −Architecture decision 0100 with `Capability::requires()`, 2 typed refusals, a static check and the trigger's race tests; −the control-plane requirement; +2 flags; +1 rollback step |
| Signals | 8 new counters and 1 extended, 2 histograms, 5 gauges, 7 alerts, 4 SDK hooks, 6 skip reasons | +1 gauge, +1 counter, 1 skip reason swapped |
| Host | about 19 rules | +the 2 s rule, the split after the deadline, the statuses not repeated, the waiting out of server reasons, the floor, the jitter |
| Documents | 1 architecture decision, 26 decisions, 9 gaps, 25 tasks | −1 architecture decision, +1 decision |

**What I would still cut:**
- the `fsync` condition, its skip reason, alert clause, failure row and gap window (Minor 5);
- `end-lease-marks`, `end_lease_marks()` and `platform_operator_actions`, replaced by "restart PostgreSQL after a restore that did not start it" (Minor 4);
- if a crash restart turns out to change the start time, the `synchronous_commit` raise as well;
- optionally, `worker_leases_provisional_overdue`, as in round 6.

Net: one table, one function, one command and one skip reason fewer.

**What fails first in production:**
1. **A partial connection failure.** A DNS failure, a connection limit or an expired authentication token breaks the registry's reconnect while the request pool works. After 30 s every engine refuses heartbeats and reports, and leases expire into unknown outcomes and reviews across the fleet (Major 3).
2. **The first sustained overload.** The breaker flaps, the protection is mostly off, and no alert fires; each reclose flags a wave that is handed back (Major 2).
3. **The first gateway or mesh that answers the new method without a reason** (a per-route timeout under 6 s, or a method missing from an allow-list):
   - busy hosts never fall back under the sliding reading;
   - hosts with full slots spend attempts (Major 1);
   - in the same minute of a slow database, the 2 s rule spends more attempts (Minor 3).
4. **The first logical restore.** The command refuses, and the operator learns `--expect-live 0` (Minor 4).

## Files referenced

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md (decisions 2, 10 and 12, lines 56, 133 to 148, 152 to 163; considered options 8 and 9; Consequences)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0082-the-instance-registry-records-requirements-only-for-the-operator-and-a-refused-process-stays-up-with-a-closed-gate.md (decisions 3, 5 and 8)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md (lines 404 to 414, 433 to 462, 537, 604 to 606, 633 to 665, 679 to 704, 731 to 745, 773 to 777, 791, 800)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md (lines 280, 360 to 383)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md (Decision 006-059 at line 632, Decision 006-062 at line 660, Decision 006-065 at line 699, Gap 006-048 at line 734)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md (Task 006-044 at line 169, Task 006-046 at line 188, Task 006-047 at line 195, Task 006-054 at line 280, Task 006-058 at line 317, Task 006-066 at line 367)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/disaster-recovery.md (line 82; section 7, step 5, line 92)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/status.rs (`x-sagat-error` on every status, lines 8 to 45)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql (`required_since = clock_timestamp()`, line 344; the census, lines 333 to 337)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/round-6/egm-design.md
