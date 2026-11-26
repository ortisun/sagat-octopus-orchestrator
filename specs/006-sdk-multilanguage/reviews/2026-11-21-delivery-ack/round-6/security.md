**Security review, round 6: delivery acknowledgement (Architecture decision 0099 and Architecture decision 0100, worker protocol 1.8)**

**Verdict: no Blocker, 1 Major, 4 Minor, 4 Nit.**

The round-5 Major (mixed fleet) is resolved for a fleet at rest. One residue remains: the registry's liveness window at the moment of activation (new Minor 3). Round-5 Minors 1 and 3 and all four Nits are resolved, with small residues. Minor 2 is resolved as I asked. However, the new third fallback trigger undoes its purpose: the trigger now counts the server's own statuses. Together with the acknowledgement's narrow admission band, that gives another tenant, or anyone who can load the engine, a cheap way to switch the protection off for every host on that engine (new Major 1).

"Verified" means I read the cited code in `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack`, or in `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases` where named. "Design reading" means the defect is in the documents alone.

---

## Status of round 5

### My report

| Round-5 finding | Status | Reason |
|---|---|---|
| Major 1: mixed fleet, a 1.8 hand-back withdrawn after a floor process served an unrecorded capability | **Resolved for a fleet at rest; one residue** | Decision 006-064: the hand-back, and every withdrawal of a marked lease, reads `delivery-acknowledgement-v1` under the lock its removal waits for. Before activation and after removal, a 1.8 hand-back is an ordinary failure. The rollback order is now switch off, drain, remove. A mixed-fleet case is in Task 006-066. **Residues:** (a) the claim "the requirement, once recorded, proves that every live process is of this build" ignores the registry's documented liveness window (new Minor 3); (b) the test names a floor *control plane* serving `ResolveSecrets`, but only the engine serves worker calls (Nit 3; the mistake started in my round-5 text). |
| Minor 1: same-timeline losses | **Resolved; two residues** | The acknowledgement raises `synchronous_commit` to `local`, the probe refuses to mark with `fsync` off, the runbook step after every restore exists, and Gap 006-048 is rewritten. **Residues:** the capability record of an unmarked 1.8 lease is still committed asynchronously (new Minor 1). Engines left running across a same-address restore run the pass before the operator's step (new Minor 2). |
| Minor 2: the server does not bound its own answer | **Resolved as asked; its purpose is undone** | The deadline now runs from arrival, includes pool acquisition, and answers `UNAVAILABLE` (`ACKNOWLEDGEMENT_DEADLINE`). I accepted that fix because such a status "never counts". The round-6 third trigger counts it, whatever the status, while other calls are answered (new Major 1). |
| Minor 3: audit key | **Resolved in the decision; contract text not aligned** | Decision 006-053 and decision 12: the resource is the step, the reason comes only from stored values, the actions are in `collapses_repeats` with a distinct cap of 100, and the 100-generation test is in Task 006-053. `stream-completion.md:687` still says "audited once per step and withdrawn token" and does not state the mechanism (Nit 4). |
| Nit 1: `end_lease_marks()` privileges | Resolved, apart from the record's destination | `SECURITY INVOKER`, pinned search path, `public.steps`, revoked from `PUBLIC`, granted to no role, and a privilege-audit assertion. Where the audit record goes is new Minor 4. |
| Nit 2: `sagat_database_timeline()` declaration | **Partly resolved** | `VOLATILE`, `pg_catalog` names, grants, and null means "no mark counts", with null covered in the parity test. But `LANGUAGE sql` cannot return null for a missing execute right (Nit 2). |
| Nit 3: reservation not bound to the timeline | Resolved for a timeline change | `lease_claim_timeline` is stamped on every 1.8 claim, and a hand-back of another timeline is an ordinary failure. The same loss without a timeline change is new Minor 1. |
| Nit 4: no `Debug` on the request type | Resolved | Decision 006-043. |

### Findings of the other round-5 reports in my domain

