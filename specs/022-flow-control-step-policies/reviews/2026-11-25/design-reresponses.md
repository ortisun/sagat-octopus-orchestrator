# Responses to the re-reviews of 2026-11-25

Where each finding of [design-rereviews-and-rulings.md](design-rereviews-and-rulings.md) now lives. Every ruling is a
decision of `research.md` ("Decisions of the re-review of the design of the rest of 022 (2026-11-25)", D022-242 to
D022-259), delegate (OD-2026-11-24-4). "Plan" is `plan.md` (the governing table is "Transactions, as built" in the
section "Release 0.3.0, the rest of 022"); "data model" is `data-model.md`, section "Revision of 2026-11-25 after the
re-reviews"; "tasks" is `tasks.md`, subsection "Revision of 2026-11-25 after the re-reviews". The rulings file groups
findings by section; where it names a finding only within its section, the row maps the section's decisions.

## Architecture

| Finding | Decision | Where it lives |
|---|---|---|
| N0 (Blocker) | D022-242 | ADR-0136 decision 7; ADR-0050 decision 7, deviation extended to the lineage key; plan rows T8, T9; data model, `failure_handler_dispatches`; spec FR-022-016 line after the re-reviews; `contracts/journal-events.md`, `contracts/errors.md`, OpenAPI fragment (dispatch state, redrive); tasks T022-238 |
| N1 | D022-259 | plan: the original table marked superseded and restored from `d8d3cf79`; the governing table's rows T1 to T18 and T31 rewritten whole, T32 added |
| N2 | D022-255 | ADR-0056 decision 2, struck and restated; plan, "The order of a commit" rewritten; data model, lock order as built; tasks T022-252 |
| N3 | D022-247 | ADR-0136 decision 2; spec FR-022-013 and FR-022-014 lines after the re-reviews; data model, `runs`; plan, failure matrix; tasks T022-243 |
| N4 | D022-249 | ADR-0136 decision 9; ADR-0137 decision 5; ADR-0146 decision 2; `contracts/errors.md` (`FLOW_CONTROL_COMBINATION`); tasks T022-244 |
| N5 | D022-254, D022-255 | ADR-0056 decision 2(a); ADR-0050 decision 7, slot 5; ADR-0135 decision 2; plan rows T1, T15, T16; tasks T022-248, T022-224 |
| N6 | D022-250 | ADR-0136 decision 9; ADR-0146 decision 2; plan row T8; tasks T022-245 |
| N7 | D022-251 | ADR-0146 "Enforced by" and decision 2; tasks T022-226 as it reads now, T022-247 |
| N8 | D022-256 | plan, "Capabilities and activation", manifest; spec FR-022-031 line after the re-reviews; ADR-0136 decision 9; tasks T022-249 |
| N9 | D022-255 | plan, "The order of a commit" and row T8; ADR-0136 decision 7; data model, lock order; tasks T022-252 |
| N10 | D022-252 | ADR-0135 decisions 7 and 9; plan rows T3, T5, T18; data model, cache tables; tasks T022-246 |
| N11 | D022-242 | as N0 |
| minor 1 | D022-256 | plan, manifest and rollback order |
| minors 2 to 7 | D022-258 | ADR-0146 decision 3 (the interim refusal kept as a defence); ADR-0136 consequences (unrelated automation reruns extend the lineage, with the suppressed and redrivable row); plan, budget lines (the child `test_tree` check), "Complexity Tracking (the rest)", the Constitution Check re-dated |
| nits | D022-258 | the `crates/infra-postgres/src/definitions.rs` citations corrected (research, ADR-0136, ADR-0137, plan); the plan's persist fact in its order; D022-238 cites `plan.rs:316`; ADR-0136 decision 3 (`FAILURE_HANDLER_SELF_GATE` direct only) |

## Correctness

