Round 13 has no Blockers. It has 1 Major, 3 Minors and 7 Nits. The Major is new and was caused by the round-12 fix. The round-12 Major is partly resolved, and all five round-12 Minors are resolved.

# Architecture review, round 13: delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)

## Verdict

**What blocks:**
- **Major 1.** The fix to round-12 Major 1 moved all of Task 006-044 to the waiting list. Three tasks that still proceed need the columns that Task 006-044 creates. The text must be fixed before this design merges, and before Task 006-048, Task 006-049 or Task 006-052 starts.

**Round-12 Major 1 (Task 006-044 proceeds but needs request (9)): partly resolved.**
- What is resolved:
  - Task 006-044 waits for the follow-up.
  - Its migration sorts after the follow-up's migration.
  - Its closed lists carry `lease-withdrawal-v2`.
  - Task 006-052 rebases onto the follow-up's capability paths.
- So the duplication of request (9) and the collision between the two rewrites of the closed lists are gone.
- What is not resolved: the task was moved whole instead of split, and that created Major 1 below.

**Limits:**
- I had no shell, so I could not run `git branch -a` or `git worktree list`.
- I searched every `crates/` and `apps/` tree under `/Users/marcelo/Documents/Projects` for `lease-withdrawal-v2`, `instance_heartbeat_reporting_change` and `ACKNOWLEDGEMENT_STORE_UNREACHABLE`.
  - The only matches are the merged head's test that `lease-withdrawal-v2` is not activatable (`apps/control-plane/src/capability_activation.rs:359`), in this worktree and in `sagat-wt-withdraw-leases`.
  - So the follow-up has not started anywhere, and nothing is duplicated.

---

## Status of the round-12 findings

