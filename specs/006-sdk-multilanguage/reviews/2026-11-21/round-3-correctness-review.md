# Correctness review, round 3, 2026-11-21: lease withdrawal (pull request #260, commit 45a3f42b)

**Verdict.** There is no Blocker, but there is one Major. The fixes for round 2 hold up when I read them. The six SDKs gate the hand-back at 1.8, each at its single hand-back function. The tracker token closes the unary drain race. The replay-floor guard sees a withdrawal that committed while the removal waited, and refuses outside READ COMMITTED. A handed-back token is dead for heartbeats. Each new test I checked fails when its fix is removed, except where noted below.

The Major is in the design, not in today's code paths. D006-080 says that "from 1.8 the server withdraws the lease without spending the attempt". The server does that only when the requirement is recorded and the switch is on. In every other case a 1.8 hand-back still spends the attempt. That is the regression G006-031 described, moved from "today" to "the day 1.8 negotiates, on every fleet that has not run the activation". It cannot be reached in this build, because the build negotiates 1.6 and the ceiling is 6. Under OD-2026-11-21-4, a Major keeps ADR-0098 Proposed unless it is closed by a fix or by a recorded decision.

I ran nothing. "Read" below means I confirmed it in the code or text; "Suspect" means I reasoned it out and did not run it.

## Round-2 correctness findings

| Round-2 finding | Status | Basis |
|---|---|---|
| Major 1: G006-031, hand-back on every minor | **Closed for minors below 1.8** (but see new Major 1) | Read: the gate returns before any report in all six SDKs: `sdks/go/worker/task.go:141`, `WorkerHost.cs` `HandBack`, `WorkerHost.java` `handBack`, `host.py` `_hand_back`, `sdks/rust/src/worker.rs` `hand_back`, `host.ts` `#handBack`. Each SDK has tests of both sides, and each below-1.8 test fails if its gate is removed. No run of the six SDK suites is reported. |
| Major 2: the capability-list rule does not run on pull requests; down migrations unchecked | **Closed** | Read: `crates/infra-postgres/tests/capability_lists.rs` has both rules. `scripts/select-test-targets.py:450` selects the dependents of a changed package, so a change to `crates/application` also runs `sagat-infra-postgres`. The down-migration rule is in both checkers, with Python tests. Residual: `research.md:487` (D006-033) still says "Down migrations are not checked yet" (Minor 6). |
| Major 3: the unary-slot test passes with its fix reverted | **Closed** | Read: the probe now waits for the `Abandonment` and adds a permit to `gone`, and the test waits for two permits before the third poll. With the slot moved back into the handler, the third poll succeeds, so the test turns red. |
| Minor 1: a handed-back token still answers heartbeats | **Closed for root leases**; the scoped branch is untested | `worker.rs:2485` plus the test `a_heartbeat_with_a_handed_back_token_is_stale`. `scoped_reports.rs:383` has no test (Minor 5). |
| Minor 2: no race test for "cannot cross"; REPEATABLE READ | **Closed** | Both orders are tested with a `pg_stat_activity` barrier (`migrations.rs:831`, `:877`). The isolation guard has a test (`:907`). New timing dependency: Minor 2. |
| Minor 3: parts of D006-036 untested | **Mostly closed** | Added: the unary shutdown test, the `drain_bound` unit test, and the D006-088 token. The order "drain, then `pool.close()`" (`apps/execution-engine/src/lib.rs:827-855`) is correct but still has no test. |
| Minor 4: an expired lease handed back has no test | **Open, not recorded** | No test, no gap, no decision. |
| Minor 5: the release-0.2 assertion is vacuous | **Open, not recorded** | ADR-0098 "Enforced by" and `research.md:238` (D006-004) still cite `release_upgrade_inflight.rs`. |
| Minor 6: no six-language conformance scenario | **Moved, with a reason** | D006-081 moves it to the build of protocol 1.8. |
| Minor 7: the arithmetic of §6.5 | **Closed** | Checked: H=6 gives ⌈64/6⌉ = 11 rounds and 1500/11 ≈ 136 ms; H=7 gives 10 rounds and 150 ms, so 7 ≤ H ≤ 10. |
| Minor 8: no recorded evidence | **Open** | `specs/006-sdk-multilanguage/tasks.md` has no entry for D006-030 to D006-088. See Minor 7. |
| Minor 9: items left from round 1 | **Partly closed** | Closed: the Studio label (D006-087, with a test). Open and not recorded: the ambiguous commit in `claim_run`; security m2; the public `with_delivery_probe` (`crates/api-grpc/src/service.rs:115`); `Action::Withdraw` (no match in `crates/domain/tests/execution_properties.rs`); the stream counter. |
| Notes: hot row, scan under the epoch lock, counter, liveness of the session minor | **Mixed** | The scan is addressed by D006-085 (Minors 1, 3 and 4 concern it). Still open: the churn of the hot `FOR SHARE` row; `withdraw_root` counting a group `lapsed` before any fence check (`withdrawal.rs:358`); `lease_holder_minor` not checking that the session is live. |

