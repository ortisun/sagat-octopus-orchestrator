# Targeted re-read 1 of design 043: egm-review lenses (2026-11-16)

Doc: docs/designs/043-run-input-validation.md
Reviewed-commit: a763d9d01d36acea3bb3cad1e6e861d7c4f5ce74
Reviewer: egm-review
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: the author's summary of two lenses run on the changed text (egm-comprehension and egm-implementation-readiness), a
targeted re-read, not a complete run of the five stages.

## egm-comprehension

The eight round-2 Majors are resolved. Nits: 1. TP5 still says a refused triggered action pauses without FR-043-006 item
6's wording; 2. "matchers", "application edges", "least work" still undefined in unchanged text; 3. the inventory's place in
the pre-check pool, the queue "owner", NFR-043-PERF's "the same pool"; 4. "count" in three senses, "the summary" not
introduced; 5. TP10 "if the answer warrants it"; 6. T043-13's stored-version sentence; 7. row 13's test named by an
unrelated test; 8. the suite rule does not cover non-row targets; 9. the Overview's "each refusal"; 10. the 503's problem
code not given.

## egm-implementation-readiness

12 of 15 Majors in scope resolved. Major: 1. `sagat workflow validate` (`apps/cli/src/workflow.rs`, `lib.rs`) missing from
T043-13 (round-2 6, 15); 2. the Verification Plan rule misses the sentinel and budget targets and non-row changes (round-2
28); 3. T043-5 to T043-11 and T043-20 describe tests without names (round-2 29). Nits: 4. `run_input_prechecks_total` in no
task or test; 5. T043-11 says each down migration has guards; 6. T043-24's read; 7. T043-15's test files; 8. T043-28 has no
threshold; 9. T043-12's tense.

Counts: Blocker 0, Major 3, Nit 19