| Finding | Status |
|---|---|
| Architecture Major 1 (`synchronous_commit = off`, `fsync = off`) | Resolved for the acknowledgement. The same reasoning applies to the capability record, which the design waves through as "safe or covered" (new Minor 1). |
| Design Major 1 (restores that keep the timeline) | Resolved in the procedure, with the timing residue of new Minor 2. |
| Correctness Major 1 (the pass writes nothing) | Resolved: every outcome of the pass other than a withdrawal is `expire_attempt` in the same transaction. That is the safe direction, and it removes a starvation path a hostile host could have fed. |
| Correctness Major 2 (a call that fails alone has no end) | Answered by the new third trigger, which creates new Major 1. |
| Architecture Minor 4 (the probe closing the gate) | Resolved: the probe runs on the request pool, outside the heartbeat. |
| Architecture Minor 5 (`end_lease_marks()` leaves no record) | Partly resolved: a command, an operator name and "one audit record", but no destination (new Minor 4). |
| Architecture Minor 7 (no alert while the protection cannot work) | Resolved: `SagatDeliveryAcknowledgementInactive`. |

---

## Major

### Major 1. Another tenant, or an unauthenticated flood, can switch delivery acknowledgement off for every host on an engine with a burst of about 30 seconds. The new third trigger counts exactly the state the acknowledgement's admission band was built to produce (design reading, against verified admission code)

**Where.**
- `sdks/SPEC.md:375` and `stream-completion.md:621-627`: three calls given up "while another call to the same server was answered during each, whatever status their attempts ended with".
- Decision 006-048 (`research.md:525`).
- The admission band (`stream-completion.md:323-329`).

**What the code has (verified).**
- Admission is process-wide per class, and it runs before authentication (`crates/api-grpc/src/limits.rs:1-11`, `548-563`).
- The class `Lease` (Heartbeat, CompleteStep, FailStep, ResolveSecrets, ResolveDatabase) is admitted up to `L`. The class `Acknowledge` is admitted only below `D + (L - D) / 2`.
- So whenever the calls in flight are between `D + (L - D) / 2` and `L`, from anyone, every tenant's acknowledgement is shed with `RESOURCE_EXHAUSTED`, while every tenant's heartbeats are answered. That is the reserve working as designed.
- The second lever: the acknowledgement's 3-second deadline includes pool acquisition, while a heartbeat has the full unary timeout. Pressure on the shared pool therefore fails acknowledgements (`UNAVAILABLE`, `ACKNOWLEDGEMENT_DEADLINE`) before it fails heartbeats.

**Scenario.**
1. Tenant B runs a few worker processes. It sends its own 64-task `AcknowledgeTasks` calls, or any `Lease`-class calls, at a rate that keeps the calls in flight on engine E inside the band (eight places at `L = 64`). An unauthenticated client with garbage tokens can do the same before authentication, within the per-address connection caps.
2. Every 1.8 host of tenants A, C and so on that is connected to E and has running tasks sees its acknowledgement calls shed, while its heartbeats are answered.
3. The give-up is 6 to 120 seconds, about 24 seconds at the default time-to-live. Several frames are in flight, so three calls are given up within about one give-up period. Each host falls back: it hands back its flagged leases, then declines for 10 minutes.
4. B repeats the burst every 10 minutes. The decline doubles up to an hour.

**What the attacker obtains.**
- No data and no false "nothing ran": a fallback fails toward today's meaning.
- What it does get, for every tenant on the engine: the protection the owner asked for in 0.3.0 (Owner decision 2026-11-20-12) is off, and a lost frame lapses into a review. Before each fallback there is also a short loss of task execution, since flagged leases are handed back instead of run.
- The lever costs B seconds of load. The effect lasts 10 to 60 minutes after the load ends.
- Round 5 closed this path ("a server deadline or `RETRY` never counts"). Round 6 reopens it more cheaply, because shedding is easier to cause than database slowness.

**Why not a Blocker.** The failure is fail-safe and alerted (`SagatWorkersDecliningAcknowledgement`, `SagatAcknowledgementsFailing`).

**Fix (smallest that keeps correctness Major 2 answered).**
1. **Keep the three-call trigger, but only for failures the server does not claim as its own:** no answer at all, a refusal from the closed list, or a status without the server's reason, such as a gateway's 503.
2. **Mark every server-made transient answer with a reason, and let it count only after it persists for two minutes.** The admission's shed of this class gets a reason such as `ACKNOWLEDGEMENT_SHED`, alongside the existing `ACKNOWLEDGEMENT_DEADLINE`. The conformance case "two minutes of `UNAVAILABLE` for this call alone" already describes this. Correctness Major 2 still ends, and the attacker must sustain a saturation that `SagatAcknowledgementsFailing` sees.
3. **After authentication, give each tenant a share of the `Acknowledge` band**, counted per tenant as `TaskStreams` already counts streams (`limits.rs:47-56`). One tenant's load then sheds only its own acknowledgements.
4. **Tests:**
   - Tenant B holds the band full for 60 seconds. Tenant A's hosts do not decline, and spend no attempt.
   - A pool held full for 5 seconds, with heartbeats answered, does not fall back.
   - Unannotated `UNAVAILABLE` for this call alone does fall back.

