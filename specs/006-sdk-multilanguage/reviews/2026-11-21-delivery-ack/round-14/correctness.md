# Correctness review of Architecture decision 0099 (delivery acknowledgement, worker protocol 1.8), round 14

## Verdict

My round-13 Major is resolved. Task 006-044 now holds only the columns, so the proceeding tasks (Task 006-048, Task 006-049 and Task 006-052) have what they need. The registry part has moved to Task 006-046, which waits for the follow-up of pull request #260.

All four round-13 Minors are resolved or reduced to the residues below. All eight Nits are resolved. The round-13-to-14 changes introduce no Blocker and no Major. They do introduce seven Minors and ten Nits. Two Minors are on the edge of Major:
- **Minor 1:** a new sentence in the normative host text, read literally, switches off the early give-up whenever heartbeats are frequent.
- **Minor 2:** the new column `withdrawn_on_hand_back` has no allowed form in the static check.

**Counts:** 0 Blockers, 0 Majors, 7 Minors, 10 Nits.

I built and ran nothing. Every claim about code comes from reading `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack`.

## Status of the round-13 findings

| Round 13 | Status | Reason |
|---|---|---|
| Major 1: tasks that proceed need Task 006-044's columns | Resolved | Task 006-044 (`tasks.md:179-189`) creates the nine `steps` columns and the `worker_sessions` column, and touches no closed check. Task 006-048 needs `declines_acknowledgement`, Task 006-049 the marks, Task 006-052 `lease_capability_generation`. The acknowledgement does not read `required_since` (`stream-completion.md:533-535`), so Task 006-049 needs nothing from Task 006-046. Residues: Minor 7 (migration order against the floor release) and Nit 6. |
| Minor 1: the redial cuts attempts in flight | Resolved | The old connection is drained, and an attempt ended by the host's own close is not an attempt (`sdks/SPEC.md:371-375`). Waiting for the in-flight count to reach zero works in every SDK, so naming each SDK's means is no longer needed. Residues: Nits 1 and 3. |
| Minor 2: a receive ended by the host's own bound blocked the give-up | Resolved | `sdks/SPEC.md:397-398`, `stream-completion.md:794`, and the SDK case with a 3 s call timeout. Residue: Nit 10. |
| Minor 3: the set of "unreachable store" failures not named | Resolved for both cases I gave | A read-only server (`25006`) and time lost in the concurrency layer (now `ACKNOWLEDGEMENT_DEADLINE`) are both covered. Two new defects came with the rewrite: Minors 4 and 5. |
| Minor 4: the twelve-frame expectation | Resolved | "All given up early, none by the ordinary give-up, the last within three attempts' time plus 1 s of the first frame" fits my table (last at about 18.2 s). |
| Nit 1: stale wording; evidence that arrives during an attempt | Resolved, but the new sentence is Minor 1 | |
| Nit 2: "once any withdrawal exists" | Resolved | ADR line 305; `research.md:754` and `:860` |
| Nit 3: Decision 006-059 (7), order of the beat | Resolved | `research.md:820-825`. A beat that holds the epoch `FOR SHARE` makes an activation's `FOR UPDATE` (`instance_registry.up.sql:323`) wait. The census runs in a later statement of a volatile function in `READ COMMITTED`, so it sees the beat's `last_seen`. |
| Nit 4: the third pinned phase | Resolved | "held silent past the 30 s window" |
| Nit 5: routing in the two-engine case | Resolved for the second connection | The connection opened by a redial is not covered (Nit 2). |
| Nit 6: "held a connection" against `begin_safely` | Resolved | |
| Nit 7: a stale pool outlasting the ordinary give-up | Resolved | Recorded in Gap 006-047. |
| Nit 8: three details of the trigger | Resolved | The down of Task 006-046 drops the trigger; the trigger takes the epoch `FOR UPDATE` itself; "for the order alone". The existing guard `platform_requirements_guard` refuses an `UPDATE` (`instance_registry.up.sql:105-108`), so the new refusal cannot be bypassed by narrowing `kinds`. |

## Blocker

None.

## Major

None.

## Minor

**Minor 1. "An attempt in flight when the evidence arrives does not decide", read literally, stops any attempt from deciding while heartbeats are answered often. That contradicts the cases the design is built for.**

Confidence: confirmed in the documents.

