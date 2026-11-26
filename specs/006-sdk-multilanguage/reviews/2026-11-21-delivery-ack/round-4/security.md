# Security review, round 4: delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)

**Verdict: no Blocker, 1 Major, 5 Minor, 4 Nit.**

Both round-3 Majors are closed inside this design. The new Major is a gap in the build this design sits on. That build is the Architecture decision 0098 build, which is also the rollback floor. It still decides the reservation from the session row and knows nothing of `lease_capability_generation`. So both round-3 holes reopen whenever a process of that build serves a 1.8 session. The Minors are:
- one capability path that commits no transaction;
- an audit rule ("once per step and generation") with no mechanism behind it;
- an admission formula that breaks at small limits;
- two gaps around the declining fallback.

"Verified" means I read the cited code in `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack`, or in `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases` where named. "Design reading" means the defect is in the text alone.

---

## Status of round 3

### My report

| Round-3 finding | Status | Reason |
|---|---|---|
| Major 1: the reservation read from a session row that registering again changes | **Resolved in this design; residue in the floor build** | The claim writes `lease_hand_back_generation` in the three lease statements. The hand-back decides from it under the run lock (decision 9; "The hand-back at 1.8"; Decision 006-059 (3)). Task 006-050 has both re-registration cases. A deleted session no longer matters because the column is on the step. The residue is new Major 1: the Architecture decision 0098 build keeps `holder_minor` from the session row. |
| Major 2: capability then hand-back, and a refused capability call, not recorded | **Partly resolved** | Decision 8 and Decision 006-050 write `lease_capability_generation`. Decision 9 makes the later hand-back an ordinary failure and a violation, and decision 12 audits the refused call. Verified for the secrets path: it commits a final transaction, and values are returned only after it (`crates/application/src/worker/secrets.rs:147-219`). Also verified for ReadBlob: `audit_blob_read` commits before any byte is sent (`crates/api-grpc/src/service.rs:1339-1371`). Not covered: root `WriteBlob` commits no transaction (new Minor 1); "audited once" has no mechanism (new Minor 2); the floor build (new Major 1). |
| Minor 1: lock order and cost of a multi-namespace call | Resolved | One namespace per call, with `INVALID_ARGUMENT` before any lock (wire bullets, step 2). The binding and membership locks are taken before any row lock (step 3). Only the caller's own tokens decide, so there is no oracle. |
| Minor 2: coverage per row, node kind | Resolved | `first_uncovered` on each row's activity. Managed and non-activity rows answer `NOT_HELD`, and they are never marked anyway. |
| Minor 3: waiting on rows, where `lock_timeout` is set | Resolved | Both timeouts are set at step 1. `FOR UPDATE OF steps SKIP LOCKED` in primary-key order, `RETRY`, and the failure row is corrected. |
| Minor 4: acknowledgement with a withdrawn token counted as a call after a withdrawal | Resolved | It is counted apart as the kind `acknowledgement`, with no alert, in the failure table and in Task 006-053. |
| Minor 5: dependency "recorded" but not "for every kind"; check-then-act on removal | Resolved | The guard requires the dependency for every kind it needs, in both directions. The `down` migration refuses while a mark counts or a live process lists the capability. Verified: the replay floor of `lease-withdrawal-v1` exists on the server branch (`20270208000000_lease_withdrawal.up.sql:52-65`). |
| Minor 6: cap and run of three | Resolved (superseded) | One cap. The claim marks only under it (Decision 006-052). |
| Nit 1: timing class of the cross-tenant record | Partly resolved | The existence statement runs for foreign and missing run ids alike, and the record is spawned. No bound is named for the spawn (new Nit 1). |
| Nit 2: `CANCELLED` narrower than the fence | Resolved | The run's stop reason is read in `lock_matched`. Caveat in Nit 2. |
| Nit 3: "keep" rule for every column | Resolved | Static check, barrier test. |
| Nit 4: `Debug` of lease-token messages | Resolved | Every message with a `lease_token` goes into `skip_debug`, and the descriptor test enforces it. Residue: `session_token` (new Nit 3). |
| Nit 5: admission wording | Partly resolved | A formula is now written and tested for limits 1 to 8. As written it breaks its own promise at limits 1 to 4 (new Minor 3). |

