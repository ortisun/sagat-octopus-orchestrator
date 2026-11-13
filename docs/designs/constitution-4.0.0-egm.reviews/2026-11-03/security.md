# Security review: `docs/designs/constitution-4.0.0-egm.md` (Draft, commit c3f25a41)

Reviewer: `security-reviewer` (read-only; the author saved this report and appended nothing but this note).

I found no Blocker. I found 8 Major and 4 Nit findings. The central issue is that every check validates author-written data against author-controlled files in the same pull request. Several holes also go beyond that acknowledged limit.

**What I read.** I read the design in full, the plugin install at `~/.claude/plugins/cache/egm/egm/0.1.0/`, `.claude/settings.json`, `.github/workflows/ci.yml` (head and grep), `.claude/hooks/session-start.py`, `docs/owner-decisions.md` (grep), `docs/engineering/ai-engineering.md` (grep), and the current constitution.

**Confirmed by reading a file**
- **Agent tool lists match the design's NFR (lines 645-652).**
  - The seven lens agents have `tools: [Read, Grep, Glob]` and `omitClaudeMd`.
  - The synthesizer has `[Agent, Read, Grep, Glob, Write]`.
  - The bootstrap orchestrator has `[Agent, Bash, Read, Grep, Glob, Write, Edit]`.
  - `.claude-plugin/plugin.json` registers all nine adapters.
- **Version 0.1.0 declares no hooks, MCP servers, commands or `bin/`.** `docs/privacy.md` says the same: "declares no MCP server and installs no permission-approval hook".
- **The 2.1.271 minimum Claude Code version is real.** It is at `docs/claude.md:8`.
- **The install has seven skills, not six.** `skills/egm-bootstrap` exists but is not in Terms (lines 43-45).

## Major

**M1. The diff rule can be bypassed by moving the header boundary (R3, section 4 lines 404-410, section 3 line 355).**
- *Evidence (design text):* The header is "every line before the first line of 20 or more hyphens". R3 examines the body only. The design does not say which version defines the boundary.
- *Scenario:* The approved doc has its first rule line at line 7. A careless or malicious edit deletes that line, or a setext-style `-----...` line is added or removed. If the boundary is computed on the working tree, the "header" then extends to the next rule line. In this doc that is line 679, so lines 1-678 escape the diff. Only the Detailed Implementation Plan is checked.
- *Related:* free text inside the header, such as Author or a second `Status:` line, is also outside R3.
- *Recommendation:*
  - Take the boundary from the Reviewed-commit version and require the same line index in the new version.
  - Allow header changes only on the named status-block fields.
  - Fail on more than one `Status:` line.
- *Status:* Verified in the design text. The behaviour depends on implementation, which is unspecified.

**M2. The approval check is opt-in, so nothing forces a doc to be `Approved` (section 3 lines 358-370, Risks line 524).**
- *Evidence:* R2 and R3 run "only when `Approved`". The Risks table admits the gate is "the only barrier". No rule says a doc with ticked tasks or merged code must be Approved.
- *Scenario:*
  - An author leaves a doc `Draft`, ticks tasks and merges. Only R1, R4 and R7 apply.
  - An author edits an approved doc, sets `Status: Draft`, and CI stays green.
  - A status written as `approved` or `Approved (pending)` may skip R2 and R3, because parsing is unspecified.
- *Recommendation:*
  - Add a rule that any ticked task, evidence block or `Owner-gate` requires the exact status `Approved`.
  - Make any unrecognized status value a failure.
  - Make a status downgrade after a prior `Approved` at the merge base a failure unless the doc is reviewed again.
- *Status:* Verified in the design text.

