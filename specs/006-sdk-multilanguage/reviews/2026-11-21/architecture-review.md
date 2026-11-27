# Architecture review, 2026-11-21

The `architecture-reviewer` agent (read only), given the diff of `bugfix/withdraw-undelivered-leases` against `develop` at commit `bc4ad87d` (the merge of develop into `32aab110`) and the documents of the feature. Its final message, verbatim:

---

# Architecture review: `bugfix/withdraw-undelivered-leases` (feature 006, ADR-0098)

Three findings block the merge: **B1**, **B2**, and **B3** (B3 blocks whichever of this branch and feature 022 lands second). The layering itself is sound.

I could only read files; I had no shell. I did not run `git branch -a`, `git worktree list`, `check-architecture.py` or any test. The sibling branches were found by reading `.git/refs` and `.git/worktrees/*/HEAD`. Line numbers are in the worktree `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases` unless a path says otherwise.

---

## Blockers

### B1. Any handler can raise `TASK_NOT_STARTED`, which gives an endless retry loop with no budget and no review
- **Where:** `crates/application/src/worker.rs:2430-2462` and `crates/application/src/worker/scoped_reports.rs:376-406` decide on the code string alone. `sdks/SPEC.md:440-448` lets a handler raise `ActivityFailure(code, …)` with any code matching `[A-Z0-9_]{1,64}`. Only `OUTCOME_UNKNOWN` is reserved there, and the same is true on the SDK branch (`/Users/marcelo/Documents/Projects/sagat-wt-withdraw-sdks/sdks/SPEC.md:473-480`). `research.md:247` (D006-006) says "No cap on withdrawals of one step".
- **What goes wrong:**
  1. Handler code can raise `ActivityFailure("TASK_NOT_STARTED", …)` after doing its effect and before its first heartbeat. D006-001 itself says that window is "at least 500 ms by default".
  2. The server then withdraws the lease. The attempt is given back, the step is `READY` at once with no backoff, and no retry budget is spent. It is never `step.needs_review`.
  3. That repeats forever, and the effect repeats each time without review.
- **What it breaks:** Constitution III ("an unknown original effect MUST be resolved … before it is repeated"), ADR-0076 decision 3 and FR-037-006.
- **Why D006-006 does not hold:** it argues the host's statement "does not repeat without new polling". A handler that raises the code is new polling every time. Older SDK minors and raw protocol clients can send the code too, so reserving it in the SDKs alone is not enough.
- **Smallest fix:**
  - (a) A ceiling in the domain. Keep a count of consecutive withdrawals since the last `step.started` in `StepState`. It is derived from the journal, so the event's shape does not change. Once the count reaches N (say 3), `withdraw_lease` refuses. The `NOT_SENT` path then lets the lease lapse as before, and a `NOT_STARTED` report becomes an ordinary failure. Count the refusals under a new closed outcome, `capped`.
  - (b) Honour `NOT_STARTED` only from worker sessions whose protocol minor reserves the code. The server knows the session's minor.
  - (c) In all six SDKs, rewrite a handler-raised `TASK_NOT_STARTED` to `WORKER_ERROR`, as is done for invalid codes.
- **Fitness:** a contract test where a host sends N+1 hand-backs for one step and the last one is `step.failed`. Also add `Action::Withdraw` to the property model in `crates/domain/tests/execution_properties.rs:195`.

### B2. Removing the requirement, the documented way back, lets builds that cannot read `step.lease_withdrawn` serve again
- **Where:**
  - `docs/runbooks/lease-withdrawal.md:54-61`: step 1 removes the requirement, step 2 rolls a binary back.
  - ADR-0098 "Rollback" (line 63): "Removing the requirement stops new withdrawals at once".
  - `apps/control-plane/src/lease_withdrawal.rs:9`: "It has a way back: removing the requirement".
  - `docs/runbooks/instance-registry.md:216,218`: the troubleshooting table tells operators to "remove the requirement" when replicas are refused.
