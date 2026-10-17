# ADR-0033 — Reusable database connections and managed execution

Status: **Accepted — user approved implementation on 2026-10-16** | Date: 2026-10-16 | Feature: [017](../../specs/017-database-connections/spec.md)

## Context

The user confirmed UI-managed PostgreSQL connections, OpenBao in the first delivery, and reuse through all SDKs, visual workflows and ad hoc UI execution. The existing integration screen manages inbound brokers, while the local secret service and worker resolution already provide logical names, ACLs and lease-based access. There is no shared database-operation contract or managed database executor.

A connection selects an external effect destination. Retargeting it during retries, exposing arbitrary secret-manager paths, or treating a UI query as an unaudited shortcut would violate the platform's durability and tenant boundaries. Customer databases commonly require private networking, which cannot be enabled by weakening all existing egress checks.

## Decisions

1. **A scoped connection is a versioned resource.** Store non-secret PostgreSQL settings plus a logical credential reference. Resolve names/revisions when a run is admitted and retain the binding throughout its attempts. Subsequent edits affect subsequent runs. Current revocation is still authoritative. Preserve referenced revisions through run retention and require explicit migration for environment promotion.
2. **UI and SDKs share one database-operation contract.** Both author the same canonical connection reference, operation, bound parameters, limits and effect policy. An additive database node is a candidate wire shape, not an accepted new discriminator yet. The reviewed plan must choose its exact representation and capability negotiation without forking SDK semantics or granting ordinary workers access to managed database tasks.
3. **Execute through an isolated managed worker.** Database operations and fixed probes use the durable lease/fencing machinery. The worker carries only the authorized target capability and cannot access Sagat's state database. SDK workflow authors need no vendor driver for this managed path. Existing arbitrary activities remain their existing execution model.
4. **Private database access has its own constrained deployment boundary.** The platform operator approves reachable customer networks/ports; namespace choices can only narrow them. Enforce actual destinations and TLS identity, not merely a preflight hostname check. Keep metadata/link-local/loopback and platform state stores inaccessible. General script, webhook and HTTP egress policy does not change.
5. **Sagat owns the authorization facade; OpenBao owns new credential values.** Management API calls authenticate to Sagat, map to scoped operations and call OpenBao through a backend adapter. Use logical references in UI/SDK/IR, fixed trusted provider configuration and short-lived workload identity. This is a bounded application facade, not a wildcard reverse proxy. Provider management responses never pass through to clients.
6. **Use KV v2 for the initial credential shape.** A static credential secret contains the required credential fields; the connection holds its reference. Resolve the active permitted version per attempt. Stored-secret rotation and PostgreSQL account rotation are different operations. A later dynamic-database credential mode must specify provider leases, renewal, session lifetime and revocation; KV values do not acquire those guarantees by association.
7. **Provider provisioning is a recoverable external operation.** Persist intent/key/scope/audit in PostgreSQL, perform the provider write outside its transaction, verify the version/operation result and then activate metadata. Use provider version/CAS evidence and a deterministic operation identity. Never enqueue plaintext for a background retry. Before the provider receives a new value, a process crash can require the user to resubmit it; after receipt, reconciliation must detect that write without inventing a new version. Exact safe content-match evidence and retention are plan gates.
8. **Credential migration has explicit ownership and cutover.** Selected legacy credentials are copied/verified under the existing authorized migration path; fence rotation/revocation, preserve logical references and mark one authoritative provider. Cutover does not enable an automatic local fallback. Broader OpenBao signing-key/platform migration stays with the separate platform-wide proposal.
9. **Ad hoc work is ordinary durable execution with an immutable origin snapshot.** A one-step execution uses an internally retained definition/version and the same run-admission transaction as a workflow. The UI removes manual publication steps, not the persisted version identity. Retries replay the same request identity, refresh observes the same run, and cancellation requests stop work without promising external rollback.
10. **Result visibility is a new, bounded disclosure decision.** Provide a database-execution result endpoint/action gated by an explicit result-read permission, scope and audit. It exposes only the retained bounded result of that authorized database execution, with no-store responses and no bulk export. This does not widen ADR-0028's structural inspection or create a generic run-payload reader. Exact grants, retention, output classification and access to results of SDK-authored workflows must be accepted before the endpoint is enabled.
11. **External effects retain at-least-once semantics.** PostgreSQL target commit and Sagat completion are separate commits. Unknown outcomes require lookup/idempotency evidence or manual review. Retrying a write is allowed only under its declared proven effect contract. No target schema or receipt table is installed implicitly. Compensation runs separately and can fail.

## Affected decisions and dependencies

The managed-result retention boundary refines ADR-0023's streaming path: PostgreSQL results have
an approved 4 MiB hard maximum and are collected under the existing transfer permit/deadline for
canonical JSON and retained-credential scanning **before** any object reservation. General blobs
remain streamed. Completion independently verifies and rescans stored bytes. This bounded scan does
not evaluate workflow expressions over blobs or add a generic payload reader. Pending PUTs after a
timeout retain ADR-0023's durable cleanup/tombstone semantics. Result-response memory and concurrency
measurements remain release evidence in T017-025.

- Preserve ADR-0001 domain/adapter boundaries, ADR-0002 transactions, ADR-0004 external effects, ADR-0006 isolated execution and ADR-0007 five-language parity.
- Extend 001 contracts, 002 task/admission capabilities, 003 external-outcome integration, 006 builders/conformance, 007 connections/secrets/ad hoc/result UI, 008 instrumentation, 009 secret-provider/network/RBAC/quotas and 011 revision/promotion semantics in the approved implementation delivery.
- ADR-0028 remains the default for general run inspection. Decision 10 requires a distinct accepted disclosure policy and corresponding dependent-document updates before implementation.
- The unmerged `feature/013-otel-openbao` draft owns wider secret/key migration. Reconcile its identifiers and provider contracts; this proposal is not acceptance of that whole draft.

## Alternatives considered

- Storing DSNs/passwords in workflow input duplicates configuration, exposes credentials and cannot implement scoped lifecycle controls.
- Browser-to-OpenBao/database connections place provider authority in the wrong trust boundary.
- Supplying database credentials to every ordinary worker expands access beyond the managed operation; it does not deliver UI execution parity.
- Running probes/SQL in the control plane couples untrusted database work to API capacity and contradicts isolation.
- A generic connector marketplace would add scope without being needed for PostgreSQL plus an extensible adapter boundary.

## Review and verification gates

This proposal accepts no runtime guarantee. Implementation details and finite ceilings are recorded in the [plan](../../specs/017-database-connections/plan.md); pin provider/driver versions; specify token bootstrap, migration, private-network enforcement and default resource limits. Measure the dedicated executor's operational cost and resolution/pooling latency. Prove with real PostgreSQL/OpenBao: cross-tenant denial, provider/engine/target commit faults, DNS/TLS/route refusal, ambiguous writes, stale leases, cancellation races, failed compensation, restart, secret canaries, quota exhaustion and backup/restore. All five SDKs must execute the shared fixtures, and real-browser E2E must cover registration through ad hoc outcome.
