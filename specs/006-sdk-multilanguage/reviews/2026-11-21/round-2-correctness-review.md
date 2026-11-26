# Correctness review, round 2, 2026-11-21

The `correctness-reviewer` agent (read only), given the diff of the fixes of the first round (`0f482673..557aff1b`, the six
SDKs' code left out) and the documents of the feature. Its final message, verbatim (only its closing list of files is
shortened to paths relative to the worktree):

---

# Correctness re-review, 2026-11-21 (round 2)

Read only; I ran nothing. Every claim below comes from reading the code, the diff, the SQL and the CI workflow. I mark each one as confirmed in code or as plausible.

No Blocker. Most round-1 findings are closed in code:
- the cap of four, counted from the journal under the run lock;
- the 1.8 minor gate on `TASK_NOT_STARTED`;
- the requirement re-read under a share lock, plus the removal trigger;
- the claim stop between run commits;
- producers and unary claims on the tracker;
- the slot moved into the claim;
- the semaphore.

Three Majors remain:
1. A behaviour change against `develop` that ships by default: G006-031.
2. B3's fitness check does not run on the pull requests that could break it.
3. A new test cannot fail for the reason it is named for.

## Status of the round-1 Blocker and Major findings

| Finding | Status | Basis |
|---|---|---|
| Security B1 / Architecture B1 (a handler can raise `TASK_NOT_STARTED`) | **Closed** on the server | `withdrawal.rs:171-194` refuses below minor 8 and at the cap. Go rewrites a handler-raised code (`sdks/go/worker/task.go:379`). The server negotiates at most 1.6 (`worker.rs:75`), so the honoured path cannot be reached in production yet (G006-030). |
| Security M1 (no cap) | **Closed** | `command.rs` refuses the fifth withdrawal before any change. The count is per `instance_id`/attempt, read in the same transaction (`execution.rs:7722`), on the index `(tenant_id, run_id, instance_id, seq)`. An attempt only goes down through a withdrawal (`state.rs:1476`), and root activation refuses to schedule a step twice (`state.rs:2753`), so no reset or loop re-counts an earlier life. |
| Security M2 (unary slot) | **Closed in code**; its test is weak (Major 3) | `service.rs:600,611` |
| Architecture B2 (requirement removal) | **Closed** | See the race analysis under Minor 2. |
| Architecture B3 (capability lists) | **Partly closed** (Major 2) | |
| Architecture M1 (ADR supersession) | **Closed in text** | Acceptance is still the owner's; ADR-0098 is still Proposed. |
| Correctness M1 / M2 (claim stop, tests) | **Closed** | See below. |
| Correctness M3 / Architecture M2 (shutdown) | **Closed in code**; partly tested (Minor 3) | |
| Architecture M3 (concurrent withdrawals) | **Closed** | |
| Architecture M4 (missing evidence) | **Closed**, except that the release-0.2 assertion is vacuous (Minor 5) | |
| Architecture M5 (two activation commands) | **Closed** | |
| Correctness M4 / Architecture M6 (no SDK implements the hand-back) | **SDK code is present** | There is still no six-language conformance scenario (Minor 6). |

Why the closures hold:
- **Claim stop (D006-035):** `worker.rs:1099` checks the watch before each `claim_run`. Committed tasks are kept on a later error (`:1145-1147`), and the probe now runs after each commit (`:1122`).
- **The test `a_claim_whose_consumer_is_gone_leases_no_further_run` can fail both ways:**
  - Aborting at the probe loses the first lease, so `until_handed_over(1)` times out.
  - Ignoring the watch leases the second run, so `("READY",0,1) != ("READY",0,0)` and `step.leased` is not empty.
- **The cap tests** wait through `drain` (`lease_withdrawal.rs:192-200`), which now waits for the withdrawal task to end. So the "fifth stays `LEASED`" assertion is not a transient read.

## Major

