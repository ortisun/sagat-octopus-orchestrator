# ADR-0077: EGM design docs replace Spec Kit as the only specification workflow

**Status:** Proposed. Constitution 4.0.0 was merged on 2026-11-03 under the owner's delegation; an ADR is accepted by a person (`docs/adr/README.md`), so the owner accepts this one by changing this line.
**Decided by:** The owner chose the direction on 2026-11-02 (`docs/owner-decisions.md`, rows OD-2026-11-03-1 to -8). The process details (decisions A to W of the design) were taken by the assistant under the owner's delegation of 2026-11-03 and stay `delegate` until the owner confirms them.
**Refines / supersedes:** None. No ADR chose Spec Kit; Constitution 3.0.0 named it, and this ADR records the amendment.
**Enforced by:** `scripts/check-design-docs.py` (CI job `architecture`; tests in `scripts/tests/test_check_design_docs.py`), the settings guard and path check of `scripts/check-agent-config.py`, and `scripts/egm-tree-hash.py` with `docs/engineering/egm-accepted-hashes.json`.

## Context

Constitution 3.0.0 named Spec Kit as the way features are specified and required a Constitution Check in every `plan.md`.
Spec Kit's file layout (`spec.md`, `plan.md`, `tasks.md`) carried rules that nothing else states: a compliance gate, task
state with evidence, ids that commit footers cite, append-only registers, contracts that CI reads, a duplicate-work
warning, feature numbers and a pinned toolchain. The Elephant-Goldfish Model (EGM) supplies drafting and a zero-context
review, but no verdict, no ids, no task state and no owner gate; its review stops after one re-audit and records the rest
as accepted trade-offs. One EGM review of this repository (`tasks/reviews/2026-10-30-open-tasks/`) ended with 0 Blocker and
6 Major findings and every action open.

## Decision

1. Features and cross-cutting changes MUST be described in a design doc, `docs/designs/NNN-slug.md`, written with EGM and
   approved by the rule of Constitution 4.0.0: `egm-review` and the `architecture-reviewer`, `security-reviewer` and
   `correctness-reviewer` each end with no Blocker or Major finding open, a finding being closed by a fix or by the owner's
   written acceptance in `docs/owner-decisions.md`.
2. The constitution MUST live at `docs/constitution.md`; its eight principles are unchanged. A change of the mandated
   specification process is a MAJOR amendment.
3. After approval only a checkbox, an evidence block and follow-up tasks MAY change; anything else makes the doc a draft
   again. A task that depends on an owner decision MUST NOT be ticked until the cited owner row exists.
4. A review MUST NOT count unless it ran with an EGM plugin whose content hash is on the accepted list.
5. The Spec Kit tooling (`.specify/` and the ten `speckit-*` skills) MUST be removed. Existing `specs/NNN-*/` directories
   stay: contracts, evidence and registers are read by tests and CI, and features not yet converted keep their
   `spec.md`, `plan.md` and `tasks.md` under the constitution's transitional clause.

## Considered options

1. **EGM only, design docs in `docs/designs/`, checks in CI** — the owner's choice; it keeps one workflow and makes each
   rule that the layout supplied a named carrier or a recorded gap.
2. **Keep Spec Kit** — nothing to migrate and about 374 source files already cite its ids, but the owner no longer wants it.
3. **Delete `specs/` and move everything to `docs/designs/`** — one layout, but CI and tests read
   `specs/001-platform-foundation/contracts/` and other files, and closed tasks would lose their evidence.
4. **Use EGM for new features only** — cheapest, but two formats coexist for a long time and a task could be ticked in two
   places; the owner chose to convert the features with open tasks, in a follow-up design.
5. **No new checks, rules kept as prose** — no new code, but the rules would be unenforced, against rule 21 of `AGENTS.md`.

## Consequences

- A third-party plugin becomes part of the process. The pin is by content hash and detects drift; it cannot reproduce a
  version, and enabling the plugin in project settings loads nine agent types, two with write tools, into every session.
- The 20 features with open tasks keep hand-edited `spec.md`, `plan.md` and `tasks.md` until the conversion design lands;
  no check covers them, except that unticked `checklists/*.md` items block implementation until the owner accepts them (constitution, transitional clause; review only). Requirement-to-task coverage, the append-only check on `research.md` and a replacement for the
  `checklists/` and `taskstoissues` steps are deferred.
- The check proves that an approval record is consistent, not that a review ran in a fresh context. The assistant commits
  under the owner's git identity, so the owner's review of the pull request is the control for owner rows.
- A squash or rebase merge can make a reviewed commit unreachable; checks that need it run only for docs a pull request
  changes, and the convention is to merge with a merge commit.
- Rollback is a forward amendment, not `git revert`.

## Evidence

- Design: [constitution-4.0.0-egm.md](../designs/constitution-4.0.0-egm.md) and the review reports under
  `docs/designs/constitution-4.0.0-egm.reviews/` (three rounds, ending with the owner accepting what stays open).
- Sync Impact Report: [constitution-4.0.0-sync-impact.md](../governance/constitution-4.0.0-sync-impact.md), with the
  SHA-256 of each principle block in 3.0.0 and 4.0.0.
- The installed EGM copy and a `git archive` of upstream commit `f0ba151b252c9abf1318d9fe857a43265d06de89` hash to the same
  value (`003679ce…`), recorded in `docs/engineering/egm-accepted-hashes.json`.
