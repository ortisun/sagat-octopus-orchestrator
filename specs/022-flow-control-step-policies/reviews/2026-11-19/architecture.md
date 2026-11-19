# Architecture review of T022-097, T022-098, G1199 and G022-001 (2026-11-19)

Reviewer: `architecture-reviewer` subagent, read-only, given the commits `4cc39427`, `bc492b6b`, `d89086b9` with the
constitution, AGENTS.md, the architecture principles and the ADRs.

## Findings

1. **Major: rate keys are never re-armed** (as security 1).
2. **Major: paused heads starve a key** (as security 2).
3. **Major (rollout, D022-037): a scheduler without the key ring stalls the keys it serves** and fails every waiter.
4. **Major: the per-key bound (32) is below the plan's 256** and caps a key under SC-022-029's 100 a second.
5. **Major (merge, not build): the gate rests on the proposed ADR-0094**; ADR-0089's "Enforced by" still says no check
   exists.
6. minor: the orphan "alert" is a log line and an outcome only; the runbook does not list the new kinds.
7. minor: the reservation SQL and the pass disagree on stopping runs.
8. minor: the public test hooks `grant_gates`/`time_out_gates`; plan T26 puts the fence before the run lock.
- nits: the refusal labels say `concurrency.*` for rate gates; a placeholder bind.
- Proposed fitness function: only the gate writes holder, waiter and grant rows.
- Fine: domain purity, fail-closed defaults, closed telemetry, held-lock races, the ungated budget lines.

## Resolution (author, 2026-11-19)

| Finding | Resolution | Evidence |
|---|---|---|
| 1, 2 | As security 1 and 2 | — |
| 3 | Fixed in part (security 6); that a scheduler must hold the key ring is added to G1198, a precondition of listing the capabilities | `research.md` G1198 |
| 4 | Fixed: 256 | `GRANTS_PER_KEY_PER_PASS` |
| 5 | Owner gate: T022-096, T022-097 and T022-098 rest on ADR-0094 (O1171); ADR-0089 is an accepted text, so its "Enforced by" is answered in D022-045 instead | `research.md` D022-045 |
| 6 | Gap G022-005 (alert rule and runbook rows with T022-061, T022-082) | `research.md` |
| 7 | Fixed: the listing, the reservation and the pass use one predicate (live, not paused, before its deadline); a stopped run's waiter is refused, not busy | queue suite |
| 8 | `#[doc(hidden)]` on both hooks; the plan's order is answered in D022-045 (run, then fence, as every pass) | — |
| fitness | Added: `scripts/check-architecture.py` (`gate_row_write_errors`) | `scripts.tests.test_check_architecture.GateRowsAreWrittenOnlyByTheGate` (a writer in `workers.rs` refused, a read allowed, an empty rule an error) |
| nits | Kept: the event names are only the refusal's label | — |
