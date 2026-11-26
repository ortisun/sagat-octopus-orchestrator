# Design critique, round 8: delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)

**Review status: complete.** No target standard was named. I reviewed against the persona's audit vectors, the questions of the request and my round-7 report.

**What I read:**
- the persona and my round-7 report;
- the round-7-to-8 diff in full;
- the current Architecture decision 0099 and the stub of Architecture decision 0100;
- `stream-completion.md`, lines 251 to 809;
- `sdks/SPEC.md` §5, §6.1 to §6.8 and §9;
- `research.md`, Decision 006-040 to Decision 006-066 and Gap 006-040 to Gap 006-048;
- the phase's 25 tasks in `tasks.md`;
- `disaster-recovery.md` sections 5 to 7;
- Architecture decision 0082.

**What I checked in the code:**
- the claim query in `crates/infra-postgres/src/workers.rs`, lines 94 to 168;
- `stopped_run_status` in `crates/infra-postgres/src/scope_compensation.rs`;
- whether the floor build's branch already carries request (7) of Decision 006-059. It does not.

## Verdict

**Count: no Blockers, 2 Majors, 5 Minors, 6 Nits.**

The once-per-attempt bound is the right kind of fix. It costs no statement, needs no observation, and closes every path I could find by which the acknowledgement's failures could cycle a lease to the cap. Removing the breaker, the restore command, its function and table, and the `fsync` condition were real cuts.

Two things are still wrong as written:
- **Major 1.** Removing the breaker dropped half of what it was for. While the server's acknowledgement path fails, new leases still get flagged, so every 1.8 host holds every flagged lease in a slot for its whole give-up period. Nothing ends that automatically.
- **Major 2.** The stop rule of the acknowledgement answers `CANCELLED` for compensation and reconciliation leases. The server leases those deliberately while a run is stopping. The bound hides the defect: such leases are bounced once and are then never protected.

Both fixes are a few sentences and a test each. With them made I would approve. Minors 2 and 3 should go into the same pass, because they correct normative text.

---

## Part 1: The round-7 Majors

### Major 1 (the host's trigger and the cap): resolved

- **The bound (Decision 006-066).** Either withdrawal of a provisional lease records the attempt, and the claim does not mark that attempt again. Whatever fails on the acknowledgement's path, a lease is bounced at most once per attempt, so no withdrawal cycling can reach the cap. That removes the arithmetic the round-7 finding rested on: the 60 s floor, the full-slot case and the short time-to-live cases.
- **The counting rule.** It is the rule I proposed:
  - a call counts when it was given up with no verdict;
  - its attempts met no answer or no envelope;
  - another call was answered during it, or one of its own hand-backs was answered after its give-up;
  - the run is counted since the last `ACKNOWLEDGED`;
  - a call that does not qualify neither counts nor breaks the run;
  - there is no time floor.
- **The texts agree.** `sdks/SPEC.md` §6.2 (lines 382 to 390) and `stream-completion.md` item 5 (lines 658 to 667) say the same thing.
- **The conformance cases are added:** the full-slot case at an 18 s time-to-live, and frames faster than one a second.

There is one residue. The new claim that a failing path "never" spends an attempt is false in a case the design itself documents (Minor 2).

### Major 2 (the breaker): resolved as a finding, but its withdrawal reopens half of its purpose

The breaker's defects are gone with it: the flapping or latching, the blind alert, the minimum volume and the cross-tenant opening.

Decision 006-065 now gives its purpose as "that no failure of the path cycles leases to the cap", and the bound does meet that. But the round-6 and round-7 texts gave it a second purpose: "new leases then flow unflagged at once instead of being handed back". Nothing replaces that, and the cost is now a throughput outage rather than spent attempts (Major 1 below).

### Major 3 (the liveness rule): resolved, with a residue

