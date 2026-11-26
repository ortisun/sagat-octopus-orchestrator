# Security review, round 3: lease withdrawal (pull request #260, worktree at 45a3f42b)

**Verdict.** I found no Blocker and no Major, so the security side meets the owner's condition for accepting ADR-0098 (OD-2026-11-21-4). I could only read code; nothing was run. Three of the round-2 findings are closed with tests: the READ COMMITTED guard (round-2 minor 2), the bound and index on the removal (minor 3, in part) and the dead token after a hand-back (minor 4). In production the server never honours a host's `TASK_NOT_STARTED` as a withdrawal. It caps the holder's minor at `PROTOCOL_MINOR`, which is 6, and withdraws only from 8, so every hand-back is an ordinary failure. Two things stay open, and the branch records neither:

- **Round-2 minor 1 is not fixed.** The server still judges a hand-back by the session row, and registering again overwrites that row. Nothing stops this from going live when a feature raises the minor to 8.
- **The derivation behind the one-second bound on the removal has a false premise and an unproven claim.** "Verified" below means I traced it in the worktree. "Suspicion" means it rests on PostgreSQL behaviour I did not run.

## Round-2 security findings and their status

| Round-2 finding | Status | Evidence |
|---|---|---|
| Minor 1: the hand-back is judged by the session's current minor, which registering again overwrites | **Open, unreachable in this build, not recorded** | Unreachable because of the ceiling cap in `withdrawal.rs:232-234, 269-271` (`.min(self.hand_back_ceiling)`, which is `PROTOCOL_MINOR` = 6, `worker.rs:75`) and the 8 check at `withdrawal.rs:191`. The cause is unchanged: `lease_holder_minor` still reads `worker_sessions.accepted_protocol_minor` (`crates/infra-postgres/src/execution.rs:7705-7709`), and the upsert still overwrites it (`workers.rs:338-343`). It is not in `research.md` as moved or declined. See new finding 1. |
| Minor 2: the replay floor holds only under READ COMMITTED | **Closed, with tests** | Guard at `20270208000200_lease_withdrawal_floor.up.sql:27-43`. Tests in `tests/integration/platform-foundation/migrations.rs`: `the_requirement_is_removed_only_under_read_committed`, plus both orders (`a_removal_that_waits_for_a_withdrawal_sees_its_event_and_is_refused`, `a_withdrawal_that_waits_for_a_removal_finds_no_requirement`), with `pg_stat_activity` as the barrier. |
| Minor 3: the removal holds the epoch lock across a lock wait and a full scan | **Closed in part** | The partial index (`up.sql:16-25`) and `lock_timeout = '1s'` (`up.sql:45`) are in place. The test `the_removal_waits_for_a_lock_at_most_one_second_and_probes_an_index` checks the setting, that the index is valid, and a single held share lock. It does not prove the probe uses the index (there is no EXPLAIN). The starvation suspicion is still open, and the bound's premise is wrong. See new finding 2. |
| Minor 4: a handed-back token still gets OK heartbeats | **Closed at the root with a test. Scoped path fixed in code, no test** | `worker.rs:2485` and `scoped_reports.rs:383` set `effects.reported = None`. The stored hash stays NULL, because the update keeps NULL for a step that goes from `LEASED` to `READY` (`execution.rs:7080-7082`, `path.rs:1256-1257`). So `reported_by` is false and the heartbeat gets `STALE_ATTEMPT`. The test `a_heartbeat_with_a_handed_back_token_is_stale` covers the root only. See new finding 3. |
| Note 1: a raw 1.8 client still gets four re-runs per attempt that ignore `retryable=false` | **Open, not recorded** | Unreachable until 1.8. Neither ADR-0098 nor `research.md` names the leftover risk. ADR-0099 is not in this worktree. |
| Note 2: an attempt number reused across withdrawals | **Open, unreachable until 1.8, bounded by the cap of four** | Unchanged. |
| Note 3: least privilege of the new function | **SQL correct; pin test still missing** | `lease_withdrawal_required` is SECURITY DEFINER, has a pinned `search_path`, is revoked from PUBLIC and granted only to `sagat_app`. The recreated census (`up.sql:47-60`) has the same pin and the same revoke. `ALTER FUNCTION … SET lock_timeout` adds to `proconfig` and leaves the `search_path` entry in place. But `the_functions_are_security_definer_with_a_fixed_search_path_and_the_liveness_they_name` (`tests/integration/schedules-bulk-operations/instances.rs:1213-1223`) still lists six functions without these two. Fix: add both names and expect 8 rows. |
| Note 4: the withdrawal semaphore is per engine, not per tenant | **Open, unchanged** | Same behaviour as before the feature. |
| Note 5: public test seams on production types | **Open, unchanged** | The new `with_hand_back_ceiling` is behind `cfg(feature = "test-support")`. Only `tests/Cargo.toml:46`, a dev-dependency, enables that feature, and no production code calls it. |
| Note 6: the runbook said nothing about hand-backs when the switch is off | **Closed** | The runbook's `disabled` row now says a hand-back is an ordinary failure. |

