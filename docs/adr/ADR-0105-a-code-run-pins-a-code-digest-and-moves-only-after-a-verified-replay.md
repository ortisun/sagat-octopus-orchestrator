# ADR-0105: A code run pins a code digest, moves to other code only on a person's request after a verified replay of its own history, and resets only within its code

**Status:** Proposed
**Decided by:** agent (Claude Code, design 044), under the owner's delegation (OD-2026-11-20-6 for Owner questions 044-001, 044-002 and 044-016; OD-2026-11-24-4 for the acceptance); no person has reviewed it
**Refines / supersedes:** applies constitution 5.0.0 principle IV (a procedural run's pin changes only by a verified move) to code versions; complements feature 019, which versions activity builds only; adds the action `MOVE_CODE_VERSION` to the batch operations of ADR-0050; refines ADR-0082 decision 1, as ADR-0110 decision 8 (branch `docs/temporal-parity-readiness`) does, by holding a recorded requirement's row `FOR SHARE` in a tenant's publication; refines ADR-0050 decisions 5 and 8 for code runs; supersedes ADR-0030 decision 16 (a same-version successor) for code runs
**Enforced by:** `workflow_tasks_follow_the_current_digest`; `publication_and_removal_of_the_requirement_race_and_one_is_refused`; `a_rerun_of_a_moved_code_run_starts_on_its_current_code_version`; `a_continue_as_new_after_a_version_promotion_pins_the_recorded_version`; `a_completion_from_a_session_of_another_digest_is_stale`; `sagat_app_cannot_update_code_run_pins`; `a_pin_inserted_with_another_version_than_the_start_is_refused`; `the_move_function_refuses_a_version_of_another_workflow`; `a_move_applies_only_after_a_matching_replay`; `a_move_waits_while_the_workflow_task_is_leased`; `a_move_of_a_run_whose_last_completion_moved_is_rechecked_then_run_busy`; `a_move_off_a_review_with_no_new_events_runs_the_failed_activation`; `a_moved_run_keeps_its_step_and_compensation_identities`; `a_move_without_version_activate_is_refused`; `a_batch_move_without_run_batch_is_refused`; `a_reset_point_with_an_attempt_in_flight_is_refused`; `a_reset_of_a_code_run_keeps_its_seed`; `contract_workflow_as_code::every_new_event_kind_has_a_reset_rule`; the conformance scenario `code-patch-and-move` (design 044, Tasks 044-2, 044-6, 044-8, 044-12, 044-13)
**Required before acceptance:** the fault suite of design 044 Task 044-28 and the measurements of Task 044-25

## Context

Temporal pins a run to a build identifier the deployer chooses, and its automatic upgrade lets new code replay open runs
with no check. Constitution 5.0.0 says a procedural run pins one version at any moment, its pin changes only by a
verified move requested by a person, and no policy moves runs. Step identifiers, journal instance identifiers and
compensation identifiers are derived from `runs.workflow_version_id`, so that column cannot change. ADR-0050 makes a reset
point with a waiting step ineligible and requires a reset rule per event kind; a code run nearly always has its
workflow-task step waiting.

## Decision

1. A code version MUST name the SHA-256 digest of its code, its dependency closure and its runtime (design 044, Technical
   Plan 3.2). A code run's start version (`runs.workflow_version_id`) MUST never change and MUST be its identity origin;
   its pin MUST be one current code version (`code_run_pins.code_version_id`), equal to the start version until a move.
   Its workflow tasks MUST go only to sessions of the current version's digest, and a completion from a session of
   another digest MUST be refused as stale.
2. The pin MUST change only through a move: an explicit request of a person with `run:operate` and `version:activate`
   (and `run:batch` for many runs), always a batch job whose selection is frozen at creation so that each item is a
   per-run request, the creator re-authorized when each item applies, to a
   published code version of the same workflow and namespace (every code version of a workflow keeps its workflow type) that is approved for activation where the namespace requires
   approval, the publication of each code version holding the fleet requirements it needs; applied per run in one
   transaction with the run locked, only when the run has no stop reason and its workflow-task step is not `CLOSED`, a
   session of the target digest replayed the
   run's history up to its last completed activation with the same command chain, that activation is still the last
   completed one, the workflow-task step is not leased, and every step that can still act (outstanding steps and the succeeded members of scopes not yet settled)
   fits the target version's envelope (its covering entry has the same secrets, schemas, risk metadata and approval rule).
   The move MUST be a domain command persisted by the ordinary run transaction; only the `SECURITY DEFINER`
   function `sagat_move_code_run`, which checks the pin's integrity, that the item's replay check records a matching
   verdict for the run at its current `S(k)`, and that the appended `run.code_version_moved` names the target, MAY change
   the pin; the runtime role MUST NOT hold the update privilege on it. A move MUST clear the step's sticky session, make it `READY` with a hint when it was under review or a
   task is pending, be journaled (`run.code_version_moved`) and audited; a refused one MUST leave the run unchanged and
   record why. No policy, schedule or pipeline MAY move runs.
3. A move MUST be available as a dry run that records verdicts and moves nothing. A move MUST be refused for a run under
   a `HISTORY_LIMIT` review, whose remedy is the retry; each item MUST end in feature 018's item model (a move or a
   dry-run match `SUCCEEDED`; a mismatch, a misfit or a stopping run `SKIPPED`; a busy run deferred, then `FAILED`) and
   record its verdict (`MATCHED`, `REPLAY_MISMATCH`,
   `ENVELOPE_MISMATCH` or a refusal) and its first difference. A rerun of a code run MUST start on its current code
   version.