### Findings of the other round-3 reports in my domain

| Finding | Status |
|---|---|
| Correctness Major 1 (verdict taken from the unlocked pre-read) | Resolved: rows are locked first, and the verdict is decided on the locked rows with the clock sampled after the lock. |
| Correctness Major 3 and design Major 1 (session minor changes under the lease) | Resolved in this design. The floor residue is Major 1 below. |
| Architecture Blocker 1 (rollback floor only in prose) | Resolved, with precondition (a) of the task phase (the floor decodes `NOT_ACKNOWLEDGED`) and the replay-floor trigger I verified. |
| Architecture Major 1 (registry and command text) | Resolved: generic command entry, dependency table, guard both ways. |
| Architecture Major 4 (1.8 offered before the gate) | Resolved: Task 006-057 comes after Task 006-056. |
| Architecture Major 5 (fallback to 1.7) | Resolved by `declines_acknowledgement`. Residues are Minors 4 and 5. |
| Architecture Major 6 (nothing checks the invariant that lets the run lock go) | Resolved: static check of assignments, plus a barrier test with a run transaction loaded before an acknowledgement. |
| Design Major 4 (statement does not do what the prose says) | Resolved, as correctness Major 1. |

---

## Major

### Major 1. The floor build decides the hand-back from the session row and ignores `lease_capability_generation`. Every process of that build that serves a 1.8 session reopens both round-3 Majors (verified on the server branch)

**Where.**
- Decision 006-059 (3) and (4): "read from `lease_hand_back_generation` once this build writes it ... until then its tests keep raising the stored minor."
- Rollback, step 3 ("the floor is the build of Architecture decision 0098").
- Decision 006-055.
- Task 006-066.

**What the code has.**
- `hand_back(gate, leased, holder_minor, required, before)` withdraws whenever `holder_minor >= HAND_BACK_PROTOCOL_MINOR` (8) (`/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/worker/withdrawal.rs:23`, `171-194`).
- `holder_minor` is the minor stored on the session that `steps.leased_by` names (Decision 006-030 there).
- That build has no `lease_hand_back_generation` and no `lease_capability_generation`. Its capability paths write neither, and its hand-back reads neither.

**Failing scenario (documented rollback).**
1. The fleet ran this build. Hosts registered at 1.8, and `worker_sessions.accepted_protocol_minor = 8` is stored.
2. The operator follows "Rollback": stop marking, wait for `worker_leases_marked = 0`, roll back to the floor. Nothing in the procedure ends or re-registers the 1.8 sessions. Their tokens stay valid until they expire or re-register, so for that whole window the floor reads 8 for them.
3. A buggy or hand-written host on one of those sessions (the round-3 Major 2 scenario) calls `ResolveSecrets` on an unmarked lease. The floor serves it and records nothing.
4. The host then sends `FailStep(TASK_NOT_STARTED)`. The floor reads 8 and withdraws the lease (`NOT_STARTED`): the attempt number and idempotency key are reused, there is no review and no violation.
5. Round-3 Major 1 also returns. Take a 1.4 process and a 1.8 process under one worker name. The 1.4 process holds a lease from before the 1.8 process registered, and its handler raises `TASK_NOT_STARTED` (older SDKs forward it). The floor reads the row, now 8, and withdraws.

The same holds in any fleet where a process of the floor build serves worker calls while this build has already offered 1.8. The offer depends on the build alone, and the reservation needs only `lease-withdrawal-v1`, which both builds list. The activation of `delivery-acknowledgement-v1`, which would exclude the floor, is not involved. Release 0.3.0 ships with that requirement unrecorded (Decision 006-057).

