# Security review, round 14: delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)

**Verdict: no Blocker, no Major, 1 Minor, 4 Nits.**

All three of my round-13 Minors and my round-13 Nit are resolved. The fix for Minor 2 is only half carried through the documents, which is new Minor 1. "Verified" below means I read the base code in `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack`. "Design reading" means the defect is in the documents alone.

## Status of round 13

| Round-13 finding | Status | Reason |
|---|---|---|
| Minor 1: `ACKNOWLEDGEMENT_STORE_UNREACHABLE` could not be produced, because authentication reached the database first | **Resolved** (verified) | `verify` (`crates/api-http/src/auth/verifier.rs:154-219`) is a JSON Web Token check against the JSON Web Key Set cache and makes no database call. So "an unverified token still gets UNAUTHENTICATED without touching the database" is true. The only database work before the handler is `authenticate`'s two lookups (`crates/application/src/identity.rs:252-256`), which come after verification and after the per-subject budget (`crates/api-http/src/extract.rs:56-65`), so the new mapping applies exactly there. It creates no oracle: a verified caller who is not a member already got the generic `UNAVAILABLE` during an outage, and the new reason says nothing more. Task 006-050 covers the full pool at the authentication stage and the read-only case. The two-engine harness holds the whole pool (`tasks.md`, Task 006-058). A precision gap in "a store error the registry counts as unreachable" is Nit 2. |
| Minor 2: the stale path's journal read had no index | **Resolved as to the read**; the new column is incomplete (Minor 1) | The stale path now reads `withdrawn_on_hand_back` on the row it already holds (`stream-completion.md:493`, `:766`). There is no journal read, the column count is corrected to nine, and the cost is constant whatever a holder of a withdrawn token sends. |
| Minor 3: the acknowledgement connection was not required to be built like the main one | **Resolved** | `sdks/SPEC.md:365-370` and `stream-completion.md:603-605` require the same function, with the same target, transport credentials, TLS settings, server name, dial options and interceptors. Task 006-059 adds the test against a private certificate authority that requires a client certificate, with the interceptor check. |
| Nit 1: Task 006-046's barrier expectation | **Resolved** | `tasks.md` Task 006-046 now expects `INSTANCES_LACK_CAPABILITY` naming the engine. Decision 006-059 (7) states that this depends on READ COMMITTED (`research.md:817-822`). |

I also checked the narrowing "once any `NOT_ACKNOWLEDGED` withdrawal exists" (ADR line 302, `research.md:754`, `:860`). It is sound now that the `BEFORE DELETE` refusal exists:
- `lease-withdrawal-v2` can be removed only after `delivery-acknowledgement-v1` has been removed.
- With `delivery-acknowledgement-v1` gone, no mark counts.
- So a binary without request (7) admitted after that removal serves capability calls only to ordinary leases, as before 1.8.
- I verified in round 13 that the codec replays every cause of Architecture decision 0098 on the floor build (`crates/infra-postgres/src/event_codec.rs:538-546`).

---

## Minor

### Minor 1. `withdrawn_on_hand_back` is listed as a mark in one table only. The decision that defines the marks, the allowed-forms rule behind the static check, the withdrawal outcome rows and the withdrawal test leave it out. So the two columns can come apart, and the `NOT_ACKNOWLEDGED` heartbeat alert can be silenced (design reading)

**Where:**
- The column table lists it: `stream-completion.md:493`, "in one assignment".
- The allowed forms omit it (`stream-completion.md:560-566`). They say only "the withdrawal sets `withdrawn_lease_token_hash` from the row it holds".
- Decision 006-042, which lists the marks, omits it (`research.md:576-580`).
- The pass's outcome row omits it (`stream-completion.md:682`). It says "the lease and its marks cleared, `withdrawn_lease_token_hash` kept", which excepts only the hash from "its marks".
- The hand-back row omits it (`:685`): "`withdrawn_lease_token_hash` kept".
- Task 006-050 omits it (`tasks.md:298`): "`withdrawn_lease_token_hash` kept by every withdrawal of a marked lease".
- No text says how a NULL is read.

