# Constitution 4.0.0 — Sync Impact Report

This record accompanies the pull request that replaces Spec Kit with the Elephant-Goldfish Model (EGM) as the only
specification workflow and moves the constitution to `docs/constitution.md`. **Status: ADOPTED on 2026-11-03** by merging the
pull request that carries it, on the owner's delegation to the assistant ("Decida por mim", row OD-2026-11-03-32); the owner
may overrule it by a later amendment. The design is [constitution-4.0.0-egm.md](../designs/constitution-4.0.0-egm.md), whose Status is still
`Draft`; the decisions it records under delegation are marked `Decided by: delegate` in
[`docs/owner-decisions.md`](../owner-decisions.md) until the owner confirms them.

## Scope and why MAJOR

The mandated specification process changes: the Spec Kit cycle, the `plan.md` Constitution Check and the file location of
the constitution (`.specify/memory/constitution.md`) are replaced by EGM design docs, a design-doc check and
`docs/constitution.md`. The Versioning clause as it stood (MAJOR is a principle removed or redefined) would have made this
MINOR, so the same change amends it: a change of the mandated specification process is also MAJOR. No principle changes.

The "broader reviewed Constitution candidate" named in the 3.0.0 report is still not adopted and is out of scope.

## Complete clause impact

A **unit** of 3.0.0 is one principle, one bullet or paragraph of any other section, or the footer.

| Unit of 3.0.0 | Disposition | Result |
|---|---|---|
| The file `.specify/memory/constitution.md` | MOVE | To `docs/constitution.md` by `git mv`, in a commit of its own |
| Core Principles I to VIII | KEEP | Byte-identical; hashes in the table below |
| Delivery Constraints (four bullets) | KEEP | Unchanged |
| Development Workflow: Specification first | KEEP | Unchanged |
| Development Workflow: Spec Kit cycle | REWRITE | Replaced by the EGM procedure, Approval, Owner decisions, EGM pin and Transitional clause bullets. Its last sentence (the numbering of features 001–012) is DROPped: stale, no obligation lost |
| Development Workflow: Identifiers | REWRITE | The sentence about the generating template is removed; the obligation that an id is unique across features is stated directly; a cross-cutting design uses `T<prefix>-<n>` |
| Development Workflow: Read before you change | REWRITE | Names the design doc (or, for a feature not yet converted, its `spec.md`, `plan.md`, `tasks.md`). The obligation "MUST NOT implement what belongs to another feature without an approved dependency" stays; an approved dependency is an owner decision cited by an `Owner-gate:` line |
| Development Workflow: Same-delivery rule | REWRITE | "A checked task in `tasks.md`" becomes "a ticked task in the design doc (or in `tasks.md`)" |
| Development Workflow: CI gates | REWRITE | The design-doc check joins the list that MUST pass before merge |
| Development Workflow: Branches and commits | KEEP | Unchanged |
| Governance: supremacy, Architecture Decision Records, Amendments | KEEP | Unchanged |
| Governance: Versioning | REWRITE | MAJOR also covers a change of the mandated specification process |
| Governance: Compliance review | REWRITE | Four obligations stay (the Check is re-evaluated after design; every pull request is reviewed against it; a check is not approval of the implementation; an unavoidable violation MUST be justified in Complexity Tracking with the simpler alternative rejected). Only the noun changes, "plan" becoming "design doc"; for a feature not yet converted the Check and the table stay in its `plan.md` |
| Footer: Version line | REWRITE | `Version: 4.0.0`; `Ratified: 2026-10-11` unchanged; `Last Amended: 2026-11-03` |
| Footer: closing paragraph | REWRITE | Links to the 4.0.0, 3.0.0 and compensation-safety reports; the 3.0.0 adoption sentence is kept. The 4.0.0 adoption sentence is added by the owner's merge |

Added units, not in 3.0.0: **Approval** (a design doc is `Approved` when `egm-review` and the three repository reviewers
end with no Blocker or Major open; the owner accepts findings in writing), **Owner decisions** (moved from a bullet of
`AGENTS.md`), **EGM pin** and **Transitional clause**, which also keeps the checklist gate that the generated implement skill enforced: unticked `checklists/*.md` items of a feature not yet converted block its implementation until the owner accepts them.