**1. G006-031 ships by default: a host that stops now fails one-attempt steps for good. On `develop` the same lease lapsed into a review. (Confirmed in code.)**
- **Where:**
  - `sdks/go/worker/task.go:106-121,134-163`: the hand-back happens on every negotiated minor, with no gate on `w.minor`.
  - `crates/application/src/worker.rs:75`: `PROTOCOL_MINOR = 6`.
  - `crates/application/src/worker/withdrawal.rs:181`: below minor 8, the server takes the ordinary failure path.
- **Scenario:** a rolling restart of a Go host with one buffered lease of an Activity with `maxAttempts: 1` (a payment).
  - The host sends `FailStep(TASK_NOT_STARTED, retryable=true)`.
  - `hand_back` returns `Lapsed`, then `apply_scoped_or_root_failure` records `step.failed`.
  - With no attempts left, the run fails or takes its `onError` route (compensation) for work that never ran.
  - On `develop` it lapsed into a review that an operator can retry.
- **Why it matters:** this is a guarantee of the worker protocol. The fixes turned "free withdrawal" into "spend the attempt" for every host that exists today, and research.md defers the choice to the owner (G006-031). Until they decide, the branch ships the riskier default.
- **Why the tests miss it:** `a_hand_back_from_a_session_older_than_1_8_is_an_ordinary_failure_with_its_flag` asserts exactly this behaviour as correct.
- **Smallest fix:** every SDK hands back only when the accepted minor is at least 8 and otherwise drops the lease as `develop` does. Or get the owner's decision before merging.
- **Test that proves it:** an SDK conformance scenario where the host stops with one buffered lease of a one-attempt Activity on a 1.6 session, and the run does not end `FAILED`.

**2. The B3 fitness check does not run on pull requests, yet ADR-0098 decision 8 and D006-033 say it does. (Confirmed in code.)**
- **Where:** `.github/workflows/ci.yml:344-347`. `python3 scripts/check-architecture.py` runs only when `needs.changes.outputs.full == 'true'`, which means a release.
- The repository test `test_the_repository_passes` runs only when a PR changes `scripts/`.
- **Scenario:**
  - Feature 022 merges second with a migration newer than `20270208000000` that lists only its own values.
  - Its PR changes no script, so CI runs neither the rule nor the scripts' tests.
  - Nothing re-checks `develop` after a merge.
  - `lease-withdrawal-v1` drops out of the CHECKs and every engine's registration fails the CHECK in `instance_register`.
- Down migrations are excluded from the rule (`*.up.sql` only), so a rollback of one feature still erases the other's values (B3's second half).
- **Smallest fix:**
  - Run `capability_list_errors` in a job with no `full` condition: a separate step, cheap, no build.
  - Or correct ADR-0098 decision 8 and D006-033 to say "at release".
  - Add a down-migration rule: each down restores exactly the previous up's list.
- **Test that proves it:** a PR that adds only `crates/infra-postgres/migrations/2027030*_x.up.sql` with a short list; the "architecture rules" step must fail on it.

**3. `a_cancelled_unary_poll_keeps_its_tenant_slot_until_its_claim_ends` can pass with the fix reverted. (Plausible; it depends on scheduling.)**
- **Where:** `tests/contract/durable-execution/grpc/lease_withdrawal.rs:756-787`, with the probe `held_until_released` at `:459`.
- **The gap:** the probe ignores the `Abandonment`. The test aborts the two calls, then sends the third at once, without waiting for the server to drop the two handlers.
- **Mutation:** keep `slot` in the handler. The RST_STREAM frames are handled on the handlers' own tasks, which tokio may run after the new request's task (the LIFO slot favours the newest task). The third call then still sees both slots held and gets `RESOURCE_EXHAUSTED`, so the test passes. The ordering is not forced: this is the race-without-barrier pattern rule 18 forbids.
- **Smallest fix:** in the probe, after `notify_one`, await `abandoned.wait()` and then signal a second `Notify`. The test waits for two "abandoned" signals before the third poll, then waits for the permit. Show it red with the slot left in the handler.

## Minor

