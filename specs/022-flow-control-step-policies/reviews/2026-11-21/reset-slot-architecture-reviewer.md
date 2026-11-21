# Architecture review of task T022-086 (reset of a rate-limited start), 2026-11-21

Reviewer: `architecture-reviewer` (read-only), given the diff (`git diff HEAD` over `39c5abc9` plus the uncommitted task) and
the specification. Saved verbatim.

---

# Architecture review: task T022-086 (reset of a rate-limited start)

**Verdict: not ready to merge.** Two findings block it:

- **Blocker 1:** feature 018's reset-rule registry, which is the normative document, still says the five `rate_limit.*` events are `EXCLUDE`. The code now gives them other rules.
- **Blocker 2:** the Studio's generated client was not regenerated, so `check:api` fails.

The layering is clean and resets keep a single decision path. The migration is safe for a rolling upgrade.

Limits of this review: it is read-only and I ran nothing. I had no shell, so `git branch -a` and `git worktree list` were replaced by reading `.git/packed-refs`, `.git/refs` and `.git/worktrees`, plus the migration folders of the sibling worktrees.

## What holds

- **Layering.** The only domain change is the registry table in `crates/domain/src/execution/reset.rs:130-136`: no input or output, no framework, no `serde_json`. The new `FlowControlRefusal::SlotUnavailable` is in the application layer. `json!` is used only in `crates/api-http/src/error.rs`, which is an adapter. No empty mapper or DTO was added.
- **One decision for both gates, no second path for resets.** A reset still goes `decide_start_gates` → `decide_filed` → `start_may_wait` → `reset_refused` (`crates/infra-postgres/src/execution/readiness_gate.rs:1206-1327`, `:354-367`, `:460-462`). The interim guard was removed rather than worked around. Rate first comes from the order in `gates_of` (`:260-281`), which the `refusing` vector keeps (`:356-362`). The domain still refuses a reset that tries to queue (`crates/domain/src/execution/flow_control.rs:353`).
- **No new ADR needed.** Decision 8 of ADR-0088 and decision 3 of ADR-0056 already say a reset takes a slot or is refused, and never waits. D022-069 is a delegated decision inside them.
- **Metrics vocabulary stays closed.**
  - `RateLimitOutcome::RefusedReset` already existed and is pinned by `crates/telemetry/tests/flow_control_vocabulary.rs`.
  - The `reason` label of `workflow_reset_completions_total` is closed by `FAILURE_REASONS`.
  - `crates/application/src/reset_completion.rs:591` checks that `failure_of` never returns a name outside that list.
- **Error contract has a working check.** `tests/contract/schedules-bulk-operations/errors.rs` ties 018's `errors.md` row, the API's remediation sentence and the 001 OpenAPI together. The new row's sentence matches `error.rs:53-55`.
- **Migration.**
  - The up migration is additive (the new constraint is added `NOT VALID`, and `20270209000100` validates it).
  - The down migration refuses with `object_not_in_prerequisite_state` while an entry carries the reason, and restores exactly the list of `20270115000000`.
  - The new test covers: refused, drained, rolled back, old list in force, forward again, validated.
  - 018's `reset_expiry_migration.rs` still works across it, because `undo_to` migrates forward again after a refusal.
- **Rolling upgrade.**
  - An older completer refuses `BINARY_CAPABILITY_MISSING`, which `failure_of` maps to `None` (`reset_completion.rs:246`), so the entry is retried and never wrongly ended.
  - A new engine against the old schema hits the CHECK, which is a storage error, so it is retried too (same behaviour as `20270115000000`).
- **Release-upgrade oracles.** All three `release_upgrade*.rs` files were moved to `20270209000100`. No other test hard-codes `20270207000100`.
- **Invariant 10.** `verify_run` (`crates/infra-postgres/src/execution.rs:883-893`) now leaves out restored grants. The test support code already did (`tests/support/flow_control.rs:612,619,641`).
- **No duplicate work.** `sagat-wt-022-sdk` (`feature/022-sdk-builders`) has no migration after `20270207000100` and nothing about resets. Neither does `sagat-wt-041`. The gap at `20270208` is unexplained but harmless.

