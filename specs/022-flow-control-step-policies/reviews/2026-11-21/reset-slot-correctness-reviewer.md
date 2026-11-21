# Correctness review of task T022-086 (reset of a rate-limited start), 2026-11-21

Reviewer: `correctness-reviewer` (read-only), given the diff (`git diff HEAD` over `39c5abc9` plus the uncommitted task) and
the specification. Saved verbatim.

---

I found no Blocker. The core logic of T022-086 holds up against the code I traced: the fresh slot is decided inside the reset's admission transaction, nothing is written when a reset is refused, the gates are checked rate first, a reset follows the reservation rule (counted grants plus waiters must stay below the limit), and a restored grant is counted only once. Three Major findings stand against ticking the task: a CI gate that will fail, a newly enabled path with no test, and part of the task's scope not built. I could not run anything; the commands to check each finding are given below.

## Major

**Major 1. The Studio types were not regenerated, so `npm run check:api` will fail.**
- Where: `/Users/marcelo/Documents/Projects/sagat-wt-022/specs/001-platform-foundation/contracts/openapi.yaml` (the error-code description near line 5034 and the `failureReason` description near line 5328) and `/Users/marcelo/Documents/Projects/sagat-wt-022/frontend/apps/studio/src/api/schema.d.ts`.
- Scenario: `check-api.mjs` regenerates the schema with `openapi-typescript` and compares it byte for byte with the committed file. The generated file carries the descriptions. Line 3762 of `schema.d.ts` still has the old list (`…CONCURRENCY_PERMIT_UNAVAILABLE, RUN_PURGED…`) and the name `RATE_LIMIT_SLOT_UNAVAILABLE` appears nowhere in it.
- The diff changes Studio files, so CI runs the Studio checks on this pull request.
- Why the tests miss it: none of the evidence in `tasks.md` runs the Studio checks.
- Fix: `(cd frontend/apps/studio && npm run api && npm run check:api)`, then read the diff of `schema.d.ts`.
- Confidence: confirmed in code.

**Major 2. Resetting a run whose steps declare `rateLimit` is now possible, but no test covers it.**
- Where: `/Users/marcelo/Documents/Projects/sagat-wt-022/crates/domain/src/execution/reset.rs:132-136`, `/Users/marcelo/Documents/Projects/sagat-wt-022/crates/domain/src/execution/flow_control.rs:490-499` (`retire_restored`), and the `persist_gate_returns` skip in `/Users/marcelo/Documents/Projects/sagat-wt-022/crates/infra-postgres/src/execution.rs:5516`.
- Before this diff, every `rate_limit.*` event was `EXCLUDE`, so a run with a step-level rate limit had no reset points at all. Now such a run can be reset after a step's `rate_limit.granted` or `rate_limit.returned`. D022-069 items 1 and 2 promise three things for that case:
  - a point with a step waiting for a slot, or granted but not completed, is refused as `OpenWork`;
  - a restored step grant creates no row and counts in no window;
  - the projection of the reset run equals a replay of its journal.
- None of the four new tests uses a step-level `rateLimit`. The permit already has the two tests this case needs (`a_reset_after_a_gated_step_restores_its_permit_as_history` and `a_reset_whose_prefix_gave_a_step_permit_back_restores_it_as_history`), but they have no rate version. A rate slot is unlike a permit: it stays `Held` after the step completes and is retired to `Ended` only at `run.reset`. That is exactly the path left unexercised.
- Fix: add rate versions of both permit tests. Use `a` and `b`, each with a step-level `rateLimit` of limit 2 per hour, reset after `a`, then:
  - assert `rate_limit.granted` is restored, and no fresh `rate_limit.returned` appears;
  - assert `rate_limit_grants` for the reset run at `limit_scope = 'STEP'` has exactly one row, the one `b` takes after the reset;
  - call `assert_consistent` before and after `b` completes;
  - assert that a point taken while `b` is granted but not leased is blocked `OpenWork("b")`.
- Confidence: the missing test is confirmed; a defect in this path is plausible.

**Major 3. The task's text covers more creators than the reset.**
- Where: `/Users/marcelo/Documents/Projects/sagat-wt-022/specs/022-flow-control-step-policies/tasks.md:479`.
- T022-086 says "Run-creator cases of T022-064 extended to the rate gate … with 018 on the tree, its creators". The run creators T022-064 covers for the permit include:
  - the start API with a business id;
  - rerun as new;
  - Signal-with-Start and Update-with-Start;
  - backfill and trigger now;
  - batch rerun;
  - ad hoc workflows and their private rerun.
- `rate_creators.rs` (T022-072) covers only an event trigger, cron, a schedule action, a child and a delayed start. This diff adds only the reset. None of the creators in the first list has a rate case that "fails when a creator bypasses the port".
- Fix: either add the missing rate cases (the `Admitted` helper in `rate_creators.rs` already does the check), or record in `research.md` why the shared `Engine::start` port makes them redundant and amend the task text.
- Confidence: confirmed in code.

