# Error codes, reasons, permissions and small additive changes (draft)

No new `Error.code` value is added to the REST envelope: every refusal refines an existing code with
`details.reason`, as features 014 and 016 did. The SDK catalog gains one SDK-origin code.

## REST reasons

| Code | Reason | Where |
|---|---|---|
| `CONFLICT` | `TIME_ADVANCE_BLOCKED` (`details.blockedBy`: counts per closed kind) | testing clock advance/skip, no stop made |
| `CONFLICT` | `CLOCK_EPOCH_CHANGED` (`details.epoch`) | testing clock with `expectedEpoch` |
| `CONFLICT` | `RUN_WAITING_EXTERNAL` (raised by SDK test environments from a `WAITING_EXTERNAL` report) | SDK `waitForCompletion(skipTime)` |
| `VALIDATION_FAILED` | `CLOCK_CEILING`, `NON_POSITIVE_DURATION`, `TARGET_IN_PAST` | testing clock |
| `VALIDATION_FAILED` | `HISTORY_SEQUENCE_GAP`, `HISTORY_UNKNOWN_EVENT`, `HISTORY_UNSUPPORTED_SCHEMA`, `HISTORY_MALFORMED` (`details.seq`) | history replay |
| `BAD_REQUEST` (HTTP 413) | `HISTORY_TOO_LARGE` | history replay (same code and status as feature 020's export, decision C16) |
| `NOT_FOUND` | `ROUTE_NOT_FOUND` (existing) | every `/testing/*` path on a production deployment |
| `CONFLICT` | `REVISION_CONFLICT` (existing), `BINDING_PAUSED`, `BINDING_NOT_PAUSED`, `JOB_NOT_DEAD`, `JOB_SUPERSEDED`, `CONFLICT_NOT_OPEN` | Git sync |
| `VALIDATION_FAILED` | `GIT_DESTINATION_DENIED` (the API host is not on the operator allowlist or the egress guard refuses it; decision C16), `GIT_SETTINGS_INVALID`, `SECRET_NOT_USABLE` (existing reason) | Git binding |
| `QUOTA_EXCEEDED` | `GIT_BINDINGS`, `GIT_PENDING_EXPORTS` | Git sync |
| `CONFLICT` | `NAME_TAKEN` | template import |
| `VALIDATION_FAILED` | `TEMPLATE_SIGNATURE_INVALID`, `TEMPLATE_INDEX_EXPIRED`, `TEMPLATE_INDEX_ROLLBACK`, `TEMPLATE_DIGEST_MISMATCH`, `TEMPLATE_KIND_UNSUPPORTED`, `TEMPLATE_PLATFORM_TOO_OLD`, `TEMPLATE_DOCUMENT_INVALID` | template import |
| `RATE_LIMITED` | `TEMPLATE_IMPORT_RATE` | template import |

## Start-up refusals (process exit 2, one line on standard error)

`TEST_DEPLOYMENT_DATABASE` (a production composition root found `sagat_deployment_profile`),
`WORKFLOW_TIME_FUNCTION_REPLACED` (the body of `sagat_workflow_from` is not the canonical one; also raised by the
engine's monitor round, which then stops the engine), and on the test server
`NOT_A_TEST_DATABASE` (the database has tenant rows and no marker) and `TEST_SERVER_REQUIRES_DEV` (`SAGAT_ENV` is not
`dev`).

## Job error codes (stored, closed set; shown in job listings)

`GIT_CREDENTIALS_INVALID`, `GIT_REPOSITORY_NOT_FOUND`, `GIT_BRANCH_PROTECTED`, `GIT_RATE_LIMITED`,
`GIT_PROVIDER_UNAVAILABLE`, `GIT_MANIFEST_INVALID`, `GIT_DOCUMENT_TOO_LARGE`, `GIT_SECRET_IN_DOCUMENT`,
`GIT_DESTINATION_DENIED`, `GIT_OUTCOME_UNKNOWN`, and the import-entry outcome `GIT_REVIEW_NOT_VERIFIED` (a
`DRAFT_AND_REQUEST` import that opened no request because the probe could not show required reviews). A provider's response body is never stored or shown; at most its HTTP status
class.

## SDK catalog (`sdks/error-catalog.json`, version 1 → 2, additive)

```json
{"code": "INTERCEPTOR_FAILED", "origin": "sdk", "http": [], "grpc": "INTERNAL", "retryable": false,
 "remediation": "An interceptor of the application failed: see the application's logs for the interceptor named in the message; the call was not repeated."}
```

SDK reasons: `AFTER_RESPONSE` (with `INTERCEPTOR_FAILED`), `PROTECTED_FIELD` (with `VALIDATION_FAILED`, for a
protected field, the payload value or a protected header),
`UNSUPPORTED_BY_SERVER` (existing; a test environment against a server without `virtual-clock`).

## `PlatformVersion` (additive)

```yaml
PlatformVersion:
  properties:
    capabilities:
      type: array
      description: >-
        Closed vocabulary that only grows. `git-sync` and `template-catalog` when enabled; `virtual-clock` and
        `history-replay` only on a test server. Absent on older servers (treat as empty).
      items: {type: string, enum: [git-sync, template-catalog, virtual-clock, history-replay]}
```

## Permission catalog (`crates/domain/src/identity.rs`, additive)

| Permission | Granted to | Note |
|---|---|---|
| `gitsync:manage` | admin | plus current `secret:use` on referenced secrets |
| `gitsync:read` | admin, developer, operator | |

`testing:clock` is **not** added to the catalog: the test server defines it and maps it to the admin and developer
roles of its own tenant; no production role holds it.

Reserved actor: `system:git-sync` (joins `system:provisioning|operator|engine|gateway`). A publication of a draft last
saved by it requires the namespace's approvals plus one (research D1432).

## Alert kinds (additive enum values)

`GIT_SYNC_STUCK` (oldest pending export older than 30 minutes), `GIT_SYNC_PAUSED` (a binding paused by the platform),
`GIT_SYNC_CONFLICTS` (open conflicts) join `AlertKind` (`crates/domain/src/operations.rs`), the
`OperationalAlertWebhook.kind` enumeration of `events.asyncapi.yaml` and the alerts operation of `openapi.yaml`. Each
alert carries counts, ages and the binding's namespace, never a document or a credential, and a runbook link
(`docs/runbooks/git-sync.md`).

## Audit actions

`testing.clock.advanced`, `testing.history.replayed`, `gitsync.binding.created`, `gitsync.binding.updated`,
`gitsync.binding.paused`, `gitsync.binding.resumed`, `gitsync.binding.deleted`, `gitsync.binding.probed`,
`gitsync.export.recorded`, `gitsync.export.dead`, `gitsync.job.redriven`, `gitsync.import.applied`,
`gitsync.import.conflicted`, `gitsync.conflict.resolved`, `gitsync.webhook.refused`, `template.imported`,
`template.import.refused`.
