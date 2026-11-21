# Goldfish Review Report

**Target:** `docs/designs/constitution-4.0.0-egm.md` (`1..935`, the full file; the header "Detailed Implementation Plan" is at line 681)

**Mode:** REVIEW. **Standard:** `zoom-out-standard.md` (plugin `0.1.0`). **Date:** 2026-11-03.

## Reviewer Status

| Reviewer | Status |
| --- | --- |
| `egm-zoom-out` | completed |

One instance was run. It read the target in full, the baseline files named by the caller, and ran 75 tool calls of repository search. It did not read the earlier review reports or the concepts brief, so the read is independent of the 2026-11-02 round.

## Severity Breakdown

| Blocker | Major | Nit | Total |
| --- | --- | --- | --- |
| 0 | 10 | 4 | 14 |

The reviewer's own closing line said "9 Major, 5 Nit". The findings it listed are 10 Major and 4 Nit; this report uses the listed findings.

## Context Sufficiency (Standard section 0)

Passes. The Overview (lines 9-37) and The Problem (lines 121-152) state what is being built and why. No request for context is needed.

Some findings below touch mechanisms that the Decisions section (lines 65-81) records as chosen by the owner. For those, the finding concerns how the mechanism is built, or a cost the design does not price, and not whether to build it. The finding says so.

## Prioritized Findings (Blockers -> Major -> Nits)

### The R3 diff lock contradicts the document's own reconcile step and repository practice - Severity: Major
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Technical Plan section 4 (lines 384-420); section 2 step 6 (lines 306-309); CUJ 2 (lines 209-214); Rebase convention (lines 412-416); Risks rows at lines 522 and 525; plan item 7 (`fetch-depth`, lines 738-743). The mechanism is owner-fixed (Alternative H, lines 607-611).
* **Discrepancy Description:**
  * Step 6 has the author append unbuilt work as new tasks after `egm-drift-report` (lines 308-309). R3 allows only a checkbox flip or a line inside an evidence block, and section 4 says any other change, even a typo, makes the doc `Draft` (lines 418-420). The reconcile step therefore forces a full re-review (EGM on the changed range plus three repository reviewers). The two sections cannot both be followed cheaply.
  * Repository practice adds tasks during a build. `specs/032-multi-region-failover/tasks.md:20` records "IDs T032-058 to T032-064 were added in the review round of 2026-10-27" (confirmed by the synthesizer). `specs/AGENTS.md:6` requires amending task text together with a decision.
  * The only free-text region R3 tolerates is an evidence block of up to 40 lines (line 394). The design concedes that "a short smuggled sentence inside a block passes" (line 522). An expensive re-review for every amendment gives authors a reason to put amendments inside evidence blocks.
  * The rule needs full-history checkout in CI (`ci.yml:169` is a shallow checkout; the only `fetch-depth: 0` is at `ci.yml:106`, in the job that needs a release tag). It also depends on a merge method that no repository setting enforces (lines 415-416).
  * Alternative H dismisses a sibling task record with the con "splits tasks from the plan" (line 609). That is today's `spec.md`/`plan.md`/`tasks.md` layout, which works.
* **Rationale & Recommendation:** The simplest structure keeps mutable task state outside the approved text. Price this cost to the owner explicitly. If the lock stays, make "append new tasks" and "amend a task with a cited decision" allowed non-body changes, so that step 6 is executable without a re-review.

### Constitution 4.0.0 leaves in-flight features without a normative rule for an unbounded period - Severity: Major
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Section 1 table (lines 272-287); Decisions A and B (lines 88-96); plan items 8, 13 and 17 (lines 747-765, 791-802, 825-831).
* **Discrepancy Description:**
  * The REWRITE rows change "Read before you change", "Same-delivery rule" and "Compliance review" to name only a "design doc". Decision B rejects any transitional clause. The conversion that would bring features under the new text is a follow-up design that is not drafted and waits on a measured first conversion (Decision A).
  * Until then, 35 `specs/*/tasks.md` files (confirmed) and the 20 features with open tasks sit under a constitution and an `AGENTS.md` that tell the agent to read a design doc that does not exist. Only the nested `specs/AGENTS.md` says otherwise, and it is not normative. The reviewer counted about 1,946 task lines (not independently confirmed).
  * Goal 1 says each REWRITE or DROP "says where its obligation went" (line 161). For in-flight features the evidence-and-same-delivery obligation lapses. This is the overlay state (Alternative C, lines 577-581) that the owner rejected, now with no deadline.
  * Decision B's concern is correct: a clause that keeps the `speckit-*` skills alive would break them. A sentence that keeps the obligation without requiring any skill does not.
