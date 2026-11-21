# Architecture review of the EGM design v3

Doc: docs/designs/constitution-4.0.0-egm.md
Reviewed-commit: b3693e737138c966e688b248d0d028f6adcb48e1
Reviewer: architecture
Note: condensed by the author from the reviewer's output. Every finding, severity and location is kept; the evidence prose is shortened.

Last round's Blocker and Major findings: the transitional clause, the fenced example, delegate-versus-owner decisions, the hash allowlist, input handling and allocator coverage are resolved. Recounted: 20 features with open tasks, 1268 open tasks, 32 plans with a Check, 21 tracked files under `.specify/`, highest ADR 0076.

## Blocker

**B1. R3's header rule cannot be satisfied by any `Approved` doc (374-375, 435-439; CUJ 1).** A `Draft` has three status fields and the hyphen line at line 7; approval adds seven fields, so the line moves to about 14, and R3 requires the same line number. Fix: compare the header by removing the status-block field lines from both versions; find the hyphen line separately in each; test a three-field `Draft` becoming a ten-field `Approved`.

## Major

1. **The shared `T000` prefix makes the follow-up design collide under R4 (354-356, 394-395, 747).** `spec-conversion.md` is unnumbered too. Fix: a prefix per unnumbered doc, or scope uniqueness.
2. **The transitional clause reaches the constitution and `AGENTS.md` but not the surfaces that apply it (T000-19..22).** The reviewers, the PR template, `verify` and `parallel-agents` would tell agents on the 20 unconverted features to use a document that does not exist. Fix: the same alternative wording in all four, plus a check that greps them.
3. **A single squash or rebase merge bricks CI for every later PR (447-456, Q2).** R3 and the reachability rule scan every `Approved` doc. Fix: skip R3 when the doc is identical to its version at the merge base; require reachability only for docs a PR changes.
4. **Goal 1's "every obligation kept" has no check that fails when broken, and R8 is not implementable as written (160-165, 401-403, T000-9).** The block boundary, the report table grammar and the "Enforced by" extraction are undefined; the hashes sit in a report the same author writes. Fix: define block and table; compute the 3.0.0 column from git; add a test for MUST sentences.
5. **The order of approval against the PR split is not stated (R9, T000-20, PR 1 and PR 2).** R9 needs `Approved` to tick T000-1..7, but the approval machinery arrives in PR 2, and 3.0.0 still mandates the Spec Kit cycle. Fix: a bootstrap paragraph.

## Nit

1. `AGENTS.md:3` also holds the constitution path (T000-14).
2. REWRITE rows are thinner than the text they replace (Compliance review names one of four obligations; the footer omits the ratified date); governance is paragraphs, not bullets.
3. `check-agent-config.py` tests run on non-git temp directories; a path check that exits 2 outside git would break them.
4. `#### Follow-up tasks` is in the middle of the plan, not at its end; "None yet." goes stale.
5. Decision B names no branches; `bugfix/egm-*` branches exist.
6. Acceptance binding is weak: rows -1..-8 are citable as accepted findings without naming any.
7. `concepts_brief.md` location differs between step 1 and the references.
8. `scripts/README.md` lists check scripts as a contract; no task adds the three new ones; `AGENTS.md` line 10 does not point at the allocator.
9. Built files exist in the working tree while the doc says nothing is built; the symlink refusal is not in section 6.
10. The hook does not say which `installed_plugins.json` scope entry it uses.
11. `tests/Cargo.toml` line 3 stays true for unconverted features (T000-23).

Counts: Blocker 1, Major 5, Nit 11
