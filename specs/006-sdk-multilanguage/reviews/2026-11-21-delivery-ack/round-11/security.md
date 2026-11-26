# Security review, round 11: delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)

**Verdict: no Blocker, 1 Major, 2 Minor, 3 Nit.**

Round 11 removes the fallback and adds the overload, the latch and the narrower heartbeat rule. These close most of round 10. Three new problems remain:
- **Major 1:** the documents now allow pull request #260 to be released before its follow-up. A release made that way is the same re-admitted binary as round-10 Major 1, reached by another path.
- **Minor 2:** a beat that returns no epoch also hides changes that have already committed. So the general gate, not only capability calls, can stay open for as long as an owner transaction holds the epoch row.
- **Minor 1:** the table widens the heartbeat exemption to `NOT_ACKNOWLEDGED`, where a heartbeat always means a handler ran without an acknowledgement.

"Verified" means I read the code in `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack`. "Design reading" means the defect is in the documents alone.

## Status of round 10

| Round-10 finding | Status | Reason |
|---|---|---|
| **Major 1:** the fallback re-admits the previous release | **Partly resolved** | Part 5 (the replaced two-argument beat) is resolved by the overload. Parts 1 to 4 now depend on the order of releases: the documents no longer forbid releasing #260 without (2) and (7). See Major 1 below. |
| Minor 1: what a beat that returns no epoch means | Resolved | The latch is stated in `stream-completion.md:439-442` and Decision 006-059. The 60 s hold test is in Task 006-046. Its other effect, hiding committed changes, is Minor 2. |
| Minor 2: the privileges of the new beat | Resolved | The function is `SECURITY DEFINER` with the pinned path and `public.` names. It is revoked from `PUBLIC`, granted to `sagat_app`, and tested for `prosecdef` and `proconfig` as the runtime role. A missing row raises an error. Task 006-046's evidence is rewritten. Residue: Nit 2. |
| Minor 3: a heartbeat overtaken by its own hand-back | Resolved in Decision 006-065 (`research.md:719`) | The table and the metric extend the exemption to `NOT_ACKNOWLEDGED` (Minor 1). |
| Nit 1: the per-connection semaphore | Resolved | Held in `CappedConnectInfo`, with no map keyed by the peer. The reserve applies only from Task 006-057. |
| Nit 2: the reason `withdrawn` | Resolved | Decided in `matched`, by the same primary key and tenant. |
| Nit 3: the probe | Resolved | One more hand-back, no second probe. |
| Nit 4: the second gate check on writes | Resolved | Writes are checked before the write. Calls that hand data out are checked before their first byte. |
| Nit 5: the `mark` vocabulary | Resolved | A stale duplicate line remains (Nit 3). |
| Nit 6: the host text | Resolved | `sdks/SPEC.md:136` and `:159`. |
| Nit 7: the property test | Resolved | Now in Task 006-050's evidence. |
| Nit 8: Task 006-045 | Resolved | |