---

## Minor

### Minor 1. The capability record of an unmarked 1.8 lease can still be lost on the same timeline, so a hand-back after a served capability is withdrawn (design reading)

**Where.**
- `stream-completion.md:431-434` ("a lost claim, withdrawal or capability record is safe or covered by it").
- Decision 006-062 (a).

**What is wrong.**
- Only the acknowledgement's transaction raises `synchronous_commit`.
- A 1.8 lease that is not marked (`short_deadline`, `switch_off`, `declined`) still carries `lease_hand_back_generation` and `lease_claim_timeline`. Its hand-back is honoured while the requirement is recorded, unless `lease_capability_generation` is set.
- With `synchronous_commit = off` on the role or the database, which the design itself names as a case to handle, the record's `COMMIT` returns before the write-ahead log is flushed, and only then is the capability served.

**Scenario.**
1. A 1.8 lease with `timeoutMs = 10 s` is unmarked. Its host calls `ResolveSecrets`.
2. The record commits asynchronously, and the secret values are handed over.
3. The primary crashes within the write-ahead log writer's window (about 200 to 600 milliseconds by default). Crash recovery keeps the timeline, and the record is gone.
4. The host, buggy or hand-written (the round-3 threat), hands the lease back. The hand-back is withdrawn as `NOT_STARTED`.
5. Result: the same attempt number and idempotency key, no review, and a journal that says nothing ran after credentials were handed out.

No acknowledgement covers this lease: there is none. An acknowledged lease is safe, because its hand-back is an ordinary failure.

**Fix.**
- Apply the same settings statement, with `synchronous_commit` raised to `local` when it is `off`, to every capability transaction that writes `lease_capability_generation`. That costs one flush per lease that uses a capability, once per generation.
- Extend the Task 006-049 durability test (`ALTER ROLE sagat_app SET synchronous_commit = off`) to the capability record, root and scoped.

### Minor 2. "Before the engines start" does not hold when the engines were never stopped: a same-address restore lets the pass withdraw before the operator's step (design reading, against the runbook)

**Where.** `docs/runbooks/disaster-recovery.md:94-99` (step 5), section 3 ("your platform's snapshot tool"), section 5. Gap 006-048.

**What is wrong.**
- Section 4 tells the operator to stop the writers, but only for a point-in-time recovery.
- A volume snapshot restored in place, for example a restored persistent volume behind the same service name, or a managed service's restore to the same endpoint, keeps the timeline.
- The engines that kept running reconnect on their own (section 5 says so). The lost-lease pass runs within seconds, before any operator can run `end-lease-marks`. This is the reason the design gives for rejecting an operator-only fence (considered option 9).

**Scenario.**
1. An acknowledgement commits after the snapshot was taken, and its handler runs.
2. The snapshot is restored in place, and the engines reconnect.
3. At the lease's expiry the pass finds a provisional mark of the current timeline and withdraws it as `NOT_ACKNOWLEDGED`.
4. The step runs again with the same attempt and idempotency key. The only signal is `SagatCallsAfterWithdrawal`, after the fact.

Gap 006-048 names "done without that step", not "done after the engines already ran".

**Fix.**
- Add a first step to section 7, which also applies to section 3's snapshot and to any restore through a platform tool: scale every engine to zero before the restored database accepts connections. Run step 5, then step 6.
- Have `end-lease-marks` report, and record, the live engines the registry lists (seen within 30 seconds), so a wrong order is visible in its record.
- Name "engines left running across a restore that keeps the timeline" in Gap 006-048.
- Optional, and the owner's trade-off: also stamp `pg_control_system().system_identifier` on the claim. It changes for a new cluster, which covers a logical dump into a new cluster and the logical-replication cut-over of Gap 006-048. Confirm in Task 006-044 that the runtime role may read it.

### Minor 3. Decision 006-064 rests on "the requirement proves every live process is of this build", but the registry admits a floor engine that was silent at the census to serve until its next successful beat (verified against the registry's documented fail mode)

**Where.** Decision 006-064 (`research.md:674-681`); `stream-completion.md`, "Only while `delivery-acknowledgement-v1` is recorded".

