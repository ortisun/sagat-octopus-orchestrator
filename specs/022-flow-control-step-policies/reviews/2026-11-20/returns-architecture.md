# Architecture review of T022-021, T022-099 and T022-100 (2026-11-20, second set)

Reviewer: `architecture-reviewer` subagent, read-only, given the working-tree diff with the constitution, AGENTS.md, ADRs
0056, 0088, 0089 and 0094, the spec, plan, data model and research of feature 022, the architecture principles and the
architecture script (not the author's conclusions).

## Findings

1. **Blocker (B1): the change departs from ADR-0094 decisions 2 and 3 (accepted 2026-11-19) with nothing recorded**: a
   paused step a pause gave back is not asked, and `persist` accepts it undecided.
2. **Major (M1): the exemption is computed in three places, one in the adapter**: a domain rule in infra-postgres.
3. **Major (M2): a paused gated step ends in different states depending on what happened before the pause** (returned,
   queued with a running deadline, or decided and given back while paused), and nothing records why.
4. **Major (M3): documents must change in the same delivery**: FR-022-021 against FR-022-046; `data-model.md`; the
   tasks; a fitness row for ADR-0089 decision 1.
5. minor: `pause` beside `pause_run` in the domain API (m1); `returned_unleased` does not care why a gate was returned,
   and retry after a hold is untested (m2); invariant 13 evidence partial (m3).
6. nits: `claim_stage` copied in the claim suite; its module comment names design 038 only.
- Holds: layering (no I/O in the domain changes), compatibility (no journal or contract change), test placement
  (`gated_window` belongs in the claim suite, selected on every hot-path change).

## Resolution (author, 2026-11-20)

| Finding | Resolution | Evidence |
|---|---|---|
| B1 | ADR-0096 (proposed) refines ADR-0094 decisions 2 and 3 to serve FR-022-046; D022-048 and O1175 put it to the owner; T022-100 is not ticked until the owner answers | `docs/adr/ADR-0096-*.md`, research.md |
| M1 | Fixed: `RunState::waits_for_resume`, used by the planner, invariant 11 and `persist` (the adapter only maps the kind) | the domain and returns tests |
| M2 | Recorded in D022-048 and ADR-0096 (option 3 rejected: the journal must explain every wait); G022-008 and O1175 (c) for the queued deadline; the retry pass while paused is tested against PostgreSQL | `returns::a_retry_due_while_paused_is_granted_and_given_back_in_one_commit` |
| M3 | Fixed: G022-007 (the approved texts, revised on re-approval); tasks T022-021, T022-099 and T022-100 carry their evidence and T022-025 its note; rows in `docs/engineering/architecture-principles.md` | tasks.md, research.md, architecture-principles.md |
| m1 | Fixed: one `pause_run`, which settles and returns a `Progress`, as `resume_run` does | callers updated |
| m2 | Fixed: the doc comment says any unleased exit; retry after a hold is tested (domain and PostgreSQL) | `readiness_gate::a_review_hold_gives_back_and_a_retry_asks_again_for_the_same_attempt` |
| m3 | Fixed: the bound helper counts holders and unleased rate grants | `returns` tests |
| nits | Kept: `claim_stage` is twelve lines in two files of one suite; the module comment names SC-022-024 in the new file | review only |
