# Design docs

A design doc describes one feature (or one cross-cutting change) and carries its plan, its tasks and the record of its
approval. They are written with the Elephant-Goldfish Model (EGM): `egm-brainstorm`, `egm-draft`, then `egm-review`. The
rules below are enforced by `python3 scripts/check-design-docs.py`; the full definitions are in
[constitution-4.0.0-egm.md](constitution-4.0.0-egm.md) (sections 3 to 5), which is also the first design doc.

## Naming

- A feature: `docs/designs/NNN-slug.md`, where `NNN` is the number printed by `python3 scripts/next-feature-id.py`. It is
  the same number as `specs/NNN-slug/`, when that directory exists. Task ids are `TNNN-<n>`.
- A cross-cutting design has no number. It declares `*   **Task-prefix:** ABC` (2 to 8 capital letters, unique across
  docs) in its header, and its task ids are `TABC-<n>`.
- Review reports live in `docs/designs/<doc stem>.reviews/YYYY-MM-DD/`. Start from [TEMPLATE.md](TEMPLATE.md).

## Status block

The header is everything up to the first line of 20 or more hyphens. Its fields are `*   **Field:** value` lines. A
`Draft`, `Proposed` or `In Review` doc needs `Author`, `Status`, `Date` (and `Task-prefix` when unnumbered). An
`Approved` doc adds `Reviewed-commit`, `EGM-hash`, `Claude-Code`, `Reports` (exactly four), `Open findings` (the literal
`Blocker 0, Major 0`) and `Accepted`. A doc is `Approved` when `egm-review` and the `architecture`, `security` and
`correctness` reviewers each end with no Blocker or Major finding open; a finding the owner accepts in writing is not
open. Each report starts with `Doc:`, `Reviewed-commit:` and `Reviewer:` lines, has one `Counts: Blocker N, Major N, Nit N`
line, and carries `EGM-hash:` (egm-review) or `Agent-blob:` (the others).

## Tasks

A task is `- [ ] T<id> text`, indented continuation lines, and these lines:

- `Refs:` the requirements it closes (protected after approval);
- `Owner-gate: OD-YYYY-MM-DD-N` when it needs an owner decision (protected);
- `Evidence (YYYY-MM-DD): command and result` once ticked. The evidence block runs to the end of the task, at most 40
  lines, and has no heading, table row or code fence.

After approval only a checkbox, an evidence block and new tasks under `#### Follow-up tasks` may change. Any other
change makes the doc `Draft` again and needs a new review. Existing evidence lines are never edited.

## Owner decisions

`docs/owner-decisions.md` records them in tables with the header `ID | Decision | Decided by | Status | Covers`. Only a
row with a unique `OD-YYYY-MM-DD-N` id, `Decided by` exactly `owner` and `Status` exactly `Decided` can open a gate or
accept a finding. A gate row names the task id in `Covers`; an acceptance row names `<report path>#<finding number>`.

## Where the old Spec Kit artifacts go

| Spec Kit artifact | In a design doc |
|---|---|
| `spec.md` | Overview, The Problem, Requirements, Goals, Non-Goals, CUJs |
| `plan.md` | Technical Plan, Constitution Check, Complexity Tracking, Alternatives Considered |
| `tasks.md` | Detailed Implementation Plan (tasks with evidence) |
| `research.md` (D, G, O registers) | Stays in `specs/NNN-slug/research.md`, append-only |
| `contracts/`, `evidence/` | Stay in `specs/NNN-slug/`; tests and CI read them |
| `quickstart.md`, `checklists/` | Stay in `specs/NNN-slug/` |

Until a feature is converted, its `spec.md`, `plan.md` and `tasks.md` stand in for its design doc and are edited by hand
under the same rules (the constitution's transitional clause).