**Scenario:**
1. The implementer builds the withdrawal from the outcome rows. The pass's `NOT_ACKNOWLEDGED` withdrawal clears "its marks" (so the boolean becomes NULL) and keeps the hash.
2. Alternatively, a later edit sets the boolean in a statement separate from the hash. The static check does not list the column, so it accepts any form.
3. A host that breaks the protocol (Gap 006-042) runs a handler without `ACKNOWLEDGED` and heartbeats with the withdrawn token.
4. The stale path matches the hash and reads the boolean as NULL, or as the value left by an earlier hand-back. If either is taken as a hand-back, the call is counted `heartbeat_after_hand_back`.
5. That count is audited but not alerted (`stream-completion.md:762-766`). So `SagatCallsAfterWithdrawal`, which the design calls "the earliest sign of a handler run without an acknowledgement", does not fire.

Nothing runs that would not have run anyway, and the stale answer is unchanged, so no guarantee breaks. What is lost is the detection signal, which is why this is Minor.

**Fix:**
- Add `withdrawn_on_hand_back` to Decision 006-042's list.
- In the allowed forms, write one form: the withdrawal sets `(withdrawn_lease_token_hash, withdrawn_on_hand_back)` together, `false` for `NOT_ACKNOWLEDGED` and `true` for a hand-back. Every other statement keeps both (`ELSE` the column).
- Make the static check fail on an assignment of either column without the other, with a negative test.
- In the outcome rows of `:682` and `:685`, write "`withdrawn_lease_token_hash` and `withdrawn_on_hand_back` kept".
- State that a NULL is read as `heartbeat` (alerted), so a missing value fails toward the alert.
- Extend Task 006-050's case to assert the pair after each kind of withdrawal, including a hand-back followed by a `NOT_ACKNOWLEDGED` withdrawal of the next generation.

---

## Nits

1. **A stale definition in the same decision** (`research.md:927-933`, design reading).
   - The decision still defines `ACKNOWLEDGEMENT_STORE_UNREACHABLE` as "the server's reason when its deadline passed before it held a connection or its connection failed".
   - It still counts the blocking tries "since the evidence".
   - Fourteen lines later (`:941-947`) the same decision gives the round-14 definitions: the authentication stage, `25006`, and "between the latest evidence before the deciding attempt and its end".
   - Fix: replace the earlier clause with a pointer to the later one.

2. **"A store error the registry counts as unreachable" cannot be read at the authentication stage as written** (`stream-completion.md:614-617`, `research.md:941-944`; verified).
   - The registry's classification (`crates/infra-postgres/src/instances.rs:73-93`) is: input/output, TLS, pool timeout, pool closed, worker crashed, class `08`, and `57P01` to `57P03`.
   - The identity store merges those with classes `40`, `53`, `55` and `57` into one `StoreError::Unavailable` (`crates/infra-postgres/src/identity.rs:818-837`). That includes `57014`, a statement timeout on an engine that did reach the database.
   - `authenticate` then returns `AppError::Unavailable` with no way to tell them apart.
   - The safe direction holds: too broad a mapping only makes the host wait. But Task 006-050's test cannot pin the boundary.
   - Fix: either carry a "not reached" variant through the identity port, or state that at the authentication stage any `AppError::Unavailable` maps to the reason.

3. **The new trigger function's hardening is not written** (`tasks.md:199-204`, `stream-completion.md:411-417`; design reading, base verified).
   - Task 006-046 pins `search_path` and `public.` names for the requirements read but says nothing for the trigger function, which this round extended to take the epoch row `FOR UPDATE` and to read `serving_instances`.
   - The base revokes its guard functions from `PUBLIC` (`20270113000000_instance_registry.up.sql:393-395`).
   - The trigger exists for hand SQL, the one path whose isolation the command does not set. Under `REPEATABLE READ` the census of `serving_instances` uses the transaction's first snapshot.
     - Scenario: hand SQL records `lease-withdrawal-v2` and `delivery-acknowledgement-v1` in one transaction. A binary lacking `lease-withdrawal-v2` registers and commits between that snapshot and the epoch lock (its registration waits only on `FOR SHARE`).
     - Result: the census misses it.
     - This is narrow and needs the operator, but the check costs one line.
   - Fix:
     - Write `SET search_path = pg_catalog, pg_temp`, `public.` names and `REVOKE ALL ... FROM PUBLIC` for the trigger function.
     - Have it raise unless `current_setting('transaction_isolation') = 'read committed'`.
     - Add that refusal to the Gate row.