4. A reset of a code run MUST be to its current code version only, in one transaction within ADR-0050's prefix ceiling.
   For a code run, an eligible point MUST be the end of a completion's journal block with no attempt in flight, no unknown
   outcome, no open, aborting or compensating saga scope, no open approval or child, and no code update admitted or
   accepted without being completed; the workflow-task step waiting and `READY` steps, timers and waits MUST NOT make it
   ineligible and MUST be rematerialized; the new run MUST start on the source's current code version, rebuild its overlay from the prefix's
   command records, keep the source's seed, recompute the chain of its prefix from the journal alone, rebuild its inbox
   from the prefix's `signal.buffered` and `signal.consumed`, and, when the prefix holds `run.cancel_requested` and no
   accepted terminal command, set its cancellation deadline from the grace that event carries; rebuild its update
   transitions from the prefix's records and copy the source's initiators, adding the resetter; `reapply: UPDATES` MUST
   be refused for a code run. `run.code_version_moved` MUST be `COPY` (the pin comes from the start version), and a
   code run's `saga.aborting`, `compensation.planned` and the scope's outcome events MUST be `COPY` once its scope's
   outcome is in the prefix, the reset rebuilding that scope's saga rows from them. This refines ADR-0050 decision 5
   for code runs: rows re-emitted with `restored: true` count for the visible count, the history and the chain. Every new event kind MUST have a reset rule in the registry of
   ADR-0050 decision 8, which gains a column for rules that differ for code runs.
5. An accepted continue-as-new MUST be recorded (`run.continue_requested`, with the resolved successor version) and
   close the workflow-task step, the handoff pinning exactly the recorded version; feature 014's continuation pass MUST perform the
   handoff in its own transaction and lock order (the chain before the run), ending the run `FAILED` when the chain's or
   tree's limits refuse it, while an input outside the successor's schema, a target that is not a code version of the
   same type, or a version without a `chainPolicy` is refused at the completion (`COMMAND_INVALID`). The successor MUST
   pin the run's current code version by default, or the workflow's active version when the command asks for it, which supersedes ADR-0030 decision 16 for code runs; it MUST be deferred, not counted, while
   an activity, child or approval is open or a saga scope aborts, and refused (`COMMAND_INVALID`) while a scope is open,
   when the run does not head an open continuation chain, or when its input is not an inline JSON object; the chain's
   limits govern whatever version the successor pins.
6. Each SDK MUST offer `patched(id)` and `deprecatePatch(id)`, recorded as markers.

## Considered options

1. **Pin, plus person-requested verified per-run moves of a separate pin, and resets within the code** — a deploy cannot
   change the code under a run; long runs can take fixed code; every change under a run is proven, inputs included, and
   recorded; no derived identity changes.
2. **Replay on the newest code (Temporal's automatic upgrade)** — lost: the failure the design exists to avoid.
3. **Pin forever** — lost: months of old workers; a run under review cannot be repaired in place.
4. **Re-pin `runs.workflow_version_id`** — lost: every identity derived from it would stop matching.
5. **A pipeline or policy that moves runs at each deploy** — lost: constitution 5.0.0 says no policy moves runs.
6. **A user-chosen build identifier instead of a computed digest** — lost: a reused identifier with different code is the
   case that corrupts runs.
7. **Reset to another code version** — not in 0.3.0 (Gap 044-019): it would need a replay check and an asynchronous reset,
   which ADR-0050 decision 2 does not allow.

## Consequences

- Good: no nondeterminism from a deploy; moves are proven per run and auditable; the version list shows what still runs
  on each digest.
- Bad: workers of old digests stay up until their runs end or move; in a compiled language a change of the worker binary
  is a new digest, so the recommended topology keeps workflow code in its own binary and a person requests the move.
- Compatibility with runs in flight: graph runs are untouched; code runs exist only after this decision; the start
  version never changes. Operational cost: one replay-check task per moved run on the tenant's workers; an item no
  worker claims ends `NO_TARGET_WORKER` after 3,600 s; old-digest workers stay up until their runs move or end.

## Evidence

Design 044, Technical Plan 3.2, 6.3, 7; Owner questions 044-001, 044-002 and 044-016 and Decisions 044-075, 044-076 and
044-085 in `specs/044-workflow-as-code/research.md`; the design, security, correctness and architecture findings of rounds
1 and 2.
