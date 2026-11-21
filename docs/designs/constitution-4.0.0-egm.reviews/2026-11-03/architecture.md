# Architecture review: `docs/designs/constitution-4.0.0-egm.md` (Draft, c3f25a41)

Reviewer: `architecture-reviewer` (read-only; the author saved this report).

I read the design (all 935 lines), the 3.0.0 constitution, `AGENTS.md`, `CLAUDE.md`, `scripts/check-agent-config.py`, `.claude/settings.json`, the session-start hook, the three reviewer definitions, `.github/workflows/ci.yml`, `docs/owner-decisions.md`, `docs/governance/constitution-3.0.0-sync-impact.md`, `specs/AGENTS.md` and the installed EGM plugin (0.1.0). I also read the 016 quickstart and its test, and the git refs of the main checkout.

I did not run any command. I cannot run `gh` or `git` here, so I read `.git` refs by file. "Verified" means I read the file. "Not verified" means I could not.

## Constitution Check rows

The rows for I, II, III and VII (n/a, Pass, n/a, n/a) are true.
- II: `scripts/check-architecture.py` has no reference to `.specify`, `specs/` or the constitution. Verified.
- The only `tests/Cargo.toml` edit is a `description` string on line 3. Verified.

The rows for IV, V, VI and Delivery Constraints are overstated; see M8.

## Blocker

**B1. The removal leaves every in-flight feature without a working rule, and the 4.0.0 text contradicts itself for them.** (Verified.)
- Where:
  - Decision B ("no transitional clause").
  - Section 1 rows "Read before you change", "Same-delivery rule" and "Compliance review".
  - Item 13 (`AGENTS.md` rule 1).
  - Section 9 carrier table.
- Evidence:
  - Rule 1 and the constitution would require reading "the design doc `docs/designs/NNN-slug.md` of the active feature".
  - The design converts none of the 20 features with open tasks. 32 `plan.md` files carry a Check (verified by count). The 014, 018, 035, 036 and 037 branches are live (see M9).
  - After PR 2, no design doc exists for any of them.
  - The Same-delivery rule becomes "a ticked task in the design doc", which excludes `tasks.md`.
  - Compliance review becomes "every design doc carries a Constitution Check". The table calls this "the obligation is the same". It is not: the obligation on 32 `plan.md` files, and on every PR reviewed against them, is dropped.
  - The carrier table credits "Tasks and evidence blocks in the doc | R4, R7". Those rules scan `docs/designs/*.md` only. The 20 unconverted features have no evidence check at all once the `/speckit-*` skills are gone.
  - The only carve-out ("ticked by hand until converted") sits in `specs/AGENTS.md` and `SPECIFICATION-INDEX.md`. It is not in the constitution or in `AGENTS.md`.
  - This breaks Goal 1 ("keeps every obligation") for the features currently being built, including 018, the only approved lane.
- Recommendation: choose one.
  - (a) Add a transitional clause to 4.0.0 and to `AGENTS.md` rules 1 and 10. It would say that until a feature is converted, its `spec.md`, `plan.md` and `tasks.md` stand in for the design doc, with the same Check, evidence rule and tick rule. Then decision B's "no transitional clause" goes. The speckit skills can still be deleted, because the clause does not need them.
  - (b) Move the conversion design before PR 2.
  - In either case, state in the carrier table that R4 to R7 do not cover unconverted features.

## Major

**M1. "Approved dependency" is weakened in the rewrite.** (Verified.)
- Where: Section 1, row "Read before you change", and item 13, rule 2.
- Evidence:
  - The 3.0.0 bullet has two obligations: read the constitution and feature docs, and "MUST NOT implement what belongs to another feature without an approved dependency".
  - The disposition row says only "names the constitution and the design doc".
  - `AGENTS.md` rule 2 becomes "a dependency recorded in the design doc". Under the approval rule, a doc is `Approved` by reviewers alone, so "recorded" no longer means owner-approved.
- Recommendation: keep the sentence in the constitution. Say who approves a cross-feature dependency (the owner, as an `OD-` row or an `Owner-gate`). Name it explicitly in the row.