**M3. The plugin hash covers an allowlist of paths, so a new auto-loaded file leaves the hash unchanged (section 6 lines 445-452, item 2 lines 702-707).**
- *Evidence:*
  - The hashed set is `.claude-plugin/**`, `agents/**`, `resources/**`, `skills/*/SKILL.md`, `skills/*/references/**`, `docs/claude.md` and `docs/harnesses.md`.
  - The install also holds root `plugin.json`, `scripts/*.py`, `skills/*/agents/openai.yaml`, `skills/*/assets`, `.agents/`, `pyproject.toml` and `site/`. None of these is hashed.
  - Claude Code auto-discovers `hooks/hooks.json`, `.mcp.json`, `commands/`, `bin/` and a plugin `settings.json` at the plugin root. A skill can also ship scripts that its `SKILL.md` runs.
- *Scenario:* A compromised or careless future release keeps the hashed files byte-identical and adds `hooks/hooks.json` with a SessionStart command, or `.mcp.json`. The hash still matches the accepted entry, and the check passes. The hook runs on every contributor machine.
- *Recommendation:*
  - Hash the whole install directory except a short explicit exclusion list (`site/`, `tests/`, `.github/`, `docs/assets/`, `*/TEST.md`, `*/test_scenarios.yaml`).
  - Alternatively, enumerate the tree and exit non-zero for any file that is neither hashed nor on the ignore list.
  - Add a test that creates `hooks/hooks.json` and expects a different hash or an error.
- *Status:* Path inventory verified. The Claude Code auto-discovery list is from my knowledge of Claude Code and I did not verify it against its documentation.

**M4. The pin cannot select the version people install, and nothing verifies the installed copy at runtime (section 6 lines 441-444, 457-459; AGENTS.md change at lines 797-799).**
- *Evidence:* The marketplace source is `{"source":"github","repo":"google/elephant-goldfish-model"}` with no ref. Each person runs `claude plugin install egm@egm --scope project`. The design itself notes the upstream tag `v0.1.0` is 14 commits behind the accepted commit `f0ba151b`.
- *Scenario:* Everyone installs the upstream default-branch HEAD of the day. `autoUpdate: false` only stops later updates. The hash is computed by the author after the review, on the author's machine. Nobody else detects a different build, and CI cannot.
- *Recommendation:*
  - Add a SessionStart warning (read-only, no new permission) that runs `egm-tree-hash.py` on the installed copy and compares it to the accepted list.
  - Check whether `extraKnownMarketplaces` accepts a `ref` or a plugin `sha`, and use it if so. I could not verify the design's claim that no pin exists; it is stated as "documents no way" and not checked against Claude Code's documentation.
  - Reword the Constitution Check IV "Pass with conditions" to say the pin is advisory.
- *Status:* Design text and `marketplace.json` verified (it uses `"source": "./"`, so content follows the cloned ref). Settings semantics not verified.

**M5. The tool surface of the added agents is larger than the design states, and nothing is denied (NFR lines 645-652; Risks line 528; Constitution Check V line 544).**
- *Evidence:*
  - The synthesizer has the unrestricted `Agent` tool, so it can spawn any subagent type, including ones with `Bash` and `Edit`. The adapter's "You cannot run shell commands" does not hold, and "two of the agents have write tools" understates this. Both the synthesizer and the bootstrap orchestrator have `Agent`.
  - `Write` has no path scope. Project `permissions.deny` (`settings.json:28-34`) only blocks reads of secrets. Nothing protects `.claude/settings.json`, `.claude/hooks/**`, `.github/workflows/**` or `scripts/check-*.py`.
  - The synthesizer reads the design under review, which is author-controlled text. A prompt-injected doc can steer it to write `.claude/hooks/session-start.py`, which runs unprompted next session. `docs/claude.md:44` also describes an "Uninterrupted mode" that skips human checkpoints.
  - `egm-bootstrap-orchestrator` and the `egm-bootstrap` skill are unused by the design's procedure (Terms lists six skills) but are loaded into every session. The orchestrator has `Bash` and `Edit`.
