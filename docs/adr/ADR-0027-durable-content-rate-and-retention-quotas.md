# ADR-0027: Durable content, log, rate and retention quotas

**Status:** Accepted; engineering baseline implemented and verified. Product tier values and customer/regulatory retention classification remain pending under G42.

**Traces:** FR-009-006, NFR-009-PERF, FR-004-008; G37, G80, G117. G42 remains a product decision about tier values and customer/regulatory retention classification.

## Context

Count quotas do not bound retained content. The API's subject/tenant request budgets and the webhook gateway's token buckets are local to a replica. The database is already the authority for event admission, durable state, and administrative limits. New limits must use that authority without preventing completion, audit, or delivery of obligations already committed.

This decision adds conservative engineering admission bounds. It does not authorize deletion of customer data, change existing retention of stored records, select commercial tiers, or claim a quota on physical database/object-store disk usage, indexes, WAL, replication or backups.

## Resource contracts

| Resource | Scope | Unit and behavior |
|---|---|---|
| `tenant_content_bytes` | Tenant aggregate | Retained tenant content plus outstanding content reservations; hard admission limit |
| `tenant_log_bytes` | Tenant aggregate | Retained redacted build/test/execution log bytes; storage beyond the limit is truncated, while terminal state commits |
| `api_requests_per_second` | Namespace | Durable token bucket for authenticated, namespace-scoped public API calls |
| `events_per_second` | Namespace | Durable token bucket for new inbox events from API, webhooks and connectors |
| `event_retention_seconds` | Namespace | Maximum TTL that a newly admitted event or newly configured endpoint may select |
| `script_retention_days` | Namespace | Maximum retention that a new/changed script, publication, test or execution record may select |

Namespace policy resolves namespace setting, then tenant default, then the operator ceiling. Tenant aggregate resources have no namespace override. Effective values are always clamped to the current ceiling. Existing operator-only ceiling administration and transactional audits are reused. New byte resources need a resource-specific maximum that fits PostgreSQL `bigint`; do not raise all count resources' `MAX_LIMIT` to accommodate bytes. Initial values are explicitly provisional. Existing rows above an initial/lowered limit remain readable and removable; positive growth is refused, while release/decrease remains possible.

## Retained-content classification

The implementation maintains a checked manifest, `crates/infra-postgres/storage-classification.json`, covering **every variable-size column** in the migrated tenant-owned schema. Each entry names its class, measuring expression or protected bound, and owner/write paths. A migration integration test compares this manifest with `pg_catalog`: a new or renamed `text`, `bytea`, JSON, array or other variable-size column cannot silently escape classification.

Classes are `content`, `log`, and `protected_system`. No column is exempt merely because its name looks like metadata. A protected classification requires a bounded value object, a closed generated schema or an explicit size check, plus the test that establishes that bound. The manifest is engineering accounting, not a privacy or regulatory classification.

Mandatory content coverage includes:

- Workflow draft JSON, immutable workflow documents and release notes; publication request/decision prose and user-entered activation reasons.
- Script drafts, immutable versions and test snapshots: source, lockfile, input/output schemas, test input/result and inferred schemas. Signed artifact content and SBOM bytes are included; content-addressed artifact rows count once per tenant, not once per referencing version.
- Runtime `payloads.content` (input, output, signals and manual resolutions) and separately retained `steps.child_input`. A content-addressed payload counts once per stored row, while a second physical logical copy in another table counts again.
- `inbox_events.payload`, retained source/event/subject/correlation values, and connector dead-letter body/headers/position. A duplicate event does not add a second content charge.
- Secret ciphertext and tenant-authored configuration: connector settings, subscription destinations, egress rules, endpoint identity configuration and any other variable-size customer-controlled configuration identified by the manifest. Secret accounting measures ciphertext without decrypting or logging it.
- External blobs: measured object bytes plus still-outstanding upload reservations, counted independently of their small catalog descriptor. The existing tenant/namespace blob transfer and upload-count limits remain additional bounds.

The log class includes `script_versions.build_log`, `script_test_runs.log`, and `script_executions.log`. Other arbitrary diagnostic text must join this class or the content class; it cannot become protected merely by being called an error. Closed failure codes, hashes, UUID representations and bounded protocol identities can be protected.

