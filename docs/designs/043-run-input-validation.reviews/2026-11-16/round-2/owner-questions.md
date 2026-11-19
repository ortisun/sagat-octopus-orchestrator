# Majors left after round 2 of design 043 (2026-11-16)

Doc: docs/designs/043-run-input-validation.md
Reviewed-commit: b9b0f1b56d121ba3f977e2cea888305402f2eb47

Round 2 was a complete review (stages 5 to 9 of `egm-review` in order, and the architecture, security and correctness
reviewers) on the committed text. It ended with Blocker 0 and 52 Majors (egm-review 43, architecture 6, security 1,
correctness 2). The approval rule needs none open, so the design stays `In Review`, no approval block is written and no
task is ticked (R9). As instructed, there is no third round: each Major is listed below with one question for the owner
and a recommendation.

"Fixed after the round" means a change committed after `b9b0f1b5` (D043-173 to D043-175, and four text fixes) that no
reviewer has read yet; the question is then whether to accept the fix without another round.

Recommendations: **accept** (the finding stands as a known limit or a matter of form), **fixed** (accept the fix
unreviewed), **fix** (a change worth making before approval; a short targeted re-read would do), **owner** (a product or
architecture decision only the owner takes).

## Code and architecture (repository reviewers and design critic)

| Finding | Question | Recommendation |
|---|---|---|
| security.md#1 | Accept the 4 MiB cap on pointer bytes (D043-174) as the fix of the quadratic pre-check? | fixed |
| correctness.md#1 | Accept the counting by call (D043-173) as the fix of `reported` counted twice? | fixed |
| correctness.md#2 | Accept the row 3 test that keeps the holder waiting (D043-175) as the fix? | fixed |
| architecture.md#1 | Accept ADR-0092 (O043-007) in the same act as the approval? | owner: accept ADR-0092 |
| architecture.md#2 | Record the refinement of ADR-0050 decision 2 (`INPUT_INVALID` commits and acts on the time) in ADR-0092 with a note on ADR-0050? | fix |
| architecture.md#3 | Accept D043-173 as the fix of the double count? | fixed |
| architecture.md#4 | Route the pre-check outside a start through the start `Permits` (tenant-fair, transient `Busy`) instead of its own pool? | fix |
| architecture.md#5 | Add an ADR decision and a pin test of the subset constants for `run-input-validation-v2`? | fix (pin test); ADR line owner |
| architecture.md#6 | Add a test of the down migration's guards? | fix |
| critic_report.md#1 | Accept that the inventory exits 1 while any immutable stored version is outside the subset? | owner (O043-005 covers superseded versions); recommend a flag that lists them without failing |
| critic_report.md#2 | Accept the non-work-conserving tenant share of the cache (a lone tenant above 64 hot schemas recompiles)? | accept as a documented limit; measure in T043-25 |
| critic_report.md#3 | Same as architecture.md#4? | fix |
| critic_report.md#4 | Accept that inputs inside the subset that fail their schema are visible only in `report` mode, and reword the Risks row? | accept; reword |
| critic_report.md#5 | Accept that an engine pass can wait on permits (bounded by the backstop, head of line)? | accept as documented; revisit with architecture.md#4 |
| intermediate_zoom_out_report.md#2 | Same as architecture.md#4? | fix |
| intermediate_zoom_out_report.md#3 | Keep the per-tenant cache share, or replace it with a plain weight-bounded LRU? | owner; recommend keep with the limit documented |
| intermediate_zoom_out_report.md#4 | Narrow CUJ 5 to versions still started and let exit 1 ignore superseded ones? | owner (O043-005) |

## Text of the design (egm-review)

