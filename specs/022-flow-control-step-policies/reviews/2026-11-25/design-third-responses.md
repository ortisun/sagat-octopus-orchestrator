# Responses to the third reads of 2026-11-25

Where each ruling of [design-third-reads-and-rulings.md](design-third-reads-and-rulings.md) now lives. Each is a
decision of `research.md` ("Decisions of the third reads (2026-11-25)"), delegate (OD-2026-11-24-4). "Plan" is
`plan.md`, section "Release 0.3.0, the rest of 022"; "data model" is `data-model.md`, section "Revision of 2026-11-25
after the third reads"; "tasks" is `tasks.md`, subsection "Revision of 2026-11-25 after the third reads".

| Ruling | Findings | Decision | Where it lives |
|---|---|---|---|
| 1 The dispatch outcome in the journal | architecture Major 1, correctness Major 1 | D022-260 | ADR-0136 decision 7; plan row T8; data model (and invariant 22); `contracts/journal-events.md`; spec FR-022-016 line after the third reads; tasks T022-238 as it reads now |
| 2 A later cause during draining | correctness Major 2 | D022-261 | ADR-0136 decision 2 and "Enforced by"; plan row T32 and failure matrix; data model; `contracts/journal-events.md` (`later_causes` removed), `contracts/errors.md` (the `409`); spec FR-022-013 line after the third reads; tasks T022-242 |
| 3 The overdue pass between pick and lock | correctness Major 3 | D022-262 | ADR-0136 decision 2; plan, failure matrix; spec FR-022-013 line; tasks T022-243 |
| 4 Continue-as-new under a handler or test tree | correctness Major 4 | D022-263 | ADR-0136 decisions 8 and 9; ADR-0137 decision 5; plan row T9 and failure matrix; data model; `contracts/errors.md` (`CONTINUE_AS_NEW_REFUSED` in three places); spec FR-022-016 line; tasks T022-244 |
| 5 Budget deferrals | correctness, possible Major | D022-264 | ADR-0136 decision 8; plan row T9 and failure matrix; data model; `contracts/errors.md`; tasks T022-239 |
| 6 The producer's locks of a hit | architecture Major 2 | D022-265 | ADR-0135 decision 2; ADR-0056 decision 2(a), rewritten; ADR-0050 decision 7 acceptance line; plan rows T1, T15, T16; data model; spec FR-022-004 line; tasks T022-248, T022-254, T022-255 |
| 7 The `RunRecovery` role | security Majors 1 and 2, architecture Major 3 | D022-266 | ADR-0136 decision 4; plan, "Authorization (T022-011)" with the role's column; data model (`run_recovery_namespaces`); `contracts/errors.md`, OpenAPI fragment (the redrive); spec FR-022-014 line; tasks T022-240, T022-253 |
| 8 Slices that must merge alone | architecture Major 4 | D022-267 | ADR-0146 decision 1; tasks, T022-225, T022-226, T022-245 and T022-207 as they read now |
| 9 Orders | architecture Major 5, the open part of N2 | D022-268 | ADR-0146 decision 1 |
| 10 The draft-executions lock | architecture, ADR-0137 change | D022-269 | ADR-0137 decision 2; ADR-0050 decision 7 acceptance line (slot 4a); plan rows T15, T16; data model; spec FR-022-023 line; tasks T022-250 |
| 11 Leftovers | security A, correctness M8, architecture N0 | D022-270 | plan: the machine-identity paragraph and the two lineage rows of the failure matrix struck; data model: the `lineage_run_id` row and the dispatch note of D022-241 struck; ADR-0136: the run-deadline case under "Enforced by" and "the revision read with the definition" in option 8 struck; `design-reresponses.md`: Security B mapped to D022-244, C to D022-242 |
| Acceptance | every verdict "accept" or "accept with changes" | D022-271 | the status lines of ADR-0135, ADR-0136, ADR-0137, ADR-0146 and the index of `docs/adr/README.md`; the acceptance lines of the amendments of ADR-0050, ADR-0056 and ADR-0094; tasks T022-200 (an evidence line, not ticked) |