What is fixed:
- The rule is narrowed to capability calls. Heartbeats, reports, hand-backs, claims and acknowledgements flow.
- The boundary is measured from the start of the last successful beat, at 25 s.
- The gate is checked again at the hand-over.
- The test I asked for is in Task 006-046 (heartbeats served while the registry's connection fails).
- The amendment of Architecture decision 0082 decision 5 is scoped to capability calls.

What remains is Minor 3:
- the status of the refusal is not specified;
- the handlers that need secrets or blobs fail, and spend attempts, across the fleet;
- this build keeps the rule without needing it;
- the Consequences limit it to the floor build.

### The round-7 Minors and Nits

- **Minor 1 (the server's envelope):** resolved. There is an envelope test in Task 006-049, and a failure counter with reasons.
- **Minor 2 (the structure of §6.2):** resolved.
- **Minor 3 (the 2 s rule):** resolved, through `attempt_time_left_ms` and the heartbeat before the handler.
- **Minor 4 (the restore command):** resolved. The restart of PostgreSQL replaces the command, its function and its table.
- **Minor 5 (`fsync`):** resolved. The `synchronous_commit` raise is correctly left to Task 006-044.
- **Minor 6 (the restore cases and the one-server assumption):** resolved.
- **Nits 1, 2, 3 and 5:** resolved.
- **Nit 4 (the drain's bound):** still "the longest earlier deadline". That is correct, just not simplified (Nit 3 below).

---

## Part 2: Findings

### Major 1: Without the breaker, a sustained failure of the acknowledgement path at the server costs every flagged lease a whole give-up period, with no automatic way out

**Location:**
- Decision 006-065 ("Its purpose ... is met by the bound");
- Architecture decision 0099, considered option 8 ("nothing else is needed");
- `stream-completion.md`, the failure row at line 739 ("at most one give-up of delay") and host item 4 (lines 652 to 657);
- `sdks/SPEC.md` §6.2, lines 375 to 380;
- the gate of Task 006-056.

**What is wrong.** The bound limits each attempt to one bounce. It does not stop new attempts from being flagged. While the acknowledgement fails at the server, every new attempt's first lease goes through the same sequence:
1. It is flagged.
2. Its calls are answered with the envelope (shed, deadline, a lock or statement timeout, or a defect), and the host waits them out.
3. It holds a host slot until its give-up. That is the time left at receipt less 6 s, about 24 s at the default 30 s time-to-live and up to 120 s for long leases.
4. It is handed back, which is a run transaction with a journal event.
5. It is claimed again, unflagged.

For tasks of one second, each 1.8 host's throughput falls by roughly 25 times, for as long as the failure lasts. Each task also pays a second claim and a withdrawal on top of its normal work, while the database or the path is already in trouble.

The design gives nothing that ends this automatically:
- the host never declines on the envelope (correctly, to avoid one tenant switching off another's protection);
- the breaker is gone.

The operator has two levers, and both are bad:
- **The switch.** `SAGAT_LEASE_WITHDRAWAL=off` needs a restart of every engine, and it also stops the withdrawals of Architecture decision 0098.
- **Removing the requirement.** That makes the provisional leases in flight lapse into reviews.

The Consequences already admit "no emergency stop".

**Why it matters.** The most likely sustained fault of a new path is a defect or a regression in that path. Examples:
- the triggers of `steps` on the acknowledgement's update growing past the 1 s statement timeout;
- an admission class sized too small on a large engine.

In each case every 1.8 worker of the fleet loses most of its throughput. The failure table calls this "at most one give-up of delay", which is true per lease and hides the outage in aggregate.

**Fix (the smallest that needs no server state):**
- **The host rule.** A lease whose acknowledgement attempts have, for one receive bound since the first, all ended with a transient answer carrying the server's envelope (and no verdict) is given up at once and handed back.
  - With the bound, its next lease runs unflagged immediately, so the delay falls from the give-up period to about 6 s.
  - The rule is per lease, so no host declines and no tenant switches another off.
  - The hand-back load is the same as today, only earlier, and there are fewer repeats.
  - It goes in §6.2 and host item 4, with unit tests in each SDK.
- **A gate scenario.** Add one to Task 006-056: 5 minutes of a statement timeout injected on the acknowledgement alone. Report 1.8 throughput against unflagged 1.8, and the time to recover after the fault ends.
- **Optionally, an emergency stop.** A runtime "no new marks" switch read on the registry beat, which stops new flags within one beat without a restart and without touching Architecture decision 0098's withdrawals. It would close the "no emergency stop" consequence.
- **Correct the record.** In Decision 006-065 and considered option 8, state that the bound replaces the breaker for attempts, and that this host rule replaces it for throughput.

### Major 2: The acknowledgement answers `CANCELLED` for compensation and reconciliation leases, which the server leases deliberately while a run is stopping

**Location:**
- Architecture decision 0099, decision 7;
- `stream-completion.md`, "The server's acknowledgement", step 5 (lines 531 to 536), and the failure row at line 728;
- Decision 006-051;
- Task 006-049 ("a run stopping answered `CANCELLED`");
- `sdks/SPEC.md` §6.4 (a compensation or reconciliation "is never cancelled by the server").

**What is wrong.** The pure function answers `CANCELLED` for a provisional lease "whose step is asked to stop, or whose run has a stop reason", whatever the task's kind. The claim query does not work that way:
- `workers.rs` line 98 claims steps whose kind is not `FORWARD` while `r.stop_reason` is set;
- lines 131 and 153 claim `COMPENSATION` steps regardless of it;
- `scope_compensation.rs` decides "how a run that is being stopped ends when its scope has compensated".

So compensations and reconciliations run exactly while the run has a stop reason. At 1.8 every flagged one goes through the same sequence:
1. It is answered `CANCELLED`.
2. It is handed back and withdrawn as `NOT_STARTED`.
3. It is kept by the settle and leased again (the withdrawal table says so, line 563).
4. Because of the bound, its second lease is unflagged.

The heartbeat path has the opposite rule (§6.4).

**Why it matters:**
- **The protection never applies to them.** Compensations of a cancelled run are the tasks feature 037 protects most, and Gap 006-047 does not list this case.
- **It costs work and alerts.** Every compensation pays a wasted claim, call, hand-back and withdrawal. The `NOT_STARTED` clause of `SagatAcknowledgementsFailing` fires on any bulk cancellation of saga runs (Minor 5).
- **The bound hides it.** Under round 7's rules this cycle would have run to the cap of four, and then spent the compensation's attempt at every attempt.

**Fix:**
- Apply the stop rule as the heartbeat does: `cancel_requested` and the run's stop reason answer `CANCELLED` only for a `FORWARD` task. A compensation or reconciliation lease is acknowledged whatever the run's stop reason. The same applies to `duplicate` and `recovered`.
- Write this in decision 7, in step 5, in Decision 006-051 and in the failure row.
- Add a case to Task 006-049 and Task 006-050, root and scoped: a cancelled run's flagged compensation is acknowledged, run once and never withdrawn, and so is a reconciliation under a stop.
- Add the case to the conformance scenario.

### Minor 1: The bound is also set by the pass's `NOT_ACKNOWLEDGED`, which is the feature's own target event, so its cost lands on the losses the feature exists for

**Location:**
- Architecture decision 0099, decision 5;
- Decision 006-066;
- Gap 006-047;
- the marks table, line 431.

**What is wrong.** The round-7 problem (the acknowledgement's path failing while the host still talks to the server) always ends in a hand-back, which is answered, so it needs only the hand-back to set the column. The pass's `NOT_ACKNOWLEDGED` is a different signal: the frame was lost, the poll answer was lost, or the host died. Setting the bound on it makes the second delivery of every lost lease unprotected.

Losses are often systematic, not independent:
- a gateway whose idle timeout is close to the poll duration drops a fixed share `q` of `PollTasks` answers;
- a half-open stream loses its whole window.

Reviews then fall from about `q⁵` to `q²`.

Where the hand-backs fail too, the host's path is down. Its reports fail as well, so the second, unflagged lease lapses into a review anyway. The pass's setting buys nothing there.

**Fix:**
- Set `provisional_withdrawn_attempt` only on the provisional hand-back (`NOT_STARTED` of a provisional lease). Let the pass's silence keep the cap of four, as Architecture decision 0098 intended ("only silence, four times over").
- Narrow Gap 006-047 to "the next lease after a hand-back".
- State the trade-off: a frame that crashes hosts on decode is delivered up to four times instead of twice.
- If the owner prefers the current rule, write the cost into Gap 006-047 in these terms, with the `q²` arithmetic.

### Minor 2: "No failure of the acknowledgement's path can spend an attempt" is false, and the design's own gap says so

**Location:**
- Architecture decision 0099, Consequences ("no failure of the acknowledgement's path can spend an attempt");
- `stream-completion.md` host item 4 ("never an attempt");
- Decision 006-066 ("no attempt is ever spent");
- contradicted by Gap 006-041 and the failure row at line 722.

**What is wrong.** Two paths spend an attempt:
- **A committed acknowledgement whose answers are all lost.** The lease is acknowledged, so its hand-back is an ordinary failure. This is the canonical swallowing case: a mesh whose per-route timeout is shorter than the call while the database is slow. Each lease of the up to three counted frames before the fallback can spend an attempt. With `maxAttempts` at 1, the step fails.
- **The first heartbeat before the handler fails.** Its hand-back is also of an acknowledged lease.

**Fix:**
- Say "no withdrawal cycling spends an attempt; an acknowledgement that committed but whose every answer was lost spends one (Gap 006-041)".
- Optionally, for the owner: treat a hand-back of an acknowledged lease still `LEASED` (no heartbeat), with no capability served, as a withdrawal, under the same host's-word assumption as decision 12. That closes Gap 006-041, but it reopens the choice of round 1 (Decision 006-045), so it is the owner's call.

### Minor 3: The capability refusal on a silent registry has no status, spends attempts across the fleet, and is kept in this build without a reason

**Location:**
- Decision 006-059 (7);
- `stream-completion.md`, lines 410 to 417;
- Architecture decision 0099, Consequences ("An engine of the floor build whose registry connection fails ...");
- Task 006-046;
- `sdks/SPEC.md` §5 (no row for `ResolveSecrets`, the blob calls or `AppendStepLogs`).

**What is wrong:**
- **No status.** Neither the design nor the floor build's branch says what status the refused call gets.
- **Spent attempts.** `ctx.secrets()` and the blob calls are not in the retry table of §5. A registry connection failure of 25 s therefore fails every handler that needs a capability, as a retryable `ACTIVITY_ERROR` that spends its attempt. Such failures are common: DNS, a pooler's client limit, an expired authentication token for new connections.
- **The texts disagree on scope.** Research and the tasks say this build keeps the rule. The Consequences mention only the floor build.
- **This build does not need it for this design's safety.** Its engines record every capability before serving it, so one silent at a census cannot serve one unrecorded.

**Fix:**
- Specify the status: `UNAVAILABLE` with the envelope and a stable reason (for example `REGISTRY_SILENT`).
- Say whether the SDKs repeat capability calls on it within the handler, and add the row to §5 if they do.
- Either drop the rule from this build, or record why it is kept.
- Correct the Consequences.

### Minor 4: The lifecycle and the value of `provisional_withdrawn_attempt` are not specified, and they conflict with "the marks are lease data"

**Location:**
- `stream-completion.md`, the marks table (line 431), the allowed forms (lines 474 to 478) and line 493;
- Decision 006-042;
- Task 006-044 and Task 006-047.

**What is wrong:**
- **It is not lease data.** The column must outlive the lease. But the other statements "keep them ... or clear them with the lease".
- **Clearing is undefined.** If a completion or a withdrawal under Architecture decision 0098 (`NOT_SENT`, or an unmarked `NOT_STARTED`) clears it "with the lease", a later lease of the same attempt is marked again. That is still bounded by the cap, but it is no longer "once".
- **The value is ambiguous.** It must be the attempt of the withdrawn lease before the withdrawal sets it back by one.
- **Resets are not covered.** A step whose attempt counter is reset (an operator's retry, a reused scoped row) would be unflagged silently by a stale value.

**Fix:**
- Define it as attempt data, written only by the two withdrawals as the attempt before the set-back, and never cleared by a lease write.
- Say that it is ignored once the step's attempt differs, and that its loss is safe because the cap still holds.
- Add the reset case to the matrix of Task 006-047.

### Minor 5: The alerts do not fit the bound

**Location:** `stream-completion.md`, lines 773 to 777.

**What is wrong:**
- **False firings.** `SagatAcknowledgementsFailing` fires on `NOT_STARTED` withdrawals of marked leases above 1%. That includes:
  - hand-backs after `CANCELLED` on a stop (any bulk cancellation);
  - compensations (until Major 2 is fixed);
  - the drains of stopping hosts.

  Operators will learn to ignore it.
- **An alert that can almost never fire.** `SagatLeaseAttemptCapped` for marked leases now needs three or more withdrawals under Architecture decision 0098 of the same attempt first.

**Fix:**
- Label the hand-back withdrawals of a lease with a stop asked or a run stop reason as `stop`, and exclude them from the alert.
- Fold the marked-lease clause of the capped alert into the capped alert of Architecture decision 0098.

### Nits

1. **The first heartbeat before the handler.** Say what "fails" means:
   - its retries of §5 are bounded by the time left;
   - `STALE_ATTEMPT` means drop, not hand back (§6.4).
2. **The split and the count.** After a split (§5, item 4), say whether the two halves count as one call or as two.
3. **The drain's bound** (`stream-completion.md` line 696). It can be "at most the time-to-live plus the pass's period".
4. **The restore cases are not listed alike.** Decision 12 of Architecture decision 0099 and `disaster-recovery.md` line 82 list two cases that do not start a server. Section 7 step 5 lists three (the storage snapshot).
5. **A stale failure row.** The row "The attempt's withdrawals reach the cap" (line 737) says "the claim keeps marking". That now holds only if no provisional withdrawal happened in the attempt.
6. **Line wrap.** Decisions 6 and 13 of Architecture decision 0099 and `sdks/SPEC.md` line 400 overrun the wrap.

---

## Part 3: Answers

**Is it now complete enough to accept?**

Not yet: two Majors remain. Each is a short change with a test:
- Major 1: one host rule and one gate scenario.
- Major 2: a task-kind condition in one pure function.

Minors 2 and 3 correct normative text and belong in the same pass. Minors 1, 4 and 5 and the Nits can go into the tasks. The core has not moved since round 4 (the row lock, the lease's own deadline, the cap in the withdrawal command), and it is right.

**Does the once-per-attempt bound hold up?**

- **As a bound, yes.** I found no path that cycles a lease past one bounce on the acknowledgement's path:
  - `NOT_SENT` correctly does not set it;
  - an unmarked hand-back and a mark that does not count never mark;
  - the race of the hand-back and the pass sets it either way;
  - a floor-build claim keeps the column.
- **What it costs in protection:**
  - the second delivery of an attempt (Minor 1);
  - the next lease after any hand-back, including those made for stops, drains and fallbacks;
  - every compensation and reconciliation of a stopping run (Major 2).
- **What it does not buy:** throughput during the server's trouble (Major 1).

**What should still be cut:**
- this build's copy of the liveness rule (Minor 3);
- the marked-lease clause of `SagatLeaseAttemptCapped` (Minor 5);
- optionally, `worker_leases_provisional_overdue`, as before;
- the `synchronous_commit` raise, if Task 006-044 finds that a crash restart changes the start time.

**What the design adds, counted:**

| Area | The whole design | Since round 7 |
|---|---|---|
| Wire | 1 call, 4 messages, 1 enumeration (4 verdicts), 2 fields on existing messages, 3 stable reasons, 1 withdrawal cause | +`attempt_time_left_ms` |
| Schema | 8 columns on `steps`, 1 on `worker_sessions`, 1 function | +1 column; −1 function (`end_lease_marks()`); −1 table (`platform_operator_actions`) |
| Server | 1 service over a 5-method port, 1 admission class, 2 fence variants, the 3 lease statements, the pass, the hand-back and the capability paths, the call deadline, the witness type, the liveness rule (floor build, kept here) | −the breaker; −the `fsync` condition; +the witness; +the bound |
| Fleet and operations | 1 capability (engines), 1 activation entry with `requires`, 1 runbook step (restart PostgreSQL), 1 drill extension, a 4-step rollback | −1 command with 3 flags; −the runbook command test; −1 rollback step |
| Signals | 8 new counters and 1 extended, 2 histograms, 4 gauges, 7 alerts, 4 SDK hooks, 6 skip reasons | +1 counter; −1 gauge and 1 counter (the breaker); 2 skip reasons swapped |
| Host | about 20 rules | +the heartbeat before the handler, +the attempt-deadline rule, the envelope instead of a list, the counting run; −the floor |
| Documents | 1 architecture decision and 1 retired stub, 27 decisions, 9 gaps, 25 tasks | +1 decision, +1 stub |

**What fails first in production:**
1. **The first cancelled saga run after the activation.** Every flagged compensation is bounced, run unprotected, and counted as a `NOT_STARTED` withdrawal. `SagatAcknowledgementsFailing` fires on any bulk cancellation (Major 2, Minor 5). This one is deterministic.
2. **The first defect or sustained slowness on the acknowledgement path.** Every 1.8 host holds every flagged lease for its give-up, and throughput falls by an order of magnitude until someone restarts every engine with the switch off (Major 1).
3. **A failure of the registry's connection beyond 25 s** (DNS, a connection limit, an authentication token). Every handler that needs secrets or blobs fails and spends its attempt, across the fleet (Minor 3).
4. **A lossy `PollTasks` path.** The second delivery of each lost lease is unprotected, so reviews come at `q²` (Minor 1).
5. **A slow database behind a short mesh timeout.** Acknowledgements commit, their answers are lost, and the hand-backs spend attempts before the fallback (Minor 2).

## Files referenced

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md (decisions 5, 7 and 12, considered option 8, Consequences)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0100-withdrawn-a-fleet-requirement-may-require-another.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0082-the-instance-registry-records-requirements-only-for-the-operator-and-a-refused-process-stays-up-with-a-closed-gate.md (decisions 3 and 5)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md (lines 410 to 417, 431, 474 to 493, 531 to 536, 563, 652 to 667, 696, 722, 728, 737 to 739, 773 to 777)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md (§5 line 280; §6.2 lines 368 to 400; §6.4 lines 474 to 489; §6.7)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md (Decision 006-042, 006-051, 006-059 (7), 006-065 and 006-066 at lines 454, 550, 656, 713 and 727; Gap 006-041 and 006-047 at lines 740 and 756)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md (Task 006-046, 006-047, 006-049, 006-050 and 006-056)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/disaster-recovery.md (line 82; section 7, step 5)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/workers.rs (lines 98, 115, 131 and 153: non-forward and compensation steps claimed under a run stop reason)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/scope_compensation.rs (lines 65 to 73: compensation of a run being stopped)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/round-7/egm-design.md