| Finding | Question | Recommendation |
|---|---|---|
| zoom_out_report.md#1 | Accept a Technical Plan that records what was built, with symbols? | accept (the plan is a record by design) |
| intermediate_zoom_out_report.md#1 | Cut detail from NFR-043-BOUND and the plan to links into the subset contract? | fix |
| comprehension_report.md#1 | Link the closed `details.rule` list (`contracts/errors.md`) from FR-043-003? | fix |
| comprehension_report.md#2 | Introduce `enforce`/`report` in the Overview? | fix |
| comprehension_report.md#3 | Define the blob descriptor of row 2 in the Terms? | fix |
| comprehension_report.md#4 | Define reset, pending reset, `CANCEL_SOURCE` and completion in the Terms? | fix |
| comprehension_report.md#5 | Define ad hoc execution, connection probe, private rerun, `start_private` in the Terms? | fix |
| comprehension_report.md#6 | Reconcile "a triggered action pauses" with "trigger-now records nothing"? | fix |
| comprehension_report.md#7 | Accept the single description of the unwired `ScheduleService`? | fixed |
| comprehension_report.md#8 | Define application path, least work, programs and matchers in NFR-043-BOUND? | fix (with intermediate#1) |
| critic_report.md#14 | Accept the pointer to `research.md` section 5 in place of "Appendix A"? | fixed |
| critic_report.md#24 | Split FR-043-006, -007, -011, -013 into one obligation per sentence? | fix |
| critic_report.md#25 | Give NFR-043-BOUND its units? | fix |
| critic_report.md#26 | Accept the removal of row 12's dangling "input at"? | fixed |
| critic_report.md#27 | Rename the `deadline` outcome of a panic (or give it its own label)? | owner (a metric label is a contract); recommend `interrupted` |
| critic_report.md#28 | Give the validator one name throughout? | fix |
| critic_report.md#29 | Replace "answer" and "room" with defined terms? | fix |

## Detailed Implementation Plan and tasks (egm-review)

| Finding | Question | Recommendation |
|---|---|---|
| implementation_readiness_report.md#1 | Accept a phase order that records the order of the commits as built? | accept (built and green) |
| implementation_readiness_report.md#2 | Assign `integration_run_input_cache` to a task (T043-16)? | fix |
| implementation_readiness_report.md#3 | Add sentinels for rows 2 and 13 and the metrics text to the first sentinel test? | fix (code) |
| implementation_readiness_report.md#4 | Name the SDK error-type and transport files and a test in T043-15? | fix |
| implementation_readiness_report.md#5 | Name the test harness wiring in a task? | fix |
| implementation_readiness_report.md#6 | Add `publishing.rs`, the control plane's wiring and the CLI to Files? | fix |
| implementation_readiness_report.md#7 | Write the gate's API signatures in the plan? | accept (they are in the code the plan records) |
| implementation_readiness_report.md#13 | Same as #3? | fix (code) |
| implementation_readiness_report.md#14 | Add a row 2 case to `integration_durable_execution`? | fix (code) |
| implementation_readiness_report.md#15 | Same as #6? | fix |
| implementation_readiness_report.md#16 | Same as #4? | fix |
| implementation_readiness_report.md#17 | Same as #5? | fix |
| implementation_readiness_report.md#24 | Write the tasks in the past tense of a record? | fix |
| implementation_readiness_report.md#25 | Rewrite T043-4's sentence? | fix |
| implementation_readiness_report.md#26 | Split T043-11's sentence? | fix |
| implementation_readiness_report.md#27 | One term for "no room", "no permit", `Busy`, "transient"? | fix (with critic#29) |
| implementation_readiness_report.md#28 | State the rule that picks the suites of each row? | fix |
| implementation_readiness_report.md#29 | Name the tests that T043-2 and the others describe? | fix |

Taken together: four questions are the owner's (ADR-0092, the inventory's exit status, the cache share, the panic's
metric label); the code worth doing before approval is the shared permit pool, the subset pin test, the down-migration
guard test and the missing sentinels; the rest is a text pass. A third round, or a targeted re-read of the changed
sections, would follow that work.
