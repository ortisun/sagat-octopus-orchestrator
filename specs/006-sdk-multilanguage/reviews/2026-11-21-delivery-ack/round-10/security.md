# Security review, round 10: delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)

**Verdict: no Blocker, 1 Major, 3 Minor, 8 Nit.**

Round 10 answers my round-9 Major 1 in substance, but only partly in specification. Making the beat read the epoch `FOR SHARE SKIP LOCKED` is sound. The new defects are:
- **The Major:** the fallback branch of Decision 006-059 breaks the rollback floor that the record says is enforced.
- **Minor 1:** what a beat that returns no epoch means is not pinned down. One reading brings back the round-9 outage through readiness; another reopens the round-8 hole.
- **Minor 2:** the new form of `instance_heartbeat` comes without the constraints that keep it callable by `sagat_app`.
- **Minor 3:** the evidence rule added for round-9 Minor 1 pages operators on hosts that keep the protocol.

"Verified" below means I read the code in `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack`, or the tonic 0.14.9 source. "Design reading" means the defect is in the documents alone.

## Status of round 9

| Round-9 finding | Status | Reason |
|---|---|---|
| **Major 1:** `FOR SHARE` puts every beat behind an open owner transaction | **Partly resolved** | The mechanism is right: the epoch is read first, `SKIP LOCKED`, `last_seen` is committed and no epoch is returned. `sagat_app` cannot lock the epoch table itself: it has only `SELECT`, and every locking clause needs `UPDATE`. So no runtime-role holder can make beats skip. Two gaps remain. What a beat with no epoch does to readiness, to the 25 s window and to an existing decision is open (Minor 1). The function's privilege constraints and the 60 s hold test are missing, and Task 006-046 still asserts that the beat waits (Minor 2). |
| Minor 1: withdrawing an acknowledged `LEASED` lease rests on weak evidence | **Resolved** | (a) The token hash is kept for `NOT_STARTED`, and later calls are counted, audited and alerted. (c) Decision 006-045 and decision 12 are corrected. (d) The first-heartbeat alternative is left to the owner. (b) is only partly done (Nit 5). Item (a) creates Minor 3. |
| Minor 2: a recovered lease's hand-back | **Resolved** | Decision 7, Decision 006-045, Decision 006-051, the 037 clarification and a test case now agree. There is a host-text residue (Nit 6). |
| Minor 3: the deadline cannot sit before tonic's per-connection limit | **Resolved** | The platform's own layer sits under the stamping layer, with a reserve, and is tested with 64 transfers and more than 128 calls. An implementation constraint is in Nit 1. |
| Minor 4: the merged floor does not answer the requests | **Resolved for the preferred path** | Each request's status is listed, and the new migration form with the `prosecdef` and `proconfig` tests is specified for (6) and (8). The fallback branch is new and is Major 1. |
| Nit 1: stale lines | Resolved | |
| Nit 2: the probe's answer | Resolved | It is evidence only, with the reason `withdrawn`. Residue: Nit 3. |
| Nit 3: a stop planned for a compensation | **Partly resolved** | The property test is named only in Decision 006-051 (`research.md:696`) and in no task's evidence (Nit 7). |
| Nit 4: stopping the engines before a restore | Resolved | `disaster-recovery.md:47` says it in step 1. |
| Nit 5: the incarnation function's privilege | Accepted, not carried | Task 006-044 checks the pinned image. A revoked function makes every claim fail loudly, which fails closed. |