**Where:** `sdks/SPEC.md:398`; `stream-completion.md:794-795`; `research.md:946-947`.

**What is wrong:**
- Evidence is any call answered `OK`, or any frame, after a failed attempt (`sdks/SPEC.md:390-391`). So it keeps arriving for as long as heartbeats are answered.
- Scenario, with heartbeats answered every 100 ms (the setting of the twelve-frame case and the "two flagged calls failing together" case):
  1. Attempt A1 fails at 6 s.
  2. The heartbeat answered at 6.05 s is evidence.
  3. Attempt A2 is sent early, at about 6.2 s.
  4. The heartbeats answered at 6.3 s, 6.4 s and so on are also evidence about A1, and A2 is in flight when each of them arrives. Under the sentence, A2 does not decide.
- The same holds for every later attempt, so no lease is ever given up early. Under that reading the twelve-frame case, and the proxy case "each lease given up at the end of its deciding attempt", both fail.

**Why it matters:** six SDK owners read this sentence. Some will take it literally and some will read it as "sent before the first evidence", so they time the give-up differently. The tests catch the literal reading only after it has been built.

**Fix:** "An attempt sent before the first evidence that follows the previous failed attempt does not decide, whatever evidence arrives while it is in flight; when it ends with no verdict, the next attempt may go early." Evidence that arrives later then only refreshes the window "between the latest evidence before that attempt was sent and its end".

**Minor 2. The new column `withdrawn_on_hand_back` has no allowed form in the static check, and its only natural form is the bound flag that the check exists to refuse.**

Confidence: confirmed in the documents.

**Where:** marks table, `stream-completion.md:493`; allowed forms, `:560-569`; Task 006-047's static check and its negative test "a bound-flag form"; Task 006-053.

**What is wrong:**
- The allowed forms say "the withdrawal sets `withdrawn_lease_token_hash` from the row it holds". They do not mention `withdrawn_on_hand_back`.
- The hash can be copied from the old row. The kind cannot: it is the cause of this withdrawal, which only the caller knows.
- The pass and the hand-back write through the same three projection statements (`stream-completion.md:502-503`). So the column must be bound (`$on_hand_back`).
- "No statement assigns a column of the marks from a value it read earlier", and the check carries a negative test for a bound-flag form. As written, the needed form fails the check, or the implementer widens the check without a rule.
- The wrong direction matters here:
  - A stale or mis-bound `true` on a `NOT_ACKNOWLEDGED` withdrawal counts a later heartbeat as `heartbeat_after_hand_back`.
  - That kind is not alerted, so the earliest sign of a handler run without an acknowledgement is hidden (the purpose of round-11 correctness Minor 5).
- "In one assignment" cannot describe two columns.

**Fix:**
- Give the column the same treatment as `provisional_withdrawn_attempt`:
  - it is bound true only by the hand-back's withdrawal service, through a per-instance port method;
  - it is bound false everywhere else;
  - it is assigned only in the same `CASE` branch and the same fence that write the hash, in the three statements.
- List that form in the static check, with negative tests:
  - a bound value outside that branch;
  - an assignment without the hash.
- Say "in the same statement and branch" instead of "in one assignment".

**Minor 3. Since the trigger now refuses to remove `lease-withdrawal-v2` while `delivery-acknowledgement-v1` is recorded, half of the removal races that Task 006-051 and the Gate row ask for can no longer be set up.**

Confidence: confirmed in the documents.

**Where:** Task 006-051 (`tasks.md:334-336`): "the removal of each requirement held between the pass's read and its commit, in both orders, with an empty journal, and both removals in one transaction of hand SQL". The Gate row (`stream-completion.md:982`): "the removal of either requirement held between a withdrawal's read and its commit, in both orders".

**What is wrong:**
- A `NOT_ACKNOWLEDGED` withdrawal is written only while both requirements are recorded. In that state:
  - The order "the removal of `lease-withdrawal-v2` commits first, then the pass writes nothing" is now refused at once by the trigger.
  - The order "the pass first" ends in the trigger's refusal, not in the replay-floor guard, so the guard is not what is tested.
  - "Both removals in one transaction" with `lease-withdrawal-v2` first is refused outright.
- These are the tests that prove the witness read, the real guarantee ("a convenience, not the replay floor's guarantee", `stream-completion.md:423-427`).
- An implementer either cannot write them or quietly tests a different path.

