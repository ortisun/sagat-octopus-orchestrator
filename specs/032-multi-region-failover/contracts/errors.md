# Error vocabulary — feature 032 (draft, not implemented)

No new `Error.code` value and no change to `sdks/error-catalog.json` codes (research D1577). New closed
values of `details.reason` (REST) and `x-sagat-error` (gRPC) refine existing codes; `remediation` gets
one sentence per reason from the closed table of feature 008. T032-050 adds them to the OpenAPI
`Error.details` description and to `sdks/SPEC.md` §2.

| Surface | Code (HTTP / gRPC) | Reason | Retryable | Meaning | Remediation sentence (closed table) |
|---|---|---|---|---|---|
| REST, webhook ingress, worker API | `UNAVAILABLE` (503 / `UNAVAILABLE`) | `REGION_PASSIVE` | yes | this region is a standby of its cell | "This region is a standby: retry after Retry-After through the cell endpoint." |
| REST admissions, worker claims | `UNAVAILABLE` (503 / `UNAVAILABLE`) | `REGION_DRAINING` | yes | the active region is handing over | "The region is handing over to another: retry after Retry-After through the cell endpoint." |
| REST, webhook ingress, worker API | `UNAVAILABLE` (503 / `UNAVAILABLE`) | `REGION_SUPERSEDED` | yes | this process saw a newer epoch of its cell and stopped | "This region was replaced by another: retry through the cell endpoint." |
| REST placement, namespace creation | `VALIDATION_FAILED` (422 / —) | `PLACEMENT_RESIDENCY_CONFLICT` | no | `allowedRegions` excludes a region the namespace's data is already in | "The namespace's data is replicated to a region the declaration excludes: include every region of its cell." |
| REST placement | `CONFLICT` (409 / —) | `REVISION_CONFLICT` (existing) | no | stale revision | existing sentence |
| Worker `ReadBlob`, engine input rendering | `UNAVAILABLE` (503 / `UNAVAILABLE`) | `BLOB_NOT_REPLICATED` | yes | the object of a committed blob is not yet in this region's store, within the replica grace after an activation | "The content has not reached this region yet: retry later." |
| Step failure code (journal, `FailStep` outcome recorded by the engine) | — | `BLOB_LOST` | no | the object is still missing after the grace | the run follows its failure and compensation policy; the operator restores the object from the other region's bucket and reruns |
| REST commits in `sync_quorum` cells | `UNAVAILABLE` (503 / `UNAVAILABLE`) | `COMMIT_OUTCOME_UNKNOWN` | yes, only with the same idempotency key | the commit deadline expired while the database waited for its cross-region standby | "The platform could not confirm the change: repeat the call with the same idempotency key." |
| REST, webhook ingress, worker API in `sync_quorum` cells | `UNAVAILABLE` (503 / `UNAVAILABLE`) | `DURABILITY_UNCONFIRMED` | yes | the process's durability lease lapsed (no quorum-waited beacon commit within the lease) or its barrier after a start, reconnect or database restart has not completed; outcome reads, replays and admissions are refused | "The platform cannot confirm that its data is replicated right now: retry later through the cell endpoint." |

Rules:

- A `REGION_*` refusal is decided before authentication of a write and before any database statement;
  it never names the active region, another tenant or the database.
- The SDK retry rule (`sdks/SPEC.md` §5) is unchanged: a reason never makes a call repeatable; keyed calls
  are repeated on `UNAVAILABLE` as today. The only new SDK behaviour is closing pooled connections and
  resolving the endpoint again on a `REGION_*` reason (FR-032-025).
- `COMMIT_OUTCOME_UNKNOWN` is also what a keyed retry must tolerate: the original result if the commit
  survived, a new result if it did not; never two results. In a `sync_quorum` cell the original result is
  returned only after a quorum-waited commit of the replaying transaction (FR-032-009 b).
- Database-level refusals of an epoch-unaware binary (the lease trigger's `insufficient_privilege`, a
  read-only transaction's `25006`) are not API reasons: such a binary is not supposed to run; they surface
  as its own internal errors and in the alert `SagatRegionRoleMismatch` when a region-aware process sees
  them.

## Operator command reasons (stdout JSON `reason`, exit code 2 unless noted)

`ALREADY_INITIALIZED`, `NOT_INITIALIZED`, `IN_RECOVERY`, `EPOCH_CONFLICT`,
`NOT_ACTIVE`, `NOT_DRAINING`, `TARGET_NOT_IN_CELL`, `TARGET_IS_ACTIVE`, `TOO_SOON`, `CATCH_UP_UNVERIFIED`,
`ATTESTATION_REQUIRED`, `PREFLIGHT_BLOCKED`, `ACTIVE_REMOVED`, `RESIDENCY_CONFLICT`, `TOO_MANY_NAMESPACES`,
`SAME_DATABASE`, `NOT_DIVERGED`, `REPORT_MISMATCH`, `NO_SINK_PIPELINE`, `INPUT_INVALID`; exit code 1 with `DATABASE_UNAVAILABLE`
or `OUTCOME_UNKNOWN`.