**What the code has (verified).**
- `crates/application/src/instances.rs:27-36`: on a plain connection failure a process "keeps serving".
- "After 30 s it no longer counts as live, an activation may record a requirement it lacks, and its first successful beat closes its gate."
- The beat comes every 10 seconds, jittered up to 12 (`instances.rs:51`). The activation census counts only rows seen within 30 seconds (`20270113000000_instance_registry.up.sql:333-337`).

**Scenario.**
1. Floor engine F loses its database connections for 40 seconds. During that time the operator activates `delivery-acknowledgement-v1`. F is not live, so the census passes.
2. Engines of this build learn of the activation and mark leases.
3. F reconnects. Its gate is still open, so it serves a buggy host's `ResolveSecrets` on a provisional lease with neither the refusal nor the record. Its first beat, up to 12 seconds later, closes its gate.
4. The host's hand-back reaches an engine of this build. The requirement is recorded and no capability is recorded, so the hand-back is withdrawn.

The window is narrow, but the property is stated without it, and a fixed grace period after `required_since` does not close it, because F can reconnect at any later time.

**Fix.**
- Add a request (7) to Decision 006-059, for the floor build before it merges, and the same rule in this build: a process whose registry has been silent for longer than the liveness window refuses worker calls until a beat succeeds, and it beats at once on its first connection after the silence.
- ADR-0082's reason for not closing the gate (not stopping the fleet during an outage) does not apply: the gate reopens one round trip after the database returns.
- Test: a floor-build process held silent across an activation refuses `ResolveSecrets` after reconnecting until its beat.

### Minor 4. `end-lease-marks` has authority but no stated record: "one audit record" has no table, and `audit_records` is tenant-scoped (design reading, against the registry migration)

**Where.**
- `stream-completion.md:437-442`.
- Task 006-054.
- `docs/runbooks/disaster-recovery.md:94`.

**What is wrong.**
- The registry migration says why a deployment-level act "cannot be an `audit_records` row (that table is tenant-scoped)", and keeps `platform_requirement_history` instead (`20270113000000_instance_registry.up.sql:70-87`).
- The command changes every tenant's rows. An implementer must choose between inventing a tenant, writing one row per tenant, or logging only.
- The operator text is not bounded, and the record is not required to be in the same transaction as the update.

**Misuse, checked.**
- **A non-owner cannot run it:** it is an invoker function, revoked from `PUBLIC`, granted to no role, and the privilege audit asserts that `sagat_app` cannot run it.
- **It cannot hide a lost acknowledgement:** it only moves leases toward today's meaning, and never makes a lease withdrawable.
- **Run during normal operation**, it costs the protection of the leases in flight, which is the owner role's to spend. Concurrent decisions stay safe, because each reads the row under its lock and null fails toward an attempt.
- The real misuse is running it late (Minor 2) or skipping it.

**Fix.**
- Write a row into a deployment-level append-only table: either `platform_requirement_history` with a new action, or a twin of it, `ENABLE ALWAYS` append-only and unreadable by the runtime role.
- The row holds the operator text (bounded to 255 characters, as `actor` is), `session_user`, the count, the timeline, the live engines found (Minor 2) and the time, in the same transaction as the update.
- Restrict the update to `status IN ('LEASED', 'RUNNING') AND lease_claim_timeline IS NOT NULL`, the only rows that can count, so the bounded transaction does not scan every finished step.
- Make step 5 unconditional from the release that builds the command. Drop "Only where the requirement is recorded": the command is idempotent and harmless otherwise, and the operator then does not need to judge which database's requirement table to read.

---

## Nit

1. **Spell out the twin's privileges, and take both rows in the order removals must take them.**
   - `FOR SHARE` needs the `UPDATE` privilege, which `sagat_app` lacks on the registry (`instance_registry.up.sql:171-172`, `404`). So `delivery_acknowledgement_required()` must be `SECURITY DEFINER`, with `search_path = pg_catalog, pg_temp`, `public.` names, revoked from `PUBLIC` and granted to `sagat_app`. Task 006-044 says only "the twin".
   - Lock order (checked): the pass and the hand-back take the run lock, then the requirement rows, then the step row. A removal takes the epoch row, then the requirement row, and never a run or step lock. The heartbeat takes only its own row and a plain read of the epoch (`instance_registry.up.sql:260-269`). There is no cycle.
   - The one deadlock is the hand SQL of `instance-registry.md` section 5 removing both rows in one transaction. The trigger forces `delivery-acknowledgement-v1` first, while "one statement for both" takes `lease-withdrawal-v1` first. The deadlock is detected and costs no safety.
   - Make it one function that locks both rows with `ORDER BY capability FOR SHARE`. The order is `delivery-…` before `lease-…`, the only removal order the trigger allows.