Immutable audit records, journal control facts, lease/fence metadata and scheduling/delivery obligations are protected system records. Their current builders must remain bounded and payload-free. Specifically, `event_outbox.payload` and `webhook_deliveries.payload` currently contain generated IDs/status/trace/alert metadata, not run input/output (`execution.rs::outbox_run_finished`, `operations.rs::alert_body`). Classify and test those closed schemas explicitly; adding arbitrary event bodies requires a new content reservation design. This preserves terminal state plus outbox atomicity at a full tenant content quota.

Measure bytes deterministically: `octet_length` for UTF-8 text and `bytea`; canonical PostgreSQL JSON text for `jsonb`; a documented canonical representation for arrays. Null measures zero. Never use `pg_total_relation_size` or compressed/TOAST size for admission. Counters represent retained logical content, not a bill for physical storage.

## Exact byte accounting and transactions

Maintain durable usage/reservation rows per tenant and class. Backfill their exact values from the manifest in the additive migration, with writes excluded during backfill. Counter rows are updated in the same transaction as the corresponding data insert/update/delete; crashes and audit failures roll back both. Content-addressed `ON CONFLICT DO NOTHING` charges only actual insertion. Purges release only the rows actually deleted, including the existing count-based test-run purge. Reconciliation is a diagnostic assertion/repair tool, never the security decision.

Generate column measurement triggers from the reviewed manifest rather than hand-wiring every content writer. Immediate row triggers accumulate **net deltas only** into a private `(pg_current_xact_id(), tenant_id)` transaction-delta table. They do not lock the shared usage row. An `INITIALLY DEFERRED` constraint trigger flushes all remaining deltas for its transaction in sorted tenant order at commit, applies policy and usage checks, and removes its accumulator rows; later queued invocations find nothing to do. Check the final net change, not each historical row event: insert-then-purge and shrinking replacement must not be refused for transient intermediate growth. Commit-time quota failures map to typed store errors and roll back the complete mutation.

Trigger helpers use `SECURITY DEFINER` only with a fixed safe `search_path`, a trusted owner, explicit runtime grants, and no runtime ability to write counters/accumulators, call a privileged mutation helper directly, disable triggers or substitute a helper through a writable schema. Enable the accounting triggers `ALWAYS`. Test the actual runtime role against each bypass, as well as ordinary writes. No caller-controlled session flag authorizes accounting bypass. Log allocation remains an explicit finalization helper before durable log writes; the deferred manifest delta validates what was actually retained, and must not attempt to trim a now-immutable script version from inside a deferred trigger.

All authoritative limit reads occur **after** the corresponding policy fence is acquired in the mutation transaction. Do not reuse application-pre-read limits as authority. Administrative changes acquire the conflicting policy fence and do not revoke already committed reservations. A decrease below usage disables positive unreserved growth; it does not make cleanup or consumption of an existing reservation fail.

Use one documented lock order. Entity mutations precede the final aggregate usage update; after taking the usage row lock, perform no additional lock acquisition on existing workflow/run/script/endpoint rows. Charge only at the final mutation/commit boundary, or reserve before durable work begins. Batch maintenance handles tenants in stable UUID order or one tenant per transaction. Policy editors must never hold a usage lock while waiting for an entity lock. Prove this with an interleaved writer/purge/policy test, not just a single-counter race.

For normal author/API admission, lack of content capacity returns typed `QUOTA_EXCEEDED`, without values or payloads, and rolls back data, audit and command idempotency together. Exact idempotent replay is checked before new quota consumption. Deletions and shrinking edits still work at or above the limit.

For asynchronous work, insufficient capacity for inline output, test result or a new build artifact becomes a fenced, non-retryable terminal `STORAGE_QUOTA_EXCEEDED` result without storing the rejected content. Roll back the tentative content/success transaction completely; reopen the attempt under its original fence, check replay first, and commit the terminal failure with its journal, next scheduling, outbox and original completion idempotency fingerprint. If timeout/reclamation won meanwhile, return the existing stale-attempt verdict rather than failing a newer attempt. A duplicate of the rejected completion replays the terminal acknowledgement. Build/test completion uses the equivalent existing job token/fence and terminal failure path. An adapter error repeatedly retrying the same successful completion is not an implementation, and no version may remain permanently BUILDING after quota exhaustion. A later reservation optimization would require separate evidence; the baseline uses this explicit failure behavior.