* **Rationale & Recommendation:** Add one sentence to the rewritten "Same-delivery rule" and "Read before you change": until an unconverted feature is converted, its `spec.md`, `plan.md` and `tasks.md` stay under the same evidence rule, and no skill is required.

### The owner-gate mechanism ignores the repository's existing convention, overclaims in Goal 9, and its authority token is writable by the agent it constrains - Severity: Major
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Section 5 (lines 422-433); Goal 9 (line 178); CUJ 3 (lines 216-222); Decision H (lines 109-110); Alternative J (lines 619-623); plan item 12 (lines 783-789); R5 (line 373); Risks row at line 524. A gate on the task is owner-fixed (Alternative J).
* **Discrepancy Description:**
  * **Existing convention not weighed.** Features already use a first "owner gate" task: `specs/032-multi-region-failover/tasks.md:15` (T032-001 "Owner gate: record the answers ..."), `specs/023-script-languages/tasks.md:17` ("Blocks every later task") and, per the reviewer, `specs/027-internal-apps/tasks.md:9`. Alternative J weighs only "stop and ask" and "a review finding".
  * **Goal 9 overclaims.** It says a task needing a decision "cannot be ticked without it". Section 5 and the Risks table (line 524) say an unmarked task passes, an agent can write the row, and the check cannot tell who wrote it or whether the row relates to the task. R5 verifies only that a string exists.
  * **"Decided" cannot tell owner from delegate.** Plan item 12 records the assistant's delegated decisions A-L under the same "Decided" status as the owner's. `docs/owner-decisions.md:3` says the file holds "Decisions that only the product owner could take". Its tables use a `#` column and no id column (confirmed); old rows cannot be cited (line 431).
  * **Row count does not add up.** Rows `OD-2026-11-03-1` to `-12` (line 788) cannot hold 8 owner decisions (lines 72-81) plus 12 delegated ones (lines 88-119).
  * **Incident choice.** Alternative J cites feature 016 as the failure of "stop and ask". `AGENTS.md` records 016 as duplicated work, which an owner gate would not have prevented. The reviewer names `docs/owner-decisions.md:30` (a claimed approval with no entry) as the better fit; the synthesizer did not confirm that line.
* **Rationale & Recommendation:** Reuse the first-task owner-gate convention plus the template's "Owner decisions needed" section, and let reviewers look for unmarked tasks. If a script check stays, describe it as a dependency check that does not prove authority, and reword Goal 9 to what is verifiable. Record the delegated decisions outside the "Decided" register, or give the status a different token.

### Parity and new capability are bundled, and enforcement is built before the format has been used once - Severity: Major
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Overview items 2-5 (lines 25-28); The Problem (lines 121-152); Landing order (lines 483-494); Verification Plan "Journeys" (lines 929-930); Decision A (lines 88-91).
* **Discrepancy Description:**
  * Only Overview item 1 (constitution move) and item 6 (removal) replace Spec Kit. Items 2-5 (approval record, owner gate, plugin pin, allocator) are new enforcement.
  * A search of `scripts/*.py`, `scripts/*.sh`, `.github/workflows/*.yml` and `.claude/hooks/*.py` for `Constitution Check|Complexity Tracking|plan.md|spec.md` found nothing. Spec Kit's layout supplied conventions, not checks. The design turns those conventions into CI rules.
  * PR 1 ships the status-block grammar, the task regex, the evidence markers and the hash set before any doc has been ticked in the new format. The conversion pilot comes after PR 2 (lines 88-91), so its measurements cannot change tooling that has already shipped. The journeys run on a scratch branch (line 929), not on a real doc.
  * PR 1 is called "additive, no normative text" (line 485), but it enables a third-party plugin project-wide and changes CI checkout depth.
* **Rationale & Recommendation:** Separate the replacement (PR 2 core, README and TEMPLATE, document-content checks) from the attestation machinery (the R2, R3, pin and `Counts:` findings in this report). Use this design as the pilot document, and add the attestation checks after one real doc has been through the process.