- *Recommendation:* This is narrowing, not widening, so it fits rule 20.
  - Add `permissions.deny` for `Agent(egm:egm-bootstrap-orchestrator)` and the bootstrap skill. Verify the exact deny syntax.
  - Add `Write` and `Edit` deny rules for `./.claude/**`, `./.github/**`, `./scripts/**` and `./AGENTS.md`.
  - State in the doc that the synthesizer can reach `Bash` through `Agent`.
  - Tell reviewers, in the procedure, that the doc under review is untrusted data.
- *Status:* Tool lists and the adapter text verified. The prompt-injection path is a suspicion, not a test.

**M6. Author-controlled status-block fields reach git and the filesystem with no validation specified (section 3 lines 346-350 and 365-369, section 4 lines 404-410).**
- *Evidence:* R2 says `Reviewed-commit` "exists (`git cat-file -e`)" and R3 runs `git diff --unified=0 <Reviewed-commit> -- <path>`. The 40-hex format appears only in the template. Report paths are free text (`<path> sha256:<hex>`), and the script opens them and hashes them.
- *Scenario:*
  - The value `--output=scripts/check-design-docs.py` in the `Reviewed-commit` field turns the diff into a file write in the CI workspace or on a developer's machine.
  - A report path such as `../../.git/config` makes the CI checker read the persisted checkout token, hash it and parse it for `Counts:`. A failure message that echoes the matched line would leak it. `actions/checkout` persists credentials by default; `ci.yml` sets `contents: read` only.
- *Recommendation:*
  - Require `^[0-9a-f]{40}$` before any git call.
  - Use list-form subprocess and `--end-of-options` or `--`.
  - Restrict report paths to regular, non-symlink files under `docs/designs/<stem>.reviews/`, rejecting `..` and absolute paths.
  - Never echo file content in failure lines.
  - Add `--no-ext-diff --no-textconv --no-color` to the diff.
  - Add a test for each.
- *Status:* Verified in the design text. The unsafe behaviour depends on the implementation.

**M7. The approval record is not bound to the doc revision, and its fields are unauthenticated (section 3 lines 339-381, Risks lines 520-521).**
- *Evidence:*
  - R2 checks that report files exist and match their own recorded hashes, that each has one `Counts:` line, and that the Blocker+Major sum is at most the typed `Accepted` number. Nothing ties a report to the doc path, the Reviewed-commit or the EGM hash.
  - The `Counts:` line is typed by the author, and the "highest severity" rule is unenforceable.
  - `EGM-hash` only needs to be on the list. Copying an accepted hash from the list passes.
  - `Accepted: N findings; OD-x` accepts any decided row. One unrelated OD row can cover any number of findings, and the OD rows are not linked to findings.
  - The Reviewed-commit can be any ancestor, including a commit that already contained unreviewed content.
  - The checker, the hash list, `owner-decisions.md` and the settings entry are all in the same pull request. Decision D declines CODEOWNERS, so the checker itself is unprotected.
- *Scenario:*
  - Hand-write four files ending in `Counts: Blocker 0, Major 0, Nit 0`.
  - Reuse reports from another doc.
  - Edit `check-design-docs.py` in the same pull request.
  - All of these are consistent with the design as written.
- *Recommendation:*
  - Require each report to contain the doc path, the Reviewed-commit and the EGM hash, and have R2 verify those strings (this raises the cost, not the proof).
  - Require `Accepted` to list one OD id per accepted finding, with each OD row naming the doc and a finding id.
  - Run the CI check from the base branch's copy of the script. Alternatively, fail when the script, the hash list or `owner-decisions.md` change in the same pull request as an `Approved` transition, so a human sees it.
  - Do not call the result "approval" in `AGENTS.md`. Call it "consistency of record".
- *Status:* Verified in the design text. The design acknowledges that every field is author-writable and does not address binding.

