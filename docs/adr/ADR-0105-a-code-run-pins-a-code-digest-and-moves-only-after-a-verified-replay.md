# ADR-0105: A code run pins a code digest, and moves to other code only after a verified replay of its own history

**Status:** Proposed
**Decided by:** agent (Claude Code, design 044), under the owner's delegation (OD-2026-11-20-6 for Owner question 044-001 and Owner question 044-002; OD-2026-11-24-4 for the acceptance)
**Refines / supersedes:** applies principle IV (a run references one version; no automatic migration) to code versions; complements feature 019, which versions activity builds only
**Enforced by:** `workflow_tasks_follow_the_pinned_digest`; `the_version_of_a_run_changes_only_through_the_move_function` (a trigger on `runs.workflow_version_id`); `a_move_applies_only_after_a_matching_replay`; `a_move_of_a_run_whose_journal_grew_is_rechecked`; `a_mismatch_leaves_the_run_on_its_version_and_records_why`; `a_move_is_refused_when_an_outstanding_step_is_outside_the_target_envelope`; the conformance scenario `code-patch-and-move` (design 044, Tasks 044-6, 044-8, 044-13)

## Context

Temporal's default lets every deploy change the code under open runs, which is how nondeterminism reaches production;
its patches and worker versioning are checked by nothing before they apply. Principle IV says a published version is
immutable, every run references exactly one version, and there is no automatic migration of a running workflow. Code
runs can last months, so "never change code under a run" would keep old workers for months and leave a held run only
reset or termination.

## Decision

1. A code version MUST name the SHA-256 digest of its code (design 044, Technical Plan 3.2); a code run MUST pin one
   version at any moment, and its workflow tasks MUST go only to sessions that registered that version's workflow type
   and digest.
2. The pin of an open code run MUST change only through a move: an explicit request by a person with `run:operate`,
   applied per run, in one transaction with the run locked, only when a session of the target digest replayed the run's
   history with no mismatch, the reported chain equals the stored chain, the run's journal has not grown since the
   replayed sequence, and every outstanding step fits the target version's envelope. Each applied move MUST be journaled
   (`run.code_version_moved`) and audited; a refused one MUST leave the run unchanged and record why.
3. No policy MUST move runs by itself in 0.3.0.
4. A promotion MAY require a replay check of up to 64 recent completed runs of the active version; a mismatch MUST
   refuse the promotion.
5. Each SDK MUST offer `patched(id)` and `deprecatePatch(id)`, recorded as markers.

## Considered options

1. **Pin, plus explicit verified per-run moves** — a deploy can never make a run nondeterministic; long runs can still
   take fixed code; every change under a run is proven and recorded.
2. **Replay on the newest code (Temporal's default)** — lost: the failure the design exists to avoid.
3. **Pin forever** — lost: months of old workers; a held run cannot be repaired in place.
4. **An automatic move policy at each deploy** — not in 0.3.0: principle IV forbids automatic migration; Gap 044-006.
5. **A user-chosen build id instead of a computed digest** — lost: a reused id with different code is exactly the case
   that corrupts runs.

## Consequences

- Good: no production nondeterminism from a deploy; moves are proven per run and auditable; the version list shows what
  still runs on each digest.
- Bad: workers of old digests stay up until their runs end or move; for compiled languages any change of the binary is
  a new digest, so long runs need a move after most deploys (cheap: an unchanged workflow function always matches).
- A worker of the tenant could forge a verdict (Gap 044-005); the harm stays inside the tenant.
- Compatibility: `runs.workflow_version_id` becomes updatable through the move function only. The constitution 5.0.0
  text should say that such a move is not automatic migration (Gap 044-010).

## Evidence

Design 044, Technical Plan 3.2, 7; Owner questions 044-001 and 044-002 in `specs/044-workflow-as-code/research.md`.