**M2. Goal 9 and CUJ 3 overclaim what the owner gate does.** (Verified.)
- Where: Goal 9, CUJ 3, section 5, Risks rows 4 and 5.
- Evidence:
  - Goal 9 says a task that needs an owner decision "cannot be ticked without it".
  - Section 5 says an agent can write the `OD-` row and the gate is optional. "A missing gate on a task that needed one passes the check."
  - Decision D removes CODEOWNERS, so "the owner's review of the pull request" is a convention.
  - Section 3 also says merging is "not an approval act", yet the Risks table relies on the owner reviewing the PR.
- Recommendation: restate Goal 9 as "a task that carries a gate cannot be ticked while the cited row is absent". Do not offer the gate as a control against agents. Either name PR review by the owner as a required control, or drop it from the mitigations.

**M3. Delegated decisions A to L are recorded as owner decisions that gate and accept.** (Verified in the doc and `docs/owner-decisions.md`.)
- Where: Decisions under delegation, item 12 (`OD-2026-11-03-1` to `-12`, status "Decided"), decision J.
- Evidence:
  - A to L were taken by the assistant. They include scope (A), release timing (B) and the acceptance of reviewer contests (J).
  - `AGENTS.md` line 12 reserves product, scope and release for the owner.
  - Existing rows use "Owner approved" and "Decided; not built". Rows that begin "Decided" are what R2 and R5 accept as owner acceptance.
  - So the author and the accepter are the same agent, and the rows can be cited as owner-accepted findings.
- Recommendation: record A to L as "Decided under delegation; owner confirmation pending". Make the check accept only rows that say "Owner decided". Do not let J count as owner acceptance until the owner confirms.

**M4. The approval record is not bound to the document it approves.** (Verified from the section 3 text.)
- Where: Section 3, R2 and the status block.
- Evidence:
  - R2 requires `sum(Blocker + Major over all reports) <= Accepted`.
  - `Accepted: <n>` is a free number, unrelated to the count of `OD-` ids or to which findings they cover.
  - No report is required to name the reviewed commit or the doc hash. An old report with 0/0 counts, or a report of a different doc, satisfies the record.
  - The repository reviewers' reports are written by the author, since they have no `Write` tool.
- Recommendation:
  - Require the first line of each report to carry `Reviewed-commit: <sha>`, and let R2 compare it with the status block.
  - Require each `OD-` row used for acceptance to name the doc and the finding ids, and let R2 check that the number of ids covers `Accepted`.

**M5. Step 6 "Reconcile" contradicts R3.** (Verified.)
- Where: Section 2 step 6, section 4 (R3, "Amending an approved doc").
- Evidence:
  - Step 6 has the author append unbuilt work as new tasks by hand.
  - R3 allows only checkbox and evidence-block changes. A new task line fails CI until the doc goes back to Draft and four reviewers run again, at 15 to 33 reviewer runs per pass plus three repository reviewers.
  - A typo also forces `Draft`.
  - This pushes authors toward leaving drift unrecorded, or toward rewriting the body inside a fake evidence block.
- Recommendation: define one allowed post-approval operation for appended follow-up tasks. For example, tasks under a `## Follow-up tasks` section that R3 permits. These are subject to R4 and to a rule that they carry no new requirement text. Or state that every appended task needs a ranged re-review, and cost it in the NFR section.

**M6. The pin is inoperable on a fresh machine, and the project settings widen what every session loads.** (Verified in the plugin; the claim about Claude Code settings is not verified.)
- Where: Section 6, items 3 and 4, NFR "Security and supply chain".
- Evidence:
  - `claude plugin install egm@egm --scope project` installs the marketplace head. The accepted hash corresponds to commit `f0ba151b`.
  - A new contributor cannot reproduce the accepted tree, so every review they run fails R2 as soon as upstream moves.
  - CI only compares a string the author typed. Nothing on a developer machine computes the hash before a review runs.
  - The committed settings enable a plugin whose bootstrap orchestrator has `Bash`, `Write` and `Edit` (verified in `.claude-plugin/agents/bootstrap_orchestrator.md`). They also add a `permissions.allow` entry.
  - `AGENTS.md` rule 20 says not to widen permissions, hooks or MCP servers, and treats plugins as supply chain.
  - The owner's acceptance of this is not recorded as an `OD-` row. It appears only in Alternative I.
