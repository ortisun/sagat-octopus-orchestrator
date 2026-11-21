# Correctness re-review of task T022-086 (reset of a rate-limited start), 2026-11-21

Reviewer: `correctness-reviewer` (read-only), given the diff of commits `8dcd7f37` and `cf4f858e` (review files left out),
the specification, its first review and `reset-slot-responses.md`. Saved verbatim.

---

**Re-review of T022-086 (reset of a rate-limited start): first-review Majors, then a fresh look at the diff**

There are no Blockers and no Majors left. All three earlier Majors are resolved in substance. Two of them leave smaller gaps behind, listed below. I could not run anything; where a claim needs a run, I name the command.

## Part 1: the first review's Majors

**Major 1 (Studio types behind the OpenAPI): resolved in code.**
- `frontend/apps/studio/src/api/schema.d.ts` now has the new reason in the `details` description and in `failureReason`, plus the `estimatedGrantAt` text that had also drifted.
- `ResetRun.tsx` and its test add the row.
- The `check:api` and Vitest results appear only in `reset-slot-responses.md`, not in the task's evidence. That stays under old minor 5.

**Major 2 (reset after a rate-limited step untested): resolved as a Major; a minor remains.** I read `a_reset_after_a_rate_limited_step_restores_its_slot_as_history` (`run_creators.rs:1953-2016`) and asked, for each assertion, what defect would make it fail:
- **`step_grants() == ["b"]` (lines 2005 and 2013).** This is the assertion that does the work. It fails if:
  - a restored step grant produces a `rate_limit_grants` row for the reset run;
  - the planner decides `a` again after the reset;
  - `b` is queued instead of granted.
- **`assert_consistent` before and after `b`.** It fails on a grant row with no fresh `rate_limit.granted` (support check at `tests/support/flow_control.rs:603-613`), and on a projection that differs from the replay.
- **"No fresh `rate_limit.returned`."** It fails if the reset gives back `a`'s restored slot. No plausible mutation produces that: `give_back_ended_gates` (`plan.rs:379-423`) never returns a slot whose attempt was leased.
- **"A restored `rate_limit.granted` exists."** This only shows the prefix was copied. It fails on no defect in this path.

What the test cannot catch:
- **The limit has slack.** With `limit: 4` (line 1962), the source takes 2 slots and the fresh `b` takes 1. A defect that also counted the restored grant would make 3 of 4, so `b` would still be granted. With `limit: 3` the same defect leaves `b` queued, and line 2005 fails.
- **`retire_restored` cannot be checked through rate slots.** Removing `step.gates.retire_restored()` (`state.rs:4633`) leaves this test green. That is correct, not a defect: a rate slot that stays `Held` on a completed step has no effect, because `is_open` only treats a `Held` permit as open. But D022-069 item 2's claim that the reducer "retires the restored slots" cannot be observed for rate slots.
- **The second test I asked for is still missing.** That is the rate version of `a_reset_whose_prefix_gave_a_step_permit_back_restores_it_as_history`: a prefix with a step-scope `rate_limit.returned`, from a pause while granted and not yet leased, then resume and complete. The skip `effects.restored.is_none()` (`execution.rs:5516`) is per transaction, so the permit test already covers it. What stays untested is the reducer and codec path of a restored `rate_limit.returned`.
- **The `OpenWork("b")` assertion I asked for is absent.** That point-blocking logic does not depend on the gate type (`reset.rs:233-237`), so the risk is low.

**Major 3 (other run creators moved to T022-113): the reasoning holds; two flaws, logged below as minor 1 and nit 1.**
- **Why it holds.** Every admission-cause creator ends in `Engine::start` → `decide_start_gates` → `decide_filed`. That one call decides both gates of the start, so a creator cannot take the permit and skip the slot. What can differ per creator is the input to the key expression and how the error is mapped. The batch mapping is an exhaustive `match` (`batch_executor.rs:160-167`), so a new variant cannot be missed. Splitting the twins into their own task is a fair delegate decision.
- **Flaw 1: the cited check is the wrong one (nit 1).** Item 7 (`research.md:1258-1262`) cites `run_creator_errors`. That is design 043's checked-input and `INSERT INTO runs` rule (`check-architecture.py:724`). The one-start-point rule it means is `run_start_errors` (`:383-409`, D022-049), which commit cf4f858e just changed.
- **Flaw 2: the split takes the twins out of the production gate (minor 1).** Before the amendment the twins were part of T022-086, which `docs/runbooks/flow-control.md:195` lists in the rate production gate. T022-113 is not in that gate, and item 7 does not say whether it should be.