1. **A handed-back lease still answers heartbeats with OK.** This is round-1 correctness Minor 2, still open (confirmed in code).
   - **Where:** `worker.rs:2443` sets `effects.reported` and the withdrawal branch (`:2477-2506`) keeps it. `reported_by` (`execution.rs:5037-5058`) matches any status other than `LEASED`/`RUNNING`, so the heartbeat at `worker.rs:1887` gets `cancel_requested: false`.
   - **Why it matters:** D006-031 step 1 relies on "its token is dead", which holds for reports but not for heartbeats. This only matters for a 1.8 host that lies.
   - **Fix:** clear `effects.reported` in both `NOT_STARTED` branches.
   - **Test:** hand back on a forged 1.8 session, heartbeat with the same token, expect `STALE_ATTEMPT`.

2. **D006-032's "cannot cross" has no race test.** By reading, it holds under READ COMMITTED:
   - A DELETE's BEFORE ROW trigger locks the tuple first, so it waits for the `FOR SHARE` holder.
   - The trigger's plpgsql `EXISTS` then takes a fresh snapshot and sees the committed event.
   - The reverse order also holds: `FOR SHARE` waits for the delete, then finds no row and returns false.
   - **Exception (plausible):** a removal run in REPEATABLE READ (for example `default_transaction_isolation` set on the operator's session) keeps its first snapshot and would let the removal through. Both tests (`:657` and the registry test) are sequential.
   - **Test, in pure SQL with `pg_locks` as the barrier:**
     - (a) tx A runs `SELECT lease_withdrawal_required()`. Tx B runs `platform_requirement_remove` and blocks; wait until it shows in `pg_locks`. A inserts a `step.lease_withdrawn` row and commits. B must fail with `restrict_violation`.
     - (b) the reverse order: A must get `false`.
     - Optionally, `SET TRANSACTION ISOLATION LEVEL REPEATABLE READ` inside the trigger's guard, or check `current_setting('transaction_isolation')` there.

3. **Parts of D006-036 are untested.** The shutdown test (`:726`) covers only the stream producer and calls `Withdrawals::drain` directly.
   - Moving the unary claim back to `tokio::spawn` keeps every test green: `until_withdrawal_ran` waits on `until_handed_over` before draining.
   - `drain_bound`, `stop_seen` and the drain-before-`pool.close()` order in `apps/execution-engine/src/lib.rs:810-866` have no test.
   - **Fix:** a unary variant of the shutdown test, and a unit test of `drain_bound`.

4. **Security m1 is closed in code (`withdrawal.rs:217` uses `lease_unexpired_instance`, with the expiry condition), but has no test.**
   - **Test:** let the lease expire before the reaper runs, hand back, and assert `step.failed` with no `step.lease_withdrawn`.

5. **The release-0.2 assertion cannot fail for the reason it is named for.** In `release_upgrade_inflight.rs` (around line 239), "no `step.lease_withdrawn` before the activation" has nothing to do with the gate: that profile never drops an undelivered lease, so the count is 0 under any mutation.
   - The real proof is `without_the_recorded_requirement_an_undelivered_lease_lapses_as_before`. D006-004 should cite that test and not the upgrade path.

6. **There is no six-language conformance scenario for the hand-back or for the handler-code rewrite.** `tests/contract/sdk-conformance` has no match for either (AGENTS rule 8). Only per-SDK unit tests exist.

7. **The arithmetic of §6.5 is wrong.**
   - **Where:** `sdks/SPEC.md:496,502`.
   - "H ≥ 5 keeps at least 150 ms" is false. H=5 gives ⌈64/5⌉ = 13 rounds, so 1500/13 ≈ 115 ms; H=6 gives about 136 ms; only H ≥ 7 gives 150 ms (10 rounds).
   - The bound is 7 ≤ H ≤ 10. H = 8 still fits; the other figures check out (187.5 ms, 1.875 s, 93.75 ms, 8/128 = 6.25%, 128 = 512/4).

8. **There is no recorded evidence for the fixes.**
   - `specs/006-sdk-multilanguage/tasks.md` has no entry for D006-030 to D006-039, and no run of the new tests is recorded anywhere.
   - ADR-0098's "Enforced by" names tests whose results I cannot see.

9. **Still open from round 1, and not in this diff:**
   - correctness Minor 1 (an ambiguous commit in `claim_run` drops leases with no guard);
   - security m2 (records keyed by attempt without generation: `script_executions`, payload disclosure, the secret audit), now bounded to four extra leases per attempt;
   - Studio (`frontend/apps/studio/src/features/runs/attempts.ts:129-131` and no label for `step.lease_withdrawn`);
   - the public `with_delivery_probe` seam;
   - `Action::Withdraw` missing from `crates/domain/tests/execution_properties.rs`;
   - correctness Minor 3 is only partly fixed: a frame counted as streamed and then dropped at the window deadline or at token expiry is still counted in `worker_task_stream_tasks_total`.

## Note

- **One hot row for every withdrawal and hand-back (plausible).** Each one takes `FOR SHARE` on the same `platform_requirements` row (`lease_withdrawal_required()`).
  - Every lock writes the tuple's xmax, and concurrent holders create MultiXacts. At 5 per engine plus up to 8 per stopping host that is real churn on one page.
  - **Option:** `pg_advisory_xact_lock_shared(k)` in the function and `pg_advisory_xact_lock(k)` in the trigger before its `EXISTS`, which gives the same ordering with no tuple writes.
- **A long scan under the epoch lock.** `platform_requirement_remove` holds the epoch `FOR UPDATE` while the new trigger scans `run_events` (there is no index on `event_type`). Every `instance_register` (epoch `FOR SHARE`) waits for the length of the scan.
  - The runbook says the scan is slow, not that registrations block. It also gives no `lock_timeout` or `statement_timeout` for the manual psql call.
- **The derivation of four (D006-031) assumes rather than derives that the server's cause happens at most once per honest attempt.**
  - With 5 s windows and rolling restarts of the engines, it can recur for an honest attempt.
  - Past the cap the cost is `develop`'s behaviour (a lapse, then a review), so the cap is safe. But "a property of the client" is a claim, not a proof.
- **The forged-minor seam changes more than it is meant to.** Raising `accepted_protocol_minor` to 8 (`as_protocol_1_8`) also moves every `>=` branch on the minor in claim and encoding, for a state the server cannot produce.
- **`lease_holder_minor` (`execution.rs:7744`) does not check the session's liveness or expiry,** although its doc says "a live session". This is harmless while the lease is unexpired.
- **`withdraw_root` counts a group as `lapsed` before any fence check** (`withdrawal.rs:340`), so steps that had already moved on are counted `lapsed`, not `fence_missed`. This only affects the counter.

## Commands that would settle the open points

```sh
cargo test -p sagat-tests --test contract_grpc_durable_execution lease_withdrawal -- --nocapture
cargo test -p sagat-tests --test integration_workflow_primitives a_scoped_lease_is_withdrawn_in_its_path_for_either_cause
cargo test -p sagat-tests --test integration_platform_foundation the_lease_withdrawal_rolls_back_only_while_nothing_names_it
cargo test -p sagat-domain --test lease_withdrawal
python3 -m unittest discover -s scripts/tests -v && python3 scripts/check-architecture.py
```

For Major 3: revert `service.rs:600,611` (keep `slot` in the handler) and rerun `a_cancelled_unary_poll_keeps_its_tenant_slot_until_its_claim_ends` about 20 times. A test that can fail stays red every time.

Files relevant to this review: `crates/application/src/worker.rs`, `crates/application/src/worker/withdrawal.rs`,
`crates/application/src/worker/scoped_reports.rs`, `crates/api-grpc/src/service.rs`, `crates/api-grpc/src/undelivered.rs`,
`crates/infra-postgres/src/execution.rs`, `crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql`,
`crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql`, `apps/execution-engine/src/lib.rs`,
`sdks/go/worker/task.go`, `sdks/SPEC.md`, `.github/workflows/ci.yml`, `tests/contract/durable-execution/grpc/lease_withdrawal.rs`,
`tests/integration/workflow-primitives/release_upgrade_inflight.rs`, `specs/006-sdk-multilanguage/research.md` (all under
`/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/`).

Diff reviewed: /private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/withdraw-server-fixes.diff