### `check-design-docs.py` is not weighed against extending `check-agent-config.py`, and Complexity Tracking compares each addition only with extremes - Severity: Major
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Complexity Tracking (lines 550-559); Alternatives L, I, N and J (lines 625-641, 613-623); R1, R4, R6 and R8 (lines 363-379); scanned set (lines 358-360).
* **Discrepancy Description:**
  * The rejected alternatives are "prose only", "version only", "manual look" and "stop and ask". None is a middle option.
  * `scripts/check-agent-config.py` already hosts the ADR duplicate-number check (lines 169-185, confirmed), the reviewer tool allowlist (`WRITE_TOOLS`, line 27), settings parsing (lines 147-166) and a relative-link check (lines 188-198), and CI runs it at `ci.yml:176`. Its test helper builds a temporary tree (`scripts/tests/test_check_agent_config.py`).
  * R1 (sections exist), R4 (ids unique) and R6 (numbers unique) need only the filesystem. R6 mirrors the ADR check at `check-agent-config.py:169-185`. All three fit there as small rules.
  * The existing checks are 54 to 215 lines each and use no git plumbing (reviewer count). An eight-rule, git-aware checker with a diff parser would be the largest script in `scripts/`.
  * Cohesion is poor: a script named for design docs also lints the constitution (R8), the owner register and the hash JSON. R8's "each Enforced-by path exists" (line 379) duplicates the path-existence idea of the backticked-path check (plan item 25) in a second script.
  * R8 guards a version-line/sync-report match that prose has handled through several amendments. The sync-report names are not uniform (`docs/governance/compensation-safety-sync-impact.md` is 2.0.0; `constitution-3.0.0-sync-impact.md` is 3.0.0), so a `constitution-*` glob (line 379) is fragile.
* **Rationale & Recommendation:** Put the filesystem-only rules in `check-agent-config.py`. Keep a separate script only for the rules that need git history, and justify those individually. Drop R8, or leave it to the constitution's own review.

### The approval-record check (R2, the `Counts:` line, report hashes) verifies author-typed values - Severity: Major
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Section 3 "Counts line", "Record" and "The check" (lines 328-382); Goals 2 and 7 (lines 163-165, 176-177); Risks rows at lines 520-521; Decision G (lines 107-108); plan items 1-3 and 19 (lines 692-713, 843-852). The status block is owner-fixed.
* **Discrepancy Description:**
  * The design concedes that "every field of the record can be written by the author" (line 520), that "the check sums what the author wrote" (line 521) and that it "does not prove that a review ran" (line 381).
  * The repository reviewers cannot write files (`check-agent-config.py:27`), so the author pastes their reports and appends their `Counts:` line. The report hash is then a hash of the author's own paste. `Claude-Code`, `EGM-hash`, `Open findings` and `Accepted` are likewise self-reported.
  * The only real control is the owner's review of the pull request, which the design names. R2 is a typo detector, and it costs git plumbing, a per-report line contract, a severity mapping in three reviewer definitions (item 19) and the hash list.
  * Reports are already committed under `docs/designs/NNN-slug.reviews/` and are tamper-evident through git history.
* **Rationale & Recommendation:** Keep the status block as a human-readable record with links to the committed reports. Defer machine verification of the attestation fields until reviews can run headless, which the design already lists in Future Improvements (line 677).

