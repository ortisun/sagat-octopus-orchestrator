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

See `tasks.md`. Each checkpoint records actual commands and outcomes in `verification.md`; unchecked tasks are not shipped guarantees. Start with provider configuration/path/error/redaction tests and real OpenBao CAS/retry behavior. Then durable provisioning and catalog tests against PostgreSQL, then managed execution, API, SDK parity and UI. Add integration/fault targets under `tests` and reuse the repository's Postgres fixtures. Reuse the same contract fixtures in all five SDKs.

Required release evidence: current tenant/ACL checks; crash before/after provider write and activation; concurrent rotation; provider outage with no fallback; legacy migration fence; connection revision pinning; ordinary-worker denial; query/write/cancellation/unknown outcome; result disclosure and canary checks; real browser save/probe/ad hoc flow; shared TypeScript/Go/Python/C#/Java conformance; bounded telemetry; production network/TLS profile and restore drill. Do not label pending tests successful.
