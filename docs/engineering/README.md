# Engineering guide

How this repository is built, by people and by agents. The constitution (`.specify/memory/constitution.md`) says what must stay true; `AGENTS.md` is the short list of rules every agent loads; these pages explain the practice behind them.

| Page | Read it when |
|---|---|
| [SDD workflow](sdd-workflow.md) | you start, change or close a feature: the Spec Kit cycle, its gates, evidence, registers |
| [Architecture principles](architecture-principles.md) | you decide something structural: layering, ADRs, fitness functions, quality attributes |
| [AI engineering](ai-engineering.md) | you configure or direct an agent: instruction files, hooks, skills, subagents, verification, parallel work, security |

Where each kind of guidance lives:

| It is... | It goes in |
|---|---|
| a principle that outranks everything | the constitution (change by pull request with a version bump) |
| a short rule every agent needs and cannot infer from the code | root `AGENTS.md` |
| a rule for one directory | the nested `AGENTS.md` there (and a `CLAUDE.md` containing only `@AGENTS.md`) |
| a procedure used on demand | a skill in `.claude/skills/<name>/SKILL.md` |
| an independent read-only reviewer | an agent in `.claude/agents/` |
| something that must never happen, whatever the agent decides | a hook in `.claude/hooks/` registered in `.claude/settings.json`, or a CI check |
| a structural decision that is hard to reverse | an ADR in `docs/adr/` |
| a feature-level decision, gap or open question | a `D`/`G`/`O` entry in the feature's `research.md` |
| a product, scope or release decision | `docs/owner-decisions.md` |

`python3 scripts/check-agent-config.py` keeps this structure honest (sizes, links, front matter, hooks, ADR format) and runs in CI.