## New findings

### 1. Minor (verified): nothing stops the session-row minor from going live at 1.8, and the reservation of 1.8 is not in the register

- **Where:**
  - `crates/infra-postgres/src/execution.rs:7705-7709`: the minor is read from the session row, which registering again overwrites (`workers.rs:338-343`).
  - `crates/application/src/worker/withdrawal.rs:232-234, 269-271`: the only protection is the cap at `PROTOCOL_MINOR`.
  - `sdks/SPEC.md:353` cites "OD-2026-11-21-6" for reserving 1.8. That decision is not in `docs/owner-decisions.md`, which has only OD-2026-11-21-4.
  - The register in `sdks/SPEC.md:718-730` leaves 1.8 free, and its rule says the next contract to merge (drafts 020, 021, 025, 026) takes it.
- **Why it matters:** whichever feature takes minor 8 and raises `PROTOCOL_MINOR` turns the withdrawal on with no other change. At that point round-2 minor 1 brings back the round-1 Blocker:
  1. An old SDK that forwards handler codes holds lease L under session S at minor 6.
  2. A newer process registers with the same subject and `worker_name`, so S is now stored at minor 8.
  3. The old process's handler charges a card and raises `TASK_NOT_STARTED`.
  4. The server withdraws. The effect re-runs up to four times per attempt, ignoring `retryable=false` and `maxAttempts=1`.
- **Smallest fix:**
  - In `withdrawal.rs`, add `const _: () = assert!(super::PROTOCOL_MINOR < HAND_BACK_PROTOCOL_MINOR, "read the minor per lease (ADR-0099 decision 9) before raising the ceiling to 8");`.
  - Add a 1.8 row to §9 ("reserved: delivery acknowledgement, reserves `TASK_NOT_STARTED`"). Either record OD-2026-11-21-6 in `docs/owner-decisions.md` or drop the reference.
  - Record round-2 minor 1 in `research.md` as moved to ADR-0099 decision 9.
- **Proof:** the assertion fails the build the moment someone raises `PROTOCOL_MINOR` to 8.

### 2. Minor (premise verified false; starvation is a suspicion): the derivation of D006-085 does not hold as written

- **Where:**
  - `20270208000200_lease_withdrawal_floor.up.sql:12-13` and `:45`.
  - `research.md`, D006-085.
  - `docs/runbooks/lease-withdrawal.md:103-105`.
- **The false premise (verified):** a registration has no two-second lock timeout. `register` runs `instance_register` on the shared pool (`crates/infra-postgres/src/instances.rs:275-293`), which only has a 5 s `statement_timeout` (`pool.rs:33`). The 2 s at `instances.rs:59` is `OperatorBounds`, which applies to operator transactions only. The runbook sentence "never held past their own lock timeout" describes a timeout that does not exist.
- **The starvation (suspicion, from PostgreSQL semantics):**
  - `lock_timeout` limits each lock wait separately, not the total.
  - A new `FOR SHARE` locker of the requirement row does not queue behind the waiting DELETE. It joins the existing share lockers, and the DELETE then waits again for the new set.
  - So while engines keep withdrawing in overlapping transactions shorter than 1 s, the removal holds `platform_requirement_epoch FOR UPDATE` for as long as the statement runs. From `psql` there is no `statement_timeout`, so that is without limit.
  - Every registration in that time fails at 5 s. A rolling deploy or scale-out cannot bring processes up.
  - D006-085's line "This also ends the starvation" is the unproven claim.
- **How bad:** low. It needs an operator who skips "set `SAGAT_LEASE_WITHDRAWAL=off` first" (runbook :111). A removal made while withdrawals happen is refused once it gets through anyway.
- **Smallest fix:**
  - Have the runbook run `SET statement_timeout = '1500ms';` before `platform_requirement_remove`. That limits the whole call, which `lock_timeout` cannot do.
  - Correct the premise in the migration comment, in D006-085 and in the runbook.
  - Optional: take the requirement row `FOR UPDATE` before the epoch in the function, so any starvation happens without the epoch held.