## Findings

### Blocker 1: feature 018's reset-rule registry contradicts the code

- **Where:** `/Users/marcelo/Documents/Projects/sagat-wt-022/specs/018-schedules-bulk-operations/contracts/journal-and-events.md`
  - `:134` still says: "the five `rate_limit.*` event types, until a reset admits its run through the rate gate (T022-086) … `EXCLUDE`".
  - `:15`: the reducer effect of `run.reset` names only "a fresh `concurrency.acquired`".
  - `:55-60`: "Feature interplay" covers run permits only.
- **Rules concerned:** that section is titled "Reset-rule registry (normative; coordinator decision C1)" and says "Feature 018 owns this registry". Task T022-086 asks for the rules "registered in 018's registry". AGENTS.md rules 10 and 23 apply.
- **Why it matters:** the code now disagrees with the normative registry:
  - `reset.rs:5` says the code table mirrors this document.
  - `crates/domain/tests/reset.rs:103` says its expectations are "typed from `contracts/journal-and-events.md`". They no longer are.
  - A reader of 018's contract would believe no point after a rate event is offered.
- **Smallest correction:**
  - Replace row 134 with the two rows the code installs, using the same wording as rows 131-133:
    - `rate_limit.queued`, `granted`, `returned`: `REMATERIALIZE`. A point between a run-scope `queued` and its `granted` is refused.
    - `wait_timed_out`, `dequeued`: `REFUSE` until the end.
  - Add "or a fresh `rate_limit.granted`" to `:15`.
  - Add one bullet on rate slots at `:55`.
- **Check to add:** no test reads this registry. A contract test that parses its rows for 022 event types and compares them with `RESET_RULES` would have failed here.

### Blocker 2: the Studio's generated client is behind the OpenAPI

- **Where:** `/Users/marcelo/Documents/Projects/sagat-wt-022/frontend/apps/studio/src/api/schema.d.ts` still holds the old text in two places: the `Error` details description ("reset never queues);") and the `failureReason` description ("…CONCURRENCY_PERMIT_UNAVAILABLE, RUN_PURGED. The list grows only…").
- **Why it fails:** `frontend/apps/studio/scripts/check-api.mjs` compares the generated file byte for byte, so `npm run check:api` fails.
- **Evidence gap:** the evidence in `tasks.md:483-498` names no Studio command at all: neither `check:api` nor `npm test` for `ResetRun.test.tsx`.
- **Smallest correction:** `(cd frontend/apps/studio && npm run api && npm run check:api && npm test -- ResetRun)`. Read the diff, which should touch only those two descriptions, and add the output to the evidence.

### Major 1: a `CANCEL_SOURCE` reset on a full rate key always cancels the run and makes no replacement

- **Where:** `docs/runbooks/run-reset.md:257`, and D022-069 item 3 (`research.md:1233-1236`).
- **Why it happens:** a run's slot is never given back; it counts for one window from its grant. Cancelling the source therefore frees nothing.
  - With the key at its limit inside the window, a `CANCEL_SOURCE` request cancels the source and then reliably fails with `RATE_LIMIT_SLOT_UNAVAILABLE`.
  - `a_pending_reset_takes_a_fresh_slot_or_fails_rate_limit_slot_unavailable` shows exactly this outcome.
  - A permit is different: the cancel frees it, and the runbook's permit row warns about `CANCEL_SOURCE`. The rate row does not.
- **Rules concerned:** operability (the runbook must name the trap), and decisions (item 3 records the consequence but weighs no alternative, unlike D022-064 and D022-066).
- **Consequence:** a customer's run can be lost for a cause the platform could have predicted, and only the metric `workflow_reset_completions_total{outcome="failed"}` shows it afterwards.
- **Smallest correction:**
  - In the runbook row, say that `CANCEL_SOURCE` cancels the source first and the cancel does not free its slot. Advise `REQUIRE_TERMINAL` for rate-limited workflows near their limit.
  - In item 3, record the alternatives that were weighed:
    - an advisory synchronous refusal of a `CANCEL_SOURCE` request whose window is full when it arrives;
    - letting the reset run take over the source's slot. This goes against the approved "fresh slot" wording of `contracts/journal-events.md`, so it would be an owner question.