**Fix (smallest).**
- Change request (3) of Decision 006-059. The floor build must clamp the holder's minor to its own compiled ceiling: `min(holder_minor, PROTOCOL_MINOR)`. With its ceiling at 6 or 7, it then honours no hand-back from a 1.8 row. Its tests raise both the stored minor and the ceiling, as Task 006-048 already does here ("the constant raised in the test only").
- The first build that withdraws on `TASK_NOT_STARTED` is then this one, which reads the lease columns. The failure direction becomes safe: on the floor, a hand-back spends the attempt instead of hiding a run.
- Add to the rollback case of Task 006-066: a session stored at 8, a capability served, then a hand-back on the floor must give an ordinary failure. Run it once with and once without a capability.

---

## Minor

### Minor 1. Root `WriteBlob` commits no transaction, so "a served capability records `lease_capability_generation` in the transaction it already commits" is false for it (verified)

- **Code.**
  - `write_blob` calls `check_blob_upload` (`crates/api-grpc/src/service.rs:1218-1224`), which is `check_blob_access(writing = true, read = None)`.
  - The root branch opens the run transaction, checks it and returns `Ok(scope)` without a commit; it commits only when `audit_read` is set (`crates/application/src/worker.rs:2079-2134`). The transaction is dropped and the upload proceeds.
  - The scoped branch does commit (`crates/application/src/worker/scoped_capabilities.rs:40`).
- **Scenario.** A 1.8 host on an unmarked root lease (declining, not activated, short deadline or capped) runs a handler that uploads an output blob, then sends `FailStep(TASK_NOT_STARTED)`. The lease is withdrawn as "nothing ran". No violation is recorded, and an orphan blob of an attempt the journal says never ran is left behind.
  - The impact is lower than for secrets, since a blob stays inside the platform. Still, decision 12 claims the server records this case.
- **Fix.** In the root branch of `check_blob_access` with `writing = true`, write `lease_capability_generation = generation` and commit before the upload is accepted. This is one `UPDATE` and a commit, under the run lock already held. Add `WriteBlob` (root and scoped) to the descriptor-driven test: a hand-back after the upload is an ordinary failure and a violation. Alternatively, name root `WriteBlob` as an accepted residue in Gap 006-042.

### Minor 2. "Audited once per step and generation" has no mechanism for the two cases that repeat (design reading, against verified code)

- **Which cases.**
  - A capability call refused `TASK_NOT_ACKNOWLEDGED` leaves the lease provisional, so the same holder can repeat it until the deadline.
  - Calls carrying a withdrawn token can repeat without bound.
  - The other violations end the provisional state, so they are naturally once: a heartbeat moves the step to `RUNNING`, and a report ends the attempt.
- **What the code does.** The refusal path rolls back today: the fence returns an error and the transaction is dropped. The withdrawn hash is "read only on the stale path", and nothing writes a marker. So each repeat either writes a new audit row or writes none.
- **Existing mechanism.** One exists: the identical-record insert `... WHERE NOT EXISTS (SELECT 1 FROM audit_records WHERE tenant_id AND actor AND action AND resource ...)` (`crates/infra-postgres/src/identity.rs:266-275`, `327-333`).
- **Fix.**
  - Name the mechanism. Use that identical-record rule, with a reason that carries the step id and the generation (or a short hash prefix of the withdrawn token, never the hash itself).
  - Say that the refusal path commits its audit, as the secret denial does ("The denial is written and committed", `secrets.rs:114-124`).
  - Test: 100 refused calls on one provisional lease give one record; the same for 100 stale calls with one withdrawn token.

### Minor 3. The admission formula of the class `Acknowledge` breaks its own promise for limits 1 to 4, and at limit 1 exceeds the global ceiling (verified)

- **Code.** `discovery_max = max(1, max * 3 / 4)` (`crates/api-grpc/src/limits.rs:595`). The class `Lease` is admitted while in flight is under `max` (`limits.rs:548-557`).
- **The design's formula.** An acknowledgement is admitted under `D + max(1, (L - D) / 2)`, "so that at least half of the reserve, and never less than one place, stays for the calls of the class `Lease`".
  - L = 1: D = 1, the acknowledgement limit is 2. With a heartbeat in flight an acknowledgement is admitted, so 2 calls run against a ceiling of 1.
  - L = 2, 3, 4: the reserve is 1 and the acknowledgement limit equals L. It can take the last place, and a heartbeat is shed, which "can cost a review".
  - L = 5 to 8: the promise holds.
