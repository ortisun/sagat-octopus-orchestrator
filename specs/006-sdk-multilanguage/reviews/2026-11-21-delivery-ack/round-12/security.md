# Security review, round 12: delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)

**Verdict: no Blocker, no Major, 3 Minor, 3 Nit.**

All three round-11 security findings are resolved: Major 1, Minor 1 and Minor 2. The new capability `lease-withdrawal-v2` makes the rollback floor something the registry enforces, whatever order releases are cut in. I checked this against the base code: the guard is a trigger, the census and the beat are SQL functions. The changes add three smaller defects:

- **Minor 1:** the reordered beat creates a race that can open a lapsed engine's capability calls under an epoch that is already out of date.
- **Minor 2:** the second refusal of the activation entry, the one meant to be a guarantee, exists only in the command, and it only counts rows that the registry can delete.
- **Minor 3:** the new "send the next attempt at once" rule bypasses the backoff while acknowledgements are being shed.

"Verified" means I read the code in `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack`. "Design reading" means the defect is in the documents alone.

## Status of round 11

| Round-11 finding | Status | Reason |
|---|---|---|
| **Major 1:** an earlier release of #260 is re-admitted | **Resolved** | Request (9) adds `lease-withdrawal-v2` on both kinds, with its own activation entry and a replay-floor guard. Every marked withdrawal reads v2, so a binary that lists only `lease-withdrawal-v1` is refused registration once v2 is recorded (`instance_register` refuses any recorded requirement the binary lacks, `20270113000000_instance_registry.up.sql:199-205`). v2 cannot be removed once a journal holds the event: the guard is an `ENABLE ALWAYS` trigger, so hand SQL cannot get past it either (`20270208000000_lease_withdrawal.up.sql:52-65`). The documents now agree with each other. Residue: Minor 2, which concerns only the liveness rule (7) for a binary without it, not the ability to replay the journal. |
| Minor 1: the heartbeat exemption after `NOT_ACKNOWLEDGED` | Resolved | A heartbeat after `NOT_ACKNOWLEDGED` is now counted under `heartbeat` and alerted. The two cases are tested in Task 006-050 and Task 006-053. Residue: Nit 2 (what decides the kind) and Nit 3. |
| Minor 2: a beat with no epoch hides committed changes | Resolved | The beat is now `instance_heartbeat_reporting_change`, which returns the committed epoch and `out_change_in_progress`. A process always applies an epoch that moved. Both kinds call it, the beats are counted, and Task 006-046 tests it. The order of the two reads is a new defect: Minor 1. |
| Nit 1: the contradiction at `research.md:786` | Resolved | `research.md:796-798` and `tasks.md:142-149` now say the same thing. |
| Nit 2: the overload's third argument | Resolved | It is now a function with a name of its own, so there is no default and no ambiguous call. `PUBLIC`'s missing `EXECUTE` is tested, and the down migration drops it by its full signature. |
| Nit 3: two value sets for `mark` | Resolved | There is now one listing with four values. `provisional` can occur in `expired` (requirement missing, or at the cap), so four is right. |

Other reviewers' round-11 findings in my domain:

- **Correctness Major 2, architecture Major 1, design Major 1 (the floor):** resolved, as Major 1 above.
- **Correctness Minor 5 (heartbeat after `NOT_ACKNOWLEDGED`):** resolved.
- **Correctness Major 1, design Minor 1 (the early give-up at the end of a failover):** resolved for security. The new rule's cost under shedding is Minor 3.
- **Correctness Minor 3, architecture Minor 1 (`$flag` keyed by node):** resolved. The key is now `ExecutionInstanceId`, and a mark that is never consumed fails the commit, which is the safe direction.
- **Architecture Minor 4, design Minor 4 (the reserve):** resolved. It is keyed to the compiled minor, and the class is decided from the method path on the caller's own connection, so an unauthenticated peer can only use up places on its own connection.
- **Correctness Minor 7, architecture Minor 5, design Minor 3 (stream credit):** resolved. Acknowledgements go on a channel of their own under the SDK's permit, and a call that is not yet sent is not counted. This gives the caller no new signal.
- **Architecture Minor 3 (the `REGISTRY_SILENT` series):** resolved. The series is named and the follow-up is told to emit it.

---

## Minor

### Minor 1. The beat reads the epoch before it tries the lock. An activation that commits between the two reads gives a lapsed engine "no change in progress" together with the old epoch, so the engine reopens its capability calls under requirements that are no longer current (verified against the base code; design reading for the new function)

**Where.**
- `stream-completion.md:448`: "It reads the committed epoch with a plain read … **then** tries the epoch row `FOR SHARE SKIP LOCKED`."
- `research.md:815-822` and the ADR, decision 10 (`:166-167`), say the same thing without stating the order.

**What the code has.**
- The census of `platform_requirement_activate` counts only instances seen within 30 s (`20270113000000_instance_registry.up.sql:333-337`). The beat's `UPDATE` of `last_seen` is not visible to the census until the beat commits.
- `Member::decide` compares the beat's epoch with the gate's epoch. When they are equal, it reads nothing more and keeps the gate as it is (`crates/application/src/instances.rs:678-680`).