- **What goes wrong:**
  - `lease-withdrawal-v1` is used for two things: permission to write the event, and the ability to replay it.
  - Once one withdrawal is committed, journals hold an event that any registry-aware build without this change (any develop build since `20270113000000`) cannot decode. `execution.rs:596` turns that into an error.
  - Removing the requirement re-opens the gate for those builds. They then serve, claim and plan runs they cannot replay. An operator following the runbook gets stuck or erroring runs.
  - The down migration guard (`20270208000000_lease_withdrawal.down.sql:20-23`) only protects the schema, not a binary rollback.
- **Smallest fix:** keep the two meanings apart.
  - The requirement is a replay floor. Make `platform_requirement_remove` refuse `lease-withdrawal-v1` while any `run_events` row has `event_type = 'step.lease_withdrawn'`. Do it with a guard in the removal path, in the same shape as the down guard.
  - Add an engine-local off switch (for example `SAGAT_LEASE_WITHDRAWAL=off`). The process still lists the capability, because it can still replay, but it stops writing. Make that switch the "stop" lever.
  - Rewrite the runbook's rollback, ADR-0098's Rollback and Consequences, and add an exception at `instance-registry.md:216`.
- **Fitness:** an integration test that records a withdrawal, then shows `platform_requirement_remove('lease-withdrawal-v1', …)` is refused.

### B3. Feature 022 rewrites the same three CHECK constraints; whichever branch lands second silently drops the other's values
- **Where:**
  - This branch: `crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql:10-27` and `.down.sql:28-37`.
  - Feature 022: `/Users/marcelo/Documents/Projects/sagat-wt-022/crates/infra-postgres/migrations/20270130000200_flow_control_capabilities.up.sql:10-22` and its `.down.sql:19-28`.
- **What goes wrong:**
  - Both branches drop and re-create `serving_instances_capabilities_known`, `platform_requirements_capability_known` and `platform_requirement_history_capability_known` with hard-coded lists. Each list is the 043 list plus only that branch's own values.
  - The SQL files have different names, so git reports no conflict. Only `Capability::ALL: [Self; N]` conflicts textually.
  - If 022 lands first, `20270208` runs after it and removes `flow-control-concurrency-v1` and `flow-control-rate-v1`. If this branch lands first, a database already at `20270208` gets 022's `20270130*` applied afterwards (I believe sqlx applies a missing lower version), and that removes `lease-withdrawal-v1`.
  - Either way, every process listing the dropped value fails the CHECK in `instance_register`, so its gate closes and the fleet stops serving. Both down migrations also restore the bare 043 list, so each rollback erases the other feature's values.
  - The parity test that would catch this (`tests/integration/schedules-bulk-operations/instances.rs:1262`) only runs when that suite is selected. Nothing re-checks `develop` after a merge.
- **Fix:**
  - Whichever lands second rebases its up migration to the union of both lists and its down migration to the other feature's list.
  - Agree the order with the 022 session now.
  - Durable fix: a static check in `scripts/check-architecture.py`. Each `*_known` redefinition across the migrations must be a superset of the previous one, the last one must equal `Capability::ALL`, and each down must restore exactly the previous up. It costs one regex over the migration files and runs on every pull request.

---

## Major

### M1. ADR-0098 overturns an accepted decision but says it only "refines" it
- **Where:** `docs/adr/ADR-0029-worker-task-streams-spend-finite-receive-credit.md:69-74` (decision 5: "Never release a committed lease merely because a write, stream or client was cancelled"). ADR-0098 line 5 says "refines ADR-0029", and its status is Proposed with its reviews "still to run".
- **The problem:**
  - ADR-0098 decision 1 releases committed leases when a client cancels. That supersedes ADR-0029 decision 5.
  - The distinction is defensible: the lease was never handed to the transport, which differs from "the stream closed". But it has to be stated as a supersession, and ADR-0029's status line has to say so. AGENTS.md rule 2 asks for dependents to be updated.
  - Governance: OD-2026-11-20-12 escalated delivery acknowledgement to the owner "because it changes the guarantees of the worker protocol, which the delegation does not cover". ADR-0029 decision 5, and a new code with server meaning sent by six SDKs (D006-005), are worker-protocol guarantees of the same kind.
