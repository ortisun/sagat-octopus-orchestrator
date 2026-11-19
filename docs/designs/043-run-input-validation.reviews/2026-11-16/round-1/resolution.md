# Round 1 of 2026-11-16: what answers each finding

The complete run (the five stages of `egm-review` in order and the three repository reviewers) read
`f562614874016262dd9812e241f734038d4d70de`. It ended with 2 Blockers and 77 Majors, so a second round reads the fixed
text. FIXED: a code change with a test seen failing first. TEXT: the design, a contract or ADR-0092 now says it. CONTEST:
the evidence says otherwise. OWNER: put to the owner (O043-005 to O043-007). Registers: D043-164 to D043-170, G043-055,
O043-007.

## Code (FIXED)

| Finding | Fix | Red first |
|---|---|---|
| security 1, correctness 1 | a panic of the compile or the validation is caught and answered `Interrupted` (D043-166) | `the_caller_whose_compile_panics_is_answered_interrupted` |
| security 2 | the parse's pre-check runs off the workers, at most four at once (D043-166) | `prechecks_outside_a_start_run_at_most_four_at_once` (16) |
| correctness 2 | a `CANCEL_SOURCE` reset checks the input at its request (D043-167) | `row_5_a_cancelling_reset_whose_input_the_version_refuses_is_refused_and_cancels_nothing` |
| correctness 3 | a child step is failed only if its reservation was not claimed, under the step's lock (D043-164) | both child tests, with the condition removed |
| correctness 4 | the goldens regenerated from the conformance run, each diff read (T043-15) | the golden had no `failureCode` |
| architecture 2 | row 13 parses in the process's mode (D043-169) | the `report` case of `integration_database_connections` |
| correctness 6 | a reporting recheck is counted (D043-168) | `a_reporting_recheck_is_counted` |
| correctness 9 | a tick whose key holds a run is not audited (D043-165) | `a_refused_tick_whose_run_a_reporting_peer_created_is_not_audited` |
| the run itself | `builders.json` moved a fractional `multipleOf` (outside the subset) to `default` (D043-170) | the conformance run stopped at publishing `canonical-edges` |

## Text

- Stage 5 Majors 1 to 3, Stage 8 Majors 1 and 2, structural 18, style 37: the subset, its allowlist, limits and charges
  are `contracts/input-schema-subset.md`; Appendix A is section 5 of `research.md`; line numbers are gone; Technical Plan
  4 is a summary; a Terms block defines holder, repeat, the schedule's vocabulary and the registry's closed gate. The
  gate as a constructor argument is ADR-0092's option 5, with why it lost (CONTEST of Stage 5 Major 2 for 0.3.0). The
  legacy continue-as-new is follow-up T043-26 (Stage 5 Major 3 offered a follow-up task).
- Stage 6 Majors 1 to 11: row 12 reads under the lock and checks after it is released (Technical Plans 2 and 5,
  NFR-043-PERF); the tasks are rewritten as a record with their real files, tests and order; Terms and links for the
  registry, the ADRs, holder and the schedule's vocabulary; the inventory runs before the migration.
- Stage 7 design Majors 1 to 7: `errors.md` says `/` always; `report` refuses the transient cases, said in FR-043-013 and
  CUJ 5; the charges and limits belong to the capability (Technical Plan 6); a Capacity paragraph states the limits;
  row II is "Pass with conditions", O043-007 states what a refusal of ADR-0092 changes; a 0.2.x readers table in
  Technical Plan 10; O043-005 restated honestly. Structural Majors 17 and 18 and style Majors 30 to 37: "no remote
  reference", no "exact" counter, the heaviest schema in MiB, `RUN_INPUT_BACKSTOP` as the bound, FR-043-016 a numbered
  list, "regardless of the caller's read permission", "repeat" defined, the paths not found in a paragraph of their own,
  the allowlist as lists in the contract.
- Stage 9 Blockers 24 and 36 and Majors 1 to 14, 23 to 37, 43 to 48: the Detailed Implementation Plan rewritten from the
  tree, with the real files, tests, migrations, order and phases; three follow-up tasks; the Verification Plan names
  every target.
- architecture 1 and 3: OWNER (O043-007 for ADR-0092; O043-005 for the 016/035 narrowing). The design states the built
  behaviour and what each answer changes.
- Nits: answered in the text where a sentence settles them (registers, the CLI help, the runbook's alerts and the
  `anyOf` of two shapes, the architecture principles row, the application README, Constitution row VI, the
  `details.rule` that may grow in `errors.md`); left as Nits with a reason: architecture 7 (string match of one
  constant), 8 (the `deadline` label is a published metric value), 11 (a `doc(hidden)` hook), 12 and 13 (one definition
  each would need a shared helper across crates); security 4 and 5 (bounded cost; the evaluator's output is bounded by
  the run's limits); correctness 7 and 8 (G043-055).