For a workflow attempt, this storage failure is unrecoverable for the run: initiate the existing failed-run stop before settling the original fenced attempt. A normal `on_error` route can itself create charged child input or correlation data, so routing this failure as an ordinary activity failure would allow the terminal fallback to fail for the same exhausted resource. Use a domain command that preserves existing stop decisions, cancels remaining forward work and drains existing saga compensations/reconciliation obligations; it must not invent successful compensation or directly manufacture journal records. Protected scheduling, journal, audit and outbox effects remain in the same terminal transaction. Prove the fallback with an `on_error` route to a child and with a saga that still has compensation to perform. Catch only the tagged content-quota refusal; a transport error or uncertain commit is not evidence that the successful completion rolled back.

Preserve what is known about the external effect. A worker success whose output cannot be retained still obligates compensation, including when a prior cancellation already requested that attempt to stop; its storage failure cannot erase evidence that the effect may have happened. A worker failure keeps its original failure code and certainty under the original `fail` idempotency fingerprint, while the run's new stop records the storage refusal. A rejected reconciliation result terminates the lookup through the existing reconciliation failure/review semantics instead of leaving the originating step waiting on a completed lookup. Existing cancellation, timeout and failure stop reasons remain authoritative.

The automatic reducer boundary is the scheduler's `apply_if`: timers, wait deadlines, retry release, lease expiry, run deadlines, child completion/cancellation and rehinting pass through it. If the exact content refusal occurs during persistence or commit, roll back the whole attempt, reopen the same tenant/run, and recheck the original shard hold, candidate condition and **unchanged pre-command run version**. A changed version or lost shard skips recovery. Otherwise commit a failed-run storage stop, or settle the already established stop without replacing its reason/code. Do not replay an arbitrary consumed reducer closure. The next maintenance pass drains outstanding leases/children under that stop; it cannot release new forward child input or event-correlation data. Quota-stop telemetry must not claim that the original timer or expiry transition committed. Pending-child creation separately treats quota refusal from creation, initial reduction or commit as an atomic child-admission failure; reporting that failure to the parent uses the same protected scheduler boundary.

This is a finite admission policy, not a universal forced-stop rule. Synchronous API start, signal, resume and review operations may return typed `QUOTA_EXCEEDED` with their entire mutation and idempotency record rolled back. A worker poll is also new-lease admission until its transaction commits and any task is exposed: if the oversized-input preflight failure would allocate charged data through `on_error`, a quota refusal rolls back every tentative claim in that transaction, leaves the existing `READY` work durable and returns backpressure. No attempt/token is exposed and no external completion is lost; terminalizing this preflight rejection may wait for capacity. Event/cron starts and event wake-ups remain retryable durable admissions: no delivery acknowledgement or source advancement may claim a rejected transaction succeeded, and the inbox/schedule retains the pending work. These admission paths do not promise progress while capacity remains exhausted. Do not take an aggregate usage lock before expression materialization: later blob/entity locks would invert the entity → policy → usage order. Exact accounting remains at final persistence/commit, with the bounded recovery paths above.

Blob upload reserves its bounded allowed bytes before any backend PUT. Successful finalization replaces reservation with measured size; committing the descriptor does not charge again. Finalized garbage releases only after confirmed deletion. An upload with an unknown PUT outcome retains its reservation/tombstone until backend reconciliation proves it safe to release; a late PUT cannot restore unaccounted retained bytes. This refines ADR-0023 cleanup accounting rather than weakening it.

A provider-acknowledged PUT records its measurement even when size/hash preconditions reject the upload or garbage collection already marked it `DELETING`; it never revives a deleting object. Cleanup carries an immutable deletion-attempt token from claim through provider response to catalogue finalization. An older DELETE response cannot borrow a newer claim's timestamp to release a late upload. Release requires a matching acknowledged deletion attempt that began after the PUT was finalized; unresolved PUT outcomes keep their reservation.

## Logs cannot obstruct terminal state

After redaction, store only the UTF-8 prefix that fits the remaining log allowance under the same exact tenant usage lock. Set persisted truncation evidence even when the entire log is omitted. Build versions need additive truncation evidence equivalent to the existing test/execution fields. Do not store an uncharged arbitrary truncation message. Retrying the same terminal report must not charge again or replace its first stored result.

The quota is for retained application log bytes. Platform structured logs and immutable audit are protected operational records, with the existing bounded/sensitive-data rules. A full log quota must still permit failed/successful job state, execution evidence, journal, audit and outbox commit. Deleting expired records releases their stored log bytes atomically.

## Durable rate admission

