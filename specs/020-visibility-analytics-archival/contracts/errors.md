# Errors and closed reasons (draft, feature 020)

No new `Error.code` value is added. Every failure uses the closed set of
`specs/001-platform-foundation/contracts/openapi.yaml#/components/schemas/Error` and of
`sdks/error-catalog.json`, refined by `details.reason`. Messages never contain a query literal, an
attribute value, a record value, a destination URL, a credential or another tenant's data. Merged into
the `Error.details` description, `docs/reference/api.md` and the SDK remediation tables by T020-010 and T020-012.

| `code` | `details.reason` | When |
|---|---|---|
| `VALIDATION_FAILED` | `SEARCH_ATTRIBUTE_UNKNOWN` | A query, start value, upsert or view names an attribute the registry does not have |
| `VALIDATION_FAILED` | `SEARCH_ATTRIBUTE_TYPE_CONFLICT` | A declaration's type differs from the registry, or two publications race with different types |
| `VALIDATION_FAILED` | `SEARCH_ATTRIBUTE_RETIRED` | A declaration, start value or query names a retired attribute |
| `VALIDATION_FAILED` | `SEARCH_ATTRIBUTE_VALUE_INVALID` | An explicit start value has the wrong type, size or element count |
| `VALIDATION_FAILED` | `CLASSIFICATION` (existing, FR-016-022) | A declared level is looser than the registry or than the carrier paths its expression reads; `secret` declared |
| `VALIDATION_FAILED` | `ENCODING_READ_BY_SEARCH_ATTRIBUTE`, `ENCODING_READ_BY_ANALYTICS` (owned by 031, FR-031-006) | A source, upsert or projection reads an encoded location; 020 calls 031's check |
| `VALIDATION_FAILED` | `QUERY_SYNTAX`, `QUERY_TYPE`, `QUERY_UNSUPPORTED_OPERATOR`, `QUERY_LIMIT` | The query does not parse, a literal has the wrong type, an operator is not defined for the type, a bound is exceeded |
| `QUOTA_EXCEEDED` | `SEARCH_ATTRIBUTE_LIMIT`, `QUERY_BUDGET_EXCEEDED`, `SINK_LIMIT`, `VIEW_LIMIT`, `ANALYTICS_EXPORT_BYTES` | A count ceiling or the query statement budget is exhausted |
| `RATE_LIMITED` | `SEARCH_CONCURRENCY` | Concurrent searches of the namespace at the ceiling (`Retry-After`) |
| `BAD_REQUEST` | `CURSOR_MISMATCH` | A cursor reused with another query, order or namespace |
| `BAD_REQUEST` (HTTP 413) | `HISTORY_TOO_LARGE` | The history exceeds the export event or byte bound; same code and status as 028 |
| `FORBIDDEN` | `PII_REVEAL_NOT_GRANTED` (existing) | A `pii` attribute or value without the grant or without `reveal: ["pii"]` |
| `FORBIDDEN` | `VIEW_NOT_EVALUABLE` | A saved view uses an attribute the viewer may not use; `details.attribute` names it |
| `FORBIDDEN` | `SINK_DISCLOSURE_NOT_GRANTED` | A sink revision asks for `PII` (or a retention policy for `UP_TO_PII`) by a principal without the grant |
| `CONFLICT` | `REVISION_CONFLICT` (existing) | Registry, view, sink or retention revision mismatch |
| `CONFLICT` | `SINK_STATE_CONFLICT` | An action not valid in the sink's state (resume an `ACTIVE` sink, skip without an open batch) |
| `CONFLICT` | `RUN_NOT_TERMINAL` | Erasure of a run that is not terminal or whose tree is not |
| `CONFLICT` | `EFFECT_UNRESOLVED` | Erasure of a run ended `COMPENSATION_FAILED`/`PARTIAL` or with an unresolved outcome or review; record the resolution first |
| `CONFLICT` | `RUN_PURGED` | Rerun or reset (018) of a purged or purging run, checked after the ADR-0039 identity guard; consumed by 018 and by the ADR-0039 rerun path (018's draft currently says `NOT_FOUND`; to be aligned) |
| `VALIDATION_FAILED` | `ENVELOPES_NOT_ENABLED` | A sink revision asks for `inbound_event`/`delivery` before envelope capture is enabled |
| `VALIDATION_FAILED` | `STREAM_SUBSCRIPTION_SHARED` | A `STREAM` sink names a subscription that also receives run events |
| `VALIDATION_FAILED` | `SINK_DESTINATION_DENIED` | The destination fails the egress policy or names a platform state endpoint (`details.category`, never the address) |
| `VALIDATION_FAILED` | `RETENTION_CEILING` | `hotRetentionDays` above the operator ceiling (the effective value is returned in `details`) |
| `NOT_FOUND` | `RUN_PURGED` | A hot-only endpoint (journal, graph, attempts, payload disclosure) on a purged run; `details.archive` is `AVAILABLE` or `NONE` |
| `UNAVAILABLE` | `ARCHIVE_UNAVAILABLE` | The archive store did not answer; retryable |
| `INTERNAL` | `ARCHIVE_INTEGRITY` | An archive object does not match its manifest; never retried automatically, the job is `BLOCKED` |

Worker-protocol (`AnalyticsExportService`) failures use the existing gRPC mapping and
`x-sagat-error` values: `UNAUTHENTICATED`; `PERMISSION_DENIED` + `NOT_BOUND` (no current binding or
role); `ABORTED` + `STALE_ATTEMPT` (lease expired, generation moved, batch superseded); `RESOURCE_EXHAUSTED`
(credit/rate, `retry-after-ms`); `UNAVAILABLE`; `INVALID_ARGUMENT` + `UNSUPPORTED_PROTOCOL`.

Sink states carry closed reasons (shown in the API, never free provider text): `SINK_REJECTED`,
`SINK_AUTH_FAILED`, `SINK_SCHEMA_MISMATCH`, `CREDENTIALS_UNUSABLE`, `DISCLOSURE_REVOKED`,
`SINK_DESTINATION_DENIED`, `TRANSIENT_FAILURES`, `EXPORT_QUOTA`, `USER_PAUSED`, `STREAM_DELIVERY_DEAD`,
`CONNECTION_UNUSABLE` (the 017 connection of a `POSTGRESQL` sink is paused, deleted or denied).

Archival job blocked reasons: `ARCHIVE_INTEGRITY`, `DESTINATION_UNUSABLE`, `SINK_HOLD`,
`TREE_NOT_TERMINAL`, `POLICY_REMOVED`.
