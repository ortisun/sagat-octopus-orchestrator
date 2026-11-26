# Design critique: delivery acknowledgement of worker tasks (ADR-0099, worker protocol 1.8)

Review status: complete. I read the persona, the full diff, ADR-0098, ADR-0029, ADR-0015, ADR-0082, the constitution, `sdks/SPEC.md` §6.1 to §6.8, §9 and §10, research decisions D006-001 to D006-010, the performance report the design cites, the conformance scenarios, and the code paths it relies on (`PgStore::begin`, `WorkerService::heartbeat`, `verify`, `ensure_bound`, the lost-lease pass, the claim's lease statement, the admission classes).

## Verdict

The design takes the right general approach. An application-level acknowledgement before the handler is the only exact way to close a frame lost after the transport has taken it. A provisional mark on the lease is better than a new step status, and the exactness argument holds as far as it goes. Five things stop it from being ready to approve:

- It can loop forever through withdrawal and re-claim, with nothing to bound it.
- Its central cost claim, that the acknowledgement "loads no run state", contradicts the very function it says to reuse.
- It rejects a cheaper option on an argument that is incomplete.
- Its rollback does not work as written.
- Its test plan cannot produce the evidence it claims.

The deadline choice (the lease expiry) is sound, but the derivation behind the 7-second threshold is mostly decoration. The cost is understated, and the evidence of frequency is misattributed.

---

## Blockers

### Blocker 1: Nothing bounds repeated withdrawal and re-claim, and a common deployment fault turns it into a hot loop

**Location:**
- `stream-completion.md` lines 415 to 420 (host item 4, "Any other status: run nothing, log once, hand every lease of the call back").
- ADR-0099 decision 4 (lines 60 to 66).
- `research.md` D006-006 (line 247), which ADR-0099 inherits without revisiting.

**What is wrong.** A withdrawal consumes no attempt, and D006-006 puts no cap on withdrawals of one step. D006-006's reasoning was that withdrawals "repeat only with new polling, which the pace of the receive loop bounds". It also said to revisit the cap "if a cause other than stopping or a window deadline shows in it". `NOT_ACKNOWLEDGED` is exactly such a cause, and the 1.8 host rules create loops that the pace of polling drives rather than bounds:

- **Systematic refusal.** `AcknowledgeTasks` is answered `UNIMPLEMENTED`, `PERMISSION_DENIED` or `NOT_FOUND`. Realistic causes are a gateway or service mesh that allows gRPC methods by name, an engine replica rolled back to an earlier build (Major 4), or a misconfigured proxy. The host hands every lease back with `TASK_NOT_STARTED`. ADR-0098 withdraws it (`NOT_STARTED`), the step is `READY` again, and the same host, whose slots are free, claims it again on its next poll. Each cycle costs:
  - a claim (38 statements, 2 transactions, 5 rows);
  - an acknowledgement call;
  - a `FailStep`;
  - two journal events.

  The run never progresses or terminates, and its journal grows without limit.
- **Host clock skew.** A host whose clock runs ahead of the database by more than the time-to-live sees every lease as already expired. It sends no acknowledgement, or abandons it at once, and hands the lease back: the same loop.
- **Saturation.** The acknowledgement keeps failing with retryable errors until expiry, for example because the `Lease` admission reserve is full (Major 5). The lease is withdrawn at expiry and claimed again. The loop runs at the pace of the time-to-live instead of the poll.

The alert `SagatLeasesNotAcknowledged` counts only the cause `not_acknowledged`, so the first loop, which shows up under `NOT_STARTED`, raises no alert. The host "logs once".

**Why it matters.** A rare, bounded failure (one false review per frame lost on the wire) becomes an unbounded, silent consumption of database writes and journal growth. Its triggers are ordinary operations: a gateway rule, a rollback.

**Fix:**
- Bound consecutive withdrawals per step. For example, keep a counter on the step row, cleared by `step.started`. After a small number of withdrawals in a row, give the step a backoff on `next_attempt_at` or park it with a stable reason, and alert on any step over the bound.
- Treat `UNIMPLEMENTED`, and every other non-retryable status of `AcknowledgeTasks`, as a fault of the session, the way ADR-0029 decision 6 treats `UNIMPLEMENTED` on `StreamTasks`: stop polling, register again and negotiate again, and do not keep claiming.
- Add both cases to the conformance scenario `delivery-acknowledgement`.
- Revise D006-006 explicitly.

### Blocker 2: The acknowledgement cannot "open the run as Heartbeat does" and also "not load the run's state"

**Location:**
- ADR-0099 decision 3 (lines 51 to 59).
- `stream-completion.md` lines 327 to 339.
- `research.md` D006-014.
- ADR-0099 considered option 3 (lines 119 to 124), the "strict superset" claim.
- `tasks.md` T006-049 and T006-054.

**What is wrong.** `begin(tenant, run, Lock::Wait)` (`crates/infra-postgres/src/execution.rs` lines 1563 to 1577, then `begin_row` lines 282 to 431) does much more than take a lock. It:
- locks the run row;
- reads every step row of the run;
- loads the pinned definition;
- loads sagas, collections, loops, scope sagas and the continuation chain;
- validates scoped rows;
- builds the run's state.

The checks the design reuses depend on that state:
- `verify` uses `tx.info()`;
- `ensure_activity_bound` uses `tx.parts()` and the definition to find the activity name (`crates/application/src/worker.rs` lines 2742 to 2769);
- `ensure_bound` then makes further queries for bound patterns and the worker's authorization.

The only parts of `Heartbeat` the acknowledgement actually skips are `engine.run` (the transition) and the journal append.

**Why it matters.** The claim "loads no run state" is a MUST in decision 3, and the build cannot meet it with the mechanism the same decision prescribes. It is also the main reason given for preferring option 1 over variant 2. The real cost of an acknowledgement grows with the size of the run, as the cost of a heartbeat does. For a run with a large collection, every acknowledgement reads every step row of that run.

The budget assertion "`ACKNOWLEDGEMENT` has fewer statements than `FIRST_HEARTBEAT`" will pass trivially. It hides the fact that the saving is the transition and the append, not the load.

**Fix.** Choose one of these:
- Specify a lock-only opening of the run: the chain lock and the run row `FOR UPDATE`, with no state restore, plus a step-row query that returns the activity name or node kind needed for the binding check. Change the checks to work without `tx.parts()`.
- Or remove the "loads no run state" MUST, rewrite the comparison with variant 2 and the cost paragraph accordingly, and pin the budget line to what is actually loaded.

Either way, the decision should also be re-weighed against option 5 (Major 1).

---

## Major findings

### Major 1: Option 5 (an acknowledgement without the run lock) is rejected on an incomplete argument

**Location:** ADR-0099 considered option 5 (lines 130 to 133); `research.md` D006-011 and D006-015.

**What is wrong.** The rejection says "a write that does not hold the run lock would let a withdrawal overwrite an acknowledgement it did not see". That holds only if the withdrawal decides from an earlier read and then writes without a condition.

Exactness can come from the step row's own lock instead:
- The withdrawal's step write is conditional, and the condition includes `lease_acknowledged_at IS NULL`, the issued token hash and generation, and `lease_expires_at <= now()`.
- The transaction rolls back, journal event included, when the write touches no row.

Under PostgreSQL's read-committed re-check of updated rows, exactly one of the two takes effect. ADR-0098's withdrawal already has to be fenced on token and generation, so most of the condition exists already.

The engine's general step update (`execution.rs` lines 7036 to 7079) keeps lease columns unchanged with `ELSE` branches while the step stays `LEASED` or `RUNNING`, so it would not overwrite an acknowledgement made without the run lock.

**Why it matters.** Option 5 is one statement per call, over up to 64 rows. It takes no run lock, so it does not contend with completions on busy runs, does not block one run behind another (Major 5) and loads no state (Blocker 2). The design currently pays for the run lock on every task without showing that it needs it.

**Fix.** Weigh option 5 properly. Its real costs are:
- the binding check, which needs the activity name from the step row or an indexed join, or is dropped at acknowledgement because every capability call and report re-checks it;
- the audit of a foreign tenant on a miss;
- a conditional withdrawal write.

Then either adopt it or record why the run lock is still worth its cost.

### Major 2: The cost is understated, and the throughput has no acceptance criterion

**Location:**
- ADR-0099 Context (lines 20 to 24 and 33 to 36), considered option 1 (lines 114 to 118), Consequences, Bad (lines 147 to 154).
- `stream-completion.md` Acceptance matrix, "Throughput" (line 477).
- `tasks.md` T006-064.

**What is wrong:**
- **"Per frame" is mostly "per task".** The saving is stated as "one call per frame and r transactions" for n leases over r runs. But a stream delivers frames as work appears, so a frame under ordinary load carries one or a few leases, and the leases of one frame mostly belong to different runs (r is about n), except in a fan-out. In practice the cost is close to one call and one run-locked transaction per task, as variant 2 would be. The difference is only the transition and the journal append.
- **The extra round trip lands on the measured bottleneck.** The design's own source (`specs/038-claim-transition-efficiency/research.md`, finding 1) says throughput follows round trips. The design adds a round trip before every handler and does not estimate the effect.
- **The figure for A comes from a favourable setup.** The 0.103 seconds is from `tests/performance/reports/scalability-ha.md` lines 265 to 296. That run used workers inside the same process, PostgreSQL on the same machine, no network, runs of three chained activities with no fan-out, and a machine already loaded (load average 26). The report itself lists the network as "Not measured".
- **No decision rule.** T006-064 measures 1.8 against 1.7 but sets no threshold. Nothing says what regression is acceptable, or what happens if 1.8 costs, say, 30 percent of the transitions per second.

**Why it matters.** The owner approved the feature, not an arbitrary throughput cost. Constitution "Targets are not promises" requires the environment to be stated, and Governance requires an ADR that diverges to carry benchmark data.

**Fix:**
- Estimate the cost per task honestly from the frame-size distribution of `tests/performance/reports/sdk-streaming.md`.
- Before the release, state a regression the release will accept, with the environment, on a topology with a network between workers and services.
- Add a fan-out case: one run with many parallel items acknowledged by several hosts.

### Major 3: The frequency evidence counts cases this design does not address

**Location:** ADR-0099 considered option 4 (lines 125 to 129); `stream-completion.md` line 393 ("three observed cases since feature 037: two CI jobs and one play of feature 022").

**What is wrong.** Those three cases are the evidence of ADR-0098, whose Context says that "the server knows, in each of those cases, that the frame never left the process". They are closed by ADR-0098's `NOT_SENT` and `NOT_STARTED` withdrawals. No case has been observed of the residue this design closes: a frame lost after the transport took it, or a lost answer to `PollTasks` (gap G006-001).

**Why it matters.** The trade-off is a new protocol minor in six languages, an extra call before every handler, two columns, an operator command and a migration, against a failure class with zero observed occurrences. The owner may still want it. But the ADR must not present evidence of a different failure as the frequency of this one (constitution principle VIII).

**Fix:**
- Say plainly that no case of G006-001 has been observed.
- Name the counter that will measure it: `worker_leases_undelivered_total{outcome="lapsed"}` once ADR-0098 is active.
- Consider recording a check on that measurement before the work in the six languages, or report the absence of evidence to the owner with the cost.

### Major 4: The rollback does not work as described

**Location:** ADR-0099 "Rollback" (lines 159 to 164); `research.md` G006-008; `tasks.md` T006-045.

**What is wrong.**

(a) **The journal blocks binary rollback.** Once a single `step.lease_withdrawn` with cause `NOT_ACKNOWLEDGED` is written, every run whose journal holds it cannot be replayed by the ADR-0098 build. T006-045 even tests that the older codec refuses it. Journals are permanent, so "remove the requirement, then roll the binary back" fails for those runs.

(b) **Live 1.8 sessions break after rollback.** Sessions accepted at 1.8 "keep acknowledging until they register again", and a host registers again only on `UNAUTHENTICATED` or a restart. After the binary is rolled back, those hosts call `AcknowledgeTasks` on a server that answers `UNIMPLEMENTED`. They then hand every lease back, which is Blocker 1's loop. On a server older than ADR-0098, each hand-back is an ordinary retryable failure that consumes attempts and fails steps for good.

**Fix:**
- ADR-0098 is not built yet; `lease_withdrawn` does not exist in `crates/`. Make the `cause` of `step.lease_withdrawn` an open set in ADR-0098's codec now: an unknown cause decodes and replays exactly as a known one, since every cause has the same effect on state. That makes this design's journal change compatible with the ADR-0098 build and removes the replay reason for the second requirement. It might also allow one requirement, not two with refusals in both directions.
- Make the rollback procedure force sessions accepted at 1.8 to register again before the binary changes.
- Make the host treat `UNIMPLEMENTED` on `AcknowledgeTasks` as a reason to register again (Blocker 1).

### Major 5: One acknowledgement call serializes up to 64 run transactions inside the small `Lease` reserve

**Location:**
- ADR-0099 decision 3.
- `stream-completion.md` lines 281 to 287 and 327 to 339.
- `crates/api-grpc/src/limits.rs` lines 548 to 556 and the test at lines 706 to 751, where the reserve above discovery is two places.

**What is wrong:**
- The runs of a call are handled one after another, each with `Lock::Wait`, and the answer comes back only when all of them are done. One busy run in a frame delays the handlers of every other lease in that frame. A failure in the middle loses the verdicts of the runs already committed, so the host repeats the whole call.
- The call is placed in admission class `Lease`, the reserve that keeps heartbeats and completions alive. Its volume equals the volume of polls, so in a burst the acknowledgements compete with the calls the reserve exists to protect.
- When acknowledgements are shed, the host backs off and healthy deliveries move toward withdrawal. This is the load amplified under overload that decision 5 says it avoids.

**Fix:**
- Bound the total time of a call.
- Process the runs concurrently with a small limit, or return partial results with a per-task verdict to repeat, instead of failing the whole call.
- Either give acknowledgements a reserve of their own or show by measurement that the `Lease` reserve holds under a burst.

### Major 6: Scoped instances (collection items) are not specified

**Location:**
- ADR-0099 decisions 2 and 3.
- `stream-completion.md` "The provisional mark" and "The server's acknowledgement".
- `tasks.md` T006-047 and T006-049.

**What is wrong.** `Heartbeat` has two paths. The root path goes through `open` and `begin`. Scoped instances go through `heartbeat_path`, which uses `begin_current_path` and `ensure_path_bound` (`crates/application/src/worker/scoped.rs` lines 11 to 41 and 93 to 123).

The lease is also written by more than one statement: `execution.rs` around lines 7036 and 7379, and `execution/path.rs` around line 1232. The lost-lease pass covers scoped steps too (`maintenance.rs` line 373).

The design names only `begin`, `verify`, `lease_matches_instance` and `ensure_bound`, and "the statement that already writes the lease", in the singular.

**Why it matters.** Collections are the workload with the most leases per run. That is where run-lock contention and the cost of loading state are largest, and the design is silent about it.

**Fix.** Specify the acknowledgement, the marks and the withdrawal for the scoped path. List every statement that writes or clears a lease. Add a collection case to the race tests and to the budget.

### Major 7: The test plan cannot produce the evidence it claims

**Location:**
- `research.md` D006-024.
- `stream-completion.md` Acceptance matrix, "Six SDKs" (line 475).
- `tasks.md` T006-063.

**What is wrong:**
- **The existing scenarios do not run at 1.8.** "Every other scenario passes at 1.8 with its existing golden, unblessed" is offered as proof that the normal path writes nothing new. But every existing scenario pins its workers to `protocolVersion` 1.4 or 1.5 (`tests/contract/sdk-conformance/scenarios/*.json`), so none of them runs at 1.8 unless its input changes.
- **Some cannot run at 1.8 at all.** `worker-transport-faults`, `worker-stream-faults` and `protocol-negotiation` use `leaseTtlSeconds: 3`, and the server refuses to offer 1.8 below 7 seconds.
- **Missing cases.** The new scenario covers no answer to `AcknowledgeTasks` of `UNIMPLEMENTED` or another non-retryable status, no host clock skew, and no frame of several runs where one run is slow.

**Fix:**
- Specify how the existing scenarios are run at 1.8: an override of the worker's minor by the harness, normalized out of the golden, with time-to-live at least 7 seconds, and the list of scenarios excluded and why.
- Add the missing failure cases to `delivery-acknowledgement`.

### Major 8: The host's give-up rule depends on its wall clock

**Location:** `stream-completion.md` lines 415 to 420; `sdks/SPEC.md` §6.2 lines 362 to 365.

**What is wrong.** The host repeats the acknowledgement "until the leases have expired by the host's clock", comparing its wall clock with `expires_at` in server time. `sdks/SPEC.md` §6.4 (lines 403 to 407) already admits that clocks differ by up to the time-to-live, which is why the heartbeat interval is clamped and counted on the monotonic clock.

- A host whose clock runs ahead never acknowledges, which feeds Blocker 1.
- A host whose clock runs behind keeps repeating after the server has withdrawn the lease. That is harmless, but wasteful.

**Fix.** Derive the give-up time from the monotonic clock at receipt, using the time left as the heartbeat rule does and clamped the same way, never from a comparison of wall clocks.

---

## Minor findings

### Minor 1: The 7-second threshold and the "derivation" are guessed, not derived

**Location:** `stream-completion.md` lines 371 to 400; ADR-0099 decision 5 (lines 67 to 74); `research.md` D006-016.

**What is wrong.** Choosing the expiry as the deadline is principled and needs no derivation: safety does not depend on it. But the interval and threshold arithmetic mixes kinds of number and leaves terms out:
- R is a worst-case bound, while A is a 99th percentile from a different operation on a single machine (Major 2).
- It leaves out the host's time from reading the frame to sending the acknowledgement: parsing up to 8 MiB of input, garbage collection pauses in Java, C# and Python.
- It leaves out the network round trip, admission queueing, the wait for the run lock, and at least one retry with the backoff of §5.
- ⌈A⌉ rounds to whole seconds only because the setting is in whole seconds.

At a time-to-live of 7 seconds, the margin of 0.897 seconds must absorb everything left out. At that setting the threshold is a guess presented as a derivation.

**Fix.** Measure the delay from the host reading the frame to the commit, end to end on a networked topology, including one retry. Set the floor from a high percentile of that measurement, or simply state a round floor, for example 10 seconds, as a choice. Keep the expiry as the deadline.

### Minor 2: The cancellation path sets the acknowledgement mark and so creates a new path to a review

**Location:** ADR-0099 decision 8 (lines 84 to 87); `stream-completion.md` line 435.

**What is wrong.** `ACKNOWLEDGED` with `cancel_requested = true` writes `lease_acknowledged_at`. If the host's `TASK_NOT_STARTED` hand-back is then lost, the acknowledged lease lapses into an unknown outcome, and a run being cancelled gets a review for work that never ran. This gap is not named among G006-004 to G006-008.

**Fix.** When `cancel_requested` is true, answer a distinct verdict (for example `CANCELLED`) and leave the lease provisional. The hand-back then only speeds recovery, and its loss ends in withdrawal at the expiry.

### Minor 3: The attempt-deadline shift is described inaccurately, and its meaning depends on the host

**Location:** `research.md` D006-019; ADR-0099 decision 6 (lines 75 to 78).

**What is wrong:**
- "Delivery is no longer counted, at most R" is wrong. The shift is `now() - lease_offered_at`, which can reach the whole time-to-live, because the acknowledgement is retried until expiry.
- The meaning of a public option, `timeoutMs`, now depends on the protocol minor of whichever host the task lands on. A mixed fleet gives one workflow version two meanings.

**Fix.** Correct the bound. State in the specification of `timeoutMs` that it differs by minor during a mixed rollout. Make sure the owner's report carries this.

### Minor 4: The time-to-live is checked at negotiation, but the claiming engine's value is the one that applies

**Location:** `stream-completion.md` lines 293 to 297; `tasks.md` T006-048.

**What is wrong.** `protocol_ceiling` checks the negotiating engine's `lease_ttl`. The lease's expiry, however, uses the claiming engine's setting (`worker.rs` line 1613), and engines can be configured differently.

**Fix.** Either require the time-to-live to be the same across the fleet, or also refuse to mark a lease provisional at claim time when the claiming engine's time-to-live is below the floor.

### Minor 5: Prerequisites on the language libraries are missing

**Location:** `tasks.md` "Before the first task" (line 138).

**What is wrong.** The six libraries are at 1.5; for example `SagatSdk.PROTOCOL_VERSION = "1.5"` in Java. Minors are cumulative, so a library may offer 1.8 only after it speaks 1.6 and 1.7. Design 041 schedules that work as tasks T041-15 to T041-20. The precondition lists only the server task T041-7.

**Fix.** Add the library tasks of design 041 as prerequisites of T006-057 to T006-062, and state the risk to the 0.3.0 schedule.

### Minor 6: The withdrawal's classification should be specified on values read under the lock

**Location:** `stream-completion.md` lines 347 to 361; `crates/scheduler/src/passes.rs` lines 476 to 485.

**What is wrong.** "Its `EXPIRED` arm also returns the two marks" puts the marks in the candidate scan, which runs outside the lock. `Condition::LeaseLost` re-reads only the deadlines.

Safety holds today only because the acknowledgement also renews the expiry, and the design does not say so.

**Fix.** Re-read the marks under the lock in `lease_deadlines_instance`. Put `lease_acknowledged_at IS NULL` in the fenced write of the withdrawal. State the dependence on the renewal.

### Minor 7: The ADR records a review that has not happened

**Location:** ADR-0099 line 4.

**What is wrong.** It says the reviewers reviewed it "on 2026-11-21" and points to `specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/`. The date is in the future, and that directory does not exist.

**Fix.** Record only what has happened (constitution principle VIII).

### Minor 8: The alerts miss the failures that matter

**Location:** `stream-completion.md` lines 456 to 459.

**What is wrong:**
- `SagatLeasesNotAcknowledged` requires withdrawals "in every 5-minute window for 15 minutes". A trickle of one lease every five minutes fires it, while a burst of thousands in two minutes does not.
- It ignores `NOT_STARTED` loops (Blocker 1).
- `SagatLeaseAcknowledgementSlow` fires at a 99th percentile of 12 seconds at the default setting, when the healthy delay is milliseconds. By then most of the margin is already gone.

**Fix.** Alert on a rate of withdrawals, on any step withdrawn more than a set number of times, and on an acknowledgement delay well above the measured baseline.

### Minor 9: The design does not say that it also closes gap G006-002 for 1.8 sessions

**Location:** `research.md` gaps; `stream-completion.md` lines 236 to 238.

**What is wrong.** A process that crashes between a claim and its `NOT_SENT` withdrawal leaves a provisional lease, and this design withdraws it at expiry. So gap G006-002 is closed at 1.8, which the documents do not say. For 1.8 sessions, ADR-0098's `NOT_SENT` machinery then only speeds recovery.

**Fix.** Say so, and note it when ADR-0098's complexity is reviewed.

---

## Nits

- **Nit 1.** "`lease_offered_at` set to the commit time" (ADR-0099 decision 2; `stream-completion.md` line 316). The statement can only write the time of the change, which `clock_timestamp()` takes after the lock (`execution.rs` lines 311 to 316), not the commit time. The bound R still holds, but the wording should say "the time the claim wrote the lease".
- **Nit 2.** The counter label `duplicate` ("the mark was already set") leaves undefined how an acknowledgement of a lease that is not provisional is counted.
- **Nit 3.** "Exactly one of the acknowledgement and the withdrawal commits" (`stream-completion.md` line 363) holds only while the requirement is known to be recorded. Otherwise neither withdraws and the lease lapses. Qualify the sentence.
- **Nit 4.** `sdks/SPEC.md` §6.2 lines 351 and 352 still say that leases beyond `max_tasks` are left "to lapse". This contradicts the paragraph that follows, and ADR-0098, which hand them back.
- **Nit 5.** The refusal of `ReadBlob` on a provisional lease means an input blob is fetched only after the acknowledgement: a second serial round trip for tasks with blob inputs. Mention it in the cost.

---

## Answers to the questions asked

- **Is this the right design?** The family is right: a provisional mark on the lease, an application acknowledgement, and the expiry as the deadline. Rejecting variant 1 (a new status) is fair. The case against variant 2 is weaker than written (Blocker 2). The run-lock acknowledgement is preferred over option 5 on an incomplete argument (Major 1). The more robust form is a predicate-guarded acknowledgement without the run lock, a bounded withdrawal loop, and a forward-compatible cause in ADR-0098.
- **Is the problem solved?** Yes for hosts that keep the protocol, except for named gaps G006-004, G006-005 and G006-007, plus one unnamed gap (Minor 2). It also quietly closes G006-002 (Minor 9). The fix creates a new failure that is worse than the original (Blocker 1).
- **Is the exactness argument sound?** Yes, with the run lock and the fences as specified, and no clock enters it. It should state its dependence on the renewal and on re-reading the marks under the lock (Minor 6). It would hold equally with a row lock and a conditional write (Major 1).
- **Is the deadline derived?** The choice of the expiry is reasoned and safe. The threshold of 7 seconds and the alert threshold are guessed (Minor 1).
- **Is the cost honest?** No. It is close to one call and one run-locked transaction per task, not per frame. That transaction loads the run's state, and the throughput has no acceptance threshold (Blocker 2, Major 2). The evidence of frequency is misattributed (Major 3).
- **Over-engineered or under-specified?**
  - Over-engineered: two fleet requirements with refusals in both directions, which a forward-compatible cause would simplify (Major 4); a run lock the design may not need (Major 1).
  - Under-specified: scoped instances (Major 6), the host's clock (Major 8), the rollback of sessions (Major 4), and partial results in a call (Major 5).
- **What is hard to build, test or operate?**
  - Building: a lock-only path that opens the run without loading its state, across both the root and scoped paths.
  - Testing: running the existing conformance scenarios at 1.8 (Major 7), and `dropFrame` and `hold` hooks on server streams in six gRPC libraries.
  - Operating: rollback, which as written is unsafe once the first `NOT_ACKNOWLEDGED` is written.
- **What fails first in production?**
  1. A gateway or proxy that allows gRPC methods by name, or a replica that was rolled back, answers `AcknowledgeTasks` with `UNIMPLEMENTED`. The result is a fleet-wide hot loop of claim, hand-back and withdrawal, raising no alert (Blocker 1, Major 4).
  2. Next, acknowledgements on large collection runs wait on the run lock behind completions while loading the whole run. This slows every handler in the frame and pushes healthy deliveries toward withdrawal under load (Blocker 2, Major 5).
  3. Then hosts with skewed clocks (Major 8).

Files referenced:
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0098-a-lease-the-server-never-handed-to-the-transport-is-withdrawn-behind-a-fleet-requirement.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/execution.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker/scoped.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/maintenance.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/scheduler/src/passes.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/limits.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/tests/performance/reports/sdk-streaming.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/tests/contract/sdk-conformance/scenarios/