Of the 31 sentences of 3.0.0 that use MUST or MUST NOT, 28 are present verbatim in 4.0.0. The other three are the REWRITE
rows above: "A feature MUST NOT implement what belongs to another feature without an approved dependency" (now followed by
what an approved dependency is), the CI gates sentence (the design-doc check added) and the Complexity Tracking sentence
("the plan's" became "the design doc's"). No obligation is dropped.

## Principle hashes

Previous-source: 6a7e6d3fe26fc73159be6faf2086027d78c1ed4d

The line above is the git blob of `.specify/memory/constitution.md` at 3.0.0. A principle block is the text from its
`### ` heading through the line before the next `## ` or `### ` heading, trailing blank lines removed, LF line endings,
UTF-8, hashed as the SHA-256 of the lines joined by LF with no trailing newline.
`scripts/check-design-docs.py` (rule R8) recomputes both columns and requires them to be equal on every KEEP row.

| Principle | Disposition | sha256 (previous) | sha256 (this version) |
|---|---|---|---|
| I | KEEP | 32d2ec982a4ae0995a472628184c4762ea740f9081886bb4c78fa461237a5a21 | 32d2ec982a4ae0995a472628184c4762ea740f9081886bb4c78fa461237a5a21 |
| II | KEEP | 80fbee5c64129b11cc2cc1d978c7a4b91f9af39cfdea73510c3bcb345aad8ec2 | 80fbee5c64129b11cc2cc1d978c7a4b91f9af39cfdea73510c3bcb345aad8ec2 |
| III | KEEP | 69d38cce2aa95cfa7f3e70b2be2e93260ff8f224afa7e8f779e5558b4f9827d9 | 69d38cce2aa95cfa7f3e70b2be2e93260ff8f224afa7e8f779e5558b4f9827d9 |
| IV | KEEP | ff10043d76dfd25ac3b2db1ea27b943804c52d4d46c83838e6908cdaccc32f7b | ff10043d76dfd25ac3b2db1ea27b943804c52d4d46c83838e6908cdaccc32f7b |
| V | KEEP | b816cb76163ef1d8cc8579bb3e326f59c774b077131029cfd64fa45d470f38f9 | b816cb76163ef1d8cc8579bb3e326f59c774b077131029cfd64fa45d470f38f9 |
| VI | KEEP | 3fbc81dd01da920dafb2681958b3a7864f90b6395d3aecb2254555436c6aa8bc | 3fbc81dd01da920dafb2681958b3a7864f90b6395d3aecb2254555436c6aa8bc |
| VII | KEEP | 708ea13f72890e3ff5bf0472c30fdb2a5cab3d5e616e40e88197c880eeba2760 | 708ea13f72890e3ff5bf0472c30fdb2a5cab3d5e616e40e88197c880eeba2760 |
| VIII | KEEP | df43387c86a14df13ffde3c5033971086a075c0b7e18fa538322c1f52b3d1a37 | df43387c86a14df13ffde3c5033971086a075c0b7e18fa538322c1f52b3d1a37 |

## Dependents updated in the same change

`AGENTS.md`, `CLAUDE.md`, `README.md`, `SPECIFICATION-INDEX.md`, `.gitignore`, `.github/workflows/ci.yml`, `docs/designs/README.md`
and `TEMPLATE.md`, `scripts/README.md`, `specs/AGENTS.md`, `specs/README.md`, the checklist wording of features 020 and 036,
`docs/engineering/{sdd-workflow,README,ai-engineering,architecture-principles}.md`, the three repository reviewers under
`.claude/agents/`, the `verify` and `parallel-agents` skills, `.github/pull_request_template.md`, `tests/Cargo.toml`, the
quickstarts of features 014 and 016, `.claude/hooks/session-start.py`, `.claude/settings.json`,
`scripts/check-agent-config.py` and its allowlist, `docs/owner-decisions.md`, `docs/adr/ADR-0077-egm-replaces-spec-kit.md`,
and the deletion of `.specify/` and the ten `.claude/skills/speckit-*` skills.

## Risks and rollback

- The check proves that an approval record is consistent, not that a review ran in a fresh context; the owner's review of
  the pull request is the control, and the assistant commits under the owner's git identity.
- The 20 features with open tasks are not converted by this amendment. The transitional clause keeps a governing rule for
  them, but no check covers their `spec.md`, `plan.md` and `tasks.md` until the conversion design lands.
- EGM is a third-party plugin; the pin detects a changed install and cannot reproduce a version.
- Rollback is a forward amendment, as for 3.0.0, not `git revert`. A revert restores the Spec Kit files but also stale task
  state in the design docs.
