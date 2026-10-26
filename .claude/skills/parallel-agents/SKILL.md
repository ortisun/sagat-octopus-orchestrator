---
name: parallel-agents
description: Brief, run and merge implementer agents in sibling git worktrees without duplicating work, filling the disk or trusting unverified reports. Use before launching more than one agent or when merging their branches.
when_to_use: "run agents in parallel; split this feature; worktree; merge the agents' branches"
---

## Before you launch
- **Is it already being built?** `git branch -a | grep <id>`, `git worktree list`, recent commits on sibling branches, running processes (`pgrep -fl 'codex|claude'`). If another session owns it, do not duplicate; coordinate or leave it.
- **Is the work independent?** Agents may share files only through append-only registers (`research.md`), which you will merge. Give each a disjoint set of crates, tests and decision-number range (D/G/O), and say which numbers are theirs.
- **Machine budget:** `df -h`; at most two builds at once; each agent uses its own `CARGO_TARGET_DIR` outside the repo and deletes it (and venvs, `node_modules`) when finished. Stop below ~12 GB free.

## The brief (the agent knows only this)
Goal and scope (task ids), the worktree path and branch, files to read first (AGENTS.md, constitution, the feature's spec/plan/research/tasks), what is already ruled out, red-first evidence required, the exact gates (`/verify`), environment variables, the lessons CI taught us (see `docs/engineering/ai-engineering.md`), forbidden actions (no PR, no edit of `tasks.md`, no `Co-Authored-By`), and the final report format: behaviour changes, per clause the test names and which were red first, suite counts, what is NOT done, new decision numbers.

## Merge protocol
1. Fetch; merge each branch into the integration branch with `--no-ff`; resolve registers by keeping both sides (renumber a clash in the newer one).
2. Compile, then run the gates on the **merged** tree yourself. An agent's green was on its own branch.
3. Diff-read anything an agent said it could not run, and anything untested ("uncompiled commits" happened).
4. Tick `tasks.md` from the evidence, not from the report.
5. Remove merged worktrees and their private targets; leave other sessions' alone.
