# Architecture review of T022-096 and T022-076 (2026-11-19)

Reviewer: `architecture-reviewer` subagent, read-only, given the diff `16b0aae8..3366aeb3` with the constitution, the
spec and the ADRs.

## Findings

- **Blocker B1: a readiness test checks something no code does** (`leased_at IS NOT NULL` after a poll); the recorded
  evidence cannot be reproduced from the commit.
- **Major M1: the change diverges from accepted ADR-0089 decision 2** (the decision belongs in `persist`) and records it
  only as D022-044; a divergence needs a new ADR and a fitness check over the `Progress::pending` consumers.
- **Major M2: gate returns are locked after the decision, out of order and without the key lock**; a release and a
  decision on one key count the released holder.
- **Major M3: the new paths record no metric or span** (rule 10), and no gap says so.
- minor: m1 `resolve_gates` accepts any node and code in `refused`; m2 the test-only feature's rule does not refuse a
  production call; m3 the live-keys ceiling is not in G1199; m4 configuration errors of the key ring map to a retryable
  refusal; m5 `data-model.md` does not describe `flow_control_gates`; m6 three migrations edited in place (fine only if
  never applied elsewhere); m7 `scripts/check-upgrade-boundary.py` names an older last migration.
- nits: the canonical bytes of a key come from `serde_json::to_vec` with no golden test; a `null` key equals the group's
  single key.
- Fine: domain purity, the refusing port default, the migrations (nullable column, `NOT VALID` then validated, refusing
  down), the `jsonb` column bounded and classified, compatibility, the fail-closed state of D022-037, the claim budget.

## Resolution (author, 2026-11-19)

| Finding | Resolution | Evidence |
|---|---|---|
| B1 | As correctness 4: corrected and rerun | readiness suite |
| M1 | ADR-0094 (proposed) refines ADR-0089 decision 2; owner question O1171; T022-096 not ticked until answered. The consumers of `pending` that cannot answer refuse (G1197) | `docs/adr/ADR-0094-…`, `research.md` O1171 |
| M2 | Fixed (correctness 1) | the two held-lock and same-key tests |
| M3 | Gap G022-001 (with T022-082) | `research.md` |
| m1 | Fixed | `readiness_gate::the_gate_answer_fails_only_an_asked_step_and_only_for_its_key`, red without it |
| m2 | Fixed | `step_gates_door_errors` |
| m3 | G1199 extended | `research.md` |
| m4 | Kept: those two errors come only from parsing the configuration, never from `candidates`; `UnknownVersion` is retryable on purpose (G1193) | — |
| m5 | Described in D022-044; `data-model.md` is an approved text (constitution, Approval), left for the owner with G1195 | — |
| m6 | The three migrations were never merged into `develop` or a release (draft PR 237 only) | `git log origin/develop -- crates/infra-postgres/migrations/20270130*` is empty |
| m7 | Existing before this change; not touched | — |
| nits | Recorded in G022-001 (`null` key) and left for T022-104 / T022-067 (the canonical fixture) | — |