## minor

**minor 1. No integration case shows rate first when both gates refuse.**
- Where: `/Users/marcelo/Documents/Projects/sagat-wt-022/tests/integration/flow-control-step-policies/run_creators.rs:1844-1870`.
- In the "both" case (rate limit 1 per hour, concurrency limit 1) the source has ended, so its permit is free and only the slot refuses. In the "roomy" case only the permit refuses. The order of `refusing` produced by `gates_of` and `decide_filed` on the reset path is never tested with both gates refusing. Only the unit test of `reset_refused` sees a two-element `refusing`, so reordering `gates_of` would not be caught on a reset.
- Fix: rate limit 2 per hour, concurrency limit 1, source ended (one slot used), a holder started (second slot, holds the permit). The reset must then be `SlotUnavailable`.
- Confidence: confirmed (a gap in the test).

**minor 2. A reset counts waiters the grant routine would skip, so it can be refused indefinitely while a slot is free.**
- Where: `/Users/marcelo/Documents/Projects/sagat-wt-022/crates/infra-postgres/src/execution/readiness_gate.rs:359` with `key_count` at lines 770-781, which uses `w.waiters`. Compare `/Users/marcelo/Documents/Projects/sagat-wt-022/crates/infra-postgres/src/execution/gate_queue.rs:175-182`.
- The grant routine counts only *eligible* waiters ahead: not past their deadline, run live and not paused, `declared_limit > taken`. A new arrival counts every waiter. For an admission that is harmless, because it queues and the pass grants it. A reset cannot queue.
- Scenario: limit 1 per second, no counted grant, and one run-scope waiter whose run is paused. Every reset on that key gets `RATE_LIMIT_SLOT_UNAVAILABLE` for as long as the run stays paused. A plain start on the same key queues and is granted on the next tick, ahead of the paused run.
- The permit reset (T022-064) has the same pattern. D022-069 item 3 says "waiters ahead", which the grant routine reads as eligible waiters.
- Fix: for `StartCause::Reset`, count waiters with the grant routine's eligibility predicate. Otherwise, record the stricter rule as a decision and say it in `run-reset.md`.
- Check with: the scenario above, asserting the reset succeeds; it is expected to fail today.
- Confidence: plausible.

**minor 3. The marker that keeps a restored grant out of the counts is trusted without a check.**
- Where: `/Users/marcelo/Documents/Projects/sagat-wt-022/crates/infra-postgres/src/execution.rs:893` and the three checks in `/Users/marcelo/Documents/Projects/sagat-wt-022/tests/support/flow_control.rs:612`, `:619` and `:641`.
- Invariant 10 and those checks leave out any row whose detail says `restored: true`. After this diff the codec accepts that marker on every gate event (`event_codec.rs:1083-1086`). Nothing checks that restored rows are the prefix before the run's `run.reset`.
- Scenario: a regression that marks the fresh grant `restored` (for example an offset error in `restored_detail`). Invariant 10 and the support checks would no longer see the fresh grant, and the window could be over-granted unnoticed.
- Fix: in `verify_run`, assert that every row marked `restored` has a `seq` below the run's first `run.reset`, and that no row after it is marked.
- Confidence: plausible (a weakness in the check, not a defect seen).

**minor 4. The runbook lifts the production gate before the task is ticked.**
- Where: `/Users/marcelo/Documents/Projects/sagat-wt-022/docs/runbooks/flow-control.md:195-198`; `research.md` marks G022-016 "Closed".
- `tasks.md:481` says the runbook "forbids activating the rate capability in production before this task is ticked". The diff removes T022-086 from the gate and calls it "done", while `tasks.md` says "not ticked until its independent review".
- Fix: keep T022-086 in the gate and G022-016 "Open" until the tick, or change both in the commit that ticks the task.
- Confidence: confirmed.

**minor 5. The evidence is incomplete for a tick.**
- Where: the `tasks.md` T022-086 entry.
- These pass results are not reported:
  - `cargo test -p sagat-tests --test integration_flow_control_step_policies` for the four new `run_creators` tests and the `migrations` test (only "seen red first" is reported);
  - the three `release_upgrade*` suites whose expected final migration changed;
  - `contract_schedules_bulk_operations` (the `errors.rs` `RAISED` list);
  - the Studio `npm test` for `ResetRun.test.tsx`;
  - `check:api`.
- Confidence: confirmed.

**minor 6. The per-second case depends on timing.**
- Where: `/Users/marcelo/Documents/Projects/sagat-wt-022/tests/integration/flow-control-step-policies/run_creators.rs:1816-1820`.
- `behind` is buffered only if it is admitted less than one second after `source`'s grant. On a loaded CI runner it is granted at admission instead, and the "buffered" assertion fails. This is a false failure, not a false pass.
- The later steps are fine: `until_past` waits on the database clock, and `scheduler.tick()` is manual.
- Fix: use `per: minute` with `until_past(granted + 60 s)`, which is slower but has no margin to lose. Otherwise, document the one-second margin as the existing `rate_creators` child case does.
- Confidence: plausible.