### Major 2: only the reset part of task T022-086 is built, and the built note does not say so

- **What is missing:** the task's first clause is "Run-creator cases of T022-064 extended to the rate gate".
  - `tests/integration/flow-control-step-policies/run_creators.rs` has rate cases only for the reset (`:1680-1990`).
  - The creators of T022-064 have no rate counterpart: business-id start (`:128`), rerun as new (`:158`), signal-with-start (`:227`, `:1345`), update-with-start (`:288`), batch rerun (`:570`), refused with-start (`:663`), ad hoc and its private rerun (`:688`).
  - `rate_creators.rs` (T022-072) covers only events, cron, schedules, children and delayed starts.
- **Partial cover:** `scripts/check-architecture.py` (`run_creator_errors`) already enforces one admission point and a start only through `Engine::start`, so a bypass is caught structurally. The behaviour of each creator with a rate gate is still unproven.
- **Consequence:** the task cannot be ticked on this evidence.
- **Smallest correction:** either add one rate twin per creator (at least business-id start, rerun, with-start and batch rerun), or amend the task text and D022-069 to say this clause is covered by the structural check, with the reason.

### minor 1: the lists of reset failure reasons drift with nothing to catch it

- **Where:** `apps/cli/src/reset.rs:271` and `apps/cli/tests/reset_and_with_start.rs:995` have no sentence for `RATE_LIMIT_SLOT_UNAVAILABLE`; the Studio got one (`ResetRun.tsx:82`).
- **Check gap:** nothing checks that the Studio map, the command-line list, the 001 OpenAPI description and the database CHECK all hold `FAILURE_REASONS` plus `SOURCE_DID_NOT_END` and `RUN_PURGED`.
- **Correction:**
  - Add the command-line sentence and its test row.
  - Add one integration test that reads `pg_get_constraintdef` of `run_resets_failure_reason_valid` and asserts it equals that set.
  - Add one contract test that asserts each name appears in the Studio map, the command-line list and the OpenAPI description.

### minor 2: the runbook lifts a production gate before the task is ticked

- **Where:** `docs/runbooks/flow-control.md:196-198` says T022-086 is "done" and drops it from the production gate. `tasks.md:479` is unticked ("not ticked until its independent review").
- **Rule concerned:** D022-066 item 5 says the open gates bind where they are read.
- **Correction:** keep T022-086 in the list until the task is ticked, or tick it in the same commit after the review.

### minor 3: compatibility of the `restored` marker on rate events is not written down

- **Where:** `crates/infra-postgres/src/event_codec.rs:1085-1089` now accepts `restored` on `rate_limit.*` events. A build that already advertises `FlowControlRateV1` but predates this change decodes them as `None`, so its replay fails.
- **Why it is safe today:** no such build was released (tags are `v0.1.0` and `v0.2.0`, and feature 022 is unreleased).
- **Gap:** that is not recorded anywhere.
- **Correction:** add one sentence to D022-069 explaining why the capability stays at version 1. Add to runbook section 6: "finish the rollout before resetting a rate-limited run".

### minor 4: the rate-first order of a reset refusal is never tested end to end

- **Where:** `readiness_gate.rs:1781` passes `refusing` by hand. The integration case "both" (`run_creators.rs:1842-1868`) has only the slot refusing, or only the permit.
- **Correction:** add one case where the permit has a running holder and the window is full at the same time, and assert `SlotUnavailable`.

### minor 5: the new migration test hard-codes the previous version

- **Where:** `tests/integration/flow-control-step-policies/migrations.rs:1441` sets `BEFORE_RESET_SLOT = 20_270_207_000_100`. 018's `reset_expiry_migration.rs:16-26,81-90` computes `previous()` from `MIGRATOR` and has a guard test against renumbering.
- **Consequence:** a renumber at merge silently changes what is tested.
- **Correction:** do the same as 018's test.

### minor 6: the migration evidence is incomplete

