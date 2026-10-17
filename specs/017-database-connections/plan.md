# Implementation plan — 017 database connections

Status: Implementing. Product scope and ADR-0033 approved by the user on 2026-10-16.

## Architecture and dependency decisions

The delivery extends features 001, 002, 003, 006, 007, 008, 009 and 011 in the bounded ways described in ADR-0033. It does not depend on merging the separate 013 proposal. Keep existing engine transitions and admission transactions; extend their validation and persisted bindings. Never run customer SQL in an API or scheduler process.

1. Add domain connection configuration, typed database operations and result cells. Canonical IR uses `type: database`, `connectionRef: {name, revision?}`, `operation: query|execute`, `statement`, typed `parameters` (literal or expression), and bounded `limits`. Convert to a reserved managed activity with typed operation metadata on the node, following the existing script specialization. Reject a caller-authored ordinary activity using the reserved name. Gate managed leasing and capability resolution independently of wildcard worker bindings.
2. Keep logical secret names and ACLs. Add an explicit stored-value enum: local sealed envelope or OpenBao version reference; never disguise a remote reference as ciphertext. Add an async provider port alongside the synchronous local cipher. New managed writes use OpenBao; legacy values remain local until explicit migration. Adapt every existing consumer to async resolution before enabling new provider-backed secrets in management routes. No fallback after remote cutover.
3. OpenBao uses KV v2, server-controlled mount and UUID-derived tenant/namespace/operation paths. A provider operation writes once using CAS=0. Its UUID is allocated by durable intent, never accepted as a provider path. Separate immutable paths per logical version make a retry detectable without creating another version. A keyed digest binds intent to submitted bytes without an offline password oracle. The durable intent stores only digest, actor, expected logical revision, operation identity and safe status. Reconcile an existing receipt before writing; changed body/key reuse conflicts. Revalidate authorization and current logical revision before activation. A crash before receipt requires secure resubmission. An orphan provider value is not active. A management token has create/read on only the configured credential prefix; runtime policy is read-only.
4. Production authentication uses a short-lived token supplied by a workload identity agent through a token file, reread on each request, or Kubernetes login with the projected service-account token. No root/static token production default; TLS verification mandatory. The initial adapter supports the agent token-file path, which delegates renewal/login to OpenBao Agent. Development uses an explicit loopback-only development configuration. Never log request/response bodies, URLs containing credentials or tokens. Disable HTTP redirects/proxy inheritance.
5. Connection revisions store only nonsecret host/port/database/TLS policy, logical secret reference and limits. Run admission freezes a per-node connection binding in the same transaction as the run. Current pause/deletion, author role and secret ACL are checked again at capability use. Connections cannot target the platform database. Retain referenced revisions; deletion is a tombstone.
6. A separate `database-runner` process takes managed leased tasks over the existing worker transport and receives only that attempt's capability. It has no Sagat database URL, local keyring or OpenBao management token. A target adapter executes PostgreSQL prepared statements inside explicit transactions with server-side statement/lock deadlines. Query mode uses READ ONLY with a restricted target role; execute mode rejects DDL, administrative and transaction-control statements. Parse a single statement, but do not claim parser classification alone secures functions. Network policy and least-privilege database roles are mandatory.
7. Destination policy is operator-approved CIDRs/ports narrowed by namespace policy. Resolve once, validate every address, connect to the selected address while verifying the configured TLS hostname. Deny loopback/link-local/metadata and configured platform state endpoints. Local real-service tests use a separately named development-only exception. Production manifests enforce egress independently of application checks.
8. Probe is a retained one-step definition using a fixed operation, with no arbitrary SQL body. Ad hoc execution retains an immutable definition and passes through normal run admission. A request receipt binds principal, namespace, key and canonical body, rechecking current authorization on replay. No new transaction combines the customer database, provider and Sagat stores.
9. Explicit write effects use the existing unknown-outcome/reconciliation semantics. Initial writes have no automatic retry; an ambiguous commit requires manual review. A user-provided idempotency key is correlation, never evidence of database deduplication. An operation that needs compensation defines that operation explicitly.
10. Result cells use tagged types: null, boolean, integer (decimal string), decimal (string), float (finite number), text, timestamp (RFC3339 string), date (ISO string), binary (base64), JSON (bounded JSON). Reject unsupported types. Preserve column order, duplicate column names and nulls. Results remain bounded retained content; a separate audited result endpoint requires `database:result:read`, and sends `Cache-Control: no-store`. Ordinary run inspection remains structural.

## Authorization

New permissions are distinct even where a built-in role receives several. Admin: catalog manage/use, ad hoc execute, write and result read, plus existing secret management. Developer: connection use and ad hoc query execution, subject to current secret ACL; no write or result disclosure by default. Operator/viewer: metadata only through existing namespace access; no database use/write/result by default. A dedicated managed executor role receives worker execution and managed capability access, with current bindings required; ordinary worker/runner roles cannot claim managed database tasks. Result reads require explicit result permission and namespace/run scope, regardless of run authorship. Foreign resources use the existing opaque not-found response.

### Authority row locks with runtime privileges

The runtime role retains read-only access to operator-provisioned memberships and namespace
identity. PostgreSQL requires UPDATE privilege for direct FOR SHARE reads; use narrow SECURITY
DEFINER SQL functions for locking selected membership rows, one namespace and selected secret ACL
rows instead of granting mutation rights. Functions fix search_path to pg_catalog/pg_temp, qualify
all table names, filter tenant and exact identities, return only metadata, revoke PUBLIC execution,
and grant sagat_app EXECUTE. Locks survive in the caller's admission transaction and retain the
same ordering and revocation fence. Real runtime-role HTTP and runner tests verify this path.

