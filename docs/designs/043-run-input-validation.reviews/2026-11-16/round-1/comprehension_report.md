# egm-review Stage 6 (Comprehension Test), round 1

Doc: docs/designs/043-run-input-validation.md
Reviewed commit: f562614874016262dd9812e241f734038d4d70de
Stage reviewer: egm-comprehension (FeatureRange 1-852, DetailedRange 853-1147, no standard)
Note: condensed by the author; the paraphrases are shortened, every finding and severity is kept.

## What the feature does (paraphrase)

From 0.3.0 every run's input is validated against the `inputSchema` of the version the run pins, before the run is
created, by one gate in `crates/application`, on thirteen creation paths (API and uploaded-input starts, with-start,
reruns and resets, schedule actions, backfills and trigger-now, event and cron triggers, children, continue-as-new, ad
hoc executions) plus the schedule write. A caller who waits gets `422 VALIDATION_FAILED` with a closed reason, a pointer
and a rule, never the value; repeats, trigger-now and private reruns answer `/` and `other`. A creator nobody waits for
records one final, value-free outcome and never retries it. Transient conditions (no permit in 5 s, the backstop, a
compile panic) are retryable and never cached. The schema is a documented subset validated by the platform's own
validator with a work budget of 1,000,000 units; a pre-check runs at publication, at start (cached per tenant and
digest) and in an inventory command. Four permits per process, shared fairly. `SAGAT_RUN_INPUT_VALIDATION` selects
`enforce` (default) or `report`; only `enforce` lists the capability. `CheckedRunInput` and seven architecture rules
force future creators through the gate.

## How it will be built (paraphrase)

Twenty-four tasks, each one PR with tests seen failing first: the schema in the definition (T043-1), the validator
(T043-2), the gate and permits (T043-3), the types, stores and rules (T043-4), each creator row (T043-5 to T043-11), the
mode and registry (T043-12), publication (T043-13), contracts, SDKs, CLI and Studio (T043-14 to T043-16), the sentinel
and budget tests (T043-17, T043-18), runbook (T043-19), a fixture sweep (T043-20), budget, report and role tests
(T043-21 to T043-23), the inventory (T043-24). A verification plan; Appendix A on the search, off-worker, regex memory
and determinism.

## Findings

1. Major (653-655 vs 396, 96-97, 227, 834-835): row 12 reads the schema "under the run's lock"; a reader cannot tell
   whether validation runs inside the transaction, against FR-043-001, NFR-043-PERF and the rejected alternative.
2. Major (902): T043-3 says tokio stays a dev-dependency of `application`; the plan says a normal `rt` dependency.
3. Major (1080): T043-24 says the owner role; Technical Plan 8 says the runtime role.
4. Major (968-972 vs 624-627, 666-667): T043-11 names one migration and omits the requirement history; Technical
   Plan 6 says "the same migration".
5. Major (40-44, 855-856, 355, the checkboxes): the doc reads as neither plan nor record: built, yet unticked and Draft;
   the phase grouping contradicts the task ordering paragraph.
6. Major (922-924): T043-5 asks for graded disclosure, which FR-043-003 says is not built.
7. Major (174-180, 345-347, 669-673, 759): how the operator records a requirement, and what "stays up with its gate
   closed" means, are not said; "gate" collides with `RunInputGate`; ADR-0082 is not linked.
8. Major (49, 173, 207-209, 303, 457-464, 580, 793): ADR-0090 and ADR-0092 are load-bearing but neither linked nor
   summarized; nothing says what changes if ADR-0092 is refused.
9. Major (124-126, 33-34, 173, 341-342, 676-679): "holder", "open holder" and `USE_EXISTING` are never defined.
10. Major (127-140, 614-628): the schedule vocabulary (`decide`, `BUFFERED`, owed, exhausted, `settle_waiting`, the
    fence) is assumed from design 018.
11. Major (756, 682-684, 727): the inventory exists only in the 0.3.0 binary; whether it runs against an unmigrated
    database is not said.
12. Nit (48 vs 383): register range omits G043-054.
13. Nit (397-398, 631, 961-963, 373): `target_version` vs `target_check`, `check` vs `check_input`.
14. Nit (908-909, 193-210): the four architecture rules vs the seven; number them all.
15. Nit (683, 757-759): "rows 4 to 12" for the first upgraded engine is wrong; reuse Technical Plan 10's list.
16. Nit (143-147, 326-327, 639-641): whether a redelivery increments the metric.
17. Nit (110-112 vs 367): FR-043-003 lists only the two run reasons, though trigger-now answers schedule reasons.
18. Nit (430, 301): `Creator::Repeat` and "repeats" undefined; no row-to-creator mapping.
19. Nit (369 and others): the cron handover and features cited by number only.
20. Nit (559, 567): "k" used before it is defined.
21. Nit (575-580): MiB and MB mixed.
22. Nit (448-449 vs 208-209): when a `ScheduleService` without a gate can exist.
23. Nit (32, 280, 696): the constitution is never linked.

Counts: Blocker 0, Major 11, Nit 12