**Fix:**
- State that the `lease-withdrawal-v2` cases run with the new trigger disabled, by the same fixture the Gate row uses for "recorded for engines only".
- State that in the one-transaction case, `lease-withdrawal-v2` first is refused by the trigger, and `delivery-acknowledgement-v1` first commits only when no `NOT_ACKNOWLEDGED` withdrawal exists.
- Add the trigger's refusal under a held withdrawal as its own case.

**Minor 4. The rewritten definition of `ACKNOWLEDGEMENT_STORE_UNREACHABLE` drops "its connection failed". A connection that fails after `BEGIN` now gets an answer that decides, which widens Gap 006-047.**

Confidence: the omission is confirmed in the documents; the outcome is plausible.

**Where:** `stream-completion.md:614-617`, `research.md:939-943`, ADR decision 3 (lines 89-92), `sdks/SPEC.md:394`.

**What changed:**
- Round 13 said: "its deadline passed before it held a connection, or its connection failed".
- Round 14 lists only:
  - a transaction that did not begin (its acquisition timing out or failing with a store error the registry counts as unreachable);
  - `25006`.
- So an `Io` reset, a class `08` error, or `57P01` (`admin_shutdown`, the first thing a planned switchover sends) raised by `matched`, `lock_matched`, `acknowledge` or `COMMIT` is no longer unreachable.
- This is inconsistent: `25006` from a statement after `BEGIN` counts, but `57P01` from the same statement does not.

**Scenario:**
1. Attempt A1 failed.
2. A heartbeat is answered at 10.0 s (evidence).
3. Attempt A2 is sent at 10.1 s, and its transaction begins.
4. At 10.2 s a switchover shuts the primary down, and A2's statement fails with `57P01`. The answer is `UNAVAILABLE` without the unreachable reason, so it decides.
5. The heartbeats come every few seconds, so none failed between 10.0 s and 10.2 s.
6. The lease is given up. Its hand-back meets the failover and, when its tries of §5 end before the promotion, the lease lapses into `recovered` and a review.
7. Under the round-13 text the same attempt decided nothing, and the lease would have run as `recovered`.

**Fix:** add "or a statement or the `COMMIT` failed with an error that the registry's classifier (`crates/infra-postgres/src/instances.rs:73-93`) counts as unreachable". This is safe for a `COMMIT` with an unknown outcome: the host repeats and gets `duplicate`, or `recovered` if the failover lost the commit. Add a server test with `pg_terminate_backend` on the acknowledgement's connection between `lock_matched` and `acknowledge`.

**Minor 5. "In the authentication's lookups once the token verified" cannot be built on today's authentication path, and Task 006-049 does not name the change.**

Confidence: confirmed in the code.

**Where:** `stream-completion.md:616`, Task 006-049 (`tasks.md:270-271`).

**What the code does:**
- The lookups are `find_tenant_by_slug` and `find_membership` (`crates/application/src/identity.rs:244-262`). They are plain pool queries, not transactions.
- Their errors go through the identity store's mapping (`crates/infra-postgres/src/identity.rs:818-837`). That mapping folds `PoolTimedOut`, `Io`, `Tls`, `PoolClosed`, `WorkerCrashed` and the classes `08`, `40`, `53`, `55` and `57` (`57014`, a statement timeout, included) into one `StoreError::Unavailable`, which reaches the handler as `AppError::Unavailable` and `api(&e)` (`crates/api-grpc/src/service.rs:479-483`).
- So the handler cannot tell a lookup that did not reach the database from a lookup that timed out or deadlocked.
- The lookups' acquisition is bounded by the pool's `acquire_timeout` (3 s by default, `pool.rs:32`, configurable), not by the call's time left.

**Consequences:**
- If the implementer maps every authentication `Unavailable` to `ACKNOWLEDGEMENT_STORE_UNREACHABLE`, a statement timeout in the lookups never decides. This is the safe direction, but it differs from the handler's own rule.
- If the implementer maps none, the round-12 Major comes back through the authentication's door.
- With an `acquire_timeout` above about 1.5 s, plus a wait in the concurrency layer, the call overruns the "4 s in all" claim.
- The test "a pool held full for 5 s answered within 4.5 s, also when the authentication's lookups find the pool full" passes only at the default setting.

