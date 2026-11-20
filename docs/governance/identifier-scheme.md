# Identifiers and words in full

How documents, code comments and commit footers name things. It carries out three owner decisions of 2026-11-20
([`docs/owner-decisions.md`](../owner-decisions.md)): owner decision 2026-11-20-8 (the document leads, the code
follows), owner decision 2026-11-20-9 (write words in full, no abbreviations such as "R" for rule or "T" for test) and
owner decision 2026-11-20-10 (rename the identifiers too). The constitution (version 4.7.0, Development Workflow,
"Identifiers" and "Words in full") points here.

Sweep: pending

The line above is read by `scripts/check-plain-words.py`. While it says `pending`, the check refuses only the
abbreviations that are not identifiers (the first table below). The pull request that renames every existing identifier
(the sweep) changes it to `Sweep: done`, and from then on the check also refuses the old identifier forms in the lines a
pull request adds.

## Abbreviations that are not identifiers

These are written out in every document and every code comment, from version 4.7.0 on. They are refused by
`scripts/check-plain-words.py` in the lines a pull request adds.

| Old | Write instead | Where it was used |
|---|---|---|
| R8, "rule R8" | Rule 8 | the rules of `scripts/check-design-docs.py` |
| B1, M2, m7, N3 after a reviewer ("architecture B1", "security M2, N3", "finding m7") | Blocker 1, Major 2, minor 7, Nit 3 | review findings |
| Z3-1, C3-1, K3-1, S3-1, T3-1 | finding 1 of the third run, with the reviewer named ("the zoom-out reviewer's finding 1 of the third run") | findings of an EGM review run, numbered per reviewer and run |
| S5, S6a after "slice" ("slice S5", "S6a slice") | slice 5, slice 6a | delivery slices |
| W3 after "mutation" or "mutant" | mutation 3 | hand mutations |
| UT | unit test | tests |

A finding number, a slice or a mutation is not a stable identifier: it is local to the report or the plan that numbered
it, so the words around it say which report.

## Stable identifiers: the new forms

Each new form is a fixed pattern: one or more words, a space, then the number. A checker or `grep` finds it as easily as
the old form.

| Kind | Old form | New form | Example (old, new) |
|---|---|---|---|
| Task | `TNNN-xxx`; `T<PREFIX>-<n>` for a design without a number | `Task NNN-xxx`; `Task <PREFIX>-<n>` | T022-064, Task 022-064; TEGM-3, Task EGM-3 |
| Requirement | `FR-NNN-xxx` | `Requirement NNN-xxx` | FR-022-035, Requirement 022-035 |
| Success criterion | `SC-NNN-xxx` | `Success criterion NNN-xxx` | SC-022-012, Success criterion 022-012 |
| Non-functional requirement | `NFR-NNN-xxx` | `Non-functional requirement NNN-xxx` | NFR-010-001, Non-functional requirement 010-001 |
| User story | `US0n` (local to its feature) | `User story NNN-0n` (always with its feature) | US02 of feature 022, User story 022-02 |
| Decision (feature register) | `DNNN-xxx`; bare `Dxxxx` | `Decision NNN-xxx`; `Decision xxxx` | D041-119, Decision 041-119; D1810, Decision 1810 |
| Gap | `GNNN-xxx`; bare `Gxxx` | `Gap NNN-xxx`; `Gap xxx` | G022-015, Gap 022-015; G708, Gap 708 |
| Owner question | `ONNN-xxx`; bare `Oxxxx` | `Owner question NNN-xxx`; `Owner question xxxx` | O041-040, Owner question 041-040; O1710, Owner question 1710 |
| Architecture decision | `ADR-NNNN` | `Architecture decision NNNN` | ADR-0097, Architecture decision 0097 |
| Owner decision | `OD-YYYY-MM-DD-n` | `Owner decision YYYY-MM-DD-n` | OD-2026-11-20-10, Owner decision 2026-11-20-10 |

A user story always carries its feature number. Every other kind already does, and a story number alone (US02) means a
different story in each feature; with the feature number it is unique across the repository and has the same shape as
the other kinds.

Commit footers: `Refs: Task 022-064, Requirement 022-035`.

## The transition

Past commits keep the old forms: a commit message cannot be rewritten without a force push, and history is not
rewritten. This page is how an old form in a commit, a pull request or a review report is traced to the new one.

From version 4.7.0 until the sweep:

- Commit footers (`Refs:`) and new prose use the new forms. No tool reads a footer.
- The places a tool reads keep the old form until the sweep changes the tool and the text together: the task line of a
  design doc or a `tasks.md` (`- [ ] T041-6 ...`), the `Owner-gate:` and `Accepted:` lines and the `ID` column of
  `docs/owner-decisions.md` and of a `research.md` register, and the heading `# ADR-NNNN` of an architecture decision.
  `scripts/check-design-docs.py` and `scripts/check-agent-config.py` parse them.
- `scripts/check-plain-words.py` refuses only the abbreviations of the first table.

The sweep is one mechanical pull request, after the large open feature branches merge and before release 0.3.0. It is
planned with the move of `specs/` under `docs/`. It renames every old form in documents, code comments, test names and
strings that cite an identifier, and the scripts that parse identifiers, from the table above, by a script with a
dry-run report. It does not rename wire values, error codes, database values, URLs, quoted historical text (review
reports, the owner's words) or commit messages. It then sets `Sweep: done` above.

## What is never an abbreviation

The check does not judge text inside backticks or a fenced code block (code, paths, literal output), text inside double
quotes (a quotation), link targets, review reports (`*.reviews/` and `specs/<feature>/reviews/`, which are the
reviewers' own words), or code itself: in a code file only the comments are read. Proper names that look like an
abbreviation (Amazon S3, Cloudflare R2, Apple M3, a milestone M4) are not refused, because the check looks for the
words around a finding, a rule, a slice or a mutation, not for a letter and a number alone.