Implement token buckets using PostgreSQL time and durable state keyed by `(tenant_id, namespace_id, resource)`, plus source key when enforcing an endpoint/connector-specific sublimit. Store integer token units (or exact fixed-point units), cap the refill interval/arithmetic, and bound burst to the documented two-second capacity. Zero refuses new admissions; a lowered policy clamps available tokens immediately. Multiple replicas share one decision. Local subject/tenant/unauthenticated gates remain defense-in-depth, not authoritative namespace quotas.

API rate consumption happens once per request after authentication, RBAC and canonical namespace resolution in the application boundary. A namespace chosen only from an arbitrary URL/header is not authority. Named namespace routes, resource-ID routes, run operations and streaming upload admission all need an explicit inventory. Cross-tenant/forbidden calls do not debit the victim namespace. Health/readiness, authentication, public webhooks, and worker lease/completion protocols use their own controls. Tenant-wide management calls retain existing tenant limits and must not be assigned a fabricated namespace. A successful API bucket decision is committed independently before the operation: subsequent validation/conflict still consumed an authenticated API request. API retries consume API capacity; this is separate from event dedupe.

Event capacity is consumed inside `events.rs::admit_within`, **after** event-id duplicate/reuse detection and **before** inserting a new inbox row. Namespace and source-specific rates are judged atomically with admission. A duplicate consumes no event token or retained bytes; a rolled-back admission consumes neither. Replace the verified webhook gate's authoritative role with this shared decision, keeping its pre-authentication flood protection. Connectors neither advance offsets nor acknowledge on rate refusal. Rate refusal carries a typed retryable `429` and a bounded `Retry-After`; exhausted retained-byte capacity is the existing hard quota error. No HTTP-URL middleware substitutes for this event insertion boundary.

## Retention policy

Apply the current operator ceiling and effective namespace limit when creating or changing a TTL choice, and recheck it at authoritative event/job admission. Endpoint dedupe remains at least twice its allowed clock skew plus the existing safety margin. Reject incompatible choices instead of silently shortening a requested dedupe window. Zero may disable new retention-bearing admissions; it cannot manufacture an invalid zero-second domain TTL.

Lowering policy does not rewrite stored expiry, mutate immutable versions or authorize retroactive deletion. New executions may use `min(version retention, current effective retention)` where the API explicitly defines this as an execution-record policy, while existing records retain their stored expiry. Pending events remain protected from expiry-based purge until dispatch, as today. Customer/regulatory minima, legal holds, long-term audit retention and destructive purge policy remain G42 decisions; this ADR claims only conservative new-admission ceilings.

Production script execution is admitted when its workflow step receives a durable lease, not when its eventual execution report arrives. Resolve the script-retention policy under its shared transaction fence before granting a new forward lease, and persist the selected positive day count as the bounded numeric `scriptRetentionDays` field of that attempt's immutable `step.leased` journal detail. The current workflow reference pins an artifact digest, not a script-version ID: preserve the existing selection of the maximum retention among the tenant's READY versions of that digest, then clamp it to the effective namespace policy and freeze the result for this attempt. A zero policy withholds new forward script leases without consuming an attempt. Policy changes must not prevent completion or compensation/reconciliation of work already admitted; their retained evidence follows the policy admitted with the originating work, rather than reopening a new forward admission decision.

The forward admission also inspects its immutable compensation and reconciliation activities. If either is a script, select and freeze its own digest's retention before admitting the forward effect, even when the forward activity is external. Store at most three positive day counts: `scriptRetentionDays`, `compensationScriptRetentionDays` and `reconciliationScriptRetentionDays`, with `scriptRetentionPolicyVersion: 1`. Zero withholds any new forward lease that would create one of these script obligations. A subsequent compensation/reconciliation lease inherits the matching field from its originating forward attempt and stamps that value as its own `scriptRetentionDays`; it does not consult a later ceiling. A missing version marker identifies a legacy admission. A version-1 record missing a required pin is an invariant failure, not permission to fall back to legacy behavior. These bounded journal fields describe the admission decision and add no executable content or secret value.

An execution report resolves its tenant, namespace, run, node, attempt and digest against that durable lease before reading the pinned retention. The first stored report assigns its expiry using that pinned duration; replay preserves the first record and expiry, regardless of a later policy decrease. Existing immutable lease events without the field are explicitly grandfathered under the prior retention-selection semantics, and existing execution rows are unchanged. New writers must always stamp the field; enforcement rollout must not claim that older writers enforce the new admission rule. An execution report with no workflow-attempt tuple is a separate record admission subject to current policy; a partial tuple is invalid. Tests must include lowering the ceiling to zero between lease and report, a subsequent forward lease being withheld, old lease compatibility, and completion/compensation at the lowered ceiling.