### The EGM pin is a hand-rolled hash over a hand-picked file set - Severity: Major
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Section 6 (lines 435-462); plan items 2 and 3 (lines 702-713); Alternative I (lines 613-617); Goal 7 (lines 176-177); CUJ 4 (lines 224-230); the `EGM-hash` clause of R2; Risks row at line 528. A settings entry plus accepted hashes is owner-fixed.
* **Discrepancy Description:**
  * **Upstream already defines the runtime file set.** The plugin's `scripts/pack_plugin.py:12-24` (confirmed) lists `SHARED` and `HOST["claude"]` globs: `plugin.json`, `README.md`, `skills/*/agents/openai.yaml`, `docs/index.md`, `docs/privacy.md`, `docs/releases.md` and others, and excludes the plugin's `tests/` and `site/`. The design's set (lines 445-448) takes `.claude-plugin/**` and `agents/**` wholesale and only two `docs/` files. Any layout change upstream makes the two definitions diverge.
  * **The central negative claim has no citation.** "Claude Code documents no way to require a version or a commit there" (lines 441-444) justifies the script, the JSON, the R2 clause and a constitution clause. It needs re-verification against current marketplace source-field documentation.
  * **What the hash proves.** The author's own run produces the string; CI has no plugin and compares strings only (lines 458-459); the hash cannot reproduce or enforce a version (line 526). The design already identifies the installed copy by commit `f0ba151b` (line 437). A commit id in the accepted list gives a person the same upstream diff to read without a tree-hash script. The only extra the tree hash adds is detecting local tampering of the cache copy, a threat the design never names.
  * **Project-scope enabling.** The settings entry loads nine agent types, two of them write-capable, into every contributor's session (lines 645-649). The same repository's config check forbids write tools on its own reviewers (`check-agent-config.py:27`), and `AGENTS.md` rule 20 says not to widen permissions or hooks.
* **Rationale & Recommendation:** Pin by commit id in the accepted list, with no script. If a content hash stays, reuse the upstream runtime globs from `pack_plugin.py`. Make user-scope enabling the default; the design already offers it as a fallback (Risks row at line 528, Alternative I at line 615).

### The allocator misses the stated clash case, and its stale-fetch warning uses a path that is wrong in a worktree - Severity: Major
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Section 7 (lines 464-470); plan item 5 (lines 722-726); Decision I (lines 111-112); Risks row at line 529. That the allocator ships is owner-fixed.
* **Discrepancy Description:**
  * **Wrong refs.** The root problem, per `AGENTS.md` ("Start here") and `specs/AGENTS.md`, is sibling worktrees and unpushed branches. The allocator scans the working tree and `origin/*` only. Branches checked out in sibling worktrees live in the shared local `refs/heads`, so they would be visible at no cost if `refs/heads/*` were included. The design instead records them as an unavoidable residual risk ("Numbers taken on branches never pushed are invisible", line 529).
  * **Wrong path.** The warning reads the literal path `.git/FETCH_HEAD` (line 468). In a git worktree `.git` is a file, and `AGENTS.md` names sibling worktrees as the working model.
  * **Reuse.** The session-start hook already enumerates branches with `git branch -a --format=%(refname:short)` (`.claude/hooks/session-start.py:31`, confirmed). A short helper over `git for-each-ref` suffices. A warning feature and R6 on top is more than the allocator needs.
  * **Scope of the root problem.** `AGENTS.md`, `specs/AGENTS.md` and `docs/engineering/sdd-workflow.md` list `D`, `G`, `O`, ADR and migration-timestamp clashes as the same class. Decision I defers those, so the allocator covers the least frequent clash.
* **Rationale & Recommendation:** Scan `refs/heads` and `refs/remotes`. Drop the `FETCH_HEAD` warning, or resolve its path with `git rev-parse --git-path`. Keep R6 as the backstop.

### Permanent pointer machinery is built for a one-time migration, and its allowlist grandfathers its own failures - Severity: Major
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Section 8 (lines 472-481); Goal 4 (lines 167-170); plan item 25 (lines 888-899); plan item 24 (lines 880-886).
* **Discrepancy Description:**
  * **One-time outcome.** "No dangling pointer after removal" is verifiable once, in PR 2. The synthesizer recounted the case-insensitive search over the live surfaces named in Goal 4 (`AGENTS.md`, `CLAUDE.md`, `README.md`, `.claude/`, `.github/`, `docs/engineering/`): 100 matching lines in 16 files, of which 10 files are the skills being deleted. The residue after deletion is six files (`README.md`, `AGENTS.md`, `CLAUDE.md` and three under `docs/engineering/`), all already listed in the plan. The reviewer reported 203 matches in 22 files, probably over a wider scope.
  * **Existing pattern is filesystem-only.** The link check (`check-agent-config.py:188-198`) uses `(doc.parent / target).exists()` and its tests use a plain temp tree. The new check needs `git ls-files` (line 480), which changes the script's character.
  * **A cheap extension already covers the known pointer.** `README.md:721` links to `.specify/memory/constitution.md`. Adding `README.md`, `SPECIFICATION-INDEX.md` and `specs/README.md` to the `docs` list at `check-agent-config.py:188` would catch it.
  * **The allowlist hides failures.** Its first content is "the tokens that still fail after the pointer fixes" (lines 894-896), produced by running the check. That grandfathers current failures and is open-ended and unsized.
  * **The word ban is self-referential.** The permanent `speckit`/`Spec Kit` search needs a self-exclusion list (lines 897-898) and fails any future migration-history sentence on a live surface.