Others' round-9 findings in my domain:
- **Correctness Major 1 and architecture Minor 1 (the attempt flag's form):** resolved. The bound `$flag` is fenced on the old row, and a stale caller can only bind false, which is the safe direction. The static check's negative cases match.
- **Correctness Major 2 (early give-up during a failover):** resolved by the condition that another call of the host was answered `OK`. No security residue.
- **Correctness Minor 4 (my Minor 1) and correctness Minor 6 (my Minor 3):** resolved as above.
- **Correctness Nit 7 (lock order):** resolved. The epoch is now read first.
- **Design Major 1 (the preconditions):** resolved for the preferred path; the fallback is Major 1.
- **Design Nit 7 (`REGISTRY_SILENT` rows for the SDKs):** resolved. The residue is Nit 4.
- **Architecture Minor 5 (the 5 s margin):** resolved by the registry connection's own statement timeout.
- **Architecture Nit 4 (the probe counted with forged tokens):** resolved by the reason `withdrawn`.

---

## Major

### Major 1. In the fallback of Decision 006-059, the rollback's own removal re-admits the previous release. That release can neither replay `NOT_ACKNOWLEDGED` nor decode the new beat (verified)

**Where.**
- `research.md:806-811` (the fallback).
- `tasks.md:144-148`, precondition (a).
- ADR Rollback paragraph (`ADR-0099-…md:270-276`): "this design's own dormant build becomes the floor and the requirement is recorded for both kinds … so the registry never admits a binary below that build once any withdrawal exists".

**What the code has.**
- The released pull request #260 build (the fallback assumes it ships without (2)) lists `lease-withdrawal-v1` (`crates/application/src/instances.rs:121`, `:140`).
- Its codec rejects any cause other than `NOT_SENT` and `NOT_STARTED`: `decode` returns `None` (`crates/infra-postgres/src/event_codec.rs:495`, test at `:1519`).
- It decodes `out_epoch` as a non-null `i64` (`crates/application/src/instances.rs:212`; `crates/infra-postgres/src/instances.rs:325`).
- Its fail mode closes the whole gate after 30 s of any beat failure other than a lost connection (`crates/application/src/instances.rs:31-33`).

**Scenario.**
1. The fallback is taken. Pull request #260 is released without (2) and (7). This design's dormant build is deployed, and later `delivery-acknowledgement-v1` is activated for both kinds.
2. Any tenant produces `NOT_ACKNOWLEDGED` withdrawals at will: its own worker simply never acknowledges. Frames lost for honest tenants produce them too.
3. The operator rolls back. The switch goes off, the leases drain, and `delivery-acknowledgement-v1` is removed, as the rollback requires. The dormant build does not list that capability before Task 006-057, so it must be removed.
4. The only requirement left is `lease-withdrawal-v1`, which the #260 release lists. An engine or control plane of that release, the natural "previous release", is admitted. Every run whose journal holds `NOT_ACKNOWLEDGED` then fails to replay on it.
5. Separately, during the rolling upgrade from the #260 release, its engines beat against the replaced `instance_heartbeat`. A hand-SQL owner transaction held open for more than 30 s returns NULL to them, the decode fails, and every one of their gates closes. That is round-9 Major 1 again, for that release.

So "enforced, not described" holds only in the preferred branch. Recording the requirement for both kinds excludes the #260 release only until the rollback removes it.

**Fix (smallest).**
- In the fallback, the dormant build lists a new capability of its own (for example `withdrawal-cause-replay-v2`) for both kinds.
- The activation entry of `delivery-acknowledgement-v1` refuses until that capability is recorded for both kinds.
- Its removal gets the replay-floor guard of Architecture decision 0098 decision 8: refused while any journal holds the cause `NOT_ACKNOWLEDGED`.
- Make the beat change an overload, `instance_heartbeat(uuid, bytea, boolean)`, revoked from `PUBLIC` and granted to `sagat_app`, `SECURITY DEFINER`, `search_path = pg_catalog, pg_temp`. The two-argument function keeps its non-null contract for older binaries.
- Say that `delivery_acknowledgement_required()` reads "recorded for engines" as `'EXECUTION_ENGINE' = ANY (kinds)`, so a record for both kinds is not read as missing.
- Test: in the released-v0.2 cutover, the #260 release is refused registration after the rollback while a `NOT_ACKNOWLEDGED` event exists.

---

## Minor

### Minor 1. What a beat that returns no epoch means is not pinned down. Each plausible reading breaks something (design reading, checked against `instances.rs`)

**Where.** `stream-completion.md:427-434`; Decision 006-059 (7) (`research.md:796-803`); ADR decision 10 (`:158`).

**What the code has.** Readiness fails when the last successful beat is older than the liveness window, whatever the gate says (`crates/application/src/instances.rs:37-38`). The 25 s capability window is also measured from "the last successful beat". The text says only that a beat with no epoch means "capability calls undecided, never a registry failure".

**Readings.**
- **(A) A beat with no epoch is not a successful beat.**
  - Readiness fails across the fleet 30 s into a hand-SQL transaction held open, and the orchestrator pulls every engine out of service. That is round-9 Major 1 again.
  - Every activation by the command (up to about 8 s) also makes each engine that beats in it refuse capability calls until its next beat, 10 to 12 s later.
- **(B) It is successful, and it resets the 25 s window as a plain timer.**
  - A floor engine that lapsed at the census serves again during the open transaction, without a decision under any epoch.
  - After the commit it serves secrets to provisional leases that other engines marked, until its next beat. That is the round-8 hole that request (7) exists to close.

**Fix.** State it as a latch:
- A beat with no epoch is a successful beat for liveness, for readiness and for the rule that closes the gate.
- It keeps the engine's current capability decision, and extends its window, only while that window has not lapsed.
- It never opens capability calls on an engine whose window lapsed. That engine waits for a beat that returns an epoch and a decision under it.

Test: an owner transaction holds the epoch for 60 s. No readiness failure and no closed gate; engines already serving keep serving `ResolveSecrets`; an engine silent for 31 s before the hold refuses until after the commit and its own decision.

### Minor 2. The new `instance_heartbeat` comes without the constraints that keep it callable by `sagat_app`, and Task 006-046's evidence asserts the behaviour round 9 removed (verified)

**What the code has.**
- `sagat_app` has only `SELECT` on `platform_requirement_epoch` and on `serving_instances` (`20270113000000_instance_registry.up.sql:402-404`).
- The function is `SECURITY DEFINER SET search_path = pg_catalog, pg_temp` (`:252`).
- `CREATE OR REPLACE FUNCTION` replaces the security and `SET` clauses. Without them the function runs with the invoker's rights: its `FOR SHARE` and its `UPDATE` fail with permission denied for `sagat_app`. That is "any other failure", so every gate in the fleet closes 30 s after the deploy. Tests run as the owner would not see it.

**What the design has.**
- The `prosecdef` and `proconfig` requirement is attached only to (6) and (8) (`research.md:793-795`).
- Task 006-046 (`tasks.md:191-198`) still says "the liveness window less the beat call's bound", "checked again at the hand-over", and "a beat racing an activation … waits for the epoch and refuses after it". The last contradicts `SKIP LOCKED`.
- In the fallback, this task builds (7). Its evidence is therefore normative and tests the opposite of the design.

**Fix.**
- Extend the migration requirement of (6) and (8) to `instance_heartbeat`: a new migration, `SECURITY DEFINER`, the pinned path, `public.` names, the `prosecdef` and `proconfig` test, run with `SAGAT_TEST_AS_RUNTIME_ROLE=1`.
- When the row is not found, tell a skipped row from a missing one: `IF NOT FOUND AND NOT EXISTS (SELECT 1 FROM public.platform_requirement_epoch) THEN RAISE`. Otherwise corruption is masked as "an activation in progress" forever.
- Rewrite Task 006-046's evidence to match the text, and add the 60 s hold test of Minor 1.

### Minor 3. A host that keeps the protocol can trigger `SagatCallsAfterWithdrawal` through its heartbeat sent before the handler (design reading)

**Where.**
- `stream-completion.md:690-691`: when less than 3 s is left, the first heartbeat goes before the handler, and the host "hands the lease back if it fails".
- Marks table (`:452`) and decision 12 (`ADR:171`): the hash is now kept for `NOT_STARTED`.
- `:839` and `:857`: `worker_calls_after_withdrawal_total{kind}` has no heartbeat kind, so a heartbeat counts as `report`, and the alert fires on "any increase of the `report` … kind".

**Scenario.**
1. The heartbeat times out at the host with no answer, while it is still waiting at the server for a pool connection or the run lock.
2. The host hands back. The `FailStep` takes the run lock first and withdraws the lease as `NOT_STARTED`.
3. The heartbeat then arrives with the withdrawn token. It is counted, audited ("a handler may have run") and paged, although no handler started.

At fleet scale this recurs and teaches operators to ignore the one detective control for the trust assumption.

**Fix.** Count a `Heartbeat` after a withdrawal under its own kind, `heartbeat`, audited but not in the alert. A handler that really ran also sends a `CompleteStep`, a `FailStep` or a capability call, and those are counted and alerted. Add the race to Task 006-051.

---

## Nit

1. **The per-connection layer.**
   - tonic applies the layers of `Server::builder().layer(...)` once (`tonic-0.14.9/src/transport/server/mod.rs:812`). A tower `ConcurrencyLimit` added there is therefore global, not per connection.
   - Hold each connection's semaphore in `CappedConnectInfo` (`crates/api-grpc/src/limits.rs:327-404`), which is per connection and dropped with it. Say so, and forbid a map keyed by the peer's address: that layer runs before authentication, so any peer could grow such a map.
2. **The reason `withdrawn`** (`stream-completion.md:596`). Say it is decided inside `matched`, with `lease_token_hash = $h OR withdrawn_lease_token_hash = $h`, in the caller's tenant and by the same primary key. Then it adds no statement, no read across tenants and no difference in timing.
3. **The probe's "hands back again"** (`sdks/SPEC.md:392`). `RETRY` is not evidence that the hand-back did not land. Bound the rule to one more hand-back and no second probe, so a row held for a long time cannot loop the two.
4. **The second gate check, made before the answer,** applied to write calls. For `WriteBlob` and `AppendStepLogs` it answers `REGISTRY_SILENT` after the write has committed, and the new row of §5 (`sdks/SPEC.md:282`) makes the SDKs repeat it. The result is duplicate log lines and orphan blobs charged to the tenant. Limit the second check to calls that hand data out (`ResolveSecrets`, `ResolveDatabase`, `ReadBlob`), and check writes before the write.
5. **Round-9 Minor 1 (b), partly done.** The `mark` label is `marked` or `unmarked`, so an SDK that hands back acknowledged leases cannot be told apart from lost acknowledgements. An optional value `acknowledged` would show it.
6. **Host text.** `sdks/SPEC.md:384` and `:406` say "withdrawn whether or not it was acknowledged" without "while its mark counts".
7. **The domain property test** that a stop is never planned for a compensation or reconciliation belongs in the evidence of Task 006-049 or Task 006-050.
8. **Task 006-045 is stale.** It still says "confirmed … as the server build added it" (`tasks.md:186-187`). In the fallback it builds the cause.

---

## Classes checked with no new finding

- **Tenant boundary.**
  - `matched` and `lock_matched` stay scoped by tenant and primary key. The cross-tenant check and its audit are unchanged.
  - The new labels (`mark`, `stop`, `withdrawn`) are closed sets. `REGISTRY_SILENT` carries no tenant data.
- **Denial of service through the epoch.**
  - `sagat_app` cannot take any row lock on the epoch table directly. A runtime-role holder who parks a beat inside an open transaction holds only `FOR SHARE`, which `instance_register` already allows today (Gap 2000).
  - `SKIP LOCKED` skips only conflicting locks, so concurrent beats never make one another skip.
- **The attempt flag.** The one bound flag is computed under the run or path lock after the `FOR UPDATE` re-read. A wrong `true` only removes protection from the next lease, which then has today's meaning. It never spends an attempt.
- **The early give-up.** One tenant's load on a shared engine can make other tenants' hosts give up early. The cost is bounded at one hand-back per attempt, and the next lease is unprotected (Gap 006-047). That is already named in "Bad".
- **Secrets and logs.** Nothing new carries a token, a payload or an unbounded identifier.
- **Injection and parsing.** No dynamic SQL and no new unbounded input.

## Commands to run once built

- Minor 1 and Major 1 part 5: as the owner, run `BEGIN; SELECT platform_requirement_lock();` and hold it 60 s, then run `SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test -p sagat-tests --test integration_platform_foundation`. Expect no readiness failure and no closed gate.
- Minor 2: `SELECT prosecdef, proconfig FROM pg_proc WHERE proname = 'instance_heartbeat';` must show `t` and `{search_path=pg_catalog, pg_temp}`.
- Major 1: the released-v0.2 job's `provisional-lease-cutover` rollback case, with an engine of the #260 release registering after the removal while a `NOT_ACKNOWLEDGED` event exists. It must be refused.

## Files

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/disaster-recovery.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/instances.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/event_codec.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/instances.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/limits.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/server.rs
- /Users/marcelo/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/tonic-0.14.9/src/transport/server/mod.rs