- **Fix.**
  - Use `ack_limit = max(D, min(L - 1, D + max(1, (L - D) / 2)))`. When the reserve is a single place, the acknowledgement shares the discovery band.
  - Make the test of Task 006-049 assert the property ("in flight never above L; at least `max(1, ceil(R/2))` places left to `Lease` whenever R ≥ 1"), not the formula.

### Minor 4. A transient condition turns the protection off until the host restarts, and a neighbouring tenant can cause it (design reading, against verified admission)

- **The trigger.** "Three leases in a row given up with no `ACKNOWLEDGED` between them" leads to registering again with `declines_acknowledgement = true` "until it restarts". A give-up follows calls that keep failing transiently: `RESOURCE_EXHAUSTED`, `UNAVAILABLE`, deadlines, or `RETRY` on a busy row.
- **Why a neighbour can cause it.** In-flight admission is per process, not per tenant (`limits.rs:520-563`). Another tenant's flood of calls of the class `Lease`, for example heartbeats with forged tokens, which cost one indexed read each, can hold an engine above the acknowledgement threshold long enough for three give-ups. A run transaction that keeps one step row locked can do the same through `RETRY`.
- **What is lost.** The victim's leases fall back to today's meaning (lapse into review) until an operator restarts the host. No alert says so: only the gauge `worker_sessions{protocol, declines}` exists.
- **Fix.**
  - Count toward the fallback only `UNIMPLEMENTED` and the non-transient refusals.
  - Or time-box the fallback: register again without declining after a bounded wait, for example 10 minutes with backoff, and count each fallback.
  - Add an alert on declining sessions above zero, and a conformance case where a burst of shed calls ends without a sticky fallback.

### Minor 5. Nothing says what a host that declined does with a flagged lease it still receives (design reading)

- **How it happens.** Frames claimed before the re-registration that declines are already on their way. A sibling process under the same worker name can also register again without declining: the row is shared (`crates/infra-postgres/src/workers.rs:338-343`), so the next claims for the declining host are flagged.
- **What the texts cover.** Decision 1 forbids starting a flagged lease before `ACKNOWLEDGED` at any minor. But the host procedure (`sdks/SPEC.md` §6.2 item 5; "The host at 1.8" item 5) covers only leases "waiting ... when it falls back". Six SDK owners will read "declined" as "ignore the flag".
- **Why it matters.** If such a lease is dispatched at once and its first heartbeat is lost, it is withdrawn as `NOT_ACKNOWLEDGED` while running. It then runs again with the same attempt and idempotency key, and the journal says the first run never happened.
- **Fix.** State that a host that declined hands back every flagged lease it receives and never dispatches one. Add a conformance case: a flagged frame delivered after the declining re-registration is handed back.

---

## Nit

1. **The spawned cross-tenant record has no stated bound.** `missing_task_error` awaits its write today (`crates/application/src/worker.rs:2610-2632`). The bounded spawn exists only in `crates/api-http/src/denial.rs` (`Gate`: 32 places, 5 s). Name it, or an equivalent in the application layer, so that a flood of 64-id calls cannot pile up unbounded tasks.
2. **The join with `runs`.** `runs` has the primary key `id` alone (`migrations/20261009000100_execution.up.sql:21`), and `steps` shows no foreign key to it there. Write the join as `runs.tenant_id = steps.tenant_id AND runs.id = steps.run_id`.
   - Also state that `FOR UPDATE OF steps` leaves `runs` unlocked, and the recheck PostgreSQL makes on a locked row keeps the `runs` row the statement first read. A stop committed after that read is answered `ACKNOWLEDGED` and reaches the host through the first heartbeat.