**minor 7. The CLI has no words for the new reason.**
- Where: `/Users/marcelo/Documents/Projects/sagat-wt-022/apps/cli/src/reset.rs:249-281` and its test in `apps/cli/tests/reset_and_with_start.rs:~995`.
- The Studio got a sentence for `RATE_LIMIT_SLOT_UNAVAILABLE`. The CLI prints the raw code. The contract allows that, but AGENTS.md rule 10 asks for the clients to be updated in the same delivery.
- Fix: add the arm and its test row.
- Confidence: confirmed.

## nit

**nit 1. A doc comment is attached to the wrong function.**
- Where: `/Users/marcelo/Documents/Projects/sagat-wt-022/crates/infra-postgres/src/execution/readiness_gate.rs:1656-1661`.
- The doc of `start_refused` ("A run's start refused by a ceiling … refused_full …") now sits above `reset_refused`, so `reset_refused` carries both texts and `start_refused` (line 1685) has none.
- Fix: move lines 1656-1658 down to just above `start_refused`.

**nit 2. One metric side is inconsistent with its stated rationale.**
- A reset that declares both gates and is refused only by the permit still counts `rate_limit_decisions_total{outcome="refused_reset"}`. D022-069 item 4 justifies its change with "a rate-only reset no longer counts a concurrency refusal it did not have", but the mirror case keeps counting a rate refusal it did not have. This is consistent with "each family the start declares".
- Fix: say it in the runbook's metric row so operators do not read the counter as "the rate gate refused".

**nit 3. Stale or inconsistent text.**
- `docs/adr/ADR-0088-…md:11` still says the run-creator test T022-086 does not exist ("None exists yet").
- `docs/runbooks/run-reset.md:376` dates the work "2026-11-20", but it was built 2026-11-21.
- The task text says "four `rate_limit.*` events"; there are five.
- 018's `rest.openapi.yaml` lists the new reason after `SOURCE_DID_NOT_END`, while 001's lists it before `RUN_PURGED`.

**nit 4. An older replica cannot decode the new restored events (plausible).**
- Before this diff the codec refused a `restored` marker on a `rate_limit.*` event. So a pre-T022-086 replica running beside this build cannot decode a reset run's journal (reset points, `verify_run`, history).
- Rate limits are not released, so this matters only between unreleased builds. Worth one line under "finish the rolling upgrade" in the runbook.

## Checked and found sound
- **Transaction boundaries:**
  - `decide_start_gates` with `StartCause::Reset` runs in the reset's own transaction.
  - On refusal, `start_may_wait` returns its error before any `grant` or `queue`, and the key filing and `reclaim_windows` roll back with the transaction.
  - A pending reset records its failure in a separate `fail_reset` after that rollback.
- **Rate first:** `gates_of` builds the gates rate first, `pending` keeps that order, and `refusing.first()` names the refusal.
- **Window arithmetic:**
  - `key_count` counts run-scope grants whose `leased_at` is within the period, plus waiters, so the source's own grant counts.
  - Invariant 10 now leaves out restored grants and takes the *last* run-scope grant, which is the fresh one.
  - The three checks in `tests/support/flow_control.rs` already left out restored rows.
  - The lease recording in `maintenance.rs` and `rate_place` read the grant and waiter tables, not restored journal rows.
- **Replay and projection:**
  - `run.reset` clears `run_gates` (and refuses a start still waiting); `persist_gate_returns` is skipped for a restored prefix.
  - `a_reset_takes_a_fresh_slot_right_after_run_reset` calls `assert_consistent`, which compares the replay with the projection.
- **Migration:**
  - The CHECK is added `NOT VALID`, then validated.
  - The down migration's list matches `20270115000000`.
  - The down guard runs under `ACCESS EXCLUSIVE`.
  - The test goes through refusal, removal, rollback, the old list refusing the reason (`23514`), forward again and `convalidated`.
- **Error paths:**
  - `SlotUnavailable` maps to 409 `CONFLICT` with `retryable: false` in the API, to `RATE_LIMIT_SLOT_UNAVAILABLE` for a pending reset before the catch-all `ADMISSION_REFUSED`, and to `CONFLICT` for a batch item (a branch resets never reach).
  - `BINARY_CAPABILITY_MISSING` still applies to collections and loops.

## How to verify
- `(cd frontend/apps/studio && npm run api && git diff --stat src/api/schema.d.ts && npm run check:api && npm test -- ResetRun)`
- `cargo test -p sagat-tests --test integration_flow_control_step_policies -- run_creators:: migrations::the_reset_slot_reason`
- `cargo test -p sagat-tests --test contract_schedules_bulk_operations` and the three `release_upgrade*` targets.
- For minor 1, minor 2 and Major 2: add the scenarios above and watch them fail for the stated reason before fixing anything.