- **Where:** item 7 of the migration skill asks for `runtime_privileges` and a run with `SAGAT_TEST_AS_RUNTIME_ROLE=1`. Neither is in the evidence, and the integration tests (`run_creators`, `migrations`) are named without their results.
- **Risk:** low, because no grant changes.
- **Correction:** add the commands and their output.

### minor 7: 018's own documents name only the permit

- **Where:** `specs/018-schedules-bulk-operations/spec.md:486-493` (Requirement 018-050) and `plan.md` rows 17 and 18 still mention only the permit. 018's errors table, data model and OpenAPI now register the slot reason.
- **Correction:** add one sentence to each.

### nit 1: a doc comment is attached to the wrong function

- **Where:** `readiness_gate.rs:1656-1661`. `reset_refused` was inserted between `start_refused`'s doc comment and `start_refused` itself, so `reset_refused` carries both doc blocks and `start_refused` has none.
- **Correction:** move lines 1656-1658 to just above `start_refused`.

### nit 2: "grows only" now reads as if it applied to the new reason alone

- **Where:** `specs/018-schedules-bulk-operations/contracts/rest.openapi.yaml:916` ends with "(feature 022; grows only)".
- **Correction:** keep "grows only" as a statement about the whole list.

### nit 3: the two flow-control reset rows of feature 022's errors table read differently

- **Where:** `specs/022-flow-control-step-policies/contracts/errors.md:780`. The permit row still says "per 018 | 018 contract"; the new slot row says "409 | `CONFLICT`".
- **Correction:** align the permit row with the slot row.

### nit 4: dates disagree

- D022-069's heading and `run-reset.md:376` say 2026-11-20.
- Gap G022-016 says "Closed 2026-11-21" and the built note says "Built 2026-11-21".

### nit 5: the task text miscounts the events

- **Where:** `tasks.md:479` says "four `rate_limit.*` events"; there are five.

### nit 6: "then" reads as if both reasons were given

- **Where:** "`RATE_LIMIT_SLOT_UNAVAILABLE` then `CONCURRENCY_PERMIT_UNAVAILABLE`" in `crates/application/src/execution.rs:2235-2236`, `crates/application/src/runs.rs:1994-1995` and `readiness_gate.rs:1198`.
- **Correction:** write "rate first: X, else Y".

### nit 7: the synchronous refusal is not in the reset runbook's answer table

- **Where:** `run-reset.md` section 5 (`:330-344`) lists no `409` `RATE_LIMIT_SLOT_UNAVAILABLE`, and no permit refusal either. Section 7, "What is not built yet", describes behaviour that is now built.

### nit 8: the rate-first rule lives in the adapter

- **Where:** `reset_refused` and `start_refused` hold Requirement 022-052 (rate first) in the adapter (`readiness_gate.rs:1662`, `:1685`).
- **Suggestion:** a small domain function on `DeclaredGates` that picks the refusal from the refusing gate kinds would make it a domain fact, testable without the adapter. This repeats an existing pattern, so it is not a regression.

## Checks behind the rules this change relies on

| Rule | What enforces it |
|---|---|
| A reset never waits | Domain guard `flow_control.rs:353`; `start_may_wait`; integration test asserts no waiter row (`run_creators.rs:1813-1818`) |
| One admission point for every run creator | `scripts/check-architecture.py`, `run_creator_errors` |
| Restored grants are not counted | `verify_run` and `assert_consistent`, exercised by `a_reset_takes_a_fresh_slot_right_after_run_reset` |
| The `restored` marker accepts only `true` | Codec test (`event_codec.rs:1770-1790`) |
| The error row, the API sentence and the OpenAPI agree | `tests/contract/schedules-bulk-operations/errors.rs` |
| Metric labels are a closed vocabulary | `flow_control_vocabulary.rs`; `reset_completion.rs:591` |
| The migration's down refuses data loss | `the_reset_slot_reason_rolls_back_only_once_no_entry_carries_it` |
| 018's normative registry matches the code | **Nothing** (Blocker 1) |
| The database CHECK equals `FAILURE_REASONS` | **Nothing** (minor 1) |
| The Studio, command-line and OpenAPI reason lists agree | **Nothing** (minor 1) |
| The rate-first order when both gates refuse | **Nothing** end to end (minor 4) |