- **Fix:**
  - State "supersedes ADR-0029 decision 5" in ADR-0098, with the argument for why the old rationale does not apply.
  - Amend ADR-0029's header.
  - Either get the owner's word, or record in `docs/owner-decisions.md` why this change is inside OD-2026-11-20-6.
  - Do not merge code that implements a Proposed ADR contradicting an accepted one until ADR-0098 is accepted.

### M2. A graceful shutdown does not wait for the withdrawals that matter most (D006-016 is only partly true)
- **Where:**
  - `crates/api-grpc/src/service.rs:595`: the unary claim runs under `tokio::spawn`.
  - `service.rs:707`: the stream producer runs under `tokio::spawn`.
  - `crates/api-grpc/src/undelivered.rs:186` (`drain`) and `apps/execution-engine/src/lib.rs:809`.
- **The problem:**
  - `drain` waits only for tasks already on the `TaskTracker`.
  - A producer or unary claim still in its claim when `serve_grpc` returns is not tracked. Its leases reach the sink only later. By then `drain` may have returned on an empty tracker and the pool may be closing, so those leases are counted `error` and lapse.
  - That is exactly the stop-during-claim case D006-002 and D006-016 are about.
- **Fix:** spawn the producer and the unary claim with `withdrawals.tracker.spawn(...)`, the same tracker. `drain` then covers them, and the sink's spawn happens before they end.
- **Fitness:** a contract test that holds a claim at the probe, starts a shutdown, releases the claim and asserts `withdrawn` after `drain` returns.

### M3. Nothing limits how many withdrawals run at once
- **Where:** `undelivered.rs:64` spawns one task per dropped guard. Each takes a pool connection and the run lock with `Lock::Wait` (`withdrawal.rs:203,263`).
- **The problem:** when many streams drop at once (a load-balancer restart, a client fleet redeploy), hundreds of transactions open together on the engine's shared pool. They starve claims, heartbeats and the scheduler, and those all share that pool. The design has a ceiling for everything else (ADR-0029 decision 3), but none here.
- **Fix:** a `Semaphore` in `Withdrawals`, sized well below the pool, for example 4. A bounded queue behind it; overflow counted as a new outcome, `shed`, and left to lapse. Document the bound in the runbook.

### M4. Evidence the documents claim is missing
- **Release-0.2 upgrade proof:** D006-004 (`research.md:228`) says "the released-v0.2 upgrade path proves that no `step.lease_withdrawn` is written before the activation and that the current build replays a journal that holds one". The diff only moves the expected migration in three `release_upgrade*.rs` files. Add both assertions to `tests/integration/workflow-primitives/release_upgrade_inflight.rs`, or correct D006-004.
- **Down-migration guards:** ADR-0098's "Enforced by" (line 6) names "the migration pair". No test exercises the four refusals in `20270208000000_lease_withdrawal.down.sql:4-23` (requirement, history, registered row, journal event). Add them to `tests/integration/platform-foundation/migrations.rs`, following the `BEFORE_RUN_INPUT_VALIDATION` test at line 485.
- **Replay equals projection on PostgreSQL:** the domain tests call `assert_replays()`. `tests/contract/durable-execution/grpc/lease_withdrawal.rs` never calls `verify_run` after a withdrawal on the real store. Add `p.api.executions.verify_run(...)` to `runs_once_afterwards`.
- **Scoped leases on the real store:** these paths have no test over real PostgreSQL:
  - `withdraw_path` (`withdrawal.rs:252-309`, with `begin_current_path` and `lease_matches`);
  - the scoped `TASK_NOT_STARTED` path in `scoped_reports.rs:376`;
  - the path outbox hint (`execution/path.rs:613`).

  Add one `forEach` case for each cause.