## Part 2: fresh review of the current diff

No new Blocker or Major. Checked and sound:
- **Transactions.** The refusal returns before any `grant` or `queue` (`readiness_gate.rs:364-367` and `460-462`), and everything rolls back with it. `reset_refused`'s `None` arm cannot be reached, because it is only called when `refusing` is non-empty.
- **Window count.** Invariant 10 leaves out restored grants and takes the last run-scope grant, which is the fresh one; this also holds for a reset of a reset.
- **Reset points.** A point at `run.started` of a source whose start wait timed out is eligible, because the prefix ends before the `REFUSE` event. That matches the contract.
- **Interim refusal.** No leftover `BINARY_CAPABILITY_MISSING` check for rate resets remains in `crates/`.
- **Migration.** The down guard and its test are correct; the previous version `20270207000100` is the real previous migration.

**minor 1. The production gate dropped the creator twins without a record.**
- Where: `docs/runbooks/flow-control.md:195`; `tasks.md:480` and `:569`.
- Scenario: T022-086 is ticked. The gate then allows rate limits in production with no rate case for business-id starts, rerun, Signal- and Update-with-Start, batch rerun or ad hoc runs. Before the split, the gate required all of them.
- Fix: add T022-113 to the section 6 gate, or state in D022-069 item 7 that it does not gate production and why.
- Confidence: confirmed.

**minor 2. What is left of Major 2.**
- Where: `run_creators.rs:1962` and the missing `returned` case.
- Fix:
  - use `limit: 3`;
  - add a case: `a` granted, run paused before the lease (`rate_limit.returned`), resumed, completed; then reset after `a` and assert `assert_consistent` plus no grant row for `a`;
  - optionally, assert that a parallel-branch point is `OpenWork`.
- Check: after the change, count the restored grant in the window by hand and see `step_grants` go red. Run `cargo test -p sagat-tests --test integration_flow_control_step_policies -- run_creators::a_reset_after_a_rate_limited_step`.
- Confidence: confirmed gap.

**minor 3. The task's evidence leaves out the new test.**
- Where: `tasks.md:489-499` and D022-069 item 5 (`research.md:1250-1254`).
- Problem: neither lists `a_reset_after_a_rate_limited_step_restores_its_slot_as_history`, and no pass, red-first run or mutation is recorded for it. The hand-mutation list has no mutation of the step path.
- Fix: run it, record the result and a mutation it catches (for example, a grant row written for a restored event).
- Confidence: confirmed.

**minor 4. The per-second sub-case still depends on timing (old minor 6, still open).**
- Where: `run_creators.rs:1819`.
- Scenario: on a loaded runner, `behind` is admitted more than 1 s after `source`'s grant, so it is granted at once. The "buffered" assertion then fails: a false failure, not a false pass.
- Fix: as in the first review.
- Confidence: plausible.

**nit 1.** D022-069 item 7 cites `run_creator_errors`; it should be `run_start_errors`.

**nit 2. Two dates are still wrong although the response says they were fixed.** The responses mark the dates nit "Answered", but two places still say 2026-11-20 for work built on 2026-11-21:
- `docs/runbooks/run-reset.md:376` ("T022-086, 2026-11-20");
- `specs/022-flow-control-step-policies/contracts/journal-events.md:47` ("*2026-11-20 (T022-086, D022-069):*").

The doc-comment nit is fixed: `reset_refused` now sits above `start_refused`'s doc.

**nit 3. No test shows the remediation working.** The error text says "retry once a slot has stopped counting", but no case shows a reset accepted after the source's slot ages out. The per-second case proves the reservation rule only. The window count is shared with admission, so the risk is low. Fix: in the per-second case, after `behind`'s grant ages out, assert the reset succeeds.

**nit 4. The refusal metric counts attempts, not resets (plausible).** `refused_reset` is counted before the rollback (`readiness_gate.rs` `reset_refused`). If the separate `fail_reset` transaction of a pending reset fails and the entry is retried, the refusal is counted again. The permit has the same pattern. Worth one sentence in the runbook's metric row.

Blockers: 0, Majors: 0
