# Errors and failure codes (draft)

No new `Error.code`: every REST refusal uses the closed set of `sdks/error-catalog.json` and the OpenAPI envelope
(`code`, `message`, `retryable`, `traceId`, `requestId`, `remediation`, `details`). Messages come from a closed table and
never quote a value, an envelope member other than the pointer pattern, a key reference or a codec-server answer. Other
drafts use these codes for encoded fields (registry, coordinator C7); reason names follow C16.

## REST `details.reason` values

| Code | `details.reason` | Where | `details` also |
|---|---|---|---|
| `VALIDATION_FAILED` (422) | `ENCODING` | publish, validate (incl. `ENCODING_NOT_ACTIVATED`) | `issues[] {path, code, message}`, at most 20 |
| `VALIDATION_FAILED` (422) | `ENCODING_REQUIRED` | every REST write path of the registry (startRun, signalRun, signalExecutionChain, 018 with-start and schedules, 022 standalone, 026 public submission) | `pointer` (pattern), `carrier` |
| `VALIDATION_FAILED` (422) | `ENVELOPE_INVALID` | same, and run-input blob bind | `pointer`, `carrier` |
| `VALIDATION_FAILED` (422) | `ENVELOPE_LEVEL_MISMATCH` | same | `pointer`, `carrier` |
| `VALIDATION_FAILED` (422) | `ENVELOPE_CONTEXT_MISMATCH` | same | `pointer`, `carrier` |
| `VALIDATION_FAILED` (422) | `ENCODING_CARRIAGE_MISMATCH` | same (envelope at an undeclared location) | `pointer`, `carrier` |
| `VALIDATION_FAILED` (422) | `ENCODING_NOT_PRODUCIBLE` | 018 schedule whose `nominalTimeField` is encoded | `pointer` |
| `VALIDATION_FAILED` (422) | `CODEC_DESTINATION_DENIED` | putCodecEndpoint (origin not in the operator list) | — |
| `FORBIDDEN` (403) | `PII_REVEAL_NOT_GRANTED` (016) | putCodecEndpoint with `levelCeiling: pii`; issueCodecAssertion with `reveal` | — |
| `CONFLICT` (409) | `CODEC_DESTINATION_DENIED` | issueCodecAssertion (stored origin no longer listed) | — |
| `CONFLICT` (409) | `CODEC_ENDPOINT_NOT_CONFIGURED` | issueCodecAssertion | — |
| `CONFLICT` (409) | `CODEC_CEILING_UNCONFIRMED` | issueCodecAssertion at `pii` after the ceiling's setter lost the grant (C9) | — |
| `CONFLICT` (409) | `REVISION_CONFLICT` (existing reason) | putCodecEndpoint | — |
| `CONFLICT` (409) | `KEY_USAGE_SCAN_RUNNING` | startKeyUsageScan | — |
| `UNAVAILABLE` (503) | — | assertion signing key absent, audit failure | — |

## Journal step and run failure codes (system codes, `crates/domain/src/execution/event.rs`)

| Code | Retryable | Raised by |
|---|---|---|
| `ENCODING_REQUIRED` | no | completion, side channels (checkpoint, stream append), child and handler admission (parent step), `end` / `continueAsNew` (run) |
| `ENVELOPE_INVALID` | no | same paths |
| `ENVELOPE_LEVEL_MISMATCH` | no | same paths |
| `ENVELOPE_CONTEXT_MISMATCH` | no | same paths |
| `ENCODING_CARRIAGE_MISMATCH` | no | same paths, and cross-document carriage at admission (child, 022 handler child and `fromRun` pins, 030 handler) |

Inbox events refused by a wait record `event_deliveries.refusal_code` with one of these five values. A refused trigger
or schedule start writes `run.encoding_refused` with the code as reason and creates no run. A refused checkpoint does
not prevent the lease renewal (019 FR-019-015); the heartbeat answer states the refusal.

## Worker gRPC

`WriteBlob` of an envelope-typed blob without a valid header: `INVALID_ARGUMENT`, `x-sagat-error: ENVELOPE_INVALID`.
Completion refusals are not RPC errors (the report is acknowledged; the step fails).

## SDK catalog additions (`sdks/error-catalog.json`, origin `sdk`)

`PAYLOAD_CODEC_UNAVAILABLE` (retryable), `PAYLOAD_KEY_UNAVAILABLE`, `PAYLOAD_DECODE_FAILED`, `PAYLOAD_DECODED_TOO_LARGE`,
`PAYLOAD_ENCODE_FAILED` (not retryable), each with a payload-free remediation; `SECRET_IN_OUTPUT` keeps its server
meaning and is raised locally by the worker host before encoding.

## Codec-server item errors

`ENVELOPE_INVALID`, `ENVELOPE_UNSUPPORTED`, `CONTEXT_MISMATCH`, `KEY_NOT_ALLOWED`, `KEY_UNAVAILABLE`,
`LEVEL_NOT_PERMITTED`, `DECODE_FAILED`, `DECODED_TOO_LARGE`, `ENCODE_FAILED`
([codec-server.openapi.yaml](codec-server.openapi.yaml)). The Studio and the CLI map whole-request failures to
`CODEC_UNAVAILABLE`, `CODEC_SESSION_REQUIRED` (401 without a customer session), `CODEC_UNAUTHORIZED`,
`CODEC_CORS_REFUSED`, `CODEC_TIMEOUT`, `CODEC_RESPONSE_INVALID` (client-side states, not server codes).
