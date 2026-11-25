# Responses to the design reviews of 2026-11-25

Where each finding of [design-reviews-and-rulings.md](design-reviews-and-rulings.md) now lives. Every ruling is written
as a decision of `research.md` ("Decisions of the review of the design of the rest of 022 (2026-11-25)"), delegate
(OD-2026-11-24-4), and into the documents below. "Plan" is `plan.md`, section "Release 0.3.0, the rest of 022"; "data
model" is `data-model.md`, section "Revision of 2026-11-25"; "tasks" is `tasks.md`, subsection "Revision of 2026-11-25".

## Blockers

| Finding | Decision | Where it lives |
|---|---|---|
| Architecture Blocker 1 | D022-222 | ADR-0137 decision 4; ADR-0136 decision 8 (the handler head); ADR-0050 decision 7, revision of 2026-11-25; spec FR-022-026 amendment of 2026-11-25; plan rows T9, T16; tasks T022-224 |
| Architecture Blocker 2 | D022-223 | ADR-0136 decision 7; ADR-0050 decision 7, recorded deviation of 2026-11-25; plan row T8; data model, `failure_handler_dispatches`; tasks T022-225 |
| Correctness Blocker 1, Architecture Major 4 | D022-224 | ADR-0136 decisions 1 and 2; spec FR-022-013 and FR-022-014 amendments of 2026-11-25; data model, `runs` (`failure_handler_state`); `contracts/journal-events.md` revision of 2026-11-25; tasks T022-227 |
| Security Blocker 1, correctness minor 1 | D022-225 | ADR-0135 decisions 2 and 8; spec FR-022-002 and FR-022-004 amendments of 2026-11-25; data model, node fingerprint and `step_cache_entries`; plan row T1; tasks T022-236 |

## Majors

