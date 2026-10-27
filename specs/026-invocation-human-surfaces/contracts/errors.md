# Errors — feature 026 (draft)

No new `Error.code` value is added. Every refusal uses the closed set of
`specs/001-platform-foundation/contracts/openapi.yaml` (`Error.code`) and of `sdks/error-catalog.json`,
refined by `details.reason`. Reasons follow the shared naming (coordinator decision C16):
`<AREA>_INPUT_INVALID` for schema mismatches, `<AREA>_DESTINATION_DENIED` for egress denials. Encoded
(feature 031) fields use only feature 031's codes (decision C7). Messages are safe summaries: they never
contain a value, a token, an address, a schema fragment or another tenant's identifier. Merged into the
canonical contracts and the remediation table by T026-005.

| Surface | `code` (HTTP) | `details.reason` | When |
|---|---|---|---|
| Forms | `VALIDATION_FAILED` (422) | `FORM_HINT_INVALID` | publication/validate: unknown `x-sagat-form` key, wrong type, widget incompatible with the field type |
| Forms | `VALIDATION_FAILED` (422) | `ENCODING_READ_BY_FORM` (feature 031) | publication: an `x-sagat-form` hint, approval `form` or `summary` reads an encoded location |
| Forms | — (answer) | `NO_SCHEMA`, `UNSUPPORTED_REQUIRED_FIELD`, `ENCODING_READ_BY_FORM`, `TOO_COMPLEX` | `FormModel.status = UNAVAILABLE`; an answer, not an error |
| Invocation | `RATE_LIMITED` (429) | `WAITER_LIMIT` | waiter ceiling of subject, tenant or process reached; checked before admission |
| Invocation | `VALIDATION_FAILED` (422) | `WAIT_OUT_OF_RANGE` | `waitMs` outside 0..ceiling |
| Invocation | `CONFLICT` (409) | `IDEMPOTENCY_KEY_REUSED` | existing reason; same key, other body |
| Streams (REST) | `RATE_LIMITED` (429) | `STREAM_SUBSCRIPTION_LIMIT` | open stream ceiling |
| Streams (REST) | — (feed event) | — | `event: reset` for a cursor of another cell epoch; not an error |
| Streams (gRPC) | see [worker-protocol.md](./worker-protocol.md) | `STREAM_EVENT_INVALID`, `STREAM_LIMIT` (with `scope`), `STREAM_RATE` | append refusals |
| Streams (publication) | `VALIDATION_FAILED` (422) | `STREAM_CHANNEL_UNDECLARED`, `STREAM_CHANNEL_RESERVED`, `STREAM_SCHEMA_OPEN`, `STREAM_SCHEMA_TOO_LARGE`, `STREAM_PROGRESS_SHAPE`, `STREAM_BUDGET_TOO_LARGE` | IR checks of FR-026-013 (a `sagat.` name declared by an author; budgets above the channel ceiling or summing above the author pool) |
| Public links (management) | `QUOTA_EXCEEDED` (429) | existing quota reasons | `public_links` count (default 0) |
| Public links (management) | `VALIDATION_FAILED` (422) | `LINK_FIELD_NOT_AVAILABLE`, `LINK_SECRET_FIELD`, `ENCODING_READ_BY_FORM` (feature 031), `LINK_ORIGIN_INVALID`, `LINK_EXPIRY_OUT_OF_RANGE`, `LINK_PRESET_INVALID`, `LINK_CAPACITY_OUT_OF_RANGE` | exposed field not in an `AVAILABLE` model, a `secret` or encoded field exposed or preset, a non-https origin, expiry beyond ceiling, preset not valid against the schema, live-run cap missing or above its ceiling |
| Public links (management) | `CONFLICT` (409) | `PUBLIC_ORIGIN_NOT_CONFIGURED` | the deployment has no public-forms origin |
| Public links (management) | `FORBIDDEN` (403) | `OWNER_CANNOT_START` | the creator lacks `run:start` (or, with feature 029, cannot see the workflow) |
| Public links (management) | `CONFLICT` (409) | `REVISION_CONFLICT` | existing reason; stale `expectedRevision` |
| Public (anonymous) | `NOT_FOUND` (404) | none | every unusable link or receipt, indistinguishably; every `/v1/public/*` path on the Studio origin |
| Public (anonymous) | `RATE_LIMITED` (429) | none | local guard, durable buckets, the link's live-run cap or the namespace public-link pool; `Retry-After` |
| Public (anonymous) | `VALIDATION_FAILED` (422) | `FORM_INPUT_INVALID` with pointers | field not exposed, or schema violation; no value echoed |
| Public (anonymous) | `QUOTA_EXCEEDED` (429) | none | content or active-run quotas of the tenant (no detail to anonymous callers) |
| Approval links | `NOT_FOUND` (404) | none | unknown, foreign, malformed, MAC-invalid, expired or closed token; with feature 029, a workflow the caller cannot see |
| Decisions | `VALIDATION_FAILED` (422) | `DECISION_INPUT_INVALID` with pointers | form input invalid, or missing for the chosen decision (`requiredFor`) |
| Decisions | `FORBIDDEN` (403) | existing `NOT_ELIGIBLE`, `INITIATOR_CANNOT_DECIDE` | unchanged feature-014 checks |
| Approval summary (publication) | `VALIDATION_FAILED` (422) | `SUMMARY_LEVEL_BELOW_SOURCE` | a summary field annotated below the level of a location its expression reads |
| Channels | `FORBIDDEN` (403) | `CLASSIFICATION_CEILING_NOT_GRANTED` | `maxClassification` `internal` without `payload:read-internal`, or `pii` without the reveal grant |
| Channels | `VALIDATION_FAILED` (422) | `CHANNEL_DESTINATION_INVALID`, `CHANNEL_KIND_INVALID` | malformed settings, kind not allowed for the destination |
| Channels | `VALIDATION_FAILED` (422) | `NOTIFICATION_DESTINATION_DENIED` | the namespace egress policy refuses the destination at creation |
| Channels | `CONFLICT` (409) | `CHANNEL_ALREADY_BOUND` | an interactive Slack `(team, channel)` already bound (no information about the other tenant) |
| Channels | `QUOTA_EXCEEDED` (429) | existing quota reasons | `notification_channels`, `slack_app_bindings` |
| Slack app bindings | `VALIDATION_FAILED` (422) | `SLACK_APP_UNVERIFIED` | `auth.test` failed, or returned another team than the binding already holds |
| Slack app bindings | `UNAVAILABLE` (503) | none | Slack unreachable during `auth.test` (retryable) |
| Slack requests | `UNAUTHENTICATED` (401) | none | any signature, timestamp, format or unknown-binding failure, indistinguishably |
| Chat identity | `RATE_LIMITED` (429) | `LINK_CODE_LIMIT` | codes issued per subject, or code attempts per Slack team |
| OpenAPI | `NOT_FOUND` (404) | none | version missing or foreign |

One SDK-origin entry is added to `sdks/error-catalog.json`: `STREAM_BUFFER_FULL` (origin `sdk`, not
retryable), raised locally when a worker's bounded stream buffer is full ([worker-protocol.md](./worker-protocol.md)).

Run outcomes carry the run's own closed failure code in `RunOutcome.failureCode`; that is data of the
run, not an error of the call. Delivery attempts reuse the closed ADR-0025 attempt outcomes
(`DELIVERED`, `REJECTED`, `SERVER_ERROR`, `THROTTLED`, `TIMEOUT`, `NETWORK`, `DENIED`, `UNSIGNED`,
`CREDENTIAL_UNAVAILABLE`); a provider's own error text is never stored or logged. A channel paused by the
`pii` egress rule records `disabled_reason = PII_GRANT_REVOKED`, not an error to a caller.
