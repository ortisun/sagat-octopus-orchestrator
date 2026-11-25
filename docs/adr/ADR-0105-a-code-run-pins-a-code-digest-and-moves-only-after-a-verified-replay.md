# ADR-0105: A code run pins a code digest, and moves to other code only after a verified replay of its own history

**Status:** Proposed
**Decided by:** agent (Claude Code, design 044), under the owner's delegation (OD-2026-11-20-6 for Owner questions 044-001 and 044-002; OD-2026-11-24-4 for the acceptance)
**Refines / supersedes:** applies principle IV (a run references one version; no automatic migration) to code versions; complements feature 019, which versions activity builds only; adds the action `MOVE_CODE_VERSION` to the batch operations of ADR-0050
**Enforced by:** `workflow_tasks_follow_the_current_digest`; `a_completion_from_a_session_of_another_digest_is_stale`; `sagat_app_cannot_update_code_version_id`; `the_move_function_refuses_a_version_of_another_workflow`; `a_move_applies_only_after_a_matching_replay`; `a_move_waits_while_the_workflow_task_is_leased`; `a_move_of_a_run_whose_visible_count_grew_is_rechecked_then_run_busy`; `a_moved_run_keeps_its_step_and_compensation_identities`; `a_move_without_version_activate_is_refused`; `an_all_open_move_without_run_batch_is_refused`; the conformance scenario `code-patch-and-move` (design 044, Tasks 044-6, 044-8, 044-9, 044-13)

## Context

Temporal pins a run to a build identifier the deployer chooses, and its automatic upgrade lets new code replay open runs
with no check; its patches are checked by nothing on the server before they apply. Principle IV says a published version
is immutable, every run references exactly one version, and there is no automatic migration of a running workflow. Code
runs can last months, so "never change code under a run" would keep old workers for months and leave a run under review
only reset or termination. Step identifiers, journal instance identifiers and compensation identifiers are derived from
`runs.workflow_version_id` (`crates/application/src/execution.rs`, `crates/infra-postgres/src/execution.rs`, the saga
tables), so that column cannot change.

## Decision

1. A code version MUST name the SHA-256 digest of its code, its dependency closure and its runtime (design 044,
   Technical Plan 3.2). A code run MUST keep its start version (`runs.workflow_version_id`) for its whole life and MUST
   have one current code version (`runs.code_version_id`) at any moment, equal to the start version until a move. Its
   workflow tasks MUST go only to sessions of the current version's digest, and a completion from a session of another
   digest MUST be refused as stale.
2. The current code version of an open code run MUST change only through a move: an explicit request of a person with
   `run:operate` and `version:activate` (and `run:batch` for many runs), to a published code version of the same workflow
   and namespace that is approved for activation where the namespace requires approval, applied per run in one
   transaction with the run locked, only when a session of the target digest replayed the run's history with the same
   keyed command chain, the run's visible count has not changed since, its `~workflow` step is not leased, and every
   outstanding step fits the target version's envelope. Only the `SECURITY DEFINER` function that applies a move MAY
   change `runs.code_version_id`; the runtime role MUST NOT hold the update privilege on it. Each applied move MUST be
   journaled (`run.code_version_moved`) and audited; a refused one MUST leave the run unchanged and record why.
3. A move MUST be available as a dry run that records verdicts and moves nothing. No policy MUST move runs by itself in
   0.3.0.
4. Each SDK MUST offer `patched(id)` and `deprecatePatch(id)`, recorded as markers.

## Considered options

1. **Pin, plus explicit verified per-run moves of a separate current code version** — a deploy cannot change the code
   under a run; long runs can take fixed code; every change under a run is proven, inputs included, and recorded; no
   derived identity changes.
2. **Replay on the newest code (Temporal's automatic upgrade)** — lost: the failure the design exists to avoid.
3. **Pin forever** — lost: months of old workers; a run under review cannot be repaired in place.
4. **Re-pin `runs.workflow_version_id`** — lost: every identity derived from it would stop matching the run's own steps,
   leases and compensations.
5. **An automatic move policy at each deploy** — not in 0.3.0: principle IV forbids automatic migration (Gap 044-006).
6. **A user-chosen build identifier instead of a computed digest** — lost: a reused identifier with different code is
   the case that corrupts runs.
7. **A promotion replay check** — lost: a promotion changes no open run; a dry-run move answers the question.

## Consequences

- Good: no nondeterminism from a deploy; moves are proven per run and auditable; the version list shows what still runs
  on each digest.
- Bad: workers of old digests stay up until their runs end or move; in a compiled language a change of the worker binary
  is a new digest, so the recommended topology keeps workflow code in its own binary and the pipeline requests a move.
- Compatibility: a new column, a database function and a batch action; the start version never changes. The
  constitution 5.0.0 text should say that such a move is not automatic migration (Gap 044-010).

## Evidence

Design 044, Technical Plan 3.2, 7; Owner questions 044-001 and 044-002 and Decisions 044-042 and 044-048 in
`specs/044-workflow-as-code/research.md`; the design and security findings of round 1.