| Finding | Decision | Where it lives |
|---|---|---|
| Security Major 1 | D022-226 (built with a stated reading: no node declares an input mapping or a binding on this tree; the fingerprint is the canonical IR with a closed exclude list, and the producing worker's binding is checked at a hit) | ADR-0135 decisions 2, 8 and 10; spec FR-022-002, FR-022-007, FR-022-030 amendments; data model, node fingerprint; research gap G022-029; tasks T022-236 |
| Security Major 2 | D022-227 | ADR-0137 decision 2; spec FR-022-023 and FR-022-026 amendments; data model, `publication_policies`; `contracts/errors.md`; OpenAPI fragment (both operations); tasks T022-229 |
| Security Major 3, Correctness Major 6 | D022-228 (the "initiator" half is observable only inside the engine, gap G022-028; the cost for a person's rerun is asked as O1176) | ADR-0136 decisions 4, 5 and 7; spec FR-022-014 and FR-022-016 amendments; plan, "Authorization (T022-011)"; data model, `runs` and `failure_handler_dispatches`; `contracts/journal-events.md`; tasks T022-228, T022-215 as it reads now |
| Security Major 4 | D022-229 | ADR-0135 decision 10; spec FR-022-057 and FR-022-007 amendments; `contracts/errors.md` (`CACHE_KEY_SECRET`, `CACHE_KEY_PII`); tasks T022-230 |
| Correctness Major 1 | D022-230 | ADR-0135 decision 9; spec FR-022-005 and FR-022-006 amendments; plan rows T3, T4, T5; data model, `step_cache_entries`; tasks T022-231 |
| Correctness Major 2, Architecture Major 3 | D022-230 | ADR-0135 decisions 7 and 9; plan row T18; data model, lock order revised; spec FR-022-037 amendment; tasks T022-231, T022-211 |
| Correctness Major 3 | D022-230 | ADR-0135 decision 9; plan row T3; tasks T022-231 |
| Correctness Major 4 | D022-230 | ADR-0135 decision 9; data model, `steps.cache_lookup` and `step_cache_store_obligations`; plan row T2; `contracts/journal-events.md`; tasks T022-231 |
| Correctness Major 5, Architecture minor 2 | D022-231 | ADR-0135 decision 4; spec FR-022-003 amendment; `contracts/errors.md` (durable codes); plan row T1 and budget lines; tasks T022-232 |
| Correctness Major 7 | D022-228 | ADR-0136 decision 5; data model, `runs` (`handler_kind`, `handler_for_run_id`); tasks T022-228 |
| Correctness Major 8, Architecture Major 5, architecture minor 15 | D022-232 | ADR-0146 (new, proposed); ADR-0094 decision 4, revision of 2026-11-25; plan, code facts and row T31; spec FR-022-051 amendment; tasks T022-207 as it reads now, T022-226 |
| Correctness Major 9 | D022-233 | ADR-0137 decision 6; spec FR-022-027 amendment; data model, invariant 19; plan row T16; `contracts/journal-events.md`; tasks T022-233 |
| Architecture Major 1 | D022-234 | ADR-0056 decision 2, its sentence replaced (dated); plan, "The order of a commit"; tasks T022-234 |
| Architecture Major 2 | D022-235 | ADR-0050 decision 7, slot 7; ADR-0135 decision 7; data model, lock order revised |
| Architecture Major 6 | D022-236 | plan, "Capabilities and activation", serving rule; spec FR-022-031 amendment; ADR-0135 decision 10, ADR-0136 decision 9, ADR-0137 decision 9; tasks T022-235 |
| Architecture Major 7 | D022-237 | ADR-0136 decision 7; plan, budget lines; data model, `runs` (`test_tree`) and `namespace_failure_handlers`; spec FR-022-030 amendment; tasks T022-217 as it reads now |
| Architecture Major 8 | D022-223 | ADR-0136 decision 7; spec FR-022-015 amendment; `contracts/errors.md`; OpenAPI fragment (`setNamespaceFailureHandler`) |

## minors and nits

| Finding | Decision | Where it lives |
|---|---|---|
| Architecture minor 1, correctness minor 2 | D022-238 (the condition holds: the planner releases nothing while paused, `plan.rs:101`, `:309-316`, so the exception is removed) | ADR-0135 decisions 5 and 6; spec FR-022-003 and FR-022-051 amendments; data model, invariant 14 and the step states; plan, failure matrix; tasks T022-210 as it reads now |
| Architecture minor 3, security minor 4 | D022-225 | ADR-0135 decision 2; plan row T1; data model, lock order revised |
| Correctness minor 6 | D022-228 | ADR-0136 decision 7; spec FR-022-016 amendment |
| Security minor 1 | D022-230 (read as: `UPDATE (expires_at)` granted, the `ALWAYS` trigger refusing every other column and any raise of the expiry) | data model, `step_cache_entries` |
| Security minor 2 | D022-229 | ADR-0135 decision 2; data model, node fingerprint; spec FR-022-002 amendment |
| Security minor 3 | D022-229 | ADR-0135 decision 10; OpenAPI fragment (`StepCacheEntry.sourceRunId`, the listing); research gap G1171 |
| Security minor 5 | D022-233 | ADR-0137 decision 6; spec FR-022-029 amendment |
| Security minor 6 | D022-226 | ADR-0135 decision 10; ADR-0137 decision 5; spec FR-022-030 amendment |
| Security minor 8 | D022-240 | `contracts/errors.md`; OpenAPI fragment (`setNamespaceFailureHandler`) |
| Security minor 9 | D022-230 | ADR-0135 decision 8; data model, entries and obligations; spec FR-022-037 amendment; research gap G022-021 |
| Architecture minor 16 | D022-240 | tasks, revision of 2026-11-25 (the merge order) |
| Architecture minor 4 | D022-240 | ADR-0135 decision 10 |
| Architecture minor 5 | D022-223 | ADR-0136 decision 8; ADR-0050 decision 7 revision; plan row T9 |
| Architecture minor 6 | D022-240 | ADR-0137 decision 1; ADR-0050 decision 7 revision; plan rows T15, T16; tasks T022-224 |
| Architecture minor 7 | D022-223 | research gap G022-021 |
| Architecture minor 8 | D022-224 | ADR-0136 decision 3; spec FR-022-012 amendment; `contracts/errors.md` (`FAILURE_HANDLER_SELF_GATE`) |
| Architecture minor 9 | D022-224 | ADR-0136 decision 3; spec FR-022-012 amendment; `contracts/errors.md` (`FAILURE_HANDLER_REGION`) |
| Architecture minor 10 | D022-237 | ADR-0137 decision 5; data model, `runs` (`test_tree`); tasks T022-217 as it reads now |
| Architecture minor 11 | D022-239 | data model, `runs`; ADR-0137 consequences; tasks T022-237 |
| Architecture minor 12 | D022-239 | ADR-0135 and ADR-0136 consequences; spec FR-022-031 amendment; plan, serving rule; tasks T022-237 |
| Architecture minor 13, correctness nit 1 | D022-224 | ADR-0136 decision 2; data model, `failure_handler_state` mapping; `contracts/errors.md`; spec FR-022-014 amendment |
| Architecture minor 14 | D022-237 | tasks, T022-204 and T022-217 as they read now |
| Architecture nits | D022-240 | ADR-0135 decisions 2 and 9 (the wording; the writer takes no lease or fence); "store obligation", "dispatch obligation" and "018's obligation" qualified in the texts of this revision; ADR-0050 decision 7, slot 6 |
| Correctness minor 3 | D022-238 | data model, invariant 14 |
| Correctness minor 4 | D022-240 | spec, US01 scenario 4 and SC-022-002 amendments |
| Correctness minor 5, security minor 10 (the triggers) | D022-230 | ADR-0135 decisions 8 and 9; data model, entries and obligations; plan row T2 |
| Correctness minor 7 | D022-228 | ADR-0136 decision 5; data model, `runs` (`handler_for_run_id`) |
| Correctness minor 8 | D022-223 | ADR-0136 decision 8; plan row T9 |
| Correctness nit 3 | D022-235 | ADR-0135 decision 7 |
| Correctness nit 4 | D022-240 | ADR-0135 consequences; tasks, T022-062 as it reads now |
| Security minor 7 | D022-240 | ADR-0137 consequences; tasks, T022-061 as it reads now |
| Security minor 10 (the bounds) | D022-240 (read as: 4 KiB for `keyValue`; `mock` keeps the approved pin bound of 256 KiB, both with the JSON depth limit) | OpenAPI fragment (`StepCacheInvalidationRequest.keyValue`, `TestPin.mock`) |
| Security nits | D022-233, D022-237, D022-240 | the check order: ADR-0137 decision 2, `contracts/errors.md`, spec FR-022-026 amendment; the guard `ENABLE ALWAYS`: ADR-0137 decision 5, data model; the shared standalone group: ADR-0137 decision 3, spec FR-022-023 amendment; published script artifacts: ADR-0137 decision 3, `contracts/errors.md`; the cache listing's exposure: OpenAPI fragment (the listing's description) |
| Correctness nit 2 | — | not in the rulings file; nothing to place |

## Readings stated rather than silently changed

1. **Security Major 1.** The ruling puts "the canonical input-mapping expression and the worker binding" in the
   fingerprint. No `activity` or `script` node declares either on this tree (`workflow.schema.json`; the binding is
   namespace configuration, `crates/domain/src/binding.rs`). D022-226 keeps the intent with the canonical IR and a
   closed exclude list (a future input mapping or binding field enters by construction) and a check of the producing
   worker's binding at a hit.
2. **Security Major 3.** "A reset or rerun whose initiator is a handler-tree run" is not observable when a handler calls
   the public API with a machine identity (gap G022-028); the lineage index is what bounds that loop. Its cost (a
   person's rerun of a failed run is not dispatched again) is asked as O1176, with a recommended alternative; the
   documents follow the ruling until it is answered. *Answered 2026-11-25: O1176 option (b), decision D022-241; the
   documents now follow it, and G022-028 is narrowed.*
3. **Security minor 1.** `UPDATE (expires_at)` together with "the trigger that refuses updates kept" is read as a trigger
   that refuses every other column and any raise of the expiry.
4. **Security minor 10.** "4 KiB" is read for `keyValue`; for `mock` it would contradict the approved pin bound of 256 KiB
   (plan, ceilings table), so `mock` keeps 256 KiB with the JSON depth limit.