4. **"Doubled" understates a host's connections during a redial** (`sdks/SPEC.md:371-374`, `stream-completion.md:607-610`, `tasks.md:359-361`; design reading).
   - An old acknowledgement connection is closed only once its attempts in flight have ended.
   - A further redial can come while one is still draining (two `ACKNOWLEDGEMENT_STORE_UNREACHABLE` answers from the four permits).
   - So a host can hold its main connection plus up to five acknowledgement connections for one attempt's deadline.
   - After a promotion, a fleet behind one address can push the per-peer count (`crates/api-grpc/src/limits.rs:274`, default 256 at `server.rs:87`) past twice the fleet. The refusal then also hits main connections, including those of other tenants behind that address. This is availability only, and transient.
   - Fix: dial a new acknowledgement connection only while no older one is draining, so a host holds at most three. Write that bound in the runbook text of Task 006-054 and in the failure row at `stream-completion.md:914`.

---

## Suspicion, not verified

- **Beats holding off the epoch lock** (carried over). With `FOR SHARE SKIP LOCKED` the beat now never waits, but every beat still holds the share lock until it commits. An activation therefore waits for the slowest beat in flight. The effect is availability only.
  - Command, once built: the 1,000-instance scalability profile, timing `control-plane capability-activate` under `pgbench` load.

## Classes checked with no new finding

- **Tenant boundary:**
  - The authentication-stage reason comes only after the token verifies, and it says no more than the generic `UNAVAILABLE` did.
  - The per-connection counter in `CappedConnectInfo` (`crates/api-grpc/src/limits.rs:327-404`) is read in process by the test platform (`tests/support/worker.rs:37`). It must never reach a response, a log field or a metric label; the design says "never an address", which is enough as long as that holds.
  - The per-peer cap is keyed by internet protocol address (`limits.rs:274`). A refusal there makes the host fall back to declining, which is the behaviour below 1.8.
- **Authorization and admission:** no new operation. The `BEFORE DELETE` refusal closes the remove-and-record-again route around the activation refusal.
- **Least privilege** (verified):
  - `sagat_app` cannot insert into or delete from `platform_requirements` (`instance_registry.up.sql:404`, `SELECT` only).
  - The trigger's `FOR UPDATE` of the epoch therefore runs as the owner, either inside the `SECURITY DEFINER` activate and remove functions (`:314-323`, `:359-364`) or in the owner's hand SQL.
  - The beat takes `FOR SHARE` inside its `SECURITY DEFINER` function.
  - `TRUNCATE` stays refused (`:149`, `:163`).
- **Secrets and logs:**
  - Running the user's interceptors on acknowledgements exposes nothing they do not already see on the main connection.
  - The new boolean is not sensitive.
- **Injection and parsing:** no dynamic SQL and no new input. Merged calls stay capped at 64 leases. Redials are bounded by the four permits and the backoff.
- **Races** (verified against the base lock order):
  - The removal function takes the epoch first and only then deletes, so the trigger's own `FOR UPDATE` adds nothing.
  - A withdrawal holds `lease-withdrawal-v2` `FOR SHARE` and never the epoch, so there is no cycle.
  - Hand SQL that deletes without `platform_requirement_lock` takes the row lock before the trigger's epoch lock. A concurrent activation then deadlocks, and PostgreSQL aborts one of the two transactions, which fails closed.
  - Under READ COMMITTED, the trigger's check of `delivery-acknowledgement-v1` is a new statement after the lock, so it sees an activation that committed while it waited.

## Commands to run once built

- **Minor 1:** in `integration_durable_execution`:
  1. A hand-back withdrawal of generation 1, then a `NOT_ACKNOWLEDGED` withdrawal of generation 2.
  2. Query `SELECT withdrawn_lease_token_hash IS NOT NULL, withdrawn_on_hand_back FROM steps WHERE ...` after each withdrawal.
  3. Expect `(true, true)` and then `(true, false)`, and `SagatCallsAfterWithdrawal` firing on a heartbeat with the token of generation 2.
  4. Add `python3 scripts/check-architecture.py` with a negative fixture that assigns the boolean alone.
- **Nit 3:** as the owner, `BEGIN ISOLATION LEVEL REPEATABLE READ;` then the hand SQL of `docs/runbooks/instance-registry.md` section 5 inserting `delivery-acknowledgement-v1`. Expect it to be refused.

## Files

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-http/src/auth/verifier.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-http/src/extract.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/identity.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/identity.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/instances.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/limits.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/server.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/service.rs