## New findings

### Major 1: from protocol 1.8, a hand-back on a fleet that has not activated the requirement (or has the switch off) spends the attempt, so G006-031 comes back (confirmed in code and text; cannot happen until 1.8 negotiates)

- **Where:**
  - `crates/application/src/worker/withdrawal.rs:194-199` returns `Disabled` or `Lapsed` when the gate is closed or the requirement is not recorded.
  - `crates/application/src/worker.rs:2513-2533` then falls through to `apply_scoped_or_root_failure`.
  - The test `tests/contract/durable-execution/grpc/lease_withdrawal.rs:422-463` asserts this as correct: on a session raised to 8 with no requirement, the step is `FAILED` and the run is `FAILED`.
- **Scenario:**
  1. The delivery acknowledgement build ships, and its sessions negotiate 1.8.
  2. The operator has not run `capability-activate`. That is the default (G006-003), and the runbook's first rule is "Nothing changes until the operator records the requirement" (`docs/runbooks/lease-withdrawal.md:11`).
  3. A Go host stops with one buffered lease of an Activity with `maxAttempts: 1`, and hands it back.
  4. `hand_back` returns `Err(Lapsed)`, the server records `step.failed`, and the run fails or takes its `onError` route for work that never ran.
  5. On `develop`, the same lease lapses into `step.needs_review`.
  6. The same happens with `SAGAT_LEASE_WITHDRAWAL=off`. ADR-0098 "Rollback" (line 93) and the runbook (line 90) say "their leases lapse as before"; for hand-backs that is false.
- **Why it matters:** D006-080 removes this exact regression below 1.8, and says it "changes no guarantee". ADR-0098 decision 5 and SPEC §6.2 keep it from 1.8 on. Accepting the ADR now freezes words that the 1.8 build will implement.
- **Why the tests miss it:** the test above asserts the behaviour. Nothing tests a 1.8 session on a fleet that was not activated against `develop`'s outcome.
- **Smallest fix, either:**
  - (a) On a session of 1.8 or later, when the verdict is `Lapsed` or `Disabled`, record no failure and answer `STALE_ATTEMPT`, which every SDK already treats as a quiet end. The lease then lapses exactly as on `develop`. Keep the ordinary failure for `Capped`, which D006-031 chose on purpose, and for `FenceMissed`.
  - (b) A recorded decision by the coordinator or the owner that accepts spending the attempt for 1.8 fleets before activation, with ADR-0098 "Consequences" and "Rollback" and the runbook lines 11 and 90 corrected.
- **Test that proves it:** on a session raised to 8, with no requirement and then with the switch off, hand back a one-attempt Activity. Expect no `step.failed`, the step still `LEASED`, and after expiry `step.needs_review`.

### Minor 1: the one-second bound on the removal applies to each lock wait, not to the whole removal (suspected; based on how PostgreSQL row locks behave)

- **Where:** `20270208000200_lease_withdrawal_floor.up.sql:45` and D006-085.
- **The problem:** a new `FOR SHARE` request on a row that holds only share locks does not queue behind a waiting `DELETE`. After each holder ends, the `DELETE` waits again for the newcomers, and each wait gets a fresh `lock_timeout`.
- **Scenario:** a burst of overlapping withdrawals on several engines lasts more than 2 s. The removal keeps the epoch `FOR UPDATE` throughout. `instance_register` (2 s lock timeout, `crates/infra-postgres/src/instances.rs:59`) fails for any process that starts in that window.
- **What this contradicts:** the claim that this "also ends the starvation" is false.
- **Side effect:** the setting applies to the removal of every capability, while the specification states it only for `lease-withdrawal-v1`.
- **Fix:** have the runbook set `SET statement_timeout = '1500ms'` in the operator's session. That bounds the whole call; a function-level `SET statement_timeout` would not apply to the statement already running. Alternatively, take the requirement row `FOR UPDATE NOWAIT` before the epoch.
- **Test:** chain overlapping share holders for 3 s; a registration started meanwhile must succeed.

