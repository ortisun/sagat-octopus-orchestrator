---
name: adr
description: Write or supersede an Architecture Decision Record under docs/adr/ with alternatives, consequences and the check that enforces it. Use when a decision is hard to reverse, surprising without context, and the result of a real trade-off, or when a change diverges from an accepted ADR.
when_to_use: "new ADR; record this decision; diverge from ADR-00NN; architecture decision"
---

An ADR is for a decision that is **hard to reverse, surprising without context, and a real trade-off**. Smaller choices become a rule in the nearest `AGENTS.md` or a `D…` entry in the feature's `research.md`.

1. Take the next free number: list `docs/adr/` and grep `origin/*` and sibling worktrees (`git worktree list`) so two branches do not take the same one.
2. Copy `docs/adr/TEMPLATE.md` to `docs/adr/ADR-NNNN-<slug>.md`. Fill every section; "Considered options" needs at least two real alternatives with why they lost.
3. State the rule as MUST / MUST NOT so a reader can check it, and fill **Enforced by** with the test, script or CI step that fails when the rule is broken (or "review only" and why). If you can add that check in the same change, do.
4. Say who decided: the owner, or an agent with the owner's review, and the date. Product and scope decisions are the owner's: record them in `docs/owner-decisions.md` too.
5. Diverging from an accepted ADR: new ADR with benchmark data, failure tests, operational cost and compatibility with runs in flight; mark the old one `Superseded by ADR-NNNN`; update the documents that depend on it (constitution Governance).
6. Add the ADR to the index in `docs/adr/README.md` and link it from the spec/plan that needed it. Run `python3 scripts/check-agent-config.py`.