**Scenario.** An engine of the floor build, lapsed across the activation of `delivery-acknowledgement-v1` (or of any requirement it lacks), comes back while the activation is running:
1. The beat runs its `UPDATE last_seen` and then the plain read, which returns E. The activation holds the epoch row `FOR UPDATE`, and its new epoch is not yet visible.
2. The activation's census does not see the beat's `last_seen`, which is uncommitted, so it counts the engine as not live. The activation records the requirement, moves the epoch to E+1 and commits.
3. The beat's `FOR SHARE SKIP LOCKED` now finds the row free. It returns `out_change_in_progress = false` with epoch E.
4. E equals the gate's epoch, so the engine treats this as "a beat that reports no change in progress" and its gate as decided, and reopens capability calls. It serves `ResolveSecrets` without recording it until its next beat, 10 to 12 s later. That is the round-8 hole, in a narrow window.

The round-11 design read the epoch only through the locked read, so it had no such window. The window is short: the activation must commit between two statements of one function. No caller can trigger it, so this is Minor.

**Fix.** Try the lock first.
- If `FOR SHARE SKIP LOCKED` returns the row, return the epoch from that locked read. In `READ COMMITTED` that is the latest committed value, and it cannot move until the beat commits. Report `out_change_in_progress = false`.
- If the row is skipped, do the plain read and report `true`.
- Write the order in `stream-completion.md:448` and in Decision 006-059 (7).
- Test it with a barrier: hold the beat between its two statements while an activation commits, and require the engine to stay refusing.

### Minor 2. The refusal "while any registered instance, live or not, lacks `lease-withdrawal-v2`" is the only thing that keeps a binary without (7) from serving provisional leases. It is enforced only by the command, and only over rows the registry keeps (verified for the deletions; design reading for the guarantee)

**Where.**
- ADR, decision 10 (`:156-159`): "so no binary without the liveness rule holds an open gate when the first provisional lease is claimed."
- `research.md:653`: the refusal is "not" a convenience.
- `stream-completion.md:408-413` and `:870`.

**What is wrong.**
1. **The hand path.** The refusal is a check in `control-plane capability-activate`. A requirement is recorded by `platform_requirement_activate` (`instance_registry.up.sql:312-351`), which counts live rows only, and the operator can call it directly, as the hand SQL of section 5 of the runbook does. Recording `delivery-acknowledgement-v1` that way while v2 is recorded passes the marking condition. A lapsed engine of #260 without the follow-up, or of v0.2, is then not excluded.
2. **Rows that vanish.** The registry deletes rows in three ways:
   - a row silent for a day, deleted by any beat (`:263-268`);
   - any row not seen for 30 s, deleted by a registration when the table holds 1,000 rows (`:214-219`; a holder of the runtime role can fill the table, Gap G2000);
   - a row deregistered on a clean stop.

   A process suspended past the day, or whose row was pushed out, keeps its gate open in memory. On resume it has no row, so the refusal never saw it. Without (7) it serves capability calls over the hosts' connections that are already open until its first beat (`instances.rs:27-30`, `:659-667`). That beat finds no row, registers again, is refused and closes. So the claim "no binary without the liveness rule holds an open gate" is false for such a process.

**Why only Minor.** The exposure lasts at most one beat after the resume while the registry is reachable. It needs a host that breaks the protocol (Gap 006-042). The tenant gains nothing it could not get by acknowledging.

**Fix.**
- Move the refusal into the database so it holds for every path. One option is a check in `platform_requirement_activate`; another is a `BEFORE INSERT` trigger on `platform_requirements`, marked `ENABLE ALWAYS` like the replay-floor guard. For `delivery-acknowledgement-v1` it refuses while v2 is not recorded for both kinds, or while any `serving_instances` row of either kind lacks v2. It runs under the epoch lock the insert already holds.
- Name the census function the command reads. It is an owner function, and unlike `platform_capability_census` it does not filter on `last_seen`.
- State the residue for rows that vanish in Gap 006-042, or in a gap of its own: a process of a build without (7) whose row was deleted serves capability calls from its resume until its first beat.
- Add both cases to the released-upgrade job:
  - the hand path is refused;
  - a lapsed engine of #260 without the follow-up whose row was deleted is refused at its first beat after the resume.

### Minor 3. "On the evidence the host sends the lease's next attempt at once, outside the backoff" also applies after a shed, so a fleet that keeps heartbeating gets around the backoff while acknowledgements are being shed (design reading)

**Where.** `sdks/SPEC.md:383-389`, `stream-completion.md:752-760`, `research.md:909-915`.

**What is wrong.**
- The trigger is "after an attempt of a lease ended with no verdict". A shed (`RESOURCE_EXHAUSTED`, `ACKNOWLEDGEMENT_SHED`) is such an attempt.
- Only the give-up excludes sheds. The resend at once does not.
- "Otherwise it goes on with the backoff", but the next failure followed by the next heartbeat answered `OK` triggers another resend at once. Nothing limits how many times this happens per lease.