3. **`session_token` is outside the descriptor rule.** `RegisterWorkerResponse.session_token` and `PollTasksRequest.session_token` (`specs/001-platform-foundation/contracts/worker.proto:79-80`) are bearer secrets with the generated `Debug` (`crates/api-grpc/build.rs:17-25`). Make the descriptor test fail for any field whose name ends in `_token`.
4. **`platform_requirement_dependencies` is guarded only by its owner.** A mistaken `DELETE` in the hand SQL of `docs/runbooks/instance-registry.md` section 5 empties the guard's check. Add `UPDATE`, `DELETE` and `TRUNCATE` refusals with `ENABLE ALWAYS`, as `platform_requirement_history` has (`migrations/20270113000000_instance_registry.up.sql:157-168`).

---

## Classes checked with no new finding

- **Can a token holder or a hostile worker write the new columns for another lease?** No.
  - `lease_hand_back_generation` and the provisional mark are written only by the claim, from the claiming session's row at claim time. Registering again replaces `token_hash`, so a process holding a stale session token cannot claim.
  - `lease_capability_generation` is written only by a capability path after its fence matched, and only as `= generation`.
  - A stolen token can set the capability column on an unmarked lease, but only by being served the capability, which is already the larger harm. The resulting violation audit names the caller correctly.
  - `declines_acknowledgement` affects only the declaring identity's own session row.
- **`RETRY`.** It reaches only callers whose token matched. It writes nothing and costs nothing, and the host's give-up bounds it. The only amplification is Minor 4.
- **Tenant boundary in SQL, messages and errors.**
  - `matched` reads by primary key within the caller's tenant.
  - Foreign, missing, forged and stale tokens all answer `NOT_HELD` (`no_match`).
  - The cross-tenant existence statement runs for foreign and missing run ids alike.
  - `CANCELLED`, `RETRY` and `TASK_NOT_ACKNOWLEDGED` reach only token holders, after the binding check.
- **Lock order.** The acknowledgement takes the binding lock (shared), the secrets lock (shared) and the membership lock (`FOR SHARE`), then step rows with `SKIP LOCKED`. It never waits for a row, so no cycle with a revocation, a withdrawal or a run transaction is possible. Root capability paths and hand-backs both hold the run lock (`secret_resolution.rs:80-88`), so the capability record and the hand-back's decision are serialized. Scoped ones hold the path lock.
- **Runtime role.**
  - `GRANT SELECT, INSERT, UPDATE ON runs, steps TO sagat_app` (`execution.up.sql:273`) covers the new columns and `FOR UPDATE OF steps`. `worker_sessions` has `UPDATE` (`:277`).
  - The guard fires inside the `SECURITY DEFINER` operator functions with pinned search paths (`instance_registry.up.sql:312-378`), so it can read an owner-only table. `sagat_app` needs no access to it.
  - No `ALTER DEFAULT PRIVILEGES` exists that would grant the new table by accident.
- **Audit volume.** At most one binding denial per call, through the gate, and at most one cross-tenant record per call. The violations that end the provisional state happen once by construction. The repeating cases are Minor 2.
- **Injection and parsing.** Bound arrays; 1 to 64 tasks; distinct ids; `step_id` checked as `verify` checks it; no dynamic SQL.
- **Logs and metrics.** Labels are closed sets. Log lines carry identifiers only. Span links come from server-held trace context.
- **Rollback.** The floor's replay of `NOT_ACKNOWLEDGED` is a hard precondition. `lease-withdrawal-v1` cannot be removed once a journal holds the event (verified). The `down` migration refuses while marks count, while the history names the capability, or while a live process lists it. The open hole is Major 1.

Nothing can be run yet. Once the build exists, run:
- `SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test -p sagat-tests --test integration_durable_execution`
- `cargo test -p sagat-tests --test contract_grpc_durable_execution`
- the fault suite of Task 006-051
- the rollback case of Task 006-066 with the test named in Major 1

Files:
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/worker/withdrawal.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql`
- `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/specs/006-sdk-multilanguage/research.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker/secrets.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker/secret_resolution.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker/scoped_capabilities.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker/database.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/service.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/limits.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/build.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-http/src/denial.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/workers.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/identity.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20261009000100_execution.up.sql`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/001-platform-foundation/contracts/worker.proto`
