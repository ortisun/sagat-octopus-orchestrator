# Errors, codes and effect classes — feature 030 (draft)

No new `Error.code` is added: the closed set of `specs/001-platform-foundation/contracts/openapi.yaml`
(`BAD_REQUEST`, `UNAUTHENTICATED`, `FORBIDDEN`, `NOT_FOUND`, `CONFLICT`, `VALIDATION_FAILED`,
`RATE_LIMITED`, `QUOTA_EXCEEDED`, `UNAVAILABLE`, `INTERNAL`) is refined by `details.reason`, and
`sdks/error-catalog.json` is unchanged (D1506). Step failures of a `serviceCall` node use
`FailureCode` strings (`[A-Za-z0-9_.:-]`, 1–64), like `CHILD_FAILED`. Encoding findings use feature
031's codes and no code of this feature (C7); encoded values never cross namespaces in v1 (D1661). No message, reason or code ever contains an input,
a result, a schema fragment or text written by a caller.

## Effect classes and routing (D1512, FR-030-037)

Every settlement of a call records one effect class: `none` (no handler run was admitted), `failed`
(the handler run reported failure), `applied` (the handler run succeeded), `partial` (the handler run
ended `PARTIAL` or `COMPENSATION_FAILED`), `unknown` (a handler run was admitted and its outcome was not
known when the call settled). A failure whose class is `applied`, `partial` or `unknown` is
**effect-possible**: inside a saga it is planned for its `compensateWith` (or, if `irreversible`, the
saga ends `PARTIAL`); outside a saga the step enters the existing manual review (`step.needs_review`)
and is settled by `RESOLVE_STEP` or `RETRY_STEP` (a new call attempt). A failure with class `none` or
`failed` follows `onError`. When the caller run itself is being cancelled, the class is recorded and a
later outcome is kept for lookup; nothing is routed. `failed` means the handler run reported that the
operation did not complete; effects it made and did not undo are visible to the caller only through
its own saga outcome (`partial`), so a handler with external effects should use a saga.

## Caller step failure codes

