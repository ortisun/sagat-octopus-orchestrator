# Architecture review of T022-022 (2026-11-20, third set: the gate of a run's start)

Reviewer: `architecture-reviewer` subagent, read-only, given the working-tree diff of T022-022 with the constitution,
AGENTS.md, the architecture principles, the spec, plan, data model and research, ADR-0050/0056/0088/0089/0093/0094/0096
and the migration checklist (not the author's conclusions). Layering, the ADR-0050 slots, the shard hash and the
migrations were found sound.

## Findings and resolutions

- B1: the gate runs in `Engine::start`, not inside `PgRunStore::create` as FR-022-059 and D022-031 say, and nothing
  recorded it; the fitness rule's text described a gate that is not there. **Resolved:** D022-049 supersedes D022-031's
  placement with its reasons (delegated by OD-2026-11-20-6); the rule `run_start_errors` refuses a domain start call
  outside `Engine::start` (with unit tests: refused callers named, methods, definitions and test modules not counted, an
  empty start point an error); `record_reset` refuses a gated definition in the domain; the script's text is corrected.
- B2: an owner decision was cited for the unlocked `queued_runs` count. **Fixed** as security 2 (G022-011, D022-050 item 3).
- M1: the start's decision is a parallel copy of the step's. **Recorded, delegated:** one core is built with T022-072, the
  change that gives a start its second gate (D022-050 item 5, G022-012).
- M2: no budget line for a gated start, and a second `UPDATE runs` per run-scope gate event. **Fixed in part:** new budget
  lines in `work_budget.rs` (`a_gated_start_costs_what_the_budget_says`): granted start (47, 1, 15), queued start (47, 1,
  12), completion that ends a gated start (58, 1, 13), each itemised against the plain path; the plain lines are unchanged.
  Folding the projection into the run's own `UPDATE` saved four statements but named the new column in every run write,
  which `delay::a_schema_that_predates_the_delay_has_nothing_to_release_and_the_pass_does_not_fail` and the schema-tolerance
  rule of `run_columns_sql` forbid (a process must run on a schema that predates a column); it stays a statement of its
  own, written only by a commit with a run-scope gate event.
- M3: refused admissions were never counted. **Fixed:** `start_refused` counts
  `concurrency_acquisitions_total{scope="run",outcome="refused_full"}` at the refusal and types it; unit test
  `a_refused_start_is_counted_and_typed`.
- M4: incomplete creator evidence. **Recorded:** as correctness M4.
- minor: `Option<&NodeId>` plus an attempt encodes the level loosely; run-level errors reuse step data; lock order across
  calls is implicit (`GateLevel::Run < Step`). **Recorded** for T022-072's shared core (G022-012). The reset refusal in the
  domain is **fixed**. Docs: `data-model.md` lists `runs.flow_control_gates`; the runbook gains section 5 (queued runs,
  the refusals, the rollback guard).
- nits: `run_gates_projection` wrapper and `step_gate_ceilings`' name (kept for now, renamed with G022-012); the collapsed
  SQL of `verify_run`; the admission span before commit (the existing pattern).