## Implementation slices and ownership contract

1. **Shared policy and manifest:** domain resource names/units/ranges; application ports and typed rate result; migration, durable usage/rate state, exhaustive classification manifest and backfill tests; existing quota REST/CLI/OpenAPI views. Own `domain/quota.rs`, `application/quotas.rs`, `infra-postgres/quotas.rs`, new `infra-postgres/content_usage.rs` and migration. Preserve T010-040 activity lock hierarchy and broker resource arms.
2. **Event rate and retention:** `application/events.rs`, `infra-postgres/events.rs::admit_within`, connector ingestion/checkpoint paths, script author/publication/test/execution retention checks. Coordinate with the ADR-0026 endpoint snapshot lock owner. Start with a real two-store last-token race, duplicate non-charge and connector no-ack regression.
3. **Content and logs:** wire every manifest content/log owner, including `definitions.rs`, `versioning.rs`, `scripts.rs`, `execution.rs`, `signals.rs`, `events.rs`, `connectors.rs`, `secrets.rs`, `blobs.rs` and `maintenance.rs`; quota-safe asynchronous terminal results/reservations; all purge decrements. Do not enable the content resource after implementing only inbox/scripts.
4. **API admission:** add one shared application admission port and an exhaustive use-case/route inventory; wire authenticated canonical namespace entry points once per request. Contract tests cover an ID route and a named namespace route that target the same bucket across two server instances.

Public metrics use closed resource/outcome labels (`quota_exceeded_total`, rate admitted/refused, log truncation). Do not label metrics with tenant, namespace, activity, object ID or payload. Policy changes remain transactionally audited; denials retain bounded audit behavior. Quota responses may expose authorized tenant usage and effective limit, never content.

## Completion evidence required

- Schema manifest coverage and exact migration backfill, including existing tenants already above the default; forward/rollback safeguards.
- Two independent store/server instances racing the last byte/token, concurrent ceiling decrease, rollback/audit failure, and cross-tenant isolation.
- Content-addressed reuse, duplicate events/commands/completions, shrinking edits, expired-record/count-based purge, and external late-PUT cleanup accounting.
- Full log quota still allows build/test/step terminal state, audit and outbox; UTF-8 truncation and retry non-charge.
- Content quota exhaustion has a proved terminal/reservation path for asynchronous outputs/artifacts, including timeout races; no silently stuck lease or BUILDING job.
- Namespace API rate uses canonical authorized scope and does not debit another tenant. Event rate refusal leaves broker checkpoint/ack unchanged.
- Retention ceiling, minimum replay window, policy lowering and preservation of already stored expiry/pending inbox records.
- A bounded hot-tenant/multiple-tenant benchmark for the new usage/rate locks. Publish measured behavior rather than carrying forward the old quota benchmark as proof.

T009-037..T009-040 complete these slices and FR-009-006's engineering baseline. Evidence includes
the exhaustive manifest and migration guards; enabled-by-default durable-execution, security,
script and event suites; two-server HTTP admission tests; and the 2,000-operation quota benchmark
recorded in `specs/009-security-multitenancy/tasks.md` and `tests/performance/reports/durable-quotas.md`.
Tier sizing and regulatory retention decisions remain separate under G42.

## Initial shared-core handoff

The first implementer owns the new core modules/migration/manifest and the narrow domain/application/PostgreSQL quota changes only. No event/runtime/API integration edits overlap this slice. The coordinator owns shared OpenAPI, frontend generation and integration-target registration. Implement core contracts and their PostgreSQL tests before assigning integration owners.

| Resource | Valid scalar range | Provisional default/initial ceiling |
|---|---|---|
| `tenant_content_bytes` | 0 through 2^50 bytes | 10 GiB |
| `tenant_log_bytes` | 0 through 2^50 bytes | 1 GiB |
| `api_requests_per_second` | 0 through 1,000,000 | 100/s, burst 200 |
| `events_per_second` | 0 through 1,000,000 | 1,000/s, burst 2,000 |
| `event_retention_seconds` | 0, or 60 through 2,592,000 | 2,592,000 (existing 30-day maximum) |
| `script_retention_days` | 0 through 365 | 365 (existing maximum) |

These defaults are engineering bounds, not approved product tiers. Existing count resources retain their current ranges/defaults. Zero means disabled new admission; a zero rate returns the non-retryable hard quota verdict, while temporary token depletion returns a retryable rate verdict. A zero log limit stores no new log bytes. Existing over-limit content is not erased. Retention resource `used` and rate resource `used` are absent in the existing quota response: they are admission policies, not misleading live-object counts. Byte resources expose exact charged bytes via `used`; the fixed two-second rate burst is part of the contract and does not need another tenant setting.

