# Security review, round 2, 2026-11-21

The `security-reviewer` agent (read only), given the diff of the fixes of the first round (`0f482673..557aff1b`, the six
SDKs' code left out) and the documents of the feature. Its final message, verbatim:

---

# Security re-review: `bugfix/withdraw-undelivered-leases` at 557aff1b (server fixes)

I could only read code, so nothing below has been run. "Verified" means I traced it in the worktree. "Suspicion" means it rests on PostgreSQL behaviour I did not run. I end with the commands that would settle each point.

**Verdict:** the first round's Blocker and both Majors are closed in this build. I found no new Blocker or Major. There are four new Minor findings and some Notes. Two of the Minors sit in the `TASK_NOT_STARTED` hand-back path, which this build cannot reach yet (the server negotiates at most protocol 1.6). They must be fixed before 1.8 ships.

## Status of the first round's findings

| Finding | Status | Evidence |
|---|---|---|
| **Security B1 / Architecture B1**: a handler can raise `TASK_NOT_STARTED` and bypass the retry budget | **Closed in this build** | `hand_back` (`crates/application/src/worker/withdrawal.rs:171`) refuses any holder minor below 8. `negotiate_protocol_details` clamps the agreed minor to `protocol_ceiling` (`crates/application/src/worker.rs:88-95, 175-195`), which is at most 6. So no session reaches the withdrawal: every hand-back is an ordinary failure that keeps its `retryable` flag. Two residual risks remain, see m1 below and N1. |
| **Security M1**: no cap on withdrawals | **Closed** | The cap of 4 per (step, attempt) is checked in the domain (`command.rs` `withdraw_lease` and `withdraw_scoped`). The count is read under the run lock on all four paths (`withdrawal.rs:356, 429` and inside `root_hand_back`/`path_hand_back`). The `attempt` column is filled for `step.lease_withdrawn` (`event.rs:1413`, `event_codec.rs:65`), and `instance_id` is written for root events (`execution.rs:5199-5233`). Journal growth is now at most 8 events per attempt. |
| **Security M2**: a cancelled unary poll frees the tenant slot | **Closed** | `let slot = auth.slot;` is moved into the tracked task (`crates/api-grpc/src/service.rs:600, 611`). |
| m1: hand-back accepted on an expired lease | **Closed** | `lease_unexpired_instance` and `lease_unexpired` are now in both hand-back verdicts (`withdrawal.rs:206-260`). Both queries check `lease_expires_at > clock_timestamp()` (`execution.rs:4835, 4856`). |
| m2: attempt number reused by keys without the generation | **Open, but bounded** | Not addressed. It is dormant until 1.8, and the cap limits it to 4 reuses per attempt (N2). |
| m3: no concurrency bound on withdrawals | **Closed** | A semaphore of `max(1, pool/4)` (`undelivered.rs:66-72, 160-163`), wired from `config.database.max_connections` (`apps/execution-engine/src/lib.rs:822-826`). Fairness across tenants is not bounded (N4). |
| Note: stale gate | **Closed, better than proposed** | Every withdrawal transaction re-reads the requirement under a share lock (`withdrawal.rs:340, 413`). |
| Note: untracked claims at shutdown | **Closed** | The producer and the unary claim run on the tracker (`service.rs:610, 725`). The drain is bounded by the grace left (`apps/execution-engine/src/lib.rs` `drain_bound`). |
| Note: `with_delivery_probe` is public | **Open** | Unchanged. The new public trait hook `ClaimWatch::committed` widens the test seam slightly (N5). |
| **Architecture B2**: removing the requirement lets old builds serve | **Closed under READ COMMITTED only** | See m2 below. |
| **Correctness minor 2**: heartbeat answers OK after a hand-back | **Open** | See m4 below. |

## New findings

### m1 (Minor, verified, dormant until 1.8): the hand-back is judged by the session's current minor, not the minor the lease was issued under

- **Where:**
  - `crates/infra-postgres/src/execution.rs:7737-7760` (`lease_holder_minor`) reads `worker_sessions.accepted_protocol_minor` through `steps.leased_by` at report time.
  - `crates/infra-postgres/src/workers.rs:338-344`: a re-registration on the same `(tenant_id, subject, worker_name)` keeps the row's `id` (`ON CONFLICT … DO UPDATE`, `RETURNING id`) and overwrites `accepted_protocol_minor` and `token_hash`.
  - Reports are authorised by the bearer and the lease token only (`TaskRef`, `worker.rs:512-519`; `verify`, `worker.rs:2935-2951`), not by the session token.
- **Scenario (after 1.8 ships):**
  1. Process A runs a released SDK older than 1.8, which forwards handler codes. It holds lease L under session S, at minor 6.
  2. Process B, upgraded, registers with the same subject and `worker_name` (a fixed worker name in a deployment, or a StatefulSet pod reusing its name). S is now minor 8.
  3. A's handler charges a card and raises `ActivityFailure("TASK_NOT_STARTED")` within 500 ms. A forwards it with L's token.
  4. The server reads minor 8 and withdraws. The effect is re-run up to 4 times per attempt, ignoring `retryable=false` and `maxAttempts=1`. That is exactly the B1 bypass, through a door D006-030 assumes is shut.
- **Smallest fix:** record the minor at lease time and judge the hand-back by it. Two options:
  - a `leased_protocol_minor` column on `steps`, written by the claim next to `leased_by`, and read in place of the join;
  - the minor in the `step.leased` detail, read from the newest `step.leased` of the fence.
- **Test:** register S at 1.6 and lease. Then re-register the same name with the minor raised to 8 (the same SQL `as_protocol_1_8` uses, run after the lease, not before). Hand back with L's token and assert `step.failed`, not `step.lease_withdrawn`.

### m2 (Minor, verified by reading PostgreSQL semantics, not run): the replay floor holds only under READ COMMITTED

- **Where:** the guard `platform_requirements_replay_floor_guard`, `crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql:52-62`.
- **Why it works today:** the guard's `EXISTS` uses the snapshot its statement takes. Under READ COMMITTED that snapshot is fresh after the row-lock wait, so the guard sees a withdrawal that committed meanwhile.
- **Why it fails otherwise:** under REPEATABLE READ or SERIALIZABLE (the operator's session, or a role or database `default_transaction_isolation`), every statement uses the transaction's first snapshot. A share lock that was taken and released does not cause a serialization failure on DELETE.
- **Scenario:**
  1. Session A: `BEGIN ISOLATION LEVEL REPEATABLE READ; SELECT 1;`
  2. An engine commits the first `step.lease_withdrawn`.
  3. A: `SELECT * FROM platform_requirement_remove('lease-withdrawal-v1','ops'); COMMIT;` returns `REMOVED`.
  4. Older registry-aware builds can serve again, and a journal holds an event they cannot decode. That is architecture B2 again.
- **Smallest fix:** in the guard, for this capability, raise when `current_setting('transaction_isolation') <> 'read committed'`.
- **Test:** the scenario above in `tests/integration/platform-foundation/migrations.rs` or in the contract file; expect the refusal.

### m3 (Minor; the scan is verified, the starvation is a suspicion): removing the requirement holds the epoch lock across a lock wait and a full scan of `run_events`

- **Where:**
  - `platform_requirement_remove` takes `platform_requirement_epoch … FOR UPDATE`, then DELETEs the row (`20270113000000_instance_registry.up.sql:364, 371`).
  - The new guard then scans `run_events` by `event_type`. There is no index on that column: the only `run_events` index created in the migrations is `run_events_instance_idx`.
  - `instance_register` needs the epoch `FOR SHARE` (`:197`) and runs with a 2 s `lock_timeout` (`crates/infra-postgres/src/instances.rs:59, 120`).
- **Effect:** in the only case where removal is allowed (no event anywhere), the scan reads the whole journal. For its whole length:
  - no control plane or engine can register, or re-register after `out_found=false`. A scale-out or rolling deploy during that window cannot bring processes up;
  - any engine not yet switched off blocks inside `lease_withdrawal_required()` while holding a run lock, until its 5 s `statement_timeout`.
- **Suspicion (starvation):** PostgreSQL lets a compatible `FOR SHARE` locker join existing share lockers without queueing behind a waiting DELETE. While withdrawals keep overlapping, the removal can wait indefinitely, still holding the epoch lock. A raw `psql` removal, which the runbook prescribes, has no timeout.
- **What the runbook misses:** it mentions the scan's duration but not the registration stall.
- **Smallest fix:**
  - Add a partial index, `CREATE INDEX CONCURRENTLY run_events_lease_withdrawn_idx ON run_events (tenant_id) WHERE event_type = 'step.lease_withdrawn'`, in its own non-transactional migration. The `EXISTS` (and the down guard) becomes an index probe, and writes cost something only for this rare event.
  - Add `ALTER FUNCTION platform_requirement_remove(text, text) SET lock_timeout = '2s'` (or say so in the runbook).
- **Check:** `EXPLAIN SELECT 1 FROM run_events WHERE event_type = 'step.lease_withdrawn' LIMIT 1;` on a populated database. Then hold `SELECT lease_withdrawal_required()` open in one transaction, call `platform_requirement_remove('lease-withdrawal-v1', …)` in a second, and `instance_register(…)` in a third; expect the third to fail on its lock timeout.

### m4 (Minor, carried from correctness minor 2, still open, dormant until 1.8): a handed-back lease token still gets OK heartbeats

- **Where:**
  - `effects.reported = Some(reported_lease(task))` (`worker.rs:2443`) is passed unchanged into the withdrawal branch (`:2483`). The scoped report does the same (`scoped_reports.rs:269`).
  - The heartbeat returns OK, with `cancel_requested=false`, when `reported_by` matches (`worker.rs:1885-1893`).
- **Scenario:** a host that broke "never after start" hands back, then keeps heartbeating its running handler. It is never told `STALE_ATTEMPT` while the step is leased again, so the handler runs twice and nothing shows it.
- **Fix:** set `effects.reported = None` in both withdrawal branches.
- **Test:** hand back, heartbeat with the same token, expect `STALE_ATTEMPT`.

## Notes

- **N1 (residual B1 at 1.8):** a raw or hostile client that negotiates 1.8 still gets 4 re-runs per attempt that ignore `retryable=false`. The first round's defence in depth was not added: refuse the withdrawal when this lease resolved secrets, wrote a blob, or has a `script_executions` row. ADR-0099 should state this residual risk.
- **N2 (m2 carried):** `script_executions` keeps the first write per `(tenant, run, node, attempt)`, and the audit reasons name the attempt without the generation. This is limited by the cap and is dormant.
- **N3 (least privilege):**
  - `lease_withdrawal_required()` is correct as written: SECURITY DEFINER, `search_path = pg_catalog, pg_temp`, a schema-qualified table, revoked from PUBLIC, and granted only to `sagat_app` (`up.sql:71-85`). The guard runs under the pinned path of `platform_requirement_remove`, and `ENABLE ALWAYS` covers `session_replication_role = replica`.
  - Two side effects: the function breaks the registry's stated rule that the runtime role "writes nothing and locks nothing" (`20270113000000_instance_registry.up.sql:381-382`), and every withdrawal now writes a WAL-logged row lock (a MultiXact when concurrent) on one shared row. A pair of `pg_advisory_xact_lock_shared` / `pg_advisory_xact_lock` calls gives the same exclusion with no tuple writes.
  - No test pins the new function's definer status, its search path or its grant. `PROCESS_FUNCTIONS` (`tests/integration/schedules-bulk-operations/instances.rs:1074`) and the definer test at `:1213` still list three functions.
- **N4 (DoS, fairness):** the semaphore is per engine, not per tenant, and a permit is held across `Lock::Wait` on the run lock (`withdrawal.rs:336`). A tenant that drops streams in a loop can occupy all permits, so other tenants' withdrawals wait past their lease TTL and lapse. That is the pre-feature behaviour, not a new loss.
- **N5:** `ClaimWatch::committed` (`worker.rs`) and `with_delivery_probe` remain public test seams on production types.
- **N6 (docs):** with `SAGAT_LEASE_WITHDRAWAL=off`, hand-backs also become ordinary failures (`gate_outcome` → `Disabled` in `hand_back`). The runbook's "Stopping it" says only that leases lapse.

## Classes with no finding (looked)

- **Tenant boundary:** every new query is scoped by the transaction's tenant and run (`lease_withdrawals`, `lease_holder_minor` joining on `tenant_id`). The NOT_SENT tenant comes from the claim principal. A fence miss returns nothing to the caller.
- **Secrets in logs and metrics:** the new WARN lines carry the run, node and attempt only. Labels go through one helper with closed values (`WithdrawalOutcome::as_str`), and `UndeliveredLease` still redacts its token.
- **Operator command:** closed allow-list (`capability_activation.rs:47-62`). `schedules-v1` and unknown tokens are refused at parse time. `--operator` goes through `unprintable` and `Subject::parse` and is logged through `shown`. The fleet is counted again under the epoch lock.
- **Withdrawal versus removal under READ COMMITTED:** both orders are safe (share lock, then a fresh snapshot in the guard). Run transactions use the default READ COMMITTED; the only REPEATABLE READ transactions are read-only.

## Commands to run

```sh
SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test -p sagat-tests --test contract_grpc_durable_execution lease_withdrawal --locked
cargo test -p sagat-tests --test integration_platform_foundation the_lease_withdrawal_rolls_back_only_while_nothing_names_it --locked
cargo test -p sagat-tests --test integration_workflow_primitives a_scoped_lease_is_withdrawn_in_its_path_for_either_cause --locked
psql "$DATABASE_URL" -c "\d run_events" -c "EXPLAIN SELECT 1 FROM run_events WHERE event_type='step.lease_withdrawn' LIMIT 1"
```

The m1, m2, m3 and m4 tests above do not exist yet. Each should be seen failing on 557aff1b before its fix.

Files: the diff at /private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/withdraw-server-fixes.diff, and in the worktree:
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/worker/withdrawal.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/worker.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/worker/scoped_reports.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/src/execution.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/src/workers.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/api-grpc/src/service.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/api-grpc/src/undelivered.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/apps/control-plane/src/capability_activation.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/tests/integration/schedules-bulk-operations/instances.rs