* **Rationale & Recommendation:** Extend the link-check file list. Run the `speckit` search once as PR 2's verification step instead of as a permanent CI rule. If backticked paths are kept, scope them to the instruction files and drop the allowlist.

### Altitude drop: regexes, grammars and token rules sit in the architecture sections and are repeated - Severity: Major
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Section 4 (lines 386-410); section 3 (lines 328-331, 339-356); plan items 1, 6, 19 and 25 (lines 692-700, 728-736, 843-852, 891-896).
* **Discrepancy Description:**
  * File lists and edit locations are appropriate in the Detailed Implementation Plan. These are not: the task-item regex (line 386), the four evidence markers, the 40-line cap and the forbidden line starts `#`, `|` and triple backtick (lines 389-395), the "20 or more hyphens" header boundary (lines 355, 405), and the path-token exclusion list (lines 891-894).
  * The grammar is stated in section 4, again in plan item 1 ("exactly as written there") and again in item 6 (the README "evidence-block grammar of section 4"). Three sources of truth will drift.
  * The heuristics buy little. The 40-line cap and the forbidden line starts exist to limit smuggling, and the design admits the cap does not stop it (line 522).
  * The reviewer found no existing parser to reuse: a search of `scripts/`, `tests/*.rs`, `tests/*.py` and workflows for `tasks.md` or checkbox regexes found only the PR template and four doc comments. This is net-new bespoke format code.
* **Rationale & Recommendation:** State the contract in the architecture ("a ticked task carries a dated evidence line; the body is otherwise unchanged since review"). Put the exact grammar in one place, `docs/designs/README.md` plus the script tests. Drop the 40-line cap and the forbidden-start rules.

### "Process change is MAJOR" ties constitution MAJOR bumps to a pre-1.0 third-party plugin - Severity: Nit
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Decision C (lines 97-99); section 2 closing paragraph (lines 311-313); plan item 8, "Versioning" (line 760).
* **Discrepancy Description:** The constitution names EGM skills and a script path, and the design says an upstream step change forces an amendment (lines 311-313). Decision C makes any change to the mandated process MAJOR. The upstream tag already lags the installed commit by 14 commits (lines 437-439). Decision C is delegated, not owner-fixed, and the 3.0.0 constitution already delegates Gitflow and commit procedure to `AGENTS.md` as the single source.
* **Rationale & Recommendation:** Define "change of the mandated process" so that renames and clarifications are PATCH or MINOR, and only a change in what must be reviewed or recorded is MAJOR.

### Review-round history and delegation rationale sit in the design body - Severity: Nit
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Overview lines 35-37; Decisions A-L (lines 83-119); Non-Functional Requirements lines 658-659.
* **Discrepancy Description:** A fresh reader meets "the first review round found 8 Blockers and 31 Major findings" (line 90) and "104 raw findings" (lines 658-659), which are outside the document. Decisions J, K and L restate non-decisions (contests accepted, confirmations, an out-of-scope item). This is history, not design.
* **Rationale & Recommendation:** Keep each decision and a one-line reason. Move the review history to the `OD-` rows or leave it in git.

### The reviewer severity map does not match the reviewer files - Severity: Nit
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Plan item 19 (lines 843-852).
* **Discrepancy Description:** `.claude/agents/correctness-reviewer.md:20` reports a confidence of "confirmed in code / plausible", not "certain" or "likely". `architecture-reviewer.md:19` has no severity scale ("Say which findings are blocking"). Only `security-reviewer.md:19` uses high/medium/low (all three confirmed). The map therefore requires new output contracts for two agents, not a mapping.
* **Rationale & Recommendation:** State the severity scale in each of the three reviewer definitions and drop the confidence mapping.

