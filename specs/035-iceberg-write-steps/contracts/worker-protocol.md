# Worker protocol additions (draft, feature 035)

Additive changes to `specs/001-platform-foundation/contracts/worker.proto`, negotiated as the next free
minor when this feature merges (D035-043; drafts 020, 021, 025 and 026 also claim 1.7).

- **Activities**: all use the reserved prefix `sagat.database.` and are claimable only by
  `Role::DatabaseRunner`.
  - **Lake profile**: `sagat.database.iceberg.write`, `sagat.database.iceberg.revert`,
    `sagat.database.destination.iceberg`, `sagat.database.destination.object_storage`,
    `sagat.database.lake.probe`, `sagat.database.lake.browse`.
  - **Database profile**: the existing `sagat.database.postgresql`, plus
    `sagat.database.destination.postgresql`, `sagat.database.destination.clickhouse`,
    `sagat.database.clickhouse.probe`.
- **Profiles and versions** (D035-057):
  - Each `DatabaseRunner` worker binding names a profile, `database` or `lake`.
  - `managed_capabilities_allowed` accepts only that profile's activities, and only those the
    negotiated minor includes.
  - A runner offers the new activities only when the agreed minor includes them, so a new runner
    against an old server still registers for PostgreSQL. An old runner never sees them.
  - `ResolveDatabase` refuses a connection kind outside the caller's profile.
- **`TaskLease` input** for these activities: the existing `input` payload carries the attempt input
  of [lake-runtime.md](lake-runtime.md), minus credentials and the relay. That is `effectKey`,
  `firstAttemptStartedAt`, the table or target, mode, mapping, limits, and record references (blob
  ids or inline records). Credentials never travel in the lease.
- **`ResolvedDatabase`** gains three fields, `IcebergConnection iceberg`,
  `ObjectStorageConnection object_storage` and `ClickHouseConnection clickhouse`. Their numbers are
  assigned at merge and never reused. Each holds the pinned non-secret settings and a
  `map<string, bytes> credentials`, resolved through the same two-phase OpenBao path as
  `PostgresConnection`. Every secret of the connection (for Iceberg, `catalogSecret` and
  `storageSecret`) is compared, pinned and audited separately. The runner zeroises them when the
  attempt ends.
- **`ReadBlob`** is unchanged; the lake profile uses it for record blobs (lease-checked, audited).
- **Failures**: `FailStep` carries the codes of [errors.md](errors.md), which the server parses with
  `DatabaseFailure::parse`, as for 017 failures.