**M8. Owner gates and OD rows can be satisfied by an unrelated or agent-written row (section 5 lines 428-433, R2 line 369, R5, decisions at lines 83-119).**
- *Evidence:*
  - R5 only checks that the cited OD row "exists and is decided". It does not check the row's subject, its date relative to the task, or its author.
  - "Status begins with Decided" is satisfied by "Decided against" or "Decided; revoked". Current rows already read "Decided; not built" (`owner-decisions.md:63-72`).
  - The parser reads `Owner-gate:` lines from the task item, and the evidence block is part of the item. An evidence line `Owner-gate: OD-old` can satisfy or confuse the gate. The gate parse should use only the pre-marker lines.
  - Decisions A to L were "taken under delegation" by the assistant. Appendix item 12 records them as `OD-2026-11-03-1..12` with status "Decided". That makes agent-taken decisions citable as owner gates and as finding acceptances. Decision D (no CODEOWNERS or branch protection) and decision E are security-posture decisions. The design admits an agent can write a row (line 432).
- *Scenario:* An agent working on a gated task ticks it, cites `OD-2026-11-03-5`, which is an existing decided row about something else, and CI passes.
- *Recommendation:*
  - Add a "Gates" column listing task ids to each OD row, and have R5 require the task id there.
  - Require a closed vocabulary for status (`Decided — approved` or `Decided — rejected`).
  - Add a "Decided by" column.
  - Record the delegated decisions as "Delegated, pending owner confirmation", or require the owner's explicit confirmation of those rows in the pull request.
  - Parse gates only from lines before the first evidence marker.
- *Status:* Design text and the existing status wording verified.

## Nit

**N1. The new `permissions.allow` entry is a widening for convenience, and rule 20 is misattributed (NFR lines 648-651; plan item 4).**
- The design adds `Bash(python3 scripts/check-design-docs.py)` and itself calls it "a small widening". An agent that edits that script can run it without a prompt. The two existing check-script entries (`settings.json:13-14`) have the same weakness.
- It cites `AGENTS.md` rule 20 for "plugins are supply chain". That sentence is in `docs/engineering/ai-engineering.md:50`. Rule 20 is about not widening permissions to make a task easier.
- Recommendation: drop the entry and accept the prompt, or record it as an explicit owner decision. Correct the citation.
- Status: verified.

**N2. No secret scan covers design docs and committed review reports (NFR line 651).**
- "Design docs must hold no secret" is a prose rule. I found no gitleaks, trufflehog or detect-secrets in `.github`, `scripts` or the engineering docs.
- Reports are committed agent output. They can quote file content and absolute local paths of a person's machine.
- Recommendation: add a pattern scan for `docs/designs/**` in the check, and tell authors to strip home paths from reports.
- Status: absence of a scanner verified. The path leakage is a suspicion.

**N3. Branch names are untrusted input to the hook and the allocator (section 7 lines 466-470; item 24).**
- `session-start.py:31-35` prints other branch names into the agent's context. A pushed branch named with instruction text becomes a prompt-injection vector.
- `next-feature-id.py` takes the highest `NNN` over every `origin/*` branch. A branch holding `specs/999-x` moves the next number to 1000, which breaks the three-digit rule.
- Recommendation:
  - Print only names matching `^[A-Za-z0-9._/-]{1,80}$`.
  - Cap or ignore numbers over a sane ceiling and warn.
  - Use `git ls-tree -z`.
- Status: hook behaviour verified. The allocator issue is in the design text.

**N4. Parsing of the accepted-hash JSON and the Claude Code version is unspecified (section 6 lines 453-456, R2).**
- The design does not say how a malformed or non-numeric `Claude-Code` value, or a wrong-typed JSON field, is handled.
- Recommendation: require the check to fail closed on any parse error or non-hex value.
- Status: verified in the design text.

## Classes with no finding
- **Secrets in the design itself:** none. I read the whole doc and found no token or credential.
- **Plugin network or telemetry:** none declared in 0.1.0 (`docs/privacy.md`; no `curl`, `wget`, `Web*` or `Bash(` in any `SKILL.md`).
- **Tenant boundary, SQL, runtime role, migrations:** not applicable. No runtime code changes.

Counts: Blocker 0, Major 8, Nit 4