### Minor 2: `a_removal_that_waits_for_a_withdrawal_sees_its_event_and_is_refused` depends on wall-clock time (confirmed in code)

- **Where:** `tests/integration/platform-foundation/migrations.rs:831`.
- **The problem:** now that the removal has a 1 s `lock_timeout`, the withdrawal must commit within 1 s of the removal starting to wait. The barrier polls every 50 ms (`tests/support/mod.rs:427`). A loaded VM fails the test with "lock timeout". It fails loudly, so this is a flake and not a vacuous test.
- **Fix:** on this test's private database, raise the bound (`ALTER FUNCTION platform_requirement_remove(text, text) SET lock_timeout = '30s'`). The bound has its own test at `:947`.

### Minor 3: the test named "…and probes an index" does not check that the guard uses the index (confirmed in code)

- **Where:** `migrations.rs:947` asserts only that the index exists and is valid. A guard that scans the whole table passes.
- **Fix:** assert that `EXPLAIN (FORMAT JSON) SELECT 1 FROM run_events e WHERE e.event_type = 'step.lease_withdrawn' LIMIT 1` uses `run_events_lease_withdrawn_idx`.

### Minor 4: the down migration's look at the journal is not an index probe (confirmed in code)

- **Where:** the up migration's comment says it is (`20270208000200…up.sql:3`, and D006-085 repeats it).
- **The problem:** `20270208000200…down.sql:18` drops the index before the down of `20270208000000` runs its `EXISTS` (`20270208000000_lease_withdrawal.down.sql:19`). So that check scans `run_events` while it holds `ACCESS EXCLUSIVE` on the three registry tables.
- **Fix:** drop the index in the down of `000000`, after its guard; or correct the text.

### Minor 5: the scoped half of D006-086 has no test (confirmed in code)

- **Where:** `crates/application/src/worker/scoped_reports.rs:383`. Removing `effects.reported = None` there keeps every test green.
- **Fix:** add a scoped variant to `tests/integration/workflow-primitives/bounded_worker.rs`: a heartbeat with the handed-back token returns `STALE_ATTEMPT`.

### Minor 6: the decisions text is out of date (confirmed by reading)

- `research.md:487` (D006-033) still says "Down migrations are not checked yet", against D006-083.
- `research.md:542` and ADR-0098 "Evidence" say "D006-080 to D006-089", but the last decision is D006-088.

### Minor 7: no recorded evidence for most of what this diff changes (confirmed by reading)

The reported runs cover three suites. The diff also changes the following, and nothing records a run of them:
- `capability_lists.rs`
- `drain_tests`
- the unit tests of `capability_activation`
- `bounded_worker.rs`, which needs the ceiling of 8 to pass at all now
- the three `release_upgrade*` suites, whose migration-version constant changed
- `apps/execution-engine/tests/registry.rs`
- the Python script tests
- the six SDK suites
- the Studio tests

`tasks.md` has no entries.

Commands:
```sh
cargo test -p sagat-infra-postgres --test capability_lists
cargo test -p sagat-execution-engine --lib drain_tests
cargo test -p sagat-control-plane --lib capability_activation
cargo test -p sagat-tests --test integration_workflow_primitives -- a_scoped_lease_is_withdrawn_in_its_path_for_either_cause release_upgrade
python3 -m unittest discover -s scripts/tests && python3 scripts/check-architecture.py
```
Then the six SDK checks listed in `sdks/AGENTS.md`, and `npm test` in `frontend/apps/studio`.

### Minor 8: in the Rust SDK, a test-only constant replaces a production guard (confirmed in code)

- **Where:** `sdks/rust/src/worker.rs:121-124` sets `NEWEST_MINOR` to 8 under `cfg(test)` and to 6 otherwise.
- **The problem:** no unit test exercises the production refusal at `:569`. Changing the production value (6 to 99) stays green.
- **Fix:** take the ceiling from the options or a constructor seam, and keep one test that 1.7 is refused.

## Read versus suspected

