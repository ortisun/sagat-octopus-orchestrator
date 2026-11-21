# Responses to the reviews of task T022-086 (2026-11-21)

The three reviews are `reset-slot-security-reviewer.md`, `reset-slot-correctness-reviewer.md` and
`reset-slot-architecture-reviewer.md`. Every Blocker and Major is answered below. A re-review of the answers is still to
be run before the task is ticked; until then the runbook keeps T022-086 in the production gate and gap G022-016 stays open.

## Blockers and Majors

| Finding | Answer | Status |
|---|---|---|
| Architecture Blocker 1: 018's normative registry still says `EXCLUDE` | `specs/018-schedules-bulk-operations/contracts/journal-and-events.md`: the two rows the code installs, `run.reset`'s fresh `rate_limit.granted`, and a "022 run slots" bullet under the interplay | Answered; no test reads the registry yet (architecture's suggested check is open, below) |
| Architecture Blocker 2, correctness Major 1: the Studio client is behind the OpenAPI | `npm run api` (3 lines of `schema.d.ts`, the two descriptions), `npm run check:api`: matches; `vitest ResetRun.test.tsx` 49 passed | Answered |
| Architecture Major 1: a `CANCEL_SOURCE` reset on a full key cancels and fails | runbook `run-reset.md` warns and advises `REQUIRE_TERMINAL`; D022-069 item 3 records the two alternatives weighed; the advisory refusal is task T022-114 | Answered (design follow-up T022-114) |
| Architecture Major 2, correctness Major 3: the other creators of T022-064 have no rate twin | D022-069 item 7: the structural check `run_creator_errors` and `rate_creators.rs` cover the port; the twins are task T022-113, and T022-086's text is amended to the reset | Answered (follow-up T022-113) |
| Correctness Major 2: a reset after a rate-limited step is untested | `run_creators::a_reset_after_a_rate_limited_step_restores_its_slot_as_history` (restored grant, no fresh return, only `b`'s fresh step grant, consistency before and after `b`) | Answered |

## minors and nits

| Finding | Status |
|---|---|
| Security minor: the down migration does not refuse while restored `rate_limit.*` rows exist | Open, for the next change of this task |
| Security nit: the codec accepts the marker on `REFUSE` gate types | Open |
| Security minor and architecture Major 1: a member can fill the window to make a pending reset fail | T022-114 |
| Security nit: one bit of occupancy (G022-014) | Open: to add to G022-014's list |
| Correctness minor 1, architecture minor 4: no end-to-end case with both gates refusing | Partly: the unit test `readiness_gate::tests::a_refused_reset_is_named_by_its_first_refusing_gate_and_counted_by_each_declared_family` (red under two hand mutations); the integration case is open |
| Correctness minor 2: a reset counts waiters the pass would skip | Open (the permit has the same rule) |
| Correctness minor 3: the `restored` marker is trusted | Open |
| Correctness minor 4, architecture minor 2: the runbook lifted the gate early | Answered: T022-086 is back in the gate, G022-016 open until the tick |
| Correctness minor 5, architecture minor 6: evidence incomplete | In the task's evidence once the gates have run |
| Correctness minor 6: the per-second case depends on timing | Open |
| Correctness minor 7, architecture minor 1: the command-line words and a drift check of the reason lists | Open |
| Architecture minor 3: compatibility of the marker | Answered: D022-069 item 6, runbook section 6 |
| Architecture minor 5: the migration test hard-codes the previous version | Open |
| Architecture minor 7: 018's spec and plan name only the permit | Open |
| nits: doc comment on the wrong function | Answered |
| nits: dates, "four" events | Answered |
| nits: "grows only", the permit row of 022's errors table, "then", the runbook's answer table, rate first in the domain, the ADR-0088 sentence, the metric row | Open |