- **Test** (in `migrations.rs`, with `pg_stat_activity` as the barrier):
  1. A calls `lease_withdrawal_required()`.
  2. B calls the removal. Wait until B waits for a lock.
  3. C calls `lease_withdrawal_required()`. Assert it returns without waiting.
  4. Commit A. Assert B is still waiting. This shows the bound applies to each wait.
  5. Repeat with B under `statement_timeout = 1500ms` and assert B fails within that bound.

### 3. Minor (verified): the scoped half of the dead-token fix has no test

- **Where:** `crates/application/src/worker/scoped_reports.rs:383`. The path heartbeat reads `reported_by` at `path.rs:258-267`.
- **Why it matters:** round-2 minor 4 covered both paths. Only the root has a test that would catch a regression. I verified the scoped code by reading it.
- **Smallest fix:** a test in `tests/integration/workflow-primitives/bounded_worker.rs`, next to `a_scoped_lease_is_withdrawn_in_its_path_for_either_cause` (which already sets ceiling 8). Hand back a scoped lease, then heartbeat its instance with the same token, and expect `STALE_ATTEMPT`.

## Notes (not counted)

- **Index check by name only.** The migration accepts any index named `run_events_lease_withdrawn_idx`, checking only that it is valid (`up.sql:18-25`). If an operator pre-builds one with a wrong predicate, the floor check goes back to a full scan while the epoch lock is held. Checking `indrelid = 'run_events'::regclass` and `pg_get_expr(indpred, indrelid)` in the DO block would refuse it.
- **Test name overclaims.** `the_removal_waits_for_a_lock_at_most_one_second_and_probes_an_index` does not prove the probe uses the index. An `EXPLAIN` of the guard's query would.
- **SDK minor per session.** The TypeScript and Python hosts read the host-wide accepted version (`host.ts:3478`, `host.py:2859` in the diff), not the minor of the session that received the lease, as §6.2 says. The server's check does not depend on this, so it is a fidelity point, not a security one.

## Classes I looked at and found nothing in

- **Tenant boundary:**
  - `lease_holder_minor` joins on `tenant_id`.
  - The withdrawal groups by tenant, and the tenant comes from the claim principal.
  - The census, the floor guard and their error texts are reachable by the operator role only.
  - The partial index exposes nothing.
- **Secrets and tokens:**
  - The new SDK warnings log the task id and the reason. I checked all six "drops … lapses" lines.
  - `UndeliveredLease` still redacts its token, and the metric labels are closed enums.
  - The census's `kinds_live` tokens go through a closed `InstanceKind::parse`.
- **The operator command (`each_kind_live`):**
  - The list of capabilities is closed, and the entry for `lease-withdrawal-v1` sets the field to false.
  - The census is judged again under the epoch lock (`capability_activation.rs:303-309`).
  - A kind with only lacking processes is still refused through `lacking`, and all SQL is bound.
- **SECURITY DEFINER functions and grants:**
  - The recreated census is pinned and revoked. It had no grant to lose, and the runtime-role refusal is tested (`runtime_privileges.rs:107`).
  - The guard is not a definer function and runs under the pinned path of `platform_requirement_remove`.
  - The migration adds no grant for `sagat_app`.
- **Injection and parsing:** there is no dynamic SQL in the change.
- **Shutdown tracker (D006-088):** a token held by queued frames can delay a drain, but only up to the stream window and the grace.

## Commands to run

```sh
SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test -p sagat-tests --test contract_grpc_durable_execution lease_withdrawal --locked
cargo test -p sagat-tests --test integration_platform_foundation removal --locked
cargo test -p sagat-tests --test integration_schedules_bulk_operations the_functions_are_security_definer --locked
psql "$DATABASE_URL" -c "EXPLAIN SELECT 1 FROM run_events e WHERE e.event_type = 'step.lease_withdrawn' LIMIT 1"
```

**Counts:** 0 Blockers, 0 Majors, 3 Minors.

Files:
- crates/application/src/worker/withdrawal.rs
- crates/application/src/worker.rs
- crates/application/src/worker/scoped_reports.rs
- crates/infra-postgres/src/execution.rs
- crates/infra-postgres/src/workers.rs
- crates/infra-postgres/src/instances.rs
- crates/infra-postgres/src/pool.rs
- crates/infra-postgres/migrations/20270208000200_lease_withdrawal_floor.up.sql
- crates/infra-postgres/migrations/20270208000200_lease_withdrawal_floor.down.sql
- apps/control-plane/src/capability_activation.rs
- sdks/SPEC.md
- docs/runbooks/lease-withdrawal.md
- specs/006-sdk-multilanguage/research.md
- tests/integration/schedules-bulk-operations/instances.rs