- Recommendation:
  - Have the SessionStart hook run `egm-tree-hash.py` against the installed directory and warn when the hash is not on the list. This is about ten lines and makes the pin detectable before a review.
  - Say how to install the accepted commit. A fork or `sha` source is the only reproducible way. Alternative I mentions it; if it is rejected, say so.
  - Add an `OD-` row that records the owner's acceptance of the rule-20 widening.
  - I did not verify whether Claude Code's `extraKnownMarketplaces` accepts `ref` or `sha`. Check that before relying on the "no way to require a commit" claim.

**M7. A single squash or rebase merge breaks CI for every approved doc.** (Verified from the text. Decision D says no repository setting enforces the merge method.)
- Where: Section 4 "Rebase convention", decisions D and F, Risks row 8.
- Evidence:
  - R3 fails if `Reviewed-commit` is not an ancestor of `HEAD`, on pushes as well as pull requests.
  - One squash into `develop` makes the reviewed commit unreachable for that doc, and CI on `develop` then fails permanently.
  - The only recovery is a fresh review run, at 15 to 33 reviewer runs plus the repository reviewers.
  - The mitigation is a convention, and the design says so.
- Recommendation: either restrict the merge method in the repository (a settings change the owner must do, and decision D declined to depend on this), or make R3 fall back safely. For example, accept a `Reviewed-blob` sha256 of the body at review time in addition to the commit. This reverses decision F, so say so explicitly.

**M8. Constitution Check rows are overstated.** (Verified.)
- Where: Constitution Check, rows IV, V, VI and Delivery Constraints; Complexity Tracking.
- Evidence:
  - IV: "Toolchains and dependencies are pinned". The design admits the plugin cannot be pinned, so "Pass with conditions" is a partial violation.
  - V: nine agent types, two with write tools, loaded into every session.
  - Governance says an unavoidable violation MUST be justified in Complexity Tracking with the rejected simpler alternative. The Complexity Tracking table lists only additions, not these two conditions.
  - VI: the evidence ("approvals recorded in the repository") does not touch the principle (OTel, typed errors, runtime audit). The honest answer is n/a.
  - Delivery Constraints: "Pass" omits the Python-script and CI-step additions (acceptable) and the new third-party tooling dependency (not stated).
- Recommendation: mark IV and V as justified exceptions with Complexity Tracking rows. Mark VI n/a.

**M9. The premise of decision B is not established.** (Partly verified: refs read from `.git`.)
- Where: Decision B, Risks row 10.
- Evidence:
  - The doc says no sibling worktree or open PR exists. `.git/worktrees` lists only this one worktree.
  - `origin` holds `feature/014-capacity-protocol`, `014-capacity-acquire`, `014-capacity-coordinator`, `014-capacity-sweeper`, `018-schedules-bulk-operations`, `035-iceberg-write-steps`, `bugfix/integrate-035-iceberg-write-steps`, `036-elastic-scaling`, `037-compensation-safety` and three `bugfix/egm-*` branches.
  - I cannot see whether PRs are open for them. The project memory says other sessions own the 014 lane and 018.
  - Local `develop` (6b6f0fa4) is ahead of the doc's base (df59eb0e), so the quoted line numbers need re-checking.
- Recommendation: rerun `git branch -a`, `git worktree list` and `gh pr list` before PR 2. Replace "none exists" with a dated list. Check the `bugfix/egm-*` branches for overlap with this work and name them in the doc.

## Nit

1. **Disposition table units** (verified against the 3.0.0 text). Goal 1 defines a unit as "one heading or one bullet". The table has no rows for:
   - the H1 title;
   - the four H2 headings (Core Principles, Delivery Constraints, Development Workflow and Quality Gates, Governance);
   - the four Delivery Constraints bullets (Technology baseline, Out of scope for V1, Targets are not promises, Security-by-default), collapsed into one row;
   - the two sub-bullets of Principle V;
   - the footer's Ratified date and "Adopted on 2026-11-01" sentence, plus the 2.0.0 report link.

   Every other unit has a row, so no obligation is lost. Either redefine "unit" to match the table or add rows.