- **Confirmed by reading:**
  - every row of the table;
  - Major 1, and Minors 2 to 8;
  - the tracker-token drop order (`Drop::drop` hands the leases on before `_held` is dropped);
  - the `reported_lease_token_hash` write semantics (`execution.rs:7080`): with `reported = None` the column stays NULL from the claim;
  - the idempotent replay of a retried hand-back, which happens before `verify` (`worker.rs:2428`).
- **Suspected:**
  - Minor 1, which rests on how PostgreSQL row locks behave under overlapping share holders;
  - whether `a_graceful_shutdown_waits_for_a_unary_claim_in_progress_and_its_withdrawal` fails every time on the old code. By reading, it depends on scheduling.

**Counts: 0 Blockers, 1 Major, 8 Minors.**

Files: `crates/application/src/worker/withdrawal.rs`, `crates/application/src/worker.rs`, `crates/application/src/worker/scoped_reports.rs`, `crates/api-grpc/src/undelivered.rs`, `crates/infra-postgres/migrations/20270208000200_lease_withdrawal_floor.up.sql`, `crates/infra-postgres/migrations/20270208000200_lease_withdrawal_floor.down.sql`, `tests/contract/durable-execution/grpc/lease_withdrawal.rs`, `tests/integration/platform-foundation/migrations.rs`, `docs/adr/ADR-0098-a-lease-the-server-never-handed-to-the-transport-is-withdrawn-behind-a-fleet-requirement.md`, `docs/runbooks/lease-withdrawal.md`, `specs/006-sdk-multilanguage/research.md`, `sdks/rust/src/worker.rs`

## Re-read of the fix of Major 1 (commit 8e9a582a)

**Verdict: commit 8e9a582a closes Major 1, and the fix adds no new Blocker or Major.** I read the fix only and ran nothing.

What the fix does, confirmed by reading:
- `hand_back_verdict` (`crates/application/src/worker/withdrawal.rs:200-216`) returns `Lapse` only in one case: the gate or the requirement refused, the step is still `LEASED` to this lease, and the holder's minor is at least 8. The cap, a step that moved on and any minor below 1.8 still get the ordinary failure.
- Both callers return `Err(AppError::StaleAttempt)` before anything is written: `worker.rs:2513-2517` and `scoped_reports.rs:404-407`. Dropping the transaction rolls back the requirement read and the secret scan, and stores no idempotency record.
- The step stays `LEASED` with its token live, so it lapses as on `develop`. Each SDK already ends a hand-back quietly on `STALE_ATTEMPT` (SPEC §6.2).
- The contract test `a_hand_back_on_a_fleet_that_does_not_withdraw_leaves_the_lease_to_lapse` covers the three fleet cases: no gate, a requirement that is not kept, and the switch off. It asserts the step is `LEASED` and the run `RUNNING`, with no `step.failed`. On the old code the `expect_err` would fail, so the test can fail.
- The unit test pins each edge of the verdict.
- ADR-0098 decision 5, SPEC §6.2, `stream-completion.md` rule 5 and the runbook's counter rows now say the same thing.

Two new Minors:

1. **The scoped half of the change has no behavioural test.** At `crates/application/src/worker/scoped_reports.rs:404-407`, replacing the `Lapse` body with the `Fail` body keeps every test green. The contract test covers the root lease only, and the unit test covers only the verdict. The smallest fix is a scoped variant in `tests/integration/workflow-primitives/bounded_worker.rs`: with no requirement recorded, a 1.8 hand-back gets `StaleAttempt` and the instance stays `LEASED`. Confirmed by reading.

2. **A hand-back of an expired but not yet reaped lease still spends the attempt.**
   - **Where:** `withdrawal.rs:210` requires `leased`, and `leased` includes `lease_unexpired_instance`.
   - **Scenario:** a 1.8 host's hand-back is retried until after the lease's time-to-live. It gets `Fail(FenceMissed)`, so a step with one attempt fails for good. On `develop` the reaper would send that lease to a review.
   - **Status:** this predates the fix (round-2 Minor 4, still untested), but D006-089 leaves it out of the lapse.
   - **Fix:** treat a step still `LEASED` to this token, past its expiry, as `Lapse`; or record it as a decision.
   - **Confidence:** plausible, because it needs the retry to outlast the time-to-live.

**Counts: 0 Blockers, 0 Majors, 2 Minors.** The eight Minors from my round-3 report are not re-checked here.
