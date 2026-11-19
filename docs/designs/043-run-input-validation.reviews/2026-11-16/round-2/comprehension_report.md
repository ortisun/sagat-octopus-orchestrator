# egm-review Stage 6 (Comprehension Test), round 2

Doc: docs/designs/043-run-input-validation.md
Reviewed commit: b9b0f1b56d121ba3f977e2cea888305402f2eb47
Stage reviewer: egm-comprehension (FeatureRange 1-852, DetailedRange 853-1195)
Note: condensed by the author; the paraphrases are shortened, every finding and severity is kept.

## What the feature does (paraphrase)

Every new run's input is checked against its pinned version's `inputSchema` before creation, by one gate, on thirteen
paths plus the schedule write. A waiting caller gets `422` with a closed reason, pointer and rule, never the value; `/`
and `other` for inputs the caller did not send. Creators nobody waits for record one final outcome; capacity problems are
transient. A subset of JSON Schema, the platform's validator with a work budget, a pre-check at publication, an
inventory command; `enforce` or `report`, locked through the registry; a type only the gate builds and seven rules.

## How it will be built (paraphrase)

The tasks were built before approval in three phases; T043-1 to T043-24 by area, three follow-ups. Two points could not
be paraphrased with confidence: what an uploaded input is checked as, and what a "triggered action" is.

## Findings

1. Major (111-113): the closed `details.rule` enum is a public field the design never lists or links.
2. Major (9-22, 158-166): the Overview never mentions `enforce`/`report`; FR-043-011 uses them before FR-043-013.
3. Major (53, 255, 392, 931): the blob descriptor of row 2 is never defined.
4. Major (38, 395, 583-588, 1109): reset, pending reset, `CANCEL_SOURCE` and the completion are not explained.
5. Major (121, 160, 403, 565, 581, 835): ad hoc execution, connection probe, private rerun and `start_private` undefined.
6. Major (134-137, 593-596): "triggered action" pauses but "trigger-now records nothing": contradictory or undefined.
7. Major (484-491): the `ScheduleService` without a gate is described two ways.
8. Major (237-243, 526-548): NFR-043-BOUND's terms (application path, least work, programs, matchers) undefined.
9. to 24. Nits: undefined project terms; the cron handover; the child reservation; FR-043-011's row list; CUJ 5 vs the
   plain holder audit; eviction wording; the synchronous heading; two broken fragments (rows 12, line 631); O043-005
   opacity and "O043-006 (b)"; "keep their closed gate"; phase 2 branch and unticked meaning; `Mode` vs
   `InputValidationMode`, `validated_document_in` vs `parse_with`; the cache test in no task; T043-17's other suites;
   G043-003's premise; task details without context.

Counts: Blocker 0, Major 8, Nit 16