## Finite ceilings

| Resource | Default | Hard ceiling |
|---|---:|---:|
| SQL UTF-8 bytes | 16 KiB | 64 KiB |
| Parameters | 64 | 256 |
| Parameter bytes total | 64 KiB | 256 KiB |
| Result rows | 1,000 | 10,000 |
| Result bytes | 1 MiB | 4 MiB, also existing payload quota |
| Query deadline | 10 s | 30 s |
| Connect / provider request deadline | 5 s | 10 s |
| PostgreSQL connections per target | 4 | 16 |
| Idle connection lifetime | 30 s | 60 s |
| Concurrent operations per namespace / tenant | 16 / 64 | 64 / 256 |
| Pending provider operations per namespace | 32 | 128 |
| Active connections per namespace | 100 | 1,000 |
| Retained secret versions | 5 | 5 |

Effective values are the minimum of request, namespace and deployment limits. Zero is not unlimited. Queue admission fails before accepting work beyond a ceiling. Existing retained-content, run and request quotas also apply.

## Delivery sequence and verification

### Ad hoc and probe admission (T017-014/016)

Retain one private immutable workflow/version per tenant, namespace, actor and canonical IR digest.
Repeated identical submissions by the same actor reuse that definition, including at the workflow
count ceiling. Different actors retain their own publisher identity. Internal workflows cannot be
activated, edited, published, promoted or discovered by the ordinary definition APIs. They count
toward the existing namespace workflow quota (default 1,000) and retained-content quota. This
delivery does not delete immutable versions automatically; distinct definitions consume finite
retention capacity and admission fails when full.

The workflow quota lock precedes private definition lookup; charge the count only on insertion.
Definition, version, run, connection pins, initial journal/tasks/outbox, audit and request receipt
commit together. Generated identifiers are excluded from request fingerprints. The receipt binds
actor, namespace, origin, canonical definition, input, timeout and rerun source/reason. Replay
rechecks current role, ACL and persisted pins under the existing secret/catalog → membership lock
order. Ad hoc authorization (`run:start`, `database:adhoc`, `database:use`, and write permission
when applicable) is distinct from publication approval, including in protected namespaces.

`POST /namespaces/{id}/database-executions` accepts mode `step` or `workflow`. Step mode contains
exactly one database node; workflow mode supports database, timer, condition, parallel and end,
requires at least one database node, allows at most 64 nodes and no triggers. Its timeout defaults
to five minutes and is at most one hour. SQL still executes only in the isolated managed runner.
`POST /namespaces/{id}/connections/{name}/probe` accepts only an optional revision and uses a
server-generated `SELECT 1::bigint` operation. Paused connections refuse probe admission as well.
The origin `CONNECTION_PROBE` cannot be supplied to the ad hoc endpoint. Probe metadata comes from
the durable run and its pinned revision; a prior revision's success is not current evidence.

Runs expose optional origin `AD_HOC_STEP`, `AD_HOC_WORKFLOW` or `CONNECTION_PROBE`; ordinary runs
omit it. Generic rerun of an internal run requires its original actor and current ad hoc authority,
loads its private retained definition and repeats the same admission path, preserving probe
provenance. Normal start cannot execute an internal version. Result access remains separately
authorized and audited.

See `tasks.md`. Each checkpoint records actual commands and outcomes in `verification.md`; unchecked tasks are not shipped guarantees. Start with provider configuration/path/error/redaction tests and real OpenBao CAS/retry behavior. Then durable provisioning and catalog tests against PostgreSQL, then managed execution, API, SDK parity and UI. Add integration/fault targets under `tests` and reuse the repository's Postgres fixtures. Reuse the same contract fixtures in all five SDKs.

Required release evidence: current tenant/ACL checks; crash before/after provider write and activation; concurrent rotation; provider outage with no fallback; legacy migration fence; connection revision pinning; ordinary-worker denial; query/write/cancellation/unknown outcome; result disclosure and canary checks; real browser save/probe/ad hoc flow; shared TypeScript/Go/Python/C#/Java conformance; bounded telemetry; production network/TLS profile and restore drill. Do not label pending tests successful.

## Studio managed execution controls (T017-021/022)

- Register the authored `database` node in the shared visual editor. Use the same
  connection reference, typed parameter JSON and finite limits contract as the SDKs.
  Incomplete JSON remains an invalid draft; it must never submit an older valid value.
  Nested parameter expressions participate in rename and dangling-reference checks.
- Compose the ad hoc editor from the existing node list/inspector/connect controls;
  omit triggers, autosave and public workflow identity. Preserve uncertain requests
  (body + idempotency key) in bounded session memory across SPA/namespace navigation.
  Later access/rate refusals cannot resolve prior uncertainty. Clear on session end;
  browser reload does not retain SQL and requires locating/reconciling the existing run.
- Probe controls submit only a connection revision and show the durable probe run,
  safe failure and tested revision. Saving or admitting a probe is not a health claim.
- Runtime graph nodes expose authored type `database`, without SQL, values or credential
  references. Private run origins replace broken normal-definition links and permit
  run-list filtering. Ordinary metadata views still disclose no database results.
- Results are fetched only on explicit authorized action into ephemeral component
  state. Bound the complete response to 4 MiB/30 seconds, clear on close/scope/session
  change, paginate rows/columns and bound cell previews. Preserve numeric JSON tokens
  using pinned lossless-json; reject unsafe authoring numbers before conversion.