**Fix:**
- In Task 006-049, name the change:
  - the identity store reports unreachable separately, through the registry's classifier, as `StoreError::DatabaseUnavailable` does;
  - the acknowledgement's authentication runs under the call's deadline, its acquisition bounded by the time left.
- Run the test with `acquire_timeout` set to 10 s.

**Minor 6. The connection-cap text gets the `max_connections` case wrong. It also omits that the process-wide cap halves the number of hosts an engine can serve, and that the connection left out can be a host's main connection, which is an outage, not a fallback.**

Confidence: confirmed in the code for the listener; the outcome is plausible.

**Where:** `stream-completion.md:607-610`, the new failure row (`stream-completion.md`, "An acknowledgement connection refused at `max_connections_per_peer` or `max_connections`"), ADR Consequences (lines 271-273), Task 006-054.

**Code:**
- At `max_connections` the listener does not refuse. It stops accepting until a place frees, "so the kernel's backlog ... absorbs a flood" (`crates/api-grpc/src/limits.rs:217-270`). No counter rises.
- Only the per-peer cap drops the socket and counts `grpc_connections_refused_total{reason="per_peer"}` (`limits.rs:274-279`).
- So at the process cap, the acknowledgement connection's attempts and probes hang until the host's own 6 s bound rather than "fail without the envelope". The named signal never rises.

**Capacity:**
- With two connections per 1.8 host, an engine at the default 2,048 serves 1,024 hosts, whether or not they share an address. The text asks only for the per-peer cap to be doubled.

**Wrong outcome:**
- After an engine restart, every host dials again in a random order.
- Example: 200 hosts behind one network address translation, per-peer cap 256. At 1.7 there are 200 connections, which fit. At 1.8 there are 400, so 144 are dropped.
- Some of the dropped connections are main connections. Those hosts cannot claim at all while others hold two places.
- The design says only "a host refused at a cap falls back to declining". It also does not say whether a declining host closes its acknowledgement connection, which is what would give the place back.

**Fix:**
- State both caps as needing room for two connections per 1.8 host.
- Say that at `max_connections` the connection waits in the backlog, with no refusal counter (name the saturation gauge, or add a counter).
- Say that a host that falls back closes its acknowledgement connection.
- Say that a host whose main connection cannot get in has the same outage it would have at 1.7 when over the cap.
- Add a case with the per-peer cap set to one host's two connections minus one.

**Minor 7. "May sort before the follow-up's" lets Task 006-044's migration become one that the rollback to the floor build cannot revert.**

Confidence: plausible.

**Where:** `tasks.md:147` and `:179`; the rollback, `stream-completion.md:859-871`; `.claude/skills/migration/SKILL.md:7`.

**How migrations behave in this repository:**
- Migrations are applied by `MIGRATOR.run` (`crates/infra-postgres/src/lib.rs:107-116`) with no `ignore_missing`.
- sqlx applies a missing older version out of order.
- sqlx reverts strictly in descending version order.

**Scenario:**
1. Task 006-044's file (version V44) sits on this design's branch.
2. The follow-up's migration (version VF) is written later. Following the checklist, it takes a later timestamp, so V44 < VF.
3. The floor release R1 is cut without V44.
4. A database at R1 is upgraded to this build. sqlx applies V44 out of order, then V46.
5. Rolling back to R1:
   - Reverting down to VF reverts only V46.
   - Reverting V44 would first need the follow-up's down, which is refused while `lease-withdrawal-v2` is recorded.
6. V44 stays applied. R1's `control-plane migrate` (`apps/control-plane/src/main.rs:109-110`) then fails on the missing applied version.
7. The down that rollback step 4 relies on cannot run either.

Today this harms no data: the floor binary ignores the columns. But the rollback case of `provisional-lease-cutover` and the next migration of the floor fail.

**Fix (either):**
- Task 006-044 merges into `develop` before the follow-up's release is cut, so the floor release carries it and the rollback never reverts it. Then the rollback text says the columns stay.
- Or the file is renamed at the rebase so that it sorts after the follow-up's.

Also say that the rollback does not revert Task 006-044. Its refusal "while a provisional mark counts" is unreachable anyway, because Task 006-046's down, which runs first, refuses once the history names the capability.

## Nit

