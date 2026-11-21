# Constitution 4.7.0 — Sync Impact Report

This record accompanies the pull request that adds the rules "Documentation first" and "Words in full" and renames the
identifiers to words. **Status: PROPOSED**; it is adopted by merging the pull request that carries it, on three
instructions of the owner of 2026-11-20, recorded verbatim in [`docs/owner-decisions.md`](../owner-decisions.md) as
Owner decisions 2026-11-20-8 (any change or new rule updates the document before the code; the code follows the
document, never the reverse), 2026-11-20-9 (write words in full, no abbreviations such as "R" for rule or "T" for
test) and 2026-11-20-10 ("Rename everything": the stable identifiers too). The owner may overrule it by a later
amendment.

## Scope and why MINOR

Three bullets of Development Workflow change; nothing else does.

- **Documentation first** (added, after the same-delivery rule, which it makes stronger). The same-delivery rule asks
  that spec, contracts, tests and documentation change together with the code; it says nothing about which leads, so
  a pull request could ship code and then write the document to match it. The new bullet says the document is the
  source of truth: a change of behaviour or a new rule is written in its document first, and the code comes with it or
  after it, never ahead of it. When code and document disagree, the wrong one is fixed and they are never left apart.
  It also says how `docs/` is written: for people, in plain language, with an index in every folder and diagrams where
  they help. Every pull request MUST pass `scripts/check-docs-first.py`.
- **Words in full** (added). Documents and code comments write "Rule 8", "test", "Blocker 1", "Major 2", "minor 7",
  "slice 5", "mutation 3", not the abbreviations. Every pull request MUST pass `scripts/check-plain-words.py` over the
  lines it adds.
- **Identifiers** (rewritten). The identifiers keep their numbers and their meaning; their written form becomes words,
  a space and the number (`Task 022-064`, `Requirement 022-035`, `User story 022-02`, `Decision 041-119`,
  `Architecture decision 0097`, `Owner decision 2026-11-20-10` and the others). The table from old to new and the
  transition live in [`identifier-scheme.md`](identifier-scheme.md): past commits keep the old forms; until one
  mechanical sweep renames the existing text and the tools, the places a tool reads keep the old forms.

No principle changes, and the specification process (EGM, the design doc, its stages, approval) is unchanged, so the
amendment is not MAJOR. The rename of the identifiers is the closest call: it changes how every document cites a
requirement, but not what a requirement, a task or a decision is, nor how one is created, reviewed or closed, and it
keeps every number, so a citation can always be traced. If the owner reads it as a change of the mandated process, the
same text is a MAJOR (5.0.0) with no other change. It adds rules and obligations, so it is not PATCH.

Why checks and not only prose: AGENTS.md rule 21 says that a rule worth keeping gets a check that fails when it is
broken. Both checks are deliberately narrow (see below).

## Complete clause impact

| Unit of 4.6.0 | Disposition | Result |
|---|---|---|
| Core Principles I to VIII | KEEP | Byte-identical; hashes in the table below |
| Delivery Constraints | KEEP | Unchanged |
| Development Workflow: Identifiers | REWRITE | The identifiers are written as words and the number; the old forms are mapped in `identifier-scheme.md`, with the transition |
| Development Workflow: Words in full | ADD | Words in full in documents and code comments; every pull request passes `scripts/check-plain-words.py` |
| Development Workflow: Documentation first | ADD | The document leads and the code follows; `docs/` is written for people; every pull request passes `scripts/check-docs-first.py` |
| Every other Development Workflow bullet | KEEP | Unchanged; the CI gates bullet still decides which tests a pull request runs, and the two checks run in addition to them |
| Governance | KEEP | Unchanged |
| Footer | REWRITE | `Version: 4.7.0`, the link to this report and the adoption sentence |

## Principle hashes

Previous-source: 2d745823b77027b8afd6449f7a417ced3a0cd6b6

The line above is the git blob of `docs/constitution.md` at 4.6.0. A principle block is hashed as in the 4.0.0 report.

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

## What the documentation-first check can and cannot see

`scripts/check-docs-first.py --base <base> --head <head>` reads the pull request's own commits (`base..head`, merges left
out). It judges a commit only if the commit's tree already contains the check itself, so commits written before the rule
existed (on branches open when it merged) are never judged; a commit made after such a branch merges `develop` is.

- **Behaviour code** is any file that is not Markdown under `crates/`, `apps/`, `sdks/`, `runners/`, `frontend/apps/`
  or `infra/`, and any file in a `migrations/` directory.
- **A document** is any file under `docs/` or `specs/`.
- The pull request passes when no judged commit changes behaviour code, or when any of its own commits changes a
  document. Otherwise every judged commit that changes behaviour code MUST carry the footer `Docs: none, <reason>`,
  and the check prints each one so that a reviewer can judge the reason.
