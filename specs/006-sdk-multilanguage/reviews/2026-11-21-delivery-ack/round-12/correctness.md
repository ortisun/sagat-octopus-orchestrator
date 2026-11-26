# Correctness review of Architecture decision 0099 (delivery acknowledgement, worker protocol 1.8), round 12

## Verdict

Both round-11 Majors are resolved.
- **Old Major 1 (the early give-up at the end of a failover):** an attempt sent after the evidence now decides the give-up. That attempt is answered in the scenario I gave.
- **Old Major 2 (the floor of an earlier release of #260):** `lease-withdrawal-v2` makes the floor the registry's, whatever the order of releases.

The new decisive attempt has an edge of its own. It is sent at the first sign of recovery. That is the moment when the acknowledgement's own path is most likely to be the one that has not recovered yet, and this round moved that path onto a channel of its own (Major 1).

**Counts:** 0 Blockers, 1 Major, 4 Minors, 6 Nits.

Nothing was built or run. Claims about the code come from reading `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack`.

## Status of the round-11 findings

### Major 1 from my report (the early give-up fired at the end of a failover): resolved

The give-up now needs an attempt sent after the evidence, and that attempt must also fail. This is written in all of these places:
- `sdks/SPEC.md:382-390`;
- item 4 (`stream-completion.md:752-765`);
- Decision 006-065 (`research.md:901` and following);
- the failover row;
- Task 006-058 and Task 006-059.

In my scenario, lease L is in backoff at the promotion and a frame arrives at P + 0.2 s. L now sends an attempt at once, which is answered `ACKNOWLEDGED` (`recovered`). The 30 s case now holds one lease in backoff across the promotion, so the case no longer depends on timing (but see Nit 5).

What is left: the decisive attempt can reach a path that has not recovered yet (new Major 1).

### Major 2 from my report (a release of #260 without the cause would be admitted): resolved

Checked against the code:
- **What the guard needs:** registration refuses a missing required capability (`instance_register`, `20270113000000_instance_registry.up.sql:199-204`). A beat that finds the epoch moved closes the gate of a process that lacks a requirement (`crates/application/src/instances.rs:678-692`). A row that has gone is registered again with the full check (`:659-676`).
- **Why that is enough:** a build of #260 without the follow-up lists only `lease-withdrawal-v1` (`instances.rs:125-141`). Once `lease-withdrawal-v2` is recorded for both kinds, that build is refused at registration, or closed at its next beat. The replay-floor guard (`20270208000000_lease_withdrawal.up.sql:52-65`) keeps `lease-withdrawal-v2` recorded once a journal holds the event, provided request (9) extends the guard to it.
- **Lapsed rows:** the second refusal of the `delivery-acknowledgement-v1` entry covers a lapsed row of such a build.
- **Remaining edges:** Nit 4 and Nit 6.

### Minors from my report

| Round 11 | Status | Reason |
|---|---|---|
| Minor 1: the dormant tasks that need the follow-up | Partly resolved | The tasks that wait are now named (`tasks.md:143-146`). Task 006-044 is listed as proceeding, but its evidence needs the follow-up (Minor 3). |
| Minor 2: the 18 s case | Resolved | It expects either branch of Gap 006-047 and reports which. Gap 006-047 now describes both branches (`research.md:983` and following). |
| Minor 3: the node key | Resolved | The key is now `mark_withdrawn_attempt(&instance, generation)` by `ExecutionInstanceId`. This matches how a scoped row is identified (`execution.rs:7089-7090`). A mark that is never consumed fails the commit, and the collection-item test runs through both scoped statements. I found no path that legitimately skips the projection call of a withdrawn instance. |
| Minor 4: the attempt deadline cannot be built | Resolved | Field 24 `attempt_deadline_epoch_ms` is "the claim's time plus timeoutMs". That is what the grant stores (`attempt_deadline_at = $14 + $21`, `execution.rs:7073-7077`), and the stored expiry is unchanged (`:7066-7068`). The `REGISTRY_SILENT` row now reads `attempt_time_left_ms`. |
| Minor 5: the heartbeat kind | Resolved | `heartbeat_after_hand_back` is used only after `NOT_STARTED` and is not alerted. `heartbeat` is used after `NOT_ACKNOWLEDGED` and is alerted. Task 006-050 and Task 006-053 test both. |
| Minor 6: the quiet host | Resolved | The text now says "no other call and no frame", and the row is pinned (slots full or no work queued). |
| Minor 7: an attempt the transport never started | Resolved | The SDK's own permit, on a channel of its own, is a rule every SDK can build. The new channel opens a failure mode of its own (Minor 4). |

### Nits from my report

- Resolved: 1, 2, 5, 6 and 7.
- Nit 3 (loose terms): part 1 is resolved ("failed try" and "since the evidence" are defined), but the new definition is ambiguous about acknowledgement attempts (Minor 2). Part 2 (a frame counts by its arrival) is not addressed (Nit 2).
- Nit 4 (timing): fixed in `tasks.md` and in the Fallback row's envelope case. Three places still say "about 6 s" or "after one receive bound" (Nit 1).

### Other round-11 reports, in my domain

| Finding | Status |
|---|---|
| Security Major 1, architecture Major 1, design Major 1 (the floor) | Resolved by request (9), as above. |
| Architecture Major 2 (tasks proceeding that need the follow-up) | Resolved except Task 006-044 (Minor 3). |
| Security Minor 1 (the heartbeat exemption) | Resolved. |
| Security Minor 2 (a beat with no epoch hid committed changes) | Resolved. The new function returns the committed epoch by a plain read, as the base's beat does (`instance_registry.up.sql:269`), and the requirements read is a plain read too (`crates/infra-postgres/src/instances.rs:351-357`). A committed change therefore closes the gate of a process that lacks it, whatever an owner transaction holds. |
| Design Minor 1 (failover window, "failed call", frame evidence) | Resolved. The residue is recorded in Gap 006-047. The frame-arrival part is Nit 2. |
| Design Minor 2 (the 18 s case) | Resolved. |
| Design Minor 3, architecture Minor 5 (a call the transport never started) | Resolved (see Minor 4). |
| Design Minor 4, architecture Minor 4 (the reserve's switch) | Resolved: it is keyed by the compiled minor. The record still says "from the activation" (Nit 3). |
| Architecture Minor 1 (the port key) | Resolved. |
| Architecture Minor 3 (the follow-up's series names) | Resolved. |

## Blocker

None.

## Major

**Major 1. The decisive attempt is sent "at once" on the first answer after a recovery. It can reach an engine, or a pooled connection, that has not recovered yet. Any failure with the server's envelope then gives the lease up, and after a promotion that hand-back is an ordinary failure that spends the attempt.**

Confidence: plausible. The documents confirm that the rule counts every failure carried in the envelope; whether an engine lags depends on the deployment and on how the pool recovers.

**Where:**
- `sdks/SPEC.md:385-390` ("sends the lease's next attempt at once … the first answer after it sends every waiting lease one more attempt, which is answered");
- `stream-completion.md:755-763`;
- Decision 006-065 (`research.md:916-919`, "never to a hand-back of a lease that was waiting in its backoff");
- the failover row;
- Gap 006-047, which does not list this case.

**Scenario:** two engines E1 and E2 behind a TCP load balancer, a 30 s time-to-live, and a failover from F to the promotion P = F + 15 s.
1. **The channels:** under this round's change, host H sends `AcknowledgeTasks` on a channel of its own. That is a second TCP connection, which the balancer can place on E2 while H's main channel and stream are on E1.
2. **The outage:** flagged lease L is received at F − 50 ms, and its attempts fail during the outage.
3. **The evidence:** at P + 0.1 s E1 has reconnected. A heartbeat sent at P + 0.15 s is answered `OK` at P + 0.2 s. Or E1's stream delivers a frame.
4. **The decisive attempt:** sent at P + 0.2 s, it goes to E2. E2's idle pooled connections still point at the old primary, and sqlx's ping-before-acquire on a dead connection whose host is gone (no reset) hangs. The acknowledgement's acquisition is bounded by its 3 s deadline (`stream-completion.md:575-578`), so it answers `UNAVAILABLE` `ACKNOWLEDGEMENT_DEADLINE` at about P + 3.2 s.
5. **The conditions all hold:**
   - no verdict, and no shed;
   - more than 6 s since the first attempt;
   - no other try failed since the evidence, because the heartbeats on E1 succeed.
6. **The cost:** L is given up and handed back through E1. The mark belongs to the old incarnation, so the hand-back is an ordinary failure, and with `maxAttempts = 1` the step fails for good.
7. **The alternative:** without the decisive attempt, the next attempt after the backoff (about 2.5 to 5 s later) would very likely find E2 recovered and be answered `recovered`.

The same thing happens on one engine when the heartbeat happens to get a freshly opened connection while the acknowledgement gets a stale idle one.

**Why the tests miss it:** the 30 s conformance case runs against one platform whose pool recovers all at once. The evidence frame and the acknowledgement go to the same engine, which recovers at the same instant.

**Smallest fix:**
- **The server:** let it say when an attempt did not reach the database. Give an envelope reason of its own (for example `ACKNOWLEDGEMENT_STORE_UNREACHABLE`) to a deadline that passed before a connection was held, and to a failed connection. Keep `ACKNOWLEDGEMENT_DEADLINE` for a deadline that passed after the transaction began.
- **The host:** a decisive attempt that ends with that reason does not decide. The host goes on with the backoff and waits for new evidence, as it does after a shed.
- **What this keeps:** it matches the design's own stance that "a busy engine waits for the ordinary give-up", since a full pool gives the same reason. It keeps the proxy-cut and statement-timeout rows, because those carry no envelope or the other reason.
- **The test:** two engines, with the acknowledgement channel on the engine whose database connection is held unreachable behind a proxy, while the other engine answers heartbeats. Expected: no give-up; after the hold is released, `ACKNOWLEDGED` (`recovered`); no attempt spent.
- **Or, if this is accepted instead:** record it in Gap 006-047 and remove "which is answered" and "never to a hand-back" from the three texts.

## Minor

**Minor 1. The attempt sent "at once, outside the backoff" also fires for a lease that can no longer be given up early, and it ignores the shed's retry hint. On a host with other traffic, a burst of sheds becomes a tight retry loop.**

Confidence: confirmed in the documents.

**Where:**
- `sdks/SPEC.md:383-386` and `stream-completion.md:753-756`: the evidence sends the next attempt at once after any attempt that ended with no verdict, a shed included.
- Against them, item 4 (`stream-completion.md:744`): "`RESOURCE_EXHAUSTED` after its hint", with the backoff of §5.

**Scenario:**
1. A host has 50 running tasks, heartbeating at most every 10 s, which gives about 5 answers `OK` per second.
2. A frame's call is shed at time t. A heartbeat sent after that is answered at t + 0.2 s. This is evidence, so the next attempt is sent at once. It is shed again, and the next `OK` sends it again.
3. A lease that has seen a shed can never be given up early (the condition "no attempt … ended in a shed"). So none of these attempts can lead to a give-up. They only add load: up to 4 calls in flight per host, repeating at the round-trip time, where the backoff would allow one every 2.5 to 5 s.

**Why the tests miss it:** the case "a 10 s burst of sheds is waited out" has no other traffic, so no evidence arrives during it.

**Fix:**
- Send the immediate attempt only when a give-up could follow it: no shed in the lease's history, and never before a retry hint has elapsed. Otherwise keep the §5 backoff.
- Add the SDK unit case: a shed burst with a task heartbeating every 500 ms. The number of acknowledgement attempts stays within the backoff schedule.

**Minor 2. "No try of a call to the database failed since the evidence" does not say whether acknowledgement attempts count. Read literally, the decisive attempt rules itself out.**

Confidence: confirmed in the documents.

**Where:** `sdks/SPEC.md:387-388` and `stream-completion.md:759-761`.

**The two readings:**
- **The decisive attempt itself:** it is a call that reaches the database. When it ends with `ACKNOWLEDGEMENT_DEADLINE` or a mesh's cut, it is "a try that ended without the database's answer". On that reading the condition is never true, and the early give-up never fires. The envelope row and the proxy row would then fail on a build that follows the text.
- **Other frames' attempts:** on a host with two frames in flight, the attempts of the other frame's call fail in the same way. That blocks the give-up for every lease whenever more than one call is in flight.
- **Result:** six SDKs can read this differently. The conformance rows would catch the difference only for the readings that they exercise.

**Fix:**
- Write "no try of another of the host's calls that reaches the database (a heartbeat, a report, a hand-back; never an `AcknowledgeTasks`) failed since the evidence".
- Say whether a stream or `PollTasks` receive that breaks is such a try.

**Minor 3. Task 006-044 is listed as proceeding before the follow-up, but its evidence needs `lease-withdrawal-v2` in the closed lists. Also, the follow-up and Task 006-044 rewrite the same three constraints.**

Confidence: confirmed in the code.

**Where:** `tasks.md:145` (Task 006-044 proceeds) and `tasks.md:186-187` (evidence: "the function's result for each recording (both kinds, engines only, none)").

**Code:**
- `platform_requirements_capability_known`, and the two other constraints, list exactly five capabilities (`20270208000000_lease_withdrawal.up.sql:12-29`).
- Recording `lease-withdrawal-v2` fails that constraint until request (9) adds it ("joins the closed lists of known capabilities", Decision 006-059).
- Both migrations replace these constraints by `DROP` and `ADD` with their own list. Whichever runs later must list both values, or it drops the other's value. If the follow-up's list lacks `delivery-acknowledgement-v1`, that is invisible in dormancy, and Task 006-057's recording then fails.

**Fix (either):**
- Move Task 006-044 to the list that waits.
- Or state that its migration is ordered after the follow-up's and lists both values.

Also say that the cases of Task 006-049 resting on marks and withdrawals (`withdrawn`, `unmarked`, `recovered`) use rows written by fixtures, since Task 006-047 and Task 006-050 wait.

**Minor 4. The acknowledgement's own channel adds a failure mode that the shared connection did not have. With no keepalive rule, a half-open acknowledgement connection makes the host decline.**

Confidence: plausible.

**Where:** `sdks/SPEC.md:365-367`, `stream-completion.md:586-588` and item 5.

**Scenario:**
1. The acknowledgement channel's TCP connection goes half-open: a balancer drops its state, or a network address translation entry expires.
2. Every attempt, the probe included (it uses the same channel), ends in `DEADLINE_EXCEEDED` with no envelope. Meanwhile the heartbeats on the main channel are answered, so the gRPC stack sees no reason to dial again.
3. Without HTTP/2 keepalive, the stack notices only at the operating system's TCP timeout, which can be minutes.
4. Three counted calls make the host fall back, and it declines for 10 minutes. On the shared connection the heartbeats would have failed too, and the channel would have been dialled again.

**Why it matters:** the text says the host's other traffic can neither delay its acknowledgements nor make it decline. The host's own channel now can.

**Fix:**
- Require HTTP/2 keepalive on the acknowledgement channel, with a ping timeout under the receive bound, and make the server's keepalive policy accept that rate (otherwise the server answers `GOAWAY`).
- Or have the host dial the channel again after an attempt ends with no answer while the main channel is answered.
- Add an SDK unit test.

## Nit

1. **Timing statements written before the decisive attempt:**
   - The failure row says "about 6 s of delay" (`stream-completion.md:861`).
   - Decision 006-065 says "within about 6 s" (`research.md:923`).
   - Under the new rule the delay is at least the first attempt plus the decisive attempt (about 8 s with the 4 s server bound), plus up to one heartbeat interval (10 s) when heartbeats are the only evidence.
   - The Fallback row's proxy case (`stream-completion.md:943`) still says "each lease given up after one receive bound", where `tasks.md` now says "at the end of its first attempt after a heartbeat answered".
   - The Bound row (`stream-completion.md:933`) has one lease with every attempt failing and no other call. That gives no evidence, so the lease waits for the ordinary give-up (12 s at an 18 s time-to-live, 24 s at 30 s). Either add a task heartbeating, or say that the test drives the hand-back itself.
2. **A frame still counts by its arrival** (my round-11 Nit 3 part 2). A frame claimed before a failure but read after an attempt ended triggers a decisive attempt. On a host with no other traffic nothing else fails, so the lease is given up during the outage. Count a frame only if it was received on a receive started after that attempt ended, or record this in Gap 006-047.
3. **The record's wording on the reserve:** the record's Consequences (`ADR-0099…md:260`) say "60 places of a connection's 64 from the activation". The reserve follows the compiled minor (Task 006-057's build), not the operator's activation.
4. **The guard makes `lease-withdrawal-v2` one-way:** the guard refuses the removal of `lease-withdrawal-v2` while a journal holds any `step.lease_withdrawn`, including the `NOT_SENT` events of Architecture decision 0098 alone. On a fleet that has already withdrawn leases, recording it is therefore irreversible before any 1.8 event exists. Either narrow the guard to the 1.8 cause, or state in the Rollback and the runbook that recording it is a one-way step.
5. **How the 30 s case holds a lease in backoff:** the case holds "one lease in its backoff across the promotion", but the harness's `hold` keeps a call in flight, not in backoff. Name the mechanism, for example a retry option whose initial delay outlasts the distance to the promotion. Otherwise the case depends on timing again.
6. **What the second refusal cannot see:** it checks "any registered instance, live or not". It does not see a process whose row the day rule deleted, nor one that was answered `FULL` (`instances.rs:669-675`, which serves without a row). A build of #260 without (7), whose registry connection fails while its pool works, can keep an open gate past the activation until its next successful beat, or until readiness (`beat_is_stale`) takes it out of service. Soften "no binary without the liveness rule holds an open gate" to say this.

## Checks made against the code

- **The attempt deadline:** the grant stores `attempt_deadline_at` as the claim's time plus `timeoutMs`, and does not cap the expiry at it (`execution.rs:7066-7077`). A renewal caps the expiry at the attempt deadline. So field 24 has a value the server already holds. No draft in `specs/` takes `TaskLease` field 24; draft 019 takes 20 to 22.
- **The registry:** registration refuses a missing requirement under the epoch `FOR SHARE`, and a beat reads the epoch plainly (`instance_registry.up.sql:197-204`, `:269`). The requirements read is plain (`crates/infra-postgres/src/instances.rs:351-357`). A missing row is registered again (`crates/application/src/instances.rs:659-676`) (Major 2 resolved; Nit 6).
- **The closed lists and the guard:** five capabilities, and the guard is keyed to `lease-withdrawal-v1` by name (`20270208000000_lease_withdrawal.up.sql:12-29`, `:52-65`). So request (9) must extend both (Minor 3, Nit 4).
- **The pool:** the acquisition timeout is 3 s, and sqlx's default ping before acquisition is kept (`pool.rs:32`, `:72-76`) (Major 1).

## Evidence to ask for once built

- `SAGAT_SDK_SCENARIO=delivery-acknowledgement cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture`:
  - two engines, with the acknowledgement channel on the engine whose database connection is held unreachable after the promotion. Expected: no give-up, then `recovered` (Major 1);
  - a shed burst with a task heartbeating, with attempts counted (Minor 1).
- The SDK unit tests:
  - a half-open acknowledgement channel detected by keepalive with no fallback (Minor 4);
  - the failed-try rule excluding `AcknowledgeTasks` (Minor 2).
- `cargo test -p sagat-tests --test integration_platform_foundation`: Task 006-044's migration applied after the follow-up's, with both capabilities accepted by all three constraints (Minor 3).

## Files consulted

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/round-11/ (correctness.md, security.md, architecture.md, egm-design.md)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/execution.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/pool.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/instances.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/instances.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/019-worker-deployments-dispatch/contracts/worker-protocol.md
- /private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/delivery-ack-round11-to-12.diff