### M5. A second, divergent operator command for the same ADR-0082 activation
- **Where:**
  - This branch: `apps/control-plane/src/lease_withdrawal.rs` (`--expect-live N`; refuses a count that differs; an empty fleet only with `--expect-live 0`).
  - Feature 022: `/Users/marcelo/Documents/Projects/sagat-wt-022/apps/control-plane/src/cli.rs:71` (`flow-control-activate --capability …`, no `--expect-live`; a SQL function `flow_control_kinds_without_capable_process` refuses a kind with no capable live process).
- **The problem:** two parallel branches implement the same pattern with different checks, flags and refusal wording. Operators learn two models, and the third capability gets a third copy. `capability_census` and `census_step` here are already general.
- **Fix:** one `control-plane capability-activate --capability <token> --operator NAME [--expect-live N] [--dry-run]` with a step per capability. Agree it with the 022 session before either merges, or record why they must differ (an ADR-0082 amendment).

### M6. `sdks/SPEC.md` gains a host rule that no SDK in this tree follows, and it overlaps the SDK branch
- **Where:** `sdks/SPEC.md:351-356` here. `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-sdks/sdks/SPEC.md:351-386` on `bugfix/withdraw-leases-sdks` edits the same paragraph more fully.
- **The problem:**
  - Merged alone, this branch publishes a normative host obligation ("All six SDKs", D006-005) with no implementation and no six-language conformance scenario (AGENTS.md rule 8).
  - The two branches will also conflict textually.
- **Fix:** move the §6.2 host text to the SDK branch only, or land both together. In either case add a conformance scenario (a hand-back in all six languages) to `tests/contract/sdk-conformance`.

---

## Minor

1. **Metric labels typed as literals.** `worker.rs:2459-2460` and `scoped_reports.rs:403-404` write `"NOT_STARTED"` and `"withdrawn"`; `undelivered.rs:78` writes `"error"`. Make `withdrawal::count` `pub(crate)` and use `WithdrawalCause::as_str()` and `WithdrawalOutcome` everywhere. Also:
   - A `TASK_NOT_STARTED` report without the requirement is not counted, although D006-018 says `lapsed` counts what activation would recover.
   - The runbook's "a rate of withdrawn that keeps growing for one step" (`lease-withdrawal.md`, Watching) cannot be seen from a counter that has no step label. Give the query on `run_events` instead.
   - There is no alert rule in `infra/observability`.
2. **Test seam on the production type.** `with_delivery_probe` (`service.rs:106`) and the exported `DeliveryProbe` and `Abandonment` (`lib.rs`), plus the public `ServingGate::keep`, let any composition root hold claims or forge "required". Put the probe behind `#[cfg(any(test, feature = "test-support"))]`. `check-architecture.py` already keeps `test-support` to dev-dependencies.
3. **`keeping_requirements` is an option nobody turns off.** `apps/control-plane/src/registry.rs:75-83` enables it for both kinds. The control plane then reads requirements it never asks about. Either always keep them (drop the flag) or enable it only from the engine. Record "a feature may wait on a recorded requirement through `ServingGate::requires`, learned within one heartbeat in both directions" as an ADR-0082 amendment, since it is now a general mechanism.
4. **The down guard is heavy and too strict.**
   - It scans all of `run_events` (no index on `event_type`) while holding `ACCESS EXCLUSIVE` on the registry tables. Every heartbeat blocks for the length of the scan.
   - "Any registered row" includes crashed, non-live rows that stay until evicted.
   - 022 uses `SHARE ROW EXCLUSIVE` for the same guard.

   Restrict the row check to live rows, use one lock mode, and say in the runbook that the scan is slow.