- When a judged commit changes behaviour code before the first commit that changes a document, the check passes and
  prints a warning: the document came after the code. It does not fail, because the only fix would be to rewrite the
  branch, and force pushes are refused here.

What it cannot see: whether the document that changed is the one that describes the code. An unrelated edit of any
file under `docs/` satisfies it. A stronger form would require the document to belong to the area or the feature of the
code, but no mapping from a source path to its document exists today: `scripts/select-test-targets.py` maps sources to
test targets, not to documents, and the design docs carry no list of the paths they govern. Building that mapping is a
follow-up (below); until then the reviewer judges whether the right document changed, as before.

## What the words-in-full check can and cannot see

`scripts/check-plain-words.py --base <base> --head <head>` reads the lines the pull request's own commits add, with the
same rule as above for commits written before it existed, and keeps only those still present at the head. It reads the
Markdown under `docs/` and `specs/`, and the comments of code files (the text after `//`, `#` or `--`, and the lines of
a block comment). It skips text in backticks, fenced code blocks, double-quoted text (a quotation), link targets, and
review reports (`*.reviews/`, `specs/<feature>/reviews/`), which are the reviewers' own words.

Its patterns are the abbreviations actually found in this repository, each with the words that make it one: a finding
number after a reviewer or the word "finding" (`architecture B1`, `security M2, N3`, `finding m7`), a rule number
after "rule" or in a list of rule numbers (`rule R8`, `R2, R3, R8`), the finding of an EGM run (`Z3-1`), a slice after
"slice" (`slice S5`), a mutation after "mutation" or "mutant" (`mutation W3`), and `UT`. A letter and a number alone are
not refused: Amazon S3, Cloudflare R2, an Apple M3, the milestones M0 to M6, the namespaces N1 and N2 of an example and
the priorities P1 to P3 all appear in the documents and are not abbreviations of a word. How the false positives were
measured is in the pull request description and in `scripts/tests/test_check_plain_words.py`.

While `identifier-scheme.md` says `Sweep: pending`, the old identifier forms are not refused (the tools still parse
them); after the sweep they are, in the lines a pull request adds.

## Dependents

- `AGENTS.md` gains rule 23 (documentation first) and rule 24 (words in full), and its "Start here" and rule 15 cite
  identifiers in the new form. `CLAUDE.md` lists no rules (it imports `AGENTS.md`) and does not change. The pull
  request template's `Refs:` example uses the new form.
- `docs/governance/identifier-scheme.md` is new: the table from old to new, the transition, and the `Sweep:` line the
  words-in-full check reads.
- `scripts/check-docs-first.py`, `scripts/check-plain-words.py` and their tests are new.
- `scripts/check-design-docs.py` prints "Rule 8", not "R8", in every failure; its tests and the documents that named a
  rule by its abbreviation follow.
- `.github/workflows/ci.yml` gains the job `documentation`, which runs the two checks on every pull request and on
  nothing else; a push to `main`, `release/**` or `hotfix/**`, a tag and a manual run start the same jobs as before.
  `scripts/tests/test_ci_workflow.py` guards that.
- `docs/engineering/architecture-principles.md` (the table of rules and their checks), `docs/engineering/sdd-workflow.md`,
  `docs/test-strategy.md` and `scripts/README.md` say what the check does.

## Follow-ups

- **A document of the right area.** Give every design doc (and every feature not yet converted) the list of source paths
  it governs, then require that a pull request which changes a governed path changes a document of the same area.
- **The move and the rename.** The owner wants every specification in `docs/` and every identifier in words. Both are
  one mechanical pull request, planned in its own document and done after the large open feature branches merge and
  before release 0.3.0, because thousands of lines name `specs/` or an identifier in its old form.
- **Naming the kind of an identifier the first time in a paragraph** is asked of writers, not checked: a check over
  changed lines cannot see the paragraph's earlier lines reliably, and tables cite identifiers by column.

## Risks and rollback

- A pull request that forgets the footer on a refactor fails, and a footer cannot be added to a pushed commit without a
  force push. The fix is to change the document that should describe the change, which is what the rule asks; if no
  document should change, a new commit cannot exempt the old one, so the branch is recut. The check prints this advice.
- An unrelated document edit passes the documentation-first check (above). Review stays the guard for that.
- The words-in-full check misses an abbreviation written without the words around it (a bare `B1`). It is tuned to
  refuse nothing that is not an abbreviation, because a check that cries wolf gets an escape added to every line.
- Until the sweep, the documents mix old and new identifier forms. The table maps them.
- Rollback is a forward amendment that removes the bullets, the job and the scripts.
