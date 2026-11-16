# Errors — design 043, run input validation

No new `Error.code` (the closed set in `sdks/error-catalog.json` stays as it is). Design 043 adds `details.reason`
refinements of `VALIDATION_FAILED`, the closed `details.rule`, three failure codes of a run or a step, the schedule
action outcome `INPUT_INVALID` and seven publication issue codes. Every refusal carries `code`, `retryable`, `traceId`
and never a value of the input or a key the caller chose; `details.field` may name properties the schema declares, which
every caller allowed to start the run may read (G043-003). The remediation sentences are the ones
`crates/api-http/src/error.rs` answers. The subset itself is `input-schema-subset.md` beside this file.

## Reasons of a refused start

| `code` | HTTP | `details.reason` | Retryable | Raised by | Remediation (one sentence, closed table) |
| --- | --- | --- | --- | --- | --- |
| `VALIDATION_FAILED` | 422 | `RUN_INPUT_INVALID` | no | `POST /runs`, the uploaded-input start, Signal-with-Start and Update-with-Start (also with an open holder), `RERUN_AS_NEW`, reset (a `CANCEL_SOURCE` reset at its request, before the source is cancelled), the ad hoc database execution | Fix the input at `details.field`: `details.rule` names the keyword of the input schema it fails (no excerpt). |
| `VALIDATION_FAILED` | 422 | `RUN_INPUT_SCHEMA_UNUSABLE` | no | the same, and the schedule writes and trigger-now | The version's input schema is outside the supported subset; publish a version within it (`control-plane input-schema-inventory` lists them). |
| `VALIDATION_FAILED` | 422 | `SCHEDULE_INPUT_INVALID` | no | schedule create and update (`details.field` under `/action/input`), trigger-now (`details.field` `/`, `details.rule` `other`: the stored input is read only through FR-018-042) | The input does not match the target version's input schema (no excerpt). |
| `VALIDATION_FAILED` | 422 | `NOMINAL_TIME_FIELD_INVALID` | no | schedule create and update (`details.field` `/action/nominalTimeField`), trigger-now (`details.field` `/`) | The field is not a declared top-level string property of the input schema. |
| `UNAVAILABLE` | 503 | (none) | yes | any of the above when the validator had no permit within 5 s, its backstop stopped it, or it failed with a defect of its own; in `report` mode too | The service cannot serve this call now: try again. |
| `VALIDATION_FAILED` | 422 | an `INPUT_SCHEMA_*` issue (below) | no | the ad hoc database execution in `enforce` mode, whose document is parsed as a publication is | The input schema of the document is outside the supported subset: the issue names the keyword and its pointer. |

`details.field` is a JSON pointer whose segments are only names the schema declares under `properties` and array
indices; a member reached through `additionalProperties` adds nothing, and `/` stands for the root. A start whose input
the caller did not send (a rerun, a batch rerun, a reset, a pending reset, a trigger-now, a private rerun) always says
`/` and `other`, whatever the caller may read (design 043, D043-121, O043-006).

`details.rule` is one of `type`, `required`, `enum`, `const`, `minimum`, `maximum`, `exclusiveMinimum`,
`exclusiveMaximum`, `multipleOf`, `minLength`, `maxLength`, `pattern`, `minItems`, `maxItems`, `minProperties`,
`maxProperties`, `additionalProperties`, `anyOf`, `oneOf`, `workLimit` (the work budget of the validation), `maxDepth`
(the input nests deeper than 64) and `other`. A later subset may add values: a client passes one it does not know
through.

## What a creator nobody waits for records

| Creator | Outcome |
| --- | --- |
| Batch `RERUN` item | `FAILED` with `VALIDATION_FAILED`; not tried again |
| Pending reset | `ADMISSION_REFUSED`; not tried again |
| Schedule action, backfill step | action `INPUT_INVALID` with the reason `SCHEDULE_INPUT_INVALID`, `NOMINAL_TIME_FIELD_INVALID` or `RUN_INPUT_SCHEMA_UNUSABLE`; a scheduled or triggered action pauses the schedule when `pauseOnFailure` is set; a refused backfill step ends the backfill `CANCELLED` and does not pause it |
| Event, webhook, message trigger | a delivery to the trigger with no run and one audit record `run.input_refused`; `trigger_events_total{outcome="input_invalid"}` or `{outcome="input_schema_unusable"}` |
| Cron trigger (legacy pass) | one audit record `run.input_refused` naming the tick's key, written with the move of the trigger past the tick |
| Child run | the parent step fails with `CHILD_INPUT_INVALID`, or `RUN_INPUT_SCHEMA_UNUSABLE` |
| Continue-as-new | the predecessor ends `FAILED` with `CONTINUATION_INPUT_INVALID`, or `RUN_INPUT_SCHEMA_UNUSABLE`; the `Run` shows the code as `failureCode` |

`INPUT_INVALID` has two meanings on the platform. Here it is the outcome of a schedule action whose run input was
refused. In `sdks/SPEC.md` it is also a failure code a worker may give a step whose activity input it refused, which is
older than design 043 and unrelated to the run input: a step failed `INPUT_INVALID` is not a refused start.

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