5. **Documents disagree with the code.**
   - ADR-0098 Evidence (line 69) and `stream-completion.md` ("Decisions D006-001 to D006-010") omit D006-011 to D006-018 and G006-004.
   - ADR-0098 decision 1 says "one fenced transaction per run"; the code is per run and instance path (D006-012).
   - D006-004 and ADR decision 4 say "with no step of its own"; the code passes `census_step`, which `lease_withdrawal.rs:6` admits.
6. **Studio reads the journal by attempt number.** `frontend/apps/studio/src/features/runs/attempts.ts:129-131` merges the withdrawn lease and the next lease of the same number, and keeps the withdrawn lease's `leasedAt`. `src/model/events.ts:11` has no label for `step.lease_withdrawn`. The ADR names "every reader of the journal" as a cost; this reader was not updated (AGENTS.md rule 10).
7. **The code is a bare string in the application.** `TASK_NOT_STARTED` should be a `FailureCode` constructor in the domain next to the other reserved codes, so B1's reservation has one home.
8. **Telemetry and audit.** A `NOT_SENT` withdrawal runs on a detached task with no span and no link to the claim's `request_trace`. Carry the traceparent in `UndeliveredLease`. Also confirm that `Effects::by(Actor::System("lease-withdrawal"), None)` writes an audit record and not only the journal row.

---

## Notes

- **Layering is right.**
  - The domain transitions are pure: `command.rs` `withdraw_lease` and `withdraw_scoped`, `state.rs` `withdraw_lease`, and the closed `WithdrawalCause` with no serde.
  - The application holds the orchestration and the gate check (`worker/withdrawal.rs`, no tokio).
  - The adapter holds the "handed to the transport" guard (`undelivered.rs`, `service.rs`).
  - The apps do the wiring. Nothing is an empty mapper.
  - The ready hint for `step.lease_withdrawn` goes through the outbox, so the broker-down path still works by polling.
- **Branch hygiene.** `bugfix/withdraw-leases-sdks` (the SDK side) and `docs/delivery-acknowledgement` (O006-001) complement this branch and do not duplicate it. Only 022 overlaps (B3, M5).
- **When 022 merges,** check that 022's gated steps (ADR-0089) neither take nor give back a permit on `step.lease_withdrawn` (`LEASED` to `READY`). The exhaustive matches will force a look, but not the semantics.
- **ADR-0082 decision 4's window** (a not-live old process learning the requirement at its next heartbeat) here means read errors, not corrupted writes. Say so in ADR-0098.
- **Removing `timeout_at(producer_end, claim)` is correct** for D006-002. Concurrent background claims stay bounded by the per-claim discovery ticket.

---

## Fitness functions: what exists and the smallest addition

| Rule relied on | Check today | Smallest addition |
|---|---|---|
| Domain free of adapters | `scripts/check-architecture.py` (not run by me) | none |
| Event written only with the requirement | contract test `without_the_recorded_requirement_an_undelivered_lease_lapses_as_before` | a source-text test like `managed_kind_sites.rs`: `withdraw_lease` / `withdraw_path_instance` called only from `withdrawal.rs` and the two report sites, each behind `withdraws_leases()` |
| Producer never aborted mid-claim | body test asserts `!producer.is_finished()` | the same source test: no `producer.abort()` in `service.rs` |
| Capability list: Rust equals SQL | integration test `instances.rs:1262`, run only when that suite is selected | the static migration-lineage check of B3, on every pull request |
| Withdrawal cannot loop | **none** | B1's ceiling and its tests |
| Replay equals projection on PostgreSQL | **none** for this event | `verify_run` in the contract tests (M4) |
| Down guards | **none** | migration test (M4) |
| Shutdown waits for withdrawals | **none** | M2 test |
| Closed metric vocabulary | **none** | one counter helper and a unit test over its labels (Minor 1) |
| Six SDKs, one semantics, for the hand-back | **none** in this tree | a conformance scenario (M6) |
| Upgrade boundary | `scripts/check-upgrade-boundary.py` | the release-0.2 assertions of D006-004 (M4) |
