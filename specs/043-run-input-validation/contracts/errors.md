# Errors — design 043, run input validation

No new `Error.code` (the closed set in `sdks/error-catalog.json` stays as it is). Design 043 adds `details.reason`
refinements of `VALIDATION_FAILED`, the closed `details.rule`, three failure codes of a run or a step, the schedule
action outcome `INPUT_INVALID` and seven publication issue codes. Every refusal carries `code`, `retryable`, `traceId`
and never a value of the input, a key the caller chose or a fragment of the schema's text. The remediation sentences
are the ones `crates/api-http/src/error.rs` answers.

## Reasons of a refused start

| `code` | HTTP | `details.reason` | Retryable | Raised by | Remediation (one sentence, closed table) |
| --- | --- | --- | --- | --- | --- |
| `VALIDATION_FAILED` | 422 | `RUN_INPUT_INVALID` | no | `POST /runs`, the uploaded-input start, Signal-with-Start and Update-with-Start (also with an open holder), `RERUN_AS_NEW`, reset, the ad hoc database execution | Fix the input at `details.field`: `details.rule` names the keyword of the input schema it fails (no excerpt). |
| `VALIDATION_FAILED` | 422 | `RUN_INPUT_SCHEMA_UNUSABLE` | no | the same, and the schedule writes and trigger-now | The version's input schema is outside the supported subset; publish a version within it (`control-plane input-schema-inventory` lists them). |
| `VALIDATION_FAILED` | 422 | `SCHEDULE_INPUT_INVALID` | no | schedule create and update (`details.field` under `/action/input`), trigger-now | The input does not match the target version's input schema (no excerpt). |
| `VALIDATION_FAILED` | 422 | `NOMINAL_TIME_FIELD_INVALID` | no | schedule create and update (`details.field` `/action/nominalTimeField`) | The field is not a declared top-level string property of the input schema. |
| `UNAVAILABLE` | 503 | (none) | yes | any of the above when the validator had no permit within 5 s or its backstop stopped it | The service cannot serve this call now: try again. |

`details.field` is a JSON pointer whose segments are only names the schema declares under `properties` and array
indices; a member reached through `additionalProperties` adds nothing, and `/` stands for the root. A run started again
(a rerun or a reset) says `/` and `other` unless the caller's view of the source input withholds nothing.

`details.rule` is one of `type`, `required`, `enum`, `const`, `minimum`, `maximum`, `exclusiveMinimum`,
`exclusiveMaximum`, `multipleOf`, `minLength`, `maxLength`, `pattern`, `minItems`, `maxItems`, `minProperties`,
`maxProperties`, `additionalProperties`, `anyOf`, `oneOf`, `workLimit` (the work budget of the validation), `maxDepth`
(the input nests deeper than 64) and `other`.

## What a creator nobody waits for records

| Creator | Outcome |
| --- | --- |
| Batch `RERUN` item | `FAILED` with `VALIDATION_FAILED`; not tried again |
| Pending reset | `ADMISSION_REFUSED`; not tried again |
| Schedule action, backfill step | action `INPUT_INVALID` with the reason `SCHEDULE_INPUT_INVALID`, `NOMINAL_TIME_FIELD_INVALID` or `RUN_INPUT_SCHEMA_UNUSABLE`; the schedule pauses when `pauseOnFailure` is set; a backfill ends `CANCELLED` |
| Event, webhook, message trigger | a delivery to the trigger with no run and one audit record `run.input_refused`; `trigger_events_total{outcome="input_invalid"}` or `{outcome="input_schema_unusable"}` |
| Cron trigger (legacy pass) | one audit record `run.input_refused` naming the tick's key |
| Child run | the parent step fails with `CHILD_INPUT_INVALID`, or `RUN_INPUT_SCHEMA_UNUSABLE` |
| Continue-as-new | the predecessor ends `FAILED` with `CONTINUATION_INPUT_INVALID`, or `RUN_INPUT_SCHEMA_UNUSABLE` |

## Publication issue codes

In `enforce` mode publication refuses an `inputSchema` outside the subset with one issue naming the keyword and its
pointer (`/inputSchema/...`); in `report` mode the same is a warning and the version is published. An `inputSchema` over
64 KiB or nested deeper than 32 is answered `CLASSIFICATION_TOO_COMPLEX` first, by the classification bound of feature
016 (G043-020).

| Issue code | What is wrong |
| --- | --- |
| `INPUT_SCHEMA_KEYWORD_UNSUPPORTED` | a keyword outside the subset, or one of the subset in a form it does not support |
| `INPUT_SCHEMA_REFERENCE_INVALID` | a `$ref` that is not a local pointer to a schema, has sibling keywords or closes a cycle |
| `INPUT_SCHEMA_PATTERN_UNSUPPORTED` | a `pattern` outside the syntax allowlist or past its size, states or count |
| `INPUT_SCHEMA_TOO_LARGE` | more than 64 KiB of schema (at start and in the inventory) |
| `INPUT_SCHEMA_TOO_DEEP` | nested deeper than 32, or an application path longer than 128 |
| `INPUT_SCHEMA_TOO_COSTLY` | every input the schema accepts would cost more than the work budget |
| `INPUT_SCHEMA_NAME_TOO_LONG` | a name of `properties` or `required` longer than 1 KiB |
| `INPUT_SCHEMA_APPLIES_TO_TRIGGERS` (warning) | the version has a non-manual trigger: the schema applies to the event envelope `{event, data}` |