Minimal public application types/ports (names may follow local style; semantics and ownership must remain):

```rust
pub struct RetainedUsage {
    pub content_bytes: u64,
    pub content_reserved_bytes: u64,
    pub log_bytes: u64,
}

pub enum RateDecision {
    Admitted,
    Refused { retry_after_secs: u32 },
}

pub trait NamespaceApiAdmission: Send + Sync {
    fn admit_namespace_request(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> impl Future<Output = Result<RateDecision, StoreError>> + Send;
}
```

`NamespaceApiAdmission` owns its short transaction. Callers must already have resolved and authorized the namespace, and the adapter verifies `(tenant, namespace)` belongs together. Do not accept a caller-supplied rate or application timestamp. Event admission deliberately does **not** use this separate-transaction port: its bucket helper receives the already open inbox transaction. Rate refusal can map to the existing `AppError::Busy`; zero policy maps to `QuotaExceeded`. Domain quota helpers add `maximum(resource)`, explicit unit/kind and `has_usage(resource)` rather than applying the legacy scalar `MAX_LIMIT` to bytes.

Private PostgreSQL shared primitives accept `&mut PgConnection` so existing `Transaction` callers compose without adapter types entering the application/domain:

```text
lock_policies(conn, tenant, namespace?, ordered_resources) -> ResolvedPolicies
apply_content_delta(conn, tenant, content_delta: i64, reserved_delta: i64) -> ()
take_log_prefix(conn, tenant, already_stored_bytes: u64, redacted: &str)
    -> { text: String, truncated: bool }
take_event_rate(conn, tenant, namespace, source: optional typed endpoint/connector key)
    -> RateDecision
check_event_retention(conn, tenant, namespace, requested: Duration, skew: Option<Duration>) -> ()
check_script_retention(conn, tenant, namespace, requested_days: u16) -> ()
```

`ResolvedPolicies` is private to the adapter and may only be constructed while holding the shared policy fences in the same transaction. Primitives must not accept an externally pre-read effective limit. `apply_content_delta` uses checked arithmetic, refuses underflow/corrupt counters, and allows a negative/net-zero delta even above a lowered ceiling. Log prefix storage and its exact delta are committed together; a caller cannot reserve log bytes and forget to persist/release them. Finish all non-usage row-lock work before the aggregate usage update. Resource locks are acquired in stable resource-name order; usage rows are acquired in content-then-log order. Multi-tenant maintenance splits transactions or sorts tenant UUIDs before taking usage locks.

Bucket implementation uses microtokens: one request costs 1,000,000 units; capacity is `2 * rate * 1,000,000`. Read `clock_timestamp()` once after locking its state row. Refill is `min(elapsed_us, 2_000_000) * rate`, capped at capacity, with checked integer arithmetic and nonnegative elapsed time. Compute retry seconds by ceiling division, minimum one. No process clock or float decides admission. Namespace and source bucket checks occur before either decrement; a refusal rolls back both. Policy lowering clamps the bucket to current capacity, and source policy comes from its row read in this transaction. A duplicate bypasses both event buckets.

Core tables are `tenant_retained_usage` (one tenant row with nonnegative retained/reserved/log counters) and `quota_rate_buckets` (tenant, namespace, closed resource/source kind, source UUID or nil, microtokens, database update time). A source UUID never comes from an arbitrary request string: only a stored endpoint/connector identity can select it. Add durable blob reservation size to the catalog rather than deriving old reservations from a changed process configuration. Administrative policy writes share the new resource fences. Migration rollback refuses while byte reservations or non-default new policies would be silently discarded; document the exact migration policy and prove it.

The first test matrix is deliberately small but consequential: (1) resource ranges and units; (2) exact manifest/backfill on all current tables; (3) last-byte race with rollback and concurrent ceiling decrease; (4) last-token race across two store instances plus database-time refill and restart; (5) zero/full log quota with multibyte UTF-8 prefix and release; (6) retention ceiling plus `2 * skew + 60 seconds`. Integration owners then add the per-slice terminal/replay/connector/API tests listed above. The shared core alone did not close T009-039 or FR-009-006; the integrated consumer, terminal-state, API-admission and benchmark evidence recorded by T009-039/T009-040 completes that baseline.