| Finding | Decision | Where it lives |
|---|---|---|
| B1 (Blocker) | D022-242 | as Architecture N0; research O1176 (the answer reversed), D022-241 (marked reversed), gap G022-028 (closed) |
| B2 (Blocker) | D022-246 | ADR-0136 decision 2; `contracts/journal-events.md` (`failure_handler.draining`); `contracts/errors.md` (the causes); data model, `runs` and invariant 20; plan row T32; tasks T022-242 |
| M1 | D022-246 | as B2 |
| M2 | D022-247 | as Architecture N3 |
| M3 | D022-248 | ADR-0136 decisions 3 and 7; plan rows T7, T8; tasks T022-227 as it reads now |
| M4 | D022-249 | as Architecture N4 |
| M5 | D022-243 | ADR-0136 decision 8; data model, `failure_handler_dispatches`; plan, failure matrix and run creators; tasks T022-239 |
| M6 | D022-252 | ADR-0135 decision 9; spec FR-022-005 line after the re-reviews; data model, `step_cache_entries`; tasks T022-246 |
| M7 | D022-252 | ADR-0135 decision 9; data model, `step_cache_key_invalidations`; plan row T5; tasks T022-246 |
| M8 | D022-259 | as Architecture N1 |
| M9 | D022-252 | ADR-0135 decision 4 (cumulative hits); spec FR-022-004 line after the re-reviews; tasks T022-246 |
| M10 | D022-252 | ADR-0135 decision 7 (one usage row); data model, usage; tasks T022-246 |
| minors 1 and 4 | D022-252 | ADR-0135 decisions 7 and 9; data model, invariant 18 restated |
| minor 3 | D022-251 | ADR-0146 decision 2 and "Enforced by" |
| minors 2, 5, 6, 7, 8 | D022-246, D022-248 | ADR-0136 decisions 2, 3 and 7; tasks T022-227 as it reads now, T022-242 |
| minor 9 | — | not named in the rulings file; nothing to place |
| nits | D022-252, D022-258 | as the rulings list them: the cache store's nits in ADR-0135 decision 9; the rest as Architecture nits |

## Security

| Finding | Decision | Where it lives |
|---|---|---|
| A | D022-242 | as Architecture N0 |
| B | D022-243 | as Correctness M5 |
| C | D022-244 | ADR-0136 decision 4; plan, "Authorization (T022-011)" reworded; spec FR-022-014 and FR-022-032 lines after the re-reviews; `contracts/errors.md` (`RECOVERY_ROLE_MAY_NOT_APPROVE`); tasks T022-011 as it reads now, T022-240 |
| D | D022-259 | as Architecture N1 |
| minors 1 to 4, 7 to 9 | D022-252, D022-254 | ADR-0135 decisions 2, 8 and 10; spec FR-022-002, FR-022-004, FR-022-007 lines after the re-reviews; data model, cache tables (`secret_ids`, `producer_subject`, `step_cache_names`, the obligations' grant); OpenAPI fragment (`StepCacheEntry`, the invalidation result and its `404`); plan, "Observability, added"; tasks T022-248, T022-246 |
| minor 5 | D022-245 | ADR-0136 decision 5; data model, `runs`; tasks T022-241 |
| minor 6 | D022-257 | ADR-0137 decision 2; spec FR-022-023 line after the re-reviews; `contracts/errors.md`; OpenAPI fragment (`startStepExecution`); tasks T022-250 |
| nits | D022-258 | the authorization paragraph reworded (plan); `unauthorized` read as `bypassed` (ADR-0135 decision 10, plan); the producer's classification in `namespace` scope (ADR-0135 decision 8); `public.`-qualified `SECURITY DEFINER` triggers (data model, usage); T17's fence `sagat_lock_namespace` (plan row T17) |

## Rulings not buildable as written, with the design that keeps their intent

1. **"Lineage" 1 and 2 together.** An `ON CONFLICT DO NOTHING` writes no row, so a suppressed dispatch could not be shown
   or redriven; and a redriven suppressed row would conflict with a unique index over "the live rows". Built instead
   (D022-242): `lineage_first` with a partial unique index `WHERE lineage_first`; the terminal commit's first insert is
   `ON CONFLICT ... DO NOTHING` on it, and only when it inserted nothing a second insert writes the `SUPPRESSED_LINEAGE`
   row with `lineage_first = false`, which a redrive turns `PENDING` without a conflict.
2. **"Handler phase" 1 and 3 together.** The cause `RUN_DEADLINE` cannot occur once the run deadline does not apply during
   the phase. Built instead (D022-246): the closed set of six causes without it, pinned by a decoder test.
3. **"Draft executions".** "`prod` refuses standalone activity and script steps as it refuses test runs, unless the
   setting is enabled" is read as: standalone code steps in `prod` are refused unless the setting is enabled, and draft
   test runs stay refused in `prod` whatever the setting, as O1160 decided (D022-257).
4. **A precision on "Orders and statements" 2** (buildable as ruled). There is no statement that loads a run's definition to join; the ruling's own choice,
   a left join in the statement that locks the run, is built with `FOR UPDATE OF runs` so the configuration row is not
   locked (D022-255).
