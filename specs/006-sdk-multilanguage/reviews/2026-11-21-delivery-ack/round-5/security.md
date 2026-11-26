# Security review, round 5: delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)

**Verdict: no Blocker, 1 Major, 3 Minor, 4 Nit.**

All five round-4 Minors and all four Nits are handled. The round-4 Major (the floor build reading the session row) is resolved for processes of the floor build. The new Major is the same hole seen from this build: during a rolling upgrade or a rolling rollback, this build withdraws a 1.8 hand-back while processes of the Architecture decision 0098 build are still handing out capabilities without recording them.

"Verified" below means I read the cited code in `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack`, or in `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases` where named. "Design reading" means the defect is in the documents alone.

---

## Status of round 4

### My report

| Round-4 finding | Status | Reason |
|---|---|---|
| Major 1: the floor decides the hand-back from the session row | **Resolved for the floor's own processes; residue in a mixed fleet** | Request (3) of Decision 006-059 asks for the clamp. Precondition (a) of the task phase requires it answered before merge, and Task 006-066 has the session stored at 8, with and without a capability. The server branch does not clamp yet (`/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/worker/withdrawal.rs:181`). That is expected, since this is a request and the precondition holds the gate. The residue is new Major 1. |
| Minor 1: root `WriteBlob` commits no transaction | Resolved | Decision 006-050 has the check transaction commit the record before the upload. Task 006-052 names the root and scoped `WriteBlob` tests followed by a hand-back. |
| Minor 2: "audited once" has no mechanism | **Partly resolved** | The mechanism is named and the refusal path commits its audit. Task 006-053 has the two 100-call tests. Not settled: where the generation in the reason comes from, the distinct cap, the resource, and the contract text (new Minor 3). |
| Minor 3: admission formula | Resolved | I checked `max(D, min(L - 1, D + max(1, (L - D) / 2)))` against `D = max(1, L * 3 / 4)` (`crates/api-grpc/src/limits.rs:595`) for limits 1 to 13. The acknowledgement band is D at reserves of 0 or 1 and D + floor(R/2) above that. So the requests in flight never exceed L, and exactly ceil(R/2) places stay for the class `Lease` whenever R ≥ 1. Tested as properties for limits 1 to 64. |
| Minor 4: transient fallback that a neighbour can cause | **Partly resolved** | The triggers are narrowed, calls are counted instead of leases, the decline is bounded (10 minutes, doubling up to an hour), and it is alerted (`SagatWorkersDecliningAcknowledgement`, `worker_registrations_declining_total`). The remaining trigger ("no answer at all within 6 s while other calls were answered") can still be caused by server slowness, because the server does not bound its own answer (new Minor 2). |
| Minor 5: flagged lease received while declining | Resolved | `sdks/SPEC.md` §6.2, "The host at 1.8" item 1, the failure table, and the conformance case. |
| Nit 1: bound on the cross-tenant record | Resolved | 32 places and 5 s, the gate of `crates/api-http/src/denial.rs:35-40` moved to the application layer. Past the gate the record is counted and dropped, the same accepted pattern as that gate. |
| Nit 2: join on tenant and run; `runs` read, not locked | Resolved | Step 4 and Decision 006-051. |
| Nit 3: `session_token` | Resolved | The rule is now "every field ending in `_token`". |
| Nit 4: dependency table guarded only by its owner | Resolved | The table is removed. The pair is now a constant of `platform_requirements_dependency_guard` (`ENABLE ALWAYS`), held equal to the entries' `requires` by `scripts/check-architecture.py`. |

### Findings of the other round-4 reports in my domain