**Scenario.** The Throughput gate requires "no heartbeat shed while acknowledgements are admitted in a burst" (`stream-completion.md:942`). That is exactly the state in which acknowledgements are shed and heartbeats are answered:
- A host running many tasks has a heartbeat answered every few hundred milliseconds.
- Each of its waiting leases is resent as soon as its shed answer arrives, held back only by the permit of 4 calls in flight.
- Across the fleet this is a constant retry stream against the class that is shedding. The backoff of §5 and the purpose of the shed are both cancelled.

This is availability only: no data and no tenant boundary are involved, and a hostile host can send at any rate anyway. The cost falls on conforming SDKs.

**Fix.**
- A lease whose latest attempt was shed waits its backoff, and evidence does not cut it short.
- Allow at most one attempt outside the backoff per lease per receive bound.
- Add an SDK unit test: during a burst of sheds with heartbeats answered, the attempts per lease follow the backoff.

---

## Nit

1. **The Gate row is out of date.** `stream-completion.md:939` still writes the marking cases as "`lease-withdrawal-v1` for both kinds, recorded by hand for engines only among the cases". Marking now depends on v2 (ADR decision 2, `:61`). A test written from this row would leave the v2 condition untested. Write v2 there, and keep a case in which v1 alone is recorded for both kinds and nothing is marked.
2. **What decides `heartbeat` against `heartbeat_after_hand_back`.**
   - The stale path matches `withdrawn_lease_token_hash` (`stream-completion.md:477`), but no column records which cause withdrew that token.
   - `provisional_withdrawn_attempt` does not tell the two apart. It is unset for the `NOT_STARTED` withdrawal of an unflagged lease, and it is set for a `NOT_ACKNOWLEDGED` withdrawal that reaches the cap.
   - Fix: say where the cause comes from. Either write it in the same assignment as the hash (a column with its allowed form in the static check), or read the step's last `step.lease_withdrawn` in the caller's tenant, by the same primary key. Make the answer the same whichever way it is decided.
3. **Optional, unchanged from round 11.** A second heartbeat after a hand-back's withdrawal, for the same step and generation, cannot be the overtaking race. A host that handed back and kept running heartbeats until it reports, and nothing alerts in that time. Count the second one under `heartbeat`.

---

## Suspicion, not verified

- **The epoch lock may be held off by beats** (carried from round 11). Every beat still takes `FOR SHARE SKIP LOCKED` on the epoch row, now after the plain read. The effect is availability only.
  - Command, once built: the scalability profile with 1,000 instances beating, timing `control-plane capability-activate` under `pgbench` load.

## Classes checked with no new finding

- **Tenant boundary.**
  - `attempt_deadline_epoch_ms` is the lease's own deadline, sent only to the holder.
  - The new kinds and counters (`heartbeat_after_hand_back`, `instance_heartbeats_change_in_progress_total{kind}`) are closed sets with no tenant data.
  - The caller gets the same stale answer whatever kind is counted, so the kind gives it no oracle.
- **Authorization and admission.** The `Acknowledge` reserve is decided from the method path on the caller's own connection, with nothing kept per peer before authentication. The channel of its own changes nothing on the server.
- **Least privilege.**
  - `sagat_app` still takes no lock on the epoch row that conflicts with `FOR SHARE` (`instance_registry.up.sql:404`), so only the owner can set `out_change_in_progress`.
  - The guard on v2 must be the same `ENABLE ALWAYS` trigger, extended. `lease-withdrawal-v2` must also join the three closed checks, as `20270208000000_lease_withdrawal.up.sql:12-29` did for v1.
- **Secrets and logs.** Nothing new carries a token or a payload. The internal error when a mark is never consumed must not carry the instance path; it is an internal error with no request data.
- **Injection and parsing.** No dynamic SQL. Field 24 is an `int64`, and the host clamps its give-up to between 6 s and 120 s.
- **Races, beyond Minor 1.** The refusals of the activation entry are monotone once v2 is recorded: no new row that lacks v2 can register. So checking them outside the epoch lock opens no window, apart from the hand path of Minor 2.

## Commands to run once built

- **Minor 1:** in `integration_durable_execution`, or the registry suite, as the runtime role: a barrier inside `instance_heartbeat_reporting_change` between its two reads while an owner activation commits. The lapsed engine must keep refusing `ResolveSecrets`.
- **Minor 2:**
  - `psql` as the owner: `SELECT * FROM platform_requirement_activate('delivery-acknowledgement-v1', '{execution-engine}', 'test');` with a lapsed row lacking v2 present. It must be refused.
  - In the released-upgrade job: a #260 engine without the follow-up, suspended past the deletion of its row, then resumed. It must be refused at its first beat.
- **Minor 3:** an SDK unit test with sheds and heartbeats answered `OK`, counting attempts per lease.

## Files

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/instances.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/apps/control-plane/src/capability_activation.rs