2. **`sagat_database_timeline()` cannot be `LANGUAGE sql` and also "return null when there is no execute right".**
   - PostgreSQL checks `EXECUTE` on every function in an expression when the executor initialises the expression, whether or not a `CASE` branch runs. An inlined or non-inlined SQL body therefore raises an error instead of returning null.
   - Every 1.8 claim stamps `lease_claim_timeline`, marked or not, and the operations snapshot uses the macro. On such a database the error would fail every 1.8 claim.
   - Use `LANGUAGE plpgsql` with `IF has_function_privilege('pg_catalog.pg_walfile_name(pg_lsn)', 'EXECUTE') AND has_function_privilege('pg_catalog.pg_current_wal_lsn()', 'EXECUTE') AND NOT pg_is_in_recovery()`. Do not use an `EXCEPTION` block, which would cost a subtransaction on every claim.
   - Task 006-044's "a role without the right gets null" test will show the error as the design is written now.
3. **The mixed-fleet case names the wrong kind.** `stream-completion.md:755` and Task 006-066 have "`ResolveSecrets` served by a floor control plane". The worker service is hosted only by the engine (`apps/execution-engine/src/lib.rs`; `apps/control-plane` has no `ResolveSecrets`), and the design itself says a control plane "serves no worker call". My round-5 scenario named it so; the mistake is mine. Make it a floor engine.
4. **Align the contract with Decision 006-053.** `stream-completion.md:687`: "once per step and generation, the reason from stored values only (`collapses_repeats`, distinct cap 100)".

---

## Classes checked with no new finding

- **Tenant boundary.**
  - The deadline answer is the same for every caller and independent of whose rows were addressed.
  - `CANCELLED` (`too_late`) and `recovered` are answered only after the token and the binding matched.
  - The cross-tenant existence record stays off the answer path, behind its gate.
  - `end-lease-marks` counts are seen by the operator only.
- **`synchronous_commit` raised as the runtime role.** It is a user-context setting, so `set_config(..., true)` works for `sagat_app`. It is raised only from `off` and never lowers `remote_apply`. `fsync` is readable by any role. A commit that waits on a synchronous standby holds the row locks, as every commit does today. That is not new.
- **The twin against removal, the pass and the heartbeat.** No cycle (Nit 1). A removal waits for withdrawals that hold the share lock, within their lock timeout. Registrations wait behind the removal's epoch lock; heartbeats do not.
- **A hostile worker forcing its own fallback.** It changes only its own session row, keyed by tenant, subject and worker name (`crates/infra-postgres/src/workers.rs:334-343`). It could register as declining anyway, so there is no blast radius beyond itself. Forcing other tenants' fallbacks is Major 1.
- **Injection and parsing.** No dynamic SQL. The timeline and the settings come from server functions. The only operator input to the new command is a name (bounding it is in Minor 4).
- **Logs and metrics.** The new labels are closed sets (`deadline`, `stop_asked`, `too_late`, `recovered`, `no_durability`). `ACKNOWLEDGEMENT_DEADLINE` is a stable reason, not free text. Nothing logs a token.
- **The pass's new `expire_attempt` outcome.** It fails safe, and it removes a path by which a hostile host could have filled the pass's window with leases that are never ended.

## Commands to run once the build exists

- `SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test -p sagat-tests --test integration_durable_execution` (with the capability record under `ALTER ROLE sagat_app SET synchronous_commit = off`, Minor 1)
- `SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test -p sagat-tests --test integration_platform_foundation` (the twin's grants; a role without the write-ahead-log rights gets null from the timeline function, Nit 2)
- `cargo test -p sagat-tests --test contract_grpc_durable_execution`
- the fault suite of Task 006-051, with both removals in one transaction (Nit 1)
- `SAGAT_SDK_SCENARIO=delivery-acknowledgement cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture`, with the cross-tenant band saturation case of Major 1
- the Task 006-066 mixed-fleet case with a floor engine (Nit 3), and the silent-at-census case (Minor 3)

## Files

- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0100-a-fleet-requirement-may-require-another-and-the-registry-refuses-either-order.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/disaster-recovery.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/limits.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/instances.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/workers.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql`
