# Typed failures (draft, feature 035)

Closed sets, additive (D035-042). Step failures join `crates/domain/src/database/failure.rs`
`DatabaseFailure` (its `code()` and `parse()` lists); publication and API refusals map to the
existing REST `ErrorCode` set with a `reason`; every code is added to `sdks/error-catalog.json` and its
per-SDK copies. A new test pins the full `DatabaseFailure` set so a code cannot be added to one list
only. No message carries a value, a file location, an object key, a provider body or a credential.

| Code | Retryable | Raised when |
|---|---|---|
| `ICEBERG_TABLE_NOT_FOUND` | no | the table or the target ref does not exist (FR-035-011, FR-035-012) |
| `ICEBERG_UNSUPPORTED_TABLE` | no | format v3, non-Parquet default format, unsupported transform, no usable key for upsert (detail `reason`) |
| `ICEBERG_LOCATION_NOT_ALLOWED` | no | a table, data, delete or metadata location outside `warehouseLocations` |
| `ICEBERG_SCHEMA_MISMATCH` | no | a record does not map losslessly (detail `column`, `reason`: `MISSING_REQUIRED`, `UNKNOWN_FIELD`, `TYPE`, `SCALE_OVERFLOW`, `OVERFLOW`, `ZONE`, `NULL_IN_REQUIRED`) |
| `ICEBERG_DUPLICATE_KEY` | no | a batch holds the same key twice (marked or not) |
| `ICEBERG_SCOPE_VIOLATION` | no | a row outside the filter scope, an empty batch without `allowEmpty` |
| `ICEBERG_COMMIT_CONFLICT` | yes | conflicts persisted past `maxCommitRetries` |
| `ICEBERG_REVERT_BLOCKED` | no | a later commit rewrote or removed the step's files, or a file to restore is gone |
| `ICEBERG_LIMIT_EXCEEDED` | no | records, bytes, files or columns above the effective limit |
| `CATALOG_UNAVAILABLE` | yes | catalog, metastore or catalog database unreachable or 5xx without a commit sent |
| `STORAGE_UNAVAILABLE` | yes | object storage unreachable or throttling |
| `DESTINATION_KEY_CONSTRAINT_MISSING` | no | PostgreSQL key without a primary key or unique constraint |
| `DESTINATION_SCHEMA_MISMATCH` | no | a PostgreSQL or ClickHouse column cannot hold a value, or is missing |
| `DESTINATION_DEDUPLICATION_NOT_CONFIRMED` | no | ClickHouse retry outside the confirmed window (becomes `OUTCOME_UNKNOWN` when uncertain) |
| `DESTINATION_OBJECT_CONFLICT` | no | an object with the step's name and a different checksum exists |
| `DESTINATION_DELIVERY_BOUND_EXCEEDED` | no | a broker message above 65,536 bytes, more messages than the destination's limit, or more delivery bytes than the per-completion cap |
| `DESTINATION_SUBSCRIPTION_SHARED` | no | a broker subscription that also carries run events |
| `DESTINATION_AUTH_FAILED` | no | the destination refused the credentials |
| `DESTINATION_UNAVAILABLE` | yes | the destination is unreachable or overloaded |
| `DESTINATION_SUBSCRIPTION_UNUSABLE` | no | a `STEP_OUTPUT` subscription disabled, deleted or below a field's level when the source completes |
| `DESTINATION_FAILED` | no | the run-level failure raised by a destination whose policy is `fail` (detail: the destination's own code) |
| `LAKE_HOST_PROTOCOL` | yes | the sandboxed host broke `sagat.lake.v1` (no result line, malformed or extra fields) |
| `LAKE_SANDBOX_UNAVAILABLE` | yes | the sandbox runner could not start the attempt's container |
| `CLASSIFICATION_EGRESS_REFUSED` | no | a field above the connection's ceiling, or `secret` (detail `field`, `level`) |
| `OUTCOME_UNKNOWN` | — | existing code; an inconclusive lookup or an unconfirmed deduplication after an uncertain write |

Existing 017 codes are reused for destination denial (`DATABASE_DESTINATION_DENIED`), TLS and
credential resolution failures. Publication refusals use `VALIDATION_FAILED` with reasons
`ICEBERG_NODE_NOT_ROOT`, `DESTINATION_NOT_ROOT`, `MODE_NOT_ALLOWED`, `CLASSIFICATION_EGRESS_REFUSED`,
`UNKNOWN_CONNECTION_KIND`, `BROKER_SOURCE_NOT_INLINE`, `FILTER_TOO_DEEP` (more than 8 levels or 64
comparisons), `LAKE_DISABLED` (the deployment's `lake.enabled` is off, at publication and admission).
An older engine that meets a version using these nodes refuses the run with `UNSUPPORTED_NODE`
(D035-059).
