# Correctness review of design 043 (2026-11-15)

Doc: docs/designs/043-run-input-validation.md
Reviewed-commit: ce2e10e2eab95102e658d370dcef1fcd4a48faeb
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept, numbered across the
report. The reviewer read files only and ran nothing.

## Major

1. **A refused cron tick can be audited on every round, not "at most twice"** (lines 328, 568-571, 836; G043-002).
   `cron_pass` writes the audit (`record_refusal`), then advances the trigger in a separate statement whose failure is
   only logged; a failed advance or a crash between the two leaves the tick due and the next round audits it again, N
   engines give N audits. Fix: the audit and a compare-and-set advance in one transaction, the audit only when the
   advance moved the row; rename the test and the gap to "exactly once".
2. **In `report` mode, a reported Signal-with-Start, Update-with-Start or `USE_EXISTING` start that an open holder
   answers is counted but never audited** (lines 153-154, 302, 593-595, 950-952): the report audit is written only on
   the `Created` branch of `PgRunStore::create`. Fix: audit in the holder's transaction, or state the exception in
   FR-043-013 and ADR decision 10; add a with-start case to T043-22.
3. **The row 14 write check refuses schedules whose every action would pass** (lines 117-118, 560-562). The code fills
   `nominalTimeField` with the instant of the write, not the next nominal time (D043-125), so a time-shaped `pattern`
   fails at 14:03:27; `declares_string` reads only `properties.<name>.type`, so a field declared through `$ref` or
   `allOf` is refused `NOMINAL_TIME_FIELD_INVALID`. Fix: the next nominal time, the field resolved through the
   compiled form; the write check never stricter than the action check.
4. **A schema within the published bounds is never cached** (lines 198, 219-221, 476-477, 510-520): 26 to 32 patterns
   weigh more than the 16 MiB tenant share, so every start recompiles them under a permit, outside the counter and the
   backstop; NFR-043-PERF and SC-043-004 fail for it; the doc never mentions the share. (Same defect as security #2.)
5. **The repeat rule of the doc is not what was built, and has no test** (lines 105, 322-323, 380, 546-548, 816-818):
   FR-043-003 and T043-5 require `FieldPolicy::Show` when nothing is withheld; the code always passes `Root`.
6. **The doc and ADR-0090 still say tokio is only a dev-dependency of `sagat-application`** (lines 181, 397-401, 695,
   797, 805-806): see architecture #1; a new ADR is needed (AGENTS rule 2).
7. **FR-043-015 has no plan for getting `CONTINUATION_INPUT_INVALID` to an SDK** (lines 167-170, 647-659, 903-906;
   G043-046): no `Run` field, OpenAPI change, Studio type or SDK view change in Technical Plan 9 or T043-14/15.
8. **The subset contradicts feature 016's contract and breaks feature 035 tuple schemas in `enforce`**, and neither is
   among the contract updates (lines 449-451, 697, 709, 887-900): 016's contract lists `prefixItems` as supported and
   lets `patternProperties`, `not`, `if` appear in carriers; D043-095 and D043-110 record that a 035 Iceberg write with
   `prefixItems` publishes only in `report`. Fix: 016's contract in T043-14, a Risks row, ask the owner about the 035
   regression.
9. **The text lags the register on built behaviour**: D043-120 (unwired gate fails closed, engine creators retry
   forever), D043-122 (the legacy-rule exception), D043-126 / G043-041 (the schedule path reads and validates twice),
   D043-134 (a refused backfill does not pause), D043-142 (an exhausted schedule is not checked), D043-144 (trigger-now
   answers `/`), D043-146 (a trigger-now refused at start records nothing).

## Nit

10. "The one transient case" and `SchemaRefusal` leave out `Interrupted` and the unwired `Busy`; a compile panic is
    deterministic and retried forever.
11. Lines 404-406 and ADR decision 4: a tenant "is granted a third only while no other waits", not "may hold more".
12. CUJ 5: "counted `reported` once" is wrong; the gate counts every attempt, only the audit is once.
13. Lines 39 and 41, ADR Evidence: "seventh draft" and stale register ranges.
14. Line 943: a transient refusal of an event can end in the inbox's dead letter, so a starved validator can drop a
    trigger start for good.
15. D043-147's recheck-counting test was not seen red (AGENTS rule 18); the accepted-path count has no red-first test.

Counts: Blocker 0, Major 9, Nit 6