| Round 12 | Status | Reason |
|---|---|---|
| **Major 1** (Task 006-044 proceeds) | **Partly resolved** | <ul><li>Task 006-044 waits; its migration sorts after the follow-up's and its lists carry `lease-withdrawal-v2` (`tasks.md:144-150`).</li><li>The task was moved whole instead of split, and three tasks that proceed need its columns (Major 1).</li></ul> |
| Minor 1 (the refusal lives only in the command) | Resolved | <ul><li>A `BEFORE INSERT` trigger on `platform_requirements`, enabled `ALWAYS`, is in Task 006-044.</li><li>Option 10, Architecture decision 0100's option 1 and its Consequences are rewritten.</li><li>The residues (a deleted row, a process with no row) are in Gap 006-044.</li><li>The runbook removes a silent row only once its process is confirmed stopped.</li><li>I checked whether removing `lease-withdrawal-v2` and recording it again gets round a trigger that fires only on insert. It does not: while `delivery-acknowledgement-v1` is recorded for engines, no engine of the #260 build can register, because it does not list that capability (`instance_register`, `20270113000000_instance_registry.up.sql:199-205`). The check at insert is enough.</li><li>Residues: Nits 1, 2 and 7.</li></ul> |
| Minor 2 (the guard is too wide; the one-way door is not recorded) | Resolved | <ul><li>The guard is narrowed to journals that hold the cause `NOT_ACKNOWLEDGED`.</li><li>Option 12 is added, the Refines line names the floor moving to `lease-withdrawal-v2`, and the forward order is stated.</li><li>The cause is in `detail` (`event_codec.rs:116`), so the guard's scan can filter on it at removal time.</li><li>The guard is not keyed to the history, as I suggested, but it is exact for what replay needs, and it is recorded.</li></ul> |
| Minor 3 (a channel is not a connection) | Resolved in the text, with residue | <ul><li>"A transport connection of its own", with the means of each SDK, is in `SPEC.md:365-366`.</li><li>Decision 3 says that behind a pooling proxy only the reserve holds.</li><li>The server-side assertion cannot be built as written, and fails by construction in Rust (Minor 1).</li><li>The second connection has a capacity cost that is not stated (Minor 2).</li><li>Re-dialling the connection has an effect that is not specified (Minor 3).</li></ul> |
| Minor 4 (the heartbeat kind has nothing stored) | Resolved | <ul><li>The kind is taken from the journal's latest `step.lease_withdrawn` of that generation, and no column is added.</li><li>The form and the bound of that read are not stated (Nit 5).</li></ul> |
| Minor 5 (no binary for the #260 case) | Resolved | <ul><li>Task 006-066 adds a third pinned phase at #260's merge commit, with its own source-manifest check, and the three assertions I asked for.</li><li>`scripts/test-release-upgrade-profile.py` pins one `RELEASE` constant (line 19), so the task generalises it. That is the task's work, not a design gap.</li></ul> |
| Nits 1 to 7 | Resolved | <ul><li>Nit 1: the Gate row is fixed.</li><li>Nit 2: "from the build of Task 006-057".</li><li>Nit 3: the ten register notes say "fields 23 and 24".</li><li>Nit 4: the indentation is fixed.</li><li>Nit 5: SPEC §6.5 names `lease-withdrawal-v2`.</li><li>Nit 6: the runbook step is in Task 006-046's evidence.</li><li>Nit 7: the jitter and the merged frames are added.</li></ul> |

**Other reviewers' round-12 findings in my domain:**
- **Design Minor 5** (the precondition contradicts its own rule): it is the same finding as my round-12 Major 1, and it is partly resolved in the same way (Major 1).
- **Security Minor 1** (the beat's read order): resolved. The locked read comes first, its epoch cannot move before the beat commits, and Task 006-046 has a barrier test in both orders. The task's description was not updated (Nit 6).
- **Security Minor 2 and design Minor 6** (the refusal in the command; the deleted rows): resolved, as my Minor 1 above.
- **Correctness Major 1** (the deciding attempt reaches an engine that has not recovered):
  - The new reason `ACKNOWLEDGEMENT_STORE_UNREACHABLE` is added to the closed list of `worker_task_acknowledgement_failures_total{reason}` as `store` (`stream-completion.md:911`).
  - It carries the envelope, so it never counts toward the decline.
  - It has a failure row and a two-engine case.
  - From the architecture side this is resolved. The wording "held a connection" is ambiguous next to "transport connection" (Nit 3).
- **Correctness Minor 4** (a half-open connection): the re-dial was added, and it brings Minor 3.
- **Design Minor 4** (subchannel pooling): resolved, with Minor 1's residue.
- **Design Minors 1 to 3** (the early give-up's backoff, the herd, the merged frames): outside my domain. The changes add no layer or contract problem.

---

## Major

### Major 1. Task 006-048, Task 006-049 and Task 006-052 proceed, but they need the columns of Task 006-044, which now waits

**Where:**
- Precondition (a), `tasks.md:144-150`:
  - Task 006-044 is in the waiting list.
  - "Task 006-043, Task 006-048, Task 006-049, Task 006-052 … proceed; Task 006-049 and Task 006-052 write the marks and the withdrawn rows they need by SQL fixtures".
- Task 006-044, `tasks.md:179-182`: it creates every column involved:
  - the eight `steps` columns (`lease_offered_generation`, `lease_claim_incarnation`, `lease_acknowledged_at`, `lease_capability_generation`, `withdrawn_lease_token_hash` and the others);
  - `worker_sessions.declines_acknowledgement`.
- Task 006-048, `tasks.md:240-243`: "`declines_acknowledgement` stored on the session from each registration", with the evidence "a registration that declines and a later one that does not".
- Task 006-049, `tasks.md:244-271`: the adapter reads the marks and the server's start time, decides on the locked rows, and writes `lease_acknowledged_at` (`acknowledge`).
- Task 006-052, `tasks.md:328-337`: "`lease_capability_generation` written once per generation".

**What is wrong:**
- An SQL fixture can set values in columns. It cannot create them.
- With Task 006-044 waiting, none of these three tasks can produce its evidence. The server owner must then do one of two things:
  1. **Stop,** which makes "proceed" untrue.
  2. **Bring the columns forward on their own,** which splits Task 006-044 by hand, with no stated migration order and no stated `down`. That is the improvised split that round-12 Major 1 asked the text to make.
- A precondition that sends the build the wrong way is the same defect as round-11 Major 2 and round-12 Major 1.

**Why it matters:**
- The precondition is the build's order. A contradiction in it makes the build go wrong (rule 1, the design is read before code).
- The migration discipline: whoever improvises the split also has to decide whether that migration touches the closed checks.

**Smallest correction:** split Task 006-044.
- **The part that proceeds:**
  - the eight `steps` columns and the `worker_sessions` column, with their grants;
  - the storage classification;
  - the check that the runtime role can read `pg_postmaster_start_time()`;
  - a `down` that drops the columns.
  - Its migration touches no closed check, so its version may sort before the follow-up's.
- **The part that waits:**
  - `delivery_acknowledgement_required()`;
  - the trigger on `platform_requirements`;
  - the three closed checks and `Capability::ALL`;
  - the `down` refusals that read the history and the registry.
  - Move this part into Task 006-046, which already waits and owns the registry, or into a new task with the next free number. Its migration sorts after the follow-up's.
- Update precondition (a) to match.

**Blocking:** the text before this design merges; before Task 006-048, Task 006-049 or Task 006-052 starts.

---

## Minors

### Minor 1. The server-side assertion "another connection than the stream's" cannot be read by the harness, and fails by construction in the Rust runner

**Where:**
- Task 006-058, `tasks.md:611-612`: "an assertion that the server sees the acknowledgements on another connection than the stream's (the peer held in `CappedConnectInfo`)".
- The Fallback row, `stream-completion.md:971`: "which the server sees as another peer than the stream's".
- `crates/api-grpc/src/limits.rs:327-329`: `CappedConnectInfo` holds only `watchdog: Arc<ConnectionWatchdog>`. It holds no peer address.
- `crates/api-grpc/src/limits.rs:274`: the per-peer key is the IP address (an IPv6 address counts as its /64).
- `sdks/rust/conformance/src/faults.rs:34-38` and `sdks/rust/conformance/src/faults/grpc.rs:62`, `:198`, `:212`:
  - When a gRPC fault plan is active, the Rust runner puts an in-process tonic proxy in front of the server.
  - The proxy forwards every call on one cloned upstream `Channel`.
  - The other five SDKs inject faults with client interceptors, with no proxy.

**What is wrong:**
- **Same peer, different connection.** Both connections of a host come from the same IP address, so by the server's own key they are the same peer. "Another peer" is the wrong observable.
- **Nothing to read.** No per-connection identity is exposed to the harness. The harness talks to a real server process, and `CappedConnectInfo` holds nothing it could compare.
- **Rust fails by construction.** In the Rust runner, every scenario with a gRPC fault plan (the fallback cases are such scenarios) multiplexes the stream and the acknowledgements onto one upstream HTTP/2 connection. That is exactly the pooling proxy that decision 3 says defeats the separation.
  - So the assertion fails in Rust.
  - And "a connection with more than 128 calls does not fall back" passes in Rust only because of the reserve, not because of the SDK's connection.

**Why it matters:**
- Rule 8 (one semantics in six SDKs, tested alike).
- Rule 21: a check that cannot be built, or that fails for a harness reason, does not enforce the rule.

**Smallest correction:**
- Give each accepted connection an identifier in `CappedConnectInfo` (a counter, never an address), and expose it to the harness through a test-only hook of the conformance platform.
  - Or take the assertion on the client side: each SDK's unit test compares the local socket addresses of its two connections. Then keep one server-side Rust integration test.
- Make the Rust proxy open one upstream connection for each downstream connection it accepts.
- Write "another connection", not "another peer".

**Blocking:** before Task 006-058 and Task 006-064.

### Minor 2. The second connection counts against the server's per-peer and per-process connection caps, and the design does not say so

**Where:**
- `crates/api-grpc/src/server.rs:51-54` and `:86-87`:
  - `max_connections` is 2,048, and "the next one is accepted when one closes".
  - `max_connections_per_peer` is 256. Its own comment says that workers behind one NAT or load balancer share an address.
- `crates/api-grpc/src/limits.rs:274-275`: an excess connection is refused and counted in `grpc_connections_refused_total{reason="per_peer"}`.
- Architecture decision 0099, Consequences (line 270): "each 1.8 host keeps one more connection".
- SPEC §6.2: a call counts toward the decline when every attempt ended with no answer or without the envelope, and its probe, sent on the same connection, failed the same way.

**What is wrong:**
- At 1.8 every host holds two connections. So behind one NAT address only half as many hosts fit, and only half as many fit per process (1,024 hosts instead of 2,048).
- A host past the per-peer cap has its acknowledgement connection refused, while its main connection works. Its attempts and its probes end with transport failures without the envelope, so after three calls it falls back to declining. The host then declines for 10 minutes, doubling up to an hour.
- At the process cap the dial waits, and the attempts fail the same way.
- So an upgrade to 1.8 can silently turn a fleet behind NAT into a declining fleet. That is graceful, but it is unplanned. The only signal is `SagatWorkersDecliningAcknowledgement`, which names a symptom, not the cause.

**Why it matters:**
- Operability: ceilings and quotas must be stated together with what they cost.
- A capacity regression is a consequence, so it belongs in the record.

**Smallest correction:**
- Say in Consequences and in the runbook that both caps count the acknowledgement connection, and that a 1.8 fleet behind shared addresses needs `max_connections_per_peer` doubled.
- Add a failure row: "the acknowledgement connection refused at the per-peer cap leads to the fallback, and `grpc_connections_refused_total{reason="per_peer"}` rises".
- Have the gate of Task 006-056 report the number of connections per host.

**Blocking:** before Task 006-054 and Task 006-057.

### Minor 3. Re-dialling closes the acknowledgement connection with other attempts still in flight, and the text does not say what those attempts are

**Where:**
- `sdks/SPEC.md:368-370`: "the host closes the acknowledgement connection and dials it again before its next attempt".
- `research.md` Decision 006-066 (the re-dial clause).
- Task 006-059's unit case "a half-open acknowledgement connection dialled again with no fallback".

**What is wrong:**
- Up to 4 attempts share the permit. Closing the connection after one attempt ends with no answer cancels the others.
- In Go, `ClientConn.Close` cancels the calls in flight, and so does `shutdownNow` in Java. grpc-js and .NET differ in whether a closed channel lets the calls in flight finish.
- A cancelled attempt is "an attempt that ended with no verdict". So when it was the deciding attempt sent after the evidence, the host gives the lease up because of its own action. It may also count toward the decline.
- The SDKs will differ in what their close does to the other attempts, which is a split in semantics.

**Why it matters:** rule 8 (no SDK forks the semantics); a host should not give up a lease because it closed its own connection.

**Smallest correction:** one sentence in §6.2, either of these:
- "the new connection carries the next attempts; the old one is closed once its attempts in flight have ended, at most 6 s"; or
- "an attempt ended by the host's own close is not an attempt for the give-up and counting rules, and is sent again".

Add the unit case "re-dial with three other attempts in flight: none given up, none counted" to Task 006-059 and the five tasks that follow it.

**Blocking:** before Task 006-059 to Task 006-064.

---

## Nits

1. **The `down` forgets the trigger and the lists.** `tasks.md:189-190` and `stream-completion.md:856-858` say the `down` "drops the function". It should also:
   - drop the trigger and its function, by full signature;
   - restore the three closed checks to the follow-up's lists, keeping `lease-withdrawal-v2`, not to the lists of Architecture decision 0098.
2. **A stale sentence about the trigger.** `stream-completion.md:425` still says "(a trigger of its own was considered and rejected, Decision 006-047)", in the same bullet that now introduces a trigger (line 411). Write "a trigger for the order alone".
3. **"Held a connection" is ambiguous.** In decision 3 of the record and in `stream-completion.md:607`, "before the call held a connection" follows "a transport connection of its own". Write "a database connection of the pool".
4. **"Enforced by" is incomplete.** Line 13 of the record names none of these three checks:
   - the trigger refusing a hand-SQL insert (Task 006-044 and Task 006-057);
   - the narrowed guard of `lease-withdrawal-v2` (Task 006-046);
   - the separate-connection assertion (Task 006-058).
5. **"One indexed read" on the stale path** (`stream-completion.md:757`, `research.md:731`).
   - `run_events` has the primary key `(tenant_id, run_id, seq)` (`20261009000100_execution.up.sql:101`) and, for scoped steps, the partial index `run_events_instance_idx` (`20261027000000_scoped_instance_identity.up.sql:45-47`).
   - So the read is a backward scan of the run's journal: `event_type`, `node` and `detail->>'generation'`, `ORDER BY seq DESC LIMIT 1`.
   - Say that form. Say that it runs only after the withdrawn hash has matched. Say that the run's journal ceiling bounds it.
6. **Task 006-046's description is stale.** At `tasks.md:204` it still says the beat returns "the committed epoch by a plain read and whether the epoch row is held". Its evidence was updated to the locked read first; the description should match.
7. **The trigger should take the lock itself.** "Under the epoch lock the insert already holds" (`stream-completion.md:413`) holds through `platform_requirement_activate`, which locks the epoch at line 323 before inserting at line 343. A hand insert with `sagat.registry_change` set takes that lock only at its epoch update. The race is harmless, but the trigger can simply take the epoch row `FOR UPDATE` itself.

---

## The checks you asked for

- **Layers:**
  - Nothing new puts I/O, a framework or `serde_json` into the domain.
  - The trigger and the journal read are adapter-side.
  - The new reason is a wire value of the gRPC adapter, carried in the envelope.
- **Decisions:**
  - Option 12 records the alternative to `lease-withdrawal-v2` and its trade-off.
  - Options 10 and Architecture decision 0100 are consistent with the trigger.
  - "Enforced by" misses three checks (Nit 4).
- **Compatibility:**
  - The contract changes are additive.
  - The beat keeps its two-argument function for older binaries.
  - Recording `lease-withdrawal-v2` stays reversible until the first withdrawal of 1.8.
  - The migration split is unresolved (Major 1), and the `down` is incomplete (Nit 1).
- **Operability:**
  - `store` joins a closed list, and `ACKNOWLEDGEMENT_STORE_UNREACHABLE` carries the envelope.
  - The connection caps are not stated (Minor 2).
- **Scope:** no duplication. Request (9) stays with the follow-up.

## Fitness functions for the rules this change relies on

| Rule | Enforced by | Gap and smallest addition |
|---|---|---|
| No registered instance lacks `lease-withdrawal-v2` when `delivery-acknowledgement-v1` is recorded | The trigger (Task 006-044); the hand-SQL case of Task 006-057 | Name it in "Enforced by" (Nit 4) |
| `lease-withdrawal-v2` stays once a `NOT_ACKNOWLEDGED` withdrawal exists | The narrowed guard; Task 006-046's two cases | none |
| The tasks that proceed have their schema | none | Major 1: split Task 006-044 |
| The acknowledgement has a connection of its own | The assertion of Task 006-058 | Minor 1: a connection identifier readable by the harness, or a client-side check; a Rust proxy with one upstream connection per downstream connection |
| A 1.8 fleet fits under the server's connection caps | none | Minor 2: a failure row, runbook text, and the count reported by the gate of Task 006-056 |
| A re-dial decides nothing by itself | none | Minor 3: one rule and a unit case in each SDK |
| A store that cannot be reached decides no give-up | Task 006-049's pool-full case; the two-engine case of Task 006-058 | none |
| A pinned #260 binary is refused | The third phase of Task 006-066 | none |

## Items for the owner

1. **Splitting Task 006-044 (Major 1):** whether the columns proceed now, or the server work waits entirely for the follow-up. Splitting keeps Task 006-048, Task 006-049 and Task 006-052 moving.
2. **The connection cost (Minor 2):** whether a 1.8 fleet behind shared addresses is expected. If it is, the default of `max_connections_per_peer` may need to rise with Task 006-057.
3. **Still open:**
   - accepting Architecture decision 0099, with both assumptions of decision 12;
   - Owner question 006-002;
   - who builds the follow-up of pull request #260, with requests (1), (2) and (5) to (9), and when;
   - the delegate's thresholds;
   - the constraints that stay (no emergency stop, two restarts for a rollback).

## Files relevant to this review

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md (144-150, 179-194, 204, 240-271, 328-337, 611-612)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md (411-425, 607, 757, 856-858, 911, 971)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md (651-656, 731, 846-853, 999-1004)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md (13, decision 3, decision 10, options 10 and 12, 270)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0100-withdrawn-a-fleet-requirement-may-require-another.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md (§5 line 280, §6.2 lines 365-398, §6.5 lines 421-423)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/limits.rs (274-275, 327-329)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/server.rs (51-54, 86-87)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/rust/conformance/src/faults.rs (34-38)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/rust/conformance/src/faults/grpc.rs (62, 198, 212)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql (98-168, 197-205, 323-345)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql (52-65)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20261009000100_execution.up.sql (88-106)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/event_codec.rs (111-116, 538-545)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/scripts/test-release-upgrade-profile.py (19, 170)
