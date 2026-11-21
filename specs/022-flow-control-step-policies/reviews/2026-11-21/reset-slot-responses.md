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

## Re-review (2026-11-21)

The three reviewers re-read the diff of `8dcd7f37` and `cf4f858e` with these answers: `reset-slot-rereview-security-reviewer.md`,
`reset-slot-rereview-correctness-reviewer.md` and `reset-slot-rereview-architecture-reviewer.md`. Each found **0 Blockers,
0 Majors**. The architecture reviewer made the tick conditional on its minors B, C and G; they and the cheap minors are done
in the commit that ticks the task:

| Finding | Answer |
|---|---|
| Architecture minor B: the `CANCEL_SOURCE` trap is not in the contracts | One sentence in `resetRun` of 001's `openapi.yaml` and in the reset of 018's `rest.openapi.yaml`; Studio types regenerated (`check:api` matches) |
| Architecture minor C: T022-114 is bound to nothing | Gap G022-019; D022-069 item 8 says why it is not a production gate; runbook section 6 says so |
| Architecture minor G, correctness minor 3: evidence | In T022-086's entry: both roles of `integration_flow_control_step_policies`, the privilege audit, the check scripts, `check:api` and Vitest |
| Correctness minor 1: the creator twins left the production gate | T022-113 joins runbook section 6's gate (D022-069 item 7) |
| Correctness minor 2: slack in the limit and no `returned` case | `limit: 3` in `a_reset_after_a_rate_limited_step_restores_its_slot_as_history`; new `a_reset_whose_prefix_returned_a_step_slot_restores_it_as_history`. Hand mutations: one grant more in a step window (both red at "only `b`'s fresh slot"; with the old `limit: 4` the first stays green), the codec refusing a restored `rate_limit.returned` (the new case red at load, the other green) |
| Correctness nit 1, architecture minor D: wrong enforcer cited | D022-069 item 7 names `run_start_errors` and `run_insert_errors` |
| Architecture minor E: ADR-0088's "Enforced by" stale | Rewritten with what exists and what is pending (T022-080, T022-113) |
| Architecture minor F, security minor: rolling back the engine | D022-069 item 6 and both runbooks: never roll an engine back past T022-086 once a rate-limited run was reset |
| Architecture minor H, correctness nit 4: what `refused_reset` counts | `plan.md` under the rate metrics, and runbook section 6 |
| Correctness nit 2, architecture nit 1: dates | `contracts/journal-events.md` and `run-reset.md` |
| Architecture nit 2: stale runbook sections | The built behaviour moved out of `run-reset.md` "What is not built yet"; two 409 rows in its section 5 |

Left open, with no Blocker or Major among them: architecture minor A (no test ties 018's registry to `RESET_RULES`), minor
I (the command line's sentence, a drift check of the reason lists, the hard-coded previous migration, 018's spec and plan
rows, the end-to-end case with both gates refusing), nits 3 to 7; security nits (the codec accepts the marker on `REFUSE`
types, the cost of a refused reset; G022-014 now lists the reset's two surfaces of a key's fullness); correctness minor 4 (the per-second case depends on timing),
nit 3 (no case shows the remediation working); and the first review's open rows above.