### Placement and file-list slips - Severity: Nit
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Plan item 3 (lines 709-713); plan item 26 (line 901) and Technical Plan section 8 (line 478); the Detailed Plan file list.
* **Discrepancy Description:**
  * `docs/engineering/egm-accepted-hashes.json` is machine-read by CI, but `docs/engineering/` holds the prose guides. It fits better next to `.claude/settings.json`.
  * Item 26 and section 8 say "Delete, 21 tracked files", but 19 are deleted and 2 are moved by item 8 and item 10 (confirmed: 21 files under `.specify/`, the list at lines 904-910 names 19, and lines 908-909 name the two already moved).
  * `scripts/README.md` (lines 9-12, 16, per the reviewer; not re-read) inventories the repository's check scripts. The three new scripts are not in the file list, and no check enforces the inventory.
* **Rationale & Recommendation:** Fix the three; none changes the design.

## Reuse Search Summary (from the reviewer)

| New item | Search run | Result |
| --- | --- | --- |
| Status block | `check-agent-config.py:181` | ADRs already require a Status line near the top. Precedent for a header convention; no reusable parser. |
| Task and evidence parsing | `scripts/`, `tests/*.rs`, `tests/*.py` and workflows for `tasks.md` and checkbox regexes | None. Net-new. |
| `Counts:` line | `.claude/agents/*.md` | Free-text reports. No shared format. |
| OD ids | `docs/owner-decisions.md` | Per-table `#` column, no ids. |
| Owner gate | `specs/*/tasks.md` | Existing gate-task convention (see the owner-gate finding). |
| Feature-number uniqueness | `check-agent-config.py:169-185` | Equivalent ADR check exists. |
| Path check | `check-agent-config.py:188-198` | Link check exists (see the pointer finding). |
| Branch enumeration | `.claude/hooks/session-start.py:31` | Exists (see the allocator finding). |
| Plugin file set | plugin `scripts/pack_plugin.py:12-24` | Exists (see the pin finding). |

## Checked and Found Sound

* **Repository claims that check out:** `ci.yml` job `architecture` at lines 165-176, `fetch-depth: 0` at line 106, `unittest discover` at line 171, the contract lint at lines 320-341 covering four files; `.specify/` holds 21 tracked files; the `AGENTS.md`, `CLAUDE.md`, `README.md`, PR-template and `tests/Cargo.toml` line references; the `check-agent-config.py` scope and its reviewer write-tool ban; the hook's `.specify/feature.json` read; the `.claude/settings.json` allow-list style; 32 `plan.md` files carry a Constitution Check.
* **Scope kept intact:** all eight principles are KEEP; nothing that CI or tests read moves (Goal 5); `check-architecture.py` is untouched; the hook change is minimal and forced by the `.specify/` deletion; deferring the 20-feature conversion is the right call; rollback as a forward amendment matches the 3.0.0 practice.
* **Honest self-reporting:** the Risks table states the residual limits of the record, the gate and the pin.
* **Tests:** the test-first, temporary-git-repository approach follows `scripts/tests/test_release_upgrade_profile.py:69-72`.

## Synthesizer Verification Notes

* **Confirmed by direct read or search:** the ADR duplicate-number check (`check-agent-config.py:169-185`); the link check (lines 188-198); `WRITE_TOOLS` (line 27); the hook's `git branch -a` (`session-start.py:31`) and `.specify/feature.json` read (line 22); the gate-task precedents in specs 032 and 023 and the task-addition note at `specs/032-multi-region-failover/tasks.md:20`; the plugin's `pack_plugin.py:12-24`; the three reviewer report formats; the `ci.yml` line numbers; the 21 files under `.specify/`; 35 `specs/*/tasks.md` files; the `#` column in `docs/owner-decisions.md`.
* **Not independently confirmed:** the 1,946 task-line count; `specs/027-internal-apps/tasks.md:9`; `docs/owner-decisions.md:30`; `ci.yml:169` being the shallow checkout line; the `scripts/README.md` line numbers; the reviewer's script-size range (54 to 215 lines); the 203-match figure (a recount over the Goal 4 surfaces gave 100 lines in 16 files).
* **Normalizations made:** Overview items 2-5 are lines 25-28 (the reviewer gave 23-28); the user-scope offer is at line 528 and line 615, not line 651; plan items were separated from section numbers; "21 tracked files" is in both line 478 and line 901.
* The harness flagged the reviewer's output for the string `settings.json`. It appears only as a file path in findings, and the output was treated as data.

Counts: Blocker 0, Major 10, Nit 4