| Code | When | Effect class | Route outside a saga |
|---|---|---|---|
| *declared code* (for example `CARD_DECLINED`) | The handler run failed with a code the operation declares in `errors` | `failed` | `onError` |
| `OPERATION_FAILED` | The handler run failed with an undeclared code | `failed` | `onError` |
| `OPERATION_CANCELLED` | The handler run ended `CANCELLED` (for example cancelled in its own namespace) | `failed` | `onError` |
| `OPERATION_TIMED_OUT` | The handler run ended `TIMED_OUT` | `failed` | `onError` |
| `OPERATION_TIMED_OUT` | The caller deadline passed after admission; the outcome arrived inside the lookup window | the handler's class | by class |
| `OPERATION_TIMED_OUT` | The caller deadline passed after admission and the lookup window passed without an outcome | `unknown` | manual review |
| `OPERATION_START_TIMED_OUT` | The start deadline passed while the `START` was still pending (withdrawn) | `none` | `onError` |
| `OPERATION_COMPENSATED` | The handler run ended `COMPENSATED` | `failed` | `onError` |
| `OPERATION_PARTIAL`, `OPERATION_COMPENSATION_FAILED` | The handler run ended with that saga outcome | `partial` | manual review |
| `OPERATION_RESULT_INVALID` | The handler run succeeded but its result did not validate against the output schema, exceeded 256 KiB or was a blob descriptor; no value crosses | `applied` | manual review |
| `OPERATION_ACCESS_REVOKED` | The caller namespace was removed from the allow-list, or the endpoint was deleted, before the handler run ended; no value and no failure code cross | the handler's class | by class |
| `OPERATION_INPUT_INVALID` | The input expression failed, did not validate against the pinned input schema, exceeded 256 KiB or was a blob descriptor | `none` | `onError` |
| `OPERATION_NOT_AUTHORIZED` | The caller version's publisher no longer holds `service:call` when the call is scheduled | `none` | `onError` |
| `OPERATION_QUOTA_EXCEEDED` | Caller-side quota (pending calls per namespace, active calls per run) at scheduling | `none` | `onError` |
| `OPERATION_NOT_ALLOWED` | Admission found the caller namespace absent from the allow-list | `none` | `onError` |
| `OPERATION_ENDPOINT_UNAVAILABLE` | Admission found the pinned endpoint deleted | `none` | `onError` |
| `OPERATION_CONTRACT_UNAVAILABLE` | Admission found the pinned contract revision not served | `none` | `onError` |
| `OPERATION_HANDLER_UNAVAILABLE` | The binding's workflow or version is missing, its schemas no longer equal the operation's, or it uses `continueAsNew` | `none` | `onError` |
| `ENCODING_CARRIAGE_MISMATCH`, `ENVELOPE_CONTEXT_MISMATCH` (031's codes) | Admission found an envelope in the input (encoded values do not cross namespaces, D1661) | `none` | `onError` |
| `ENCODING_CARRIAGE_MISMATCH`, `ENVELOPE_CONTEXT_MISMATCH` (031's codes) | The handler result held an envelope; the value is withheld | `applied` | manual review |
| `OPERATION_DEPTH_EXCEEDED` | The handler run would exceed the call-depth ceiling | `none` | `onError` |
| `EXPRESSION_ERROR`, `RUN_DATA_LIMIT`, `STORAGE_QUOTA_EXCEEDED` | Existing codes, with their existing meaning; `RUN_DATA_LIMIT` and `STORAGE_QUOTA_EXCEEDED` on a successful result keep class `applied` | as stated | by class |

A cancellation of the caller step records `none` (start withdrawn) or `unknown` (`abandon`,
`request_cancel` after admission); `wait_cancelled` records the handler's class.

Declared codes MUST NOT start with `OPERATION_`, `CHILD_` or `ENCODING_` and MUST NOT equal an existing
platform failure code (`WORKER_ERROR`, `LEASE_EXPIRED`, `ATTEMPT_TIMEOUT`, `EXPRESSION_ERROR`,
`WAIT_TIMED_OUT`, `BRANCH_FAILED`, `RUN_DATA_LIMIT`, `STORAGE_QUOTA_EXCEEDED`, `DATABASE_UNAVAILABLE`,
`OUTCOME_UNKNOWN` and the others in `crates/domain/src/execution/event.rs`).

## `details.reason` refinements (REST)

| `Error.code` | `details.reason` | Where |
|---|---|---|
| `VALIDATION_FAILED` | `SERVICE_CONTRACT` (with `pointer` and a closed `issue`: `OUTPUT_NOT_CLOSED`, `OUTPUT_UNBOUNDED`, `OUTPUT_SECRET`, `DISCLOSURE_MISSING`, `RESERVED_ERROR_CODE`, `TIMEOUT_CEILING`, `SCHEMA_TOO_LARGE`, `CLASSIFICATION`, `NAME_MISMATCH`) | `publishServiceRevision` |
| `VALIDATION_FAILED` | `ENCODING` with `ENCODING_NOT_APPLICABLE` | `publishServiceRevision`, for `x-sagat-encoding` anywhere in the operation schemas |
| `VALIDATION_FAILED` | `SERVICE_BINDING` (with `operation`, `contractRevision` and `issue`: `MISSING`, `SCHEMA_MISMATCH`, `SYNC_NOT_PINNED`, `SYNC_HANDLER_WAITS`, `HANDLER_CONTINUES`, `WORKFLOW_MISSING`, `VERSION_MISSING`) | endpoint create/update |
| `VALIDATION_FAILED` | `ENCODING` with `ENCODING_NOT_APPLICABLE` | endpoint create/update: the bound handler version declares encoding in its input or output schema |
| `VALIDATION_FAILED` | `SERVICE_CALL` (with `node` and `issue`: `OPERATION_MISSING`, `SERVICE_MISMATCH`, `REVISION_NOT_SERVED`, `SYNC_POLICY`, `SYNC_TIMEOUT`, `SAGA_COMPENSATION_REQUIRED`, `SAGA_POLICY`, `COMPENSATION_OPERATION_MISSING`, `INPUT_DISCLOSURE_MISSING`, `INPUT_CLASSIFICATION_LOOSENED`) | caller validate/publish |
| `VALIDATION_FAILED` | `ENCODING` with `ENCODING_CARRIAGE_MISMATCH` (`serviceCall.input` or `compensateWith.input` whose source holds an encoded location) | caller validate/publish |
| `VALIDATION_FAILED` | `UNSUPPORTED_NODE` with `dependency` `SCOPED_SERVICE_CALL` (inside a body) or `SERVICE_CALL_CAPABILITY` (capability disabled) | caller validate/publish |
| `FORBIDDEN` | `SERVICE_CALLER_NOT_ALLOWED` | caller publish (allow-list advisory check) |
| `FORBIDDEN` | existing permission reasons | all routes |
| `NOT_FOUND` | (no reason: foreign and missing are the same) | all routes, including a foreign allow-list namespace or endpoint name |
| `CONFLICT` | `SERVICE_ENDPOINT_NAME_TAKEN`, `REVISION_CONFLICT`, `SERVICE_IN_USE`, `MESSAGE_NOT_DEAD`, `ENDPOINT_DELETED`, existing `IDEMPOTENCY_KEY_REUSED` | endpoint routes, delete, redrive |
| `QUOTA_EXCEEDED` | existing `details.limit` naming `services`, `service_endpoints`, `service_revisions`, `allow_list_entries`, `endpoint_revisions` | registry writes |

## Closed sets used by metrics and listings

- Refusal reasons (`service_operations_refused_total{reason}`): `not_allowed`, `endpoint_unavailable`,
  `contract_unavailable`, `handler_unavailable`, `encoding_refused`, `depth_exceeded`.
- Deferral reasons (`deferral_reason`, `service_operations_deferred_total{reason}`): `endpoint_paused`,
  `namespace_active_runs`, `namespace_operations`, `endpoint_concurrency`, `endpoint_rate`, `transient`.
- Outcomes (`service_operations_finished_total{outcome}`): `succeeded`, `failed`, `cancelled`,
  `timed_out`, `start_timed_out`, `refused`, `access_revoked`, `result_invalid`.
- Effect classes (`effect` label and field): `none`, `failed`, `applied`, `partial`, `unknown`.
- Review reasons (`service_operations_reviews_total{reason}`): `timed_out`, `access_revoked`,
  `result_invalid`, `partial`, `cancelled`.
- Discard reasons: `caller_settled`, `caller_missing`.