Other reviewers' round-10 findings in my domain:
- **Architecture Major 1 and design Major 1 (the fallback):** same status as my Major 1, partly resolved.
- **Architecture Major 2 (Task 006-046's evidence):** resolved.
- **Architecture Minor 4 (the effect of a beat with no epoch, unrecorded):**
  - The fleet-wide outage is resolved by the latch.
  - The new alert `SagatCapabilitiesRefusedRegistrySilent` fires only once some engine has lapsed.
  - Nothing counts beats that returned no epoch. Such a counter would show a held owner transaction before it costs anything. It is part of the fix of Minor 2.
- **Architecture Minor 6 (the per-connection limit that replaces tonic's):** resolved for security. The limit is per connection and grows nothing before authentication.
- **Correctness Major 1 (the early give-up during a failover):** resolved. The new evidence, "a frame with leases arrived", is local to the host and gives no one a way to act on another tenant.
- **Correctness Minor 2 (the flag of a sibling step):** resolved. It was already the safe direction.

---

## Major

### Major 1. The precondition was weakened from "before any build of #260 is released" to "a released build before Task 006-057". An earlier release of #260 without (2) and (7) lists `lease-withdrawal-v1`, so the registry still admits it after the rollback (verified)

**Where.**
- `tasks.md:142-146`: requests (1) to (8) "go into a follow-up of pull request #260". The round-9 words "before any build of it is released" are gone.
- `tasks.md:350`: Task 006-057 needs only "a released build of the follow-up".
- ADR Rollback paragraph (`ADR-0099-…md:272-277`): "the **first** released build of Architecture decision 0098 that decodes the cause". The words assume that earlier released builds may not decode it.
- ADR decision 10 (`:148`) and `stream-completion.md:398` say the same.
- `research.md:786` still says "before any build of it is released". The documents contradict each other.

**What the code has.**
- The replay-floor guard protects only `lease-withdrawal-v1` (`20270208000000_lease_withdrawal.up.sql:52-65`).
- Registration admits any binary that lists every recorded requirement for its kind (`20270113000000_instance_registry.up.sql:199-205`).
- The merged #260 head lists `lease-withdrawal-v1` (`crates/application/src/instances.rs:121,140`). Its codec returns `None` for any cause other than `NOT_SENT` and `NOT_STARTED` (`crates/infra-postgres/src/event_codec.rs:495`).
- A process that lapsed while its connection was lost keeps serving until its first successful beat (`crates/application/src/instances.rs:27-30`).

**Scenario.**
1. #260 merges into `develop`, and a release R1 is cut before the follow-up lands. The current text allows this: Task 006-045 and Task 006-046 wait, and only Task 006-057 needs a release with the follow-up.
2. Later, release R2 has the follow-up. This design leaves dormancy, and `delivery-acknowledgement-v1` is activated.
3. Any tenant produces `NOT_ACKNOWLEDGED` withdrawals at will: its own worker never acknowledges.
4. The operator rolls back and removes `delivery-acknowledgement-v1`. The only requirement left is `lease-withdrawal-v1`, which R1 lists, so the registry admits R1's engines and control planes. Every run whose journal holds `NOT_ACKNOWLEDGED` then fails to replay on them. This is round-10 step 4 again. "The registry never admits a binary below that build" is false for R1, and the ADR's argument that no `NOT_ACKNOWLEDGED` exists before the floor is about order in time, not about admission.
5. R1 also lacks (7). An engine of R1 that was live but silent at the activation's census serves `ResolveSecrets` to provisional leases until its first beat. That is the round-8 hole for R1. It applies on the upgrade path from R1 to this build, which (7) in R2 does not cover. Decision 006-047's reason for recording the requirement for engines only ("the floor build's control planes will decode the cause") also fails for R1's control planes.

**Fix.** Pick one and write it the same way in `tasks.md`, the ADR, `stream-completion.md` and Decision 006-059.
- (a) **The order of releases.** Restore the round-9 rule: no build of #260 is released without (2) and (7), because they are part of its first release. Add to Task 006-057's evidence that every released tag whose binaries list `lease-withdrawal-v1` decodes `NOT_ACKNOWLEDGED`. The released-upgrade job can run the codec fixture against each such tag.
- (b) **Enforced whatever the order (preferred if 0.3.0 may ship #260 first).**
  - The follow-up lists a capability of its own for both kinds, for example `lease-withdrawal-v2`, meaning "decodes `NOT_ACKNOWLEDGED` and applies (7)".
  - The activation entry of `delivery-acknowledgement-v1` refuses until that capability is recorded for both kinds.
  - The replay-floor guard covers its removal too.
  - Test: R1 is refused registration after the rollback while a `NOT_ACKNOWLEDGED` event exists.

---

## Minor

### Minor 1. The heartbeat exemption from the alert is extended to `NOT_ACKNOWLEDGED`. A heartbeat there is never honest, and it is the earliest sign of the trust breach Gap 006-042 accepts (design reading)

**Where.**
- `research.md:719`: the exemption covers only "a heartbeat after a hand-back's withdrawal". That is correct.
- `stream-completion.md:830`: the row "withdrawn as `NOT_ACKNOWLEDGED` or on its hand-back … (a heartbeat under its own kind, not alerted)" applies it to both causes.
- `stream-completion.md:868` and the alert at `:888`: the `kind` label has no cause, so the alert rule cannot tell the two apart.

**Why it matters.**
- A host sends heartbeats only after the verdict `ACKNOWLEDGED`.
- `NOT_ACKNOWLEDGED` requires `lease_acknowledged_at` to be empty and the mark to count, on the same server incarnation.
- So a heartbeat after that withdrawal means a handler started without an acknowledgement: an SDK defect or a hostile host.
- For a long handler, the heartbeats arrive minutes before any report. The duplicate run on the re-leased step goes unalerted until then, and never if the first handler crashes before reporting.

**Fix.**
- The kind `heartbeat` applies only to a heartbeat carrying a token withdrawn on its hand-back (`NOT_STARTED`).
- A heartbeat after `NOT_ACKNOWLEDGED` is counted as `report`, and alerted.
- Align the table row and the metric's description.
- Add the case to Task 006-051: a heartbeat after a `NOT_ACKNOWLEDGED` withdrawal raises `SagatCallsAfterWithdrawal`, and one after a hand-back does not.
- Optional: a second heartbeat after a hand-back's withdrawal, for the same step and generation, cannot be the overtaking race. That case can be alerted too.

### Minor 2. A beat that returns no epoch also hides a change that already committed. A process that lacks a newly recorded requirement keeps its whole gate open for as long as any later owner transaction holds the epoch row (verified against `instances.rs`; design reading for the overload)

**What the code has.**
- The two-argument beat reads the epoch with a plain `SELECT` (`20270113000000_instance_registry.up.sql:269`). That read never blocks, and it returns the last committed epoch.
- `Member::decide` closes the whole gate on the first beat whose epoch moved to requirements the process lacks (`crates/application/src/instances.rs:678-691`). That beat is what limits the exposure of a process that lapsed at an activation's census to about one beat period (`:34-36`).

**What the design changes.**
- The overload returns no epoch whenever an owner transaction holds the row.
- That beat counts as successful "for the rule that closes the gate" (`stream-completion.md:439-442`).
- `heartbeat.epoch` therefore never moves, and the process does not read the requirements again.
- Which kinds call the overload is not stated. `Member` is shared by both kinds.

**Scenario.**
1. `access-narrowing-v1`, or `run-input-validation-v1`, is activated and commits at C. Process P lacks it and was silent at the census.
2. Right after C, an operator opens another owner transaction: a second `capability-activate` in the same runbook, or `BEGIN; SELECT platform_requirement_lock();` left open, the case this round addresses.
3. P comes back. Every beat it makes returns no epoch, and P keeps serving without enforcing the requirement. With a hand-SQL transaction left open, that lasts as long as the transaction. The base build closed P at its first beat.

**Fix.**
- When the overload skips the locked row, it still returns the committed epoch from a plain read, together with a boolean such as `out_change_in_progress`. That read cannot block.
- A process always applies a committed epoch that moved, and closes its gate if it lacks what was recorded. Closing is always safe.
- The latch then governs only opening: capability calls on a lapsed engine stay refused while a change is in progress.
- Say which kinds call the overload.
- Count beats that report a change in progress.
- Test: activate a requirement while a process that lacks it is lapsed, then hold the epoch in a second owner transaction. The process's first beat after it comes back closes its gate.

---

## Nit

1. **The contradiction at `research.md:786`.** Whichever option of Major 1 is taken, `research.md:786` and `tasks.md:143` must say the same thing.
2. **The overload's third argument** (`stream-completion.md:434`, `research.md:813`).
   - Its meaning is not stated. Name it.
   - Forbid a `DEFAULT` on it. With a default, PostgreSQL cannot choose between the two functions for a two-argument call, which breaks every older binary's beat.
   - The down migration drops it by its full signature.
   - Task 006-046's privilege test also checks that `PUBLIC` lacks `EXECUTE` on it.
3. **`worker_leases_expired_total{mark}` is described twice with different value sets** (`stream-completion.md:858` gives the four values, `:860` gives three). Keep one closed set.

---

## Suspicion, not verified

- **The epoch lock may be held off by beats.** Every beat of the overload now takes `FOR SHARE` on the epoch row, about 100 a second at 1,000 instances. A new share locker does not wait behind a queued `FOR UPDATE` while the current lockers are all share lockers. So under a slow database, overlapping beats may delay an activation's `platform_requirement_lock()`. The effect is availability only: nothing is skipped until the activation holds the lock.
- Command to run once built: run the scalability profile with 1,000 instances beating, and time `control-plane capability-activate` under `pgbench` load. It should complete within its bound.

## Classes checked with no new finding

- **Tenant boundary.**
  - `withdrawn` against `no_match` is decided in `matched`, in the caller's tenant and by the same primary key.
  - The new kinds, reasons and alert labels are closed sets with no tenant data.
- **Who can make beats skip.** `sagat_app` has only `SELECT` on the epoch table (`20270113000000_instance_registry.up.sql:404`). None of its functions takes a lock on the epoch that conflicts with `FOR SHARE` (`instance_register` takes `FOR SHARE`; `lease_withdrawal_required()` locks a requirement row). So only the owner can make a beat skip.
- **Least privilege.** `delivery-acknowledgement-v1` must also join the closed lists `serving_instances_capabilities_known` and `platform_requirements_capability_known`. Task 006-044's migration already covers this.
- **Secrets and logs.** Nothing new carries a token, a payload or an unbounded identifier. The `REGISTRY_SILENT` refusal carries only a reason and a retry hint.
- **Injection and parsing.** No dynamic SQL and no new unbounded input. The per-node `mark_withdrawn_attempt` stays inside the run's own transaction.
- **Races.**
  - The early give-up's new evidence, an `OK` after the lease's latest failure or a frame received, is local to the host.
  - The extra hand-back after a probe is bounded at one.
  - The second gate check now runs before writes, so a repeated `REGISTRY_SILENT` call cannot duplicate them.

## Commands to run once built

- Major 1 (b): in the released-upgrade job's `provisional-lease-cutover` rollback case, an engine and a control plane of a #260 tag without (2) register after the removal while a `NOT_ACKNOWLEDGED` event exists. They must be refused.
- Minor 2: as the owner, activate a test requirement, then `BEGIN; SELECT platform_requirement_lock();` and hold it. Bring back a process that lacks the requirement and was paused across the activation. Its gate must be closed after its first beat.
- Nit 2: `SELECT p.oid::regprocedure, p.prosecdef, p.proconfig, p.pronargdefaults, has_function_privilege('public', p.oid, 'EXECUTE') FROM pg_proc p WHERE proname = 'instance_heartbeat';` should show both functions as `t`, `{search_path=pg_catalog, pg_temp}`, `0` and `f`.

## Files

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/instances.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/event_codec.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/limits.rs