2. **Row classification.** "CI gates" is marked KEEP but gains a gate (an ADD). The Footer REWRITE is outside the process parts, which Goal 1 limits REWRITE to.
3. **R8** uses "the newest `docs/governance/constitution-*-sync-impact.md`". Newest by what? A file mtime is meaningless in git. Use semantic-version order from the file name. `compensation-safety-sync-impact.md` correctly does not match the glob.
4. **Reviewer vocabulary.**
   - `correctness-reviewer` reports confidence as "confirmed in code / plausible". The map in item 19 says "certain", "likely" and "plausible".
   - `security-reviewer` uses high/medium/low. This is fine with the map.
   - Align the vocabulary to the existing words.
5. **"Partly done" marker.** The doc says such a task stays unticked, but R7 treats a `Partly done` marker as an evidence block, so a ticked task passes. This matches `specs/AGENTS.md` line 7 only by convention. Reject a `Partly done` marker on a ticked task.
6. **Header edits go unchecked.** R3 examines the body only. Lines added to the header, other than the required fields, are not diffed.
7. **Session-start hook.** `git branch -a` returns `origin/feature/NNN-…` for a branch that is also local, and `b != branch` compares different strings. The "Other branches mention" warning therefore fires on every pushed numbered branch. Add a test case and strip `origin/`.
8. **The design's own plan has no task ids.**
   - Section 4 says tasks are in the Detailed Implementation Plan.
   - This doc is exempt from ids as an unnumbered doc, so its 26 items cannot be ticked with evidence or cited in `Refs:` footers.
   - Give the plan checkbox tasks, or say that PR 1 and PR 2 are tracked elsewhere.
9. **No ADR.**
   - `AGENTS.md` rule 21 asks for an ADR for a hard-to-reverse decision.
   - The design takes a real trade-off (an unpinned third-party plugin at 0.1.0 replacing the toolchain) and records it only inside the doc.
   - Add a short ADR or state why the Sync Impact Report suffices.
10. **Rule 21 gaps.**
    - Append-only `D`/`G`/`O` registers: a check would be small. Compare the ids in `research.md` against the merge base and require that none are removed or renumbered. The design defers it.
    - The unenforced release-blocking checklist items (34 open in `specs/036-elastic-scaling/checklists/security.md`, verified by count) are the same case.
    - Neither is a regression from today.
11. **PR template.** Item 21 changes lines 3, 19 and 25 only. Line 12 still lists only `check-architecture.py` and `check-agent-config.py`, so add `check-design-docs.py` there.

## Verified claims

Verified:
- 32 `plan.md` files carry a Constitution Check.
- 21 tracked files under `.specify/`. The delete list adds up to 19 plus the 2 moved files.
- The edit locations in `AGENTS.md` (lines 3, 7, 9, 11, 16, 43), `CLAUDE.md` (lines 7 and 10), `specs/AGENTS.md` (lines 3, 5, 9), `specs/README.md` line 5, the PR template (lines 3, 19, 25), `SPECIFICATION-INDEX.md` line 71, the quickstart commands at 014 line 23 and 016 line 46, and `tests/Cargo.toml` line 3.
- The 016 quickstart test ignores non-`sagat`, non-`curl` commands, so replacing the `.specify` call with `ls` will not break it.
- `check-agent-config.py` does what the Problem section says. That includes the reviewer tool allowlist forbidding `Write`, `Edit` and `Bash`.
- The plugin has `egm@egm`, minimum Claude Code 2.1.271, seven read-only lens agents and two write-capable agents (the synthesizer and the bootstrap orchestrator).
- `egm-review-loop` ends at two consecutive clean rounds with a cap of five. The 1-audit convergence cap exists in `egm-review`. The 15 and 33 cost figures follow.

Not verified:
- Whether Claude Code accepts a `ref` or `sha` for marketplace sources.
- Whether any pull request is open.
- Feature 016 was built twice (the memory file and `AGENTS.md` assert it, and I did not check git history).
- The upstream tag `v0.1.0` and the 14-commit gap.
- The 20-feature count.
- The README line numbers beyond 677, 681 and 721-722.

Evidence manifests under `specs/037-compensation-safety/evidence/` and `specs/014-workflow-primitives/evidence/` record `.specify/memory/constitution.md` paths and hashes. Nothing live reads them, and the design leaves them unchanged. Fine.

Counts: Blocker 1, Major 9, Nit 11