1. **The redial rule fires on the old connection too** (`sdks/SPEC.md:369-374`). "An attempt on that connection" also covers the connection being drained. On a half-open connection with three attempts in flight, each late "no answer" sets off another redial and abandons the connection the previous redial just opened. This is transient, but each extra connection counts against the caps of Minor 6. Apply the rule to attempts on the current connection only. The unit case "a redial with three other attempts in flight (none given up, none counted)" holds only when those three are answered; say so.
2. **The second redial trigger has no test.** No SDK unit case covers "two `ACKNOWLEDGEMENT_STORE_UNREACHABLE` in a row while the main connection answers `OK`", nor its reset by an answer in between. The two-engine conformance case (Task 006-058) routes "the host's second connection" to the unreachable engine but does not say where the redialled third connection goes. Its expected outcome ("then `recovered`") depends on that: an engine held unreachable for the whole lease ends in a withdrawn hand-back, not `recovered`. Name the routing and assert the redial.
3. **The resend can run at shutdown.** "An attempt ended by the host's own close ... is sent again" (`sdks/SPEC.md:374`) would also resend at `stop()` or on fallback. Limit it to the redial's own close.
4. **Task 006-046's down checks only live processes.** It refuses only while a live process lists the capability (`tasks.md:203-205`), then restores the closed checks. A lapsed row of a build of Task 006-057 that lists `delivery-acknowledgement-v1` makes the restored check fail with a constraint error. The base refuses for any registered process (`20270208000000_lease_withdrawal.down.sql:15-18`). Use the same rule.
5. **One more Gate case needs the trigger disabled.** The case "`lease-withdrawal-v1` alone recorded for both kinds" (`stream-completion.md:982`) needs the trigger disabled too, when `delivery-acknowledgement-v1` is recorded as `requirement_missing` requires.
6. **Task 006-048 depends on a waiting task.** "Read by the claim" (`tasks.md:250`) depends on Task 006-047, which waits. Move that clause to Task 006-047's evidence.
7. **Stale counts and "the down migration".** The ADR's Consequences still say "one function (the requirements read)"; there is now also a trigger and its function. The rollback's step 4 (`stream-completion.md:869-871`) still speaks of "the `down` migration", but there are now two downs with different refusals.
8. **The trigger can break the registry's lock order.** When hand SQL deletes without calling `platform_requirement_lock()` first, the trigger takes the epoch after the row lock. That goes against the order the registry documents (`instance_registry.up.sql:11-13`). Racing `platform_requirement_remove`, it ends in a deadlock that PostgreSQL detects, which is safe. Have section 5 of the runbook take the lock first.
9. **Transient connection counts.** "Each 1.8 host therefore holds two connections" (`stream-completion.md:607-608`): three while one is being drained.
10. **The receive exclusion covers polls too.** "A receive ended by the host's own bound" also excludes a unary `PollTasks`. The server answers a poll at once (`crates/api-grpc/src/service.rs:576-644`), so a poll ended by the host's bound is a real sign of failure. Limit the exclusion to a stream window.

## Evidence to ask for once built

- **SDK unit tests:**
  - dense heartbeats with the deciding attempt in flight, given up at its end (Minor 1);
  - two `ACKNOWLEDGEMENT_STORE_UNREACHABLE` answers redialling, and one followed by `OK` not redialling (Nit 2).
- `cargo test -p sagat-tests --test integration_durable_execution`:
  - `pg_terminate_backend` on the acknowledgement's connection after `BEGIN`, answered `ACKNOWLEDGEMENT_STORE_UNREACHABLE` (Minor 4);
  - the authentication's lookups with `acquire_timeout` at 10 s, answered within the bound (Minor 5).
- `python3 scripts/check-architecture.py`: the negative tests for `withdrawn_on_hand_back` (Minor 2).
- `cargo test -p sagat-tests --test integration_platform_foundation`: Task 006-044 applied before the follow-up's migration, then the rollback to the floor, then the floor's `control-plane migrate` (Minor 7).
- The barrier cases of Task 006-051, with the trigger disabled where Minor 3 says.

## Files consulted

- /private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/delivery-ack-round13-to-14.diff
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/round-13/correctness.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.down.sql
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/instances.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/identity.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/pool.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/lib.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/identity.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-http/src/extract.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/service.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/limits.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/server.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/apps/control-plane/src/lib.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/apps/control-plane/src/main.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/rust/conformance/src/faults/grpc.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/scripts/check-architecture.py
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/.claude/skills/migration/SKILL.md