| Finding | Status |
|---|---|
| Design Major 1 (a lost acknowledgement becomes a false "nothing ran") | Resolved for promotion and for point-in-time recovery by the timeline (Decision 006-062). The losses that keep the timeline are new Minor 1. |
| Design Major 2 (fallback triggered by the server's trouble) | Resolved, with the residue in new Minor 2. |
| Architecture Major 1 (the switch changes a mark's meaning) | Resolved: the switch does not reach a marked lease. No security residue. |
| Architecture Major 2 (the pass writes without the share-locked read) | Resolved. Verified that `lease_withdrawal_required()` exists: it takes `FOR SHARE` on the requirement row, is `SECURITY DEFINER` with `search_path = pg_catalog, pg_temp`, its execute right is revoked from `PUBLIC` and granted to `sagat_app` only (`/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql:71-85`). The pass takes the run lock and then that row. A removal takes the epoch row and then that row. Neither takes the other's first lock, so there is no cycle. |
| Correctness, architecture and design Majors 3 (the cap read in the claim) | Resolved by moving the cap into the withdrawal command. At the cap a hand-back spends the attempt, which is the safe direction. |

---

## Major

### Major 1. In a fleet that mixes this build with the build of Architecture decision 0098, this build withdraws a 1.8 hand-back after the older build handed out a capability it never recorded (design reading, against verified code)

**Where.**
- `stream-completion.md`, "The hand-back at 1.8": the reservation needs only `lease-withdrawal-v1`.
- Architecture decision 0099, Consequences, "Compatibility": "a hand-back that reaches the older build is an ordinary failure, the safe direction."
- "Rollback" step 3: "so a capability served before the rollback can never be followed by a withdrawal."
- Decision 006-055 and Task 006-066.

**What the code has.**
- The capability paths of the floor build write no `lease_capability_generation` and refuse nothing as `TASK_NOT_ACKNOWLEDGED` (verified in round 4).
- This build offers 1.8 "on the proofs of its build alone" (decision 1). Its claim writes `lease_hand_back_generation` for every 1.8 session, marked or not.
- Its hand-back withdraws while `lease-withdrawal-v1` is recorded. Nothing makes it check that every process serving capabilities is of this build.

**Failing scenario (rolling upgrade from the floor build, before activation).**
1. Control planes A (floor build) and B (this build) serve the worker API behind one address. A host registers on B at 1.8.
2. An engine of this build claims a lease for it, unmarked because nothing is activated yet, with `lease_hand_back_generation = g`.
3. The host is buggy or hand-written (the round-3 Major 2 threat). Its handler calls `ResolveSecrets`. The call reaches A, which hands over the values and records nothing.
4. The host then sends `FailStep(TASK_NOT_STARTED)`. The call reaches B. The step is `LEASED`, not acknowledged, with no capability recorded and a reserved generation, so B withdraws it (`NOT_STARTED`).
5. Result: the attempt number and idempotency key are reused, there is no review and no violation, and the journal says nothing ran although credentials were handed out.

The same holds after "Rollback" step 1. Removing `delivery-acknowledgement-v1` lets floor processes register again. Meanwhile the remaining processes of this build keep withdrawing hand-backs of leases that still carry `lease_hand_back_generation`. Those leases are not counted by `worker_leases_marked`, so step 2 does not wait for them.

**Fix (smallest).**
- Honour the 1.8 reservation only while `delivery-acknowledgement-v1` is recorded. Read it in the hand-back's transaction under the share lock of its row, with a twin of `lease_withdrawal_required()`. Otherwise the hand-back is an ordinary failure.
- This is exact in both directions:
  - Activation proves that no live process lacks the capability, and the registry refuses one that does not list it.
  - A removal waits for any hand-back that holds the share lock. A floor process admitted after the removal can only serve a lease that was not withdrawn before it.
- The cost is that a 1.8 hand-back before activation spends the attempt, which is the meaning the design already accepts on the floor.
- Add a mixed-fleet case to Task 006-066, before activation and after removal: `ResolveSecrets` served by a floor control plane, the hand-back received by this build, the result an ordinary failure.

---

## Minor

### Minor 1. Two common ways of losing a committed acknowledgement keep the timeline, and only the logical dump is named (verified against the runbook)

**What is wrong.** The timeline rule (Decision 006-062, Gap 006-048) covers promotion and archive recovery. Two other cases lose recent commits on the same timeline:
- **Asynchronous commit.** A primary that crashes with `synchronous_commit = off` (a setting per role or per database, and a common tuning) loses its last commits. Crash recovery keeps the timeline. With `fsync = off` the loss is unbounded.
- **Storage snapshot restore.** `docs/runbooks/disaster-recovery.md:36` allows "your platform's snapshot tool" for base backups. Restoring a volume snapshot as it is, without `recovery.signal`, runs crash recovery on the old timeline. Section 7 then asks for `end_lease_marks()` only "by any other means, such as a logical dump", and the operator has to work out that a snapshot restore falls under it.

**Scenario.** An acknowledgement commits and the host starts the handler. The primary crashes before the asynchronous WAL flush, or the volume is restored from a snapshot taken before that commit. The database now holds a provisional mark on the current timeline. The pass withdraws it as `NOT_ACKNOWLEDGED`: the handler runs again with the same attempt and idempotency key, and no review is opened.

**Fix.**
- The engine's probe also reads `current_setting('fsync')` and `current_setting('synchronous_commit')`, and marks nothing while either is `off` (a new skip reason).
- The acknowledgement's settings statement raises `synchronous_commit` to `local` for its own transaction when it finds `off`. It never lowers `remote_apply`.
- Section 7 runs `end_lease_marks()` after every restore, whatever the means, before the engines start. It only gives up protection for leases in flight, so running it always costs little.
- Gap 006-048 then covers only a skipped step. Add one failure row for each case.

### Minor 2. The server does not bound its own answer below the host's attempt bound, so server slowness, including a neighbour's load, still reads as "the path swallows the call" (design reading, against verified pool configuration)

**What is wrong.**
- Step 1 checks the 3 s deadline only before `acknowledge`.
- Before that check come:
  - the pool acquire, which defaults to 3 s (`crates/infra-postgres/src/pool.rs:32`);
  - `matched`;
  - the binding advisory lock and the pattern read;
  - the secrets lock;
  - the membership lock;
  - `lock_matched`.

  Each statement and lock wait is bounded only by the 1 s timeouts.
- So an admitted call can go past the host's 6 s attempt bound without any status. Meanwhile the host's single-transaction heartbeats are still answered.

**Scenario.** Another tenant's load holds the shared pool close to exhaustion, or slows the database. Three acknowledgement calls of a victim host in a row get no answer within 6 s while its heartbeats are answered. The third fallback trigger fires and the host declines for 10 minutes, doubling up to an hour while the load lasts. Its leases fall back to lapse-into-review. The decline is alerted, but the neighbour caused it.

**Fix.**
- Bound the whole handler, from admission to commit, by the 3 s deadline: a timeout around the service call that answers `UNAVAILABLE` (which never counts), the pool acquire bounded by the time left, and each statement's `statement_timeout` set to the time left.
- An answer then always arrives within half the receive bound, and "no answer at all" really means the path dropped the call.
- Test: a pool held full for 5 s gives `UNAVAILABLE` within 3 s and no fallback.

### Minor 3. The audit key for the new records is not pinned down, and on the stale path a token holder can choose the text (verified against `identity.rs` and `audit.rs`)

**What the code has.**
- The rule collapses on tenant, actor, action, `resource_id` **and** `reason` (`crates/infra-postgres/src/identity.rs:270-277`).
- The number of different records per key is capped only for actions with a `distinct_cap`: today only `ScriptViolation` and `SecretDeliveryRefused`, at 100 (`crates/domain/src/audit.rs:311-316`).
- The existing capability audits build the reason from the request's fence, for example `task.fence.generation` (`crates/application/src/worker/database.rs:118-123`). That is safe there because the fence matched.

**Scenario.**
- On the stale path the token matched only `withdrawn_lease_token_hash`, not the current fence.
- If the implementer follows the existing pattern and takes the generation in the reason from the request, a host holding a withdrawn token can change `generation` on every call. Each call is then a new record, so the audit trail grows without bound at the admission rate.
- Also, `resource_id` is not named. If it is the run or the namespace, the `count(*)` that each insert evaluates scans every record of that key.

**Fix.**
- Build the reason only from stored values: the row's step id and its stored generation.
- Use the step as the resource (run and step).
- Add the new actions to `collapses_repeats`, with `repeat_window` `None` and a `distinct_cap` of 100.
- Align the contract. `stream-completion.md:642` still says "once per step and withdrawn token" and does not name the mechanism.
- Extend Task 006-053: 100 stale calls with one withdrawn token and 100 different request generations give one record.

---

## Nit

1. **`end_lease_marks()` has no stated privileges.**
   - "Owner function" does not say `REVOKE ALL ... FROM PUBLIC` (PostgreSQL grants execute to `PUBLIC` by default; every function in the migrations revokes it, for example `20270114000000_trigger_handover.up.sql:240-248`), nor invoker or definer, nor search path. Defined with `SECURITY DEFINER` and no revoke, any role that can connect could end every tenant's marks.
   - Make it `SECURITY INVOKER` with `SET search_path = pg_catalog, pg_temp`, refer to `public.steps` by full name, add `WHERE lease_offered_timeline IS NOT NULL`, revoke it from `PUBLIC` and grant it to nobody.
   - Have the privilege audit assert that `sagat_app` cannot run it.
   - It is a platform-wide manual override (constitution principle VI), so it should write one history or audit row with `current_user` and the number of marks ended.
2. **`sagat_database_timeline()` declaration.**
   - Declare it `VOLATILE`, never `IMMUTABLE`, which a cached generic plan could fold into a constant.
   - Call `pg_catalog.pg_walfile_name` by full name.
   - Revoke and grant it as the other functions are.
   - A `NULL` timeline (left by `end_lease_marks()`) must count as "earlier" in both the SQL macro and the pure function, and the parity test should include it.
   - Suspicion to confirm in Task 006-044: to my knowledge PostgreSQL 16 does not revoke `pg_current_wal_lsn()` or `pg_walfile_name()` from `PUBLIC`.
3. **The reservation is not bound to the timeline.** An asynchronous failover can lose a committed `lease_capability_generation` of an unmarked 1.8 lease. A buggy host's later hand-back is then withdrawn. Stamp the timeline whenever `lease_hand_back_generation` is written, and make a hand-back of a lease claimed on an earlier timeline an ordinary failure.
4. **The application-side request type of the acknowledgement must not derive `Debug`.** The descriptor rule covers only protobuf messages. `TaskRef` deliberately has none (`crates/application/src/worker.rs:482-489`). Keep the new type the same way and use `skip_all` on its span.

---

## Classes checked with no new finding

- **Can a hostile worker or a neighbouring tenant abuse the timeline?** No.
  - The timeline belongs to the database. No tenant or worker can move it.
  - An earlier-timeline mark fails toward today's meaning: the lease lapses, an unknown outcome, and is never withdrawn.
  - The `recovered` verdict reaches only holders of the token.
  - A worker cannot acknowledge, or cause to be withdrawn, another tenant's lease.
  - `declines_acknowledgement` changes only the caller's own row, keyed by tenant, subject and worker name (`crates/infra-postgres/src/workers.rs:338-343`).
- **Capability record: race and cost.**
  - The record and the hand-back both take the run lock (or the path lock for scoped instances), so they cannot interleave. The acknowledgement cannot race the record, because a provisional lease is refused before any record is written.
  - Updates that touch only the marks do no quota work: `sagat_quota_row_delta` compares the measured columns first (`20261024000000_durable_quotas.up.sql:204-212`). The admission and charge triggers fire only on `status` and identity columns.
  - `ReadBlob` commits its record in `audit_blob_read` before the stream starts (`crates/api-grpc/src/service.rs:1339-1371`).
  - One correctness note for the build: the "once per generation" condition and the "lease still held" condition must be separate. Otherwise a second call that updates zero rows is refused as not held.
- **Tenant boundary.**
  - The new statements are keyed by the caller's tenant.
  - `TASK_NOT_ACKNOWLEDGED`, `RETRY`, `CANCELLED` and `recovered` are given only after the token and the binding matched, so there is no oracle.
  - The cross-tenant record is gated.
- **Least privilege.**
  - The dependency trigger is an invoker function that fires only inside the owner's `SECURITY DEFINER` operator functions or the owner's own SQL.
  - `lease_withdrawal_required()` is verified as above.
  - No new grant is given to `sagat_app` beyond the timeline function.
- **Injection and parsing.** No dynamic SQL. The timeline comes from a server function, not from input. The request bounds are unchanged.
- **Logs and metrics.** Labels are closed sets. The `_token` rule covers both session and lease tokens. Audit reasons name the generation, never the hash.
- **Rollback with the clamp.** It is correct for floor processes. The mixed part is Major 1.

## Commands to run once the build exists

- `SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test -p sagat-tests --test integration_durable_execution`
- `cargo test -p sagat-tests --test contract_grpc_durable_execution`
- `SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test -p sagat-tests --test integration_platform_foundation` (the privilege audit, including that `sagat_app` cannot run `end_lease_marks()`)
- the fault suite of Task 006-051
- the rollback case of Task 006-066, with the mixed-fleet case of Major 1

## Files

- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/disaster-recovery.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/identity.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/domain/src/audit.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker/database.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/service.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/build.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/pool.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/workers.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20261024000000_durable_quotas.up.sql`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270114000000_trigger_handover.up.sql`
- `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/worker/withdrawal.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql`
