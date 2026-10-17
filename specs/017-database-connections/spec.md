# Feature Specification: Database connections, OpenBao secrets and shared execution

**Feature Branch:** `feature/017-database-connections`

**Created:** 2026-10-16

**Status:** Implemented and locally verified on 2026-10-17; see [delivery evidence](verification.md#final-delivery-gates--t017-025). Production installation is operator-managed.

**Input:** Configure databases in the Sagat UI, create access secrets backed by OpenBao, and reuse the configuration in SDK-authored workflows, visual workflows and ad hoc workflows/steps.

## Confirmed scope and objective

The user selected PostgreSQL for the first delivery, an extensible driver boundary, and real OpenBao integration in this delivery. OpenBao remains the selected provider after comparison with alternatives. Merely preparing a future adapter does not satisfy this feature.

An administrator registers a database connection and creates or selects its credential secret in the UI. An authorized author then selects that same connection in a visual database step or references it from any of the five SDKs. An authorized operator can also execute an ad hoc database step or workflow in the UI and inspect its durable outcome. No consumer needs to copy database passwords into workflow definitions, SDK source or run inputs.

The database is an existing customer database. This feature does not provision a server, create a database/schema, or grant itself database privileges. Sagat's internal PostgreSQL and NATS remain deployment resources.

## Dependencies and ownership

Reuse 001 identity/contracts, 002 durable execution, 003 ambiguity/reconciliation, 006 SDK conformance, 007 Studio, 008 telemetry, 009 authorization/secrets/egress/quotas and 011 immutable publication. The new managed database executor follows 005's isolation principles without giving general script sandboxes network access.

The separate draft `feature/013-otel-openbao` already proposes platform-wide OpenBao adoption. The user's confirmation authorizes OpenBao integration for this delivery. Feature 017 owns the connection/credential slice: secret management facade, provider adapter, scoped resolution, migration of credentials selected for these connections, and its reproducible deployment/tests. It does not absorb platform-wide signing-key migration or telemetry replacement. Reconcile the drafts and record affected feature dependencies before implementation; do not claim that unmerged code exists. See [research](research.md).

The [accepted ADR](../../docs/adr/ADR-0035-database-connections-and-managed-execution.md) records the execution, private-network and result-access decisions approved for this delivery. Existing transactional and isolation ADRs remain authoritative.

## User scenarios and acceptance tests

### US01 — Register a connection and secret in the UI (P1)

**Given** an administrator in namespace A, **when** they create a PostgreSQL connection and a new credential, **then** the credential value is stored in OpenBao and the Sagat connection stores a scoped secret reference and non-secret settings. Reloading or editing returns metadata only. Selecting an existing authorized secret is also supported.

**Failure scenario:** OpenBao accepts the credential but the connection save fails. The UI reports the saved secret and incomplete connection, and retry reuses the existing operation/reference without writing another secret version. Another tenant cannot discover either resource.

### US02 — Test and operate the connection (P1)

**Given** a saved connection, **when** an authorized user chooses Test connection, **then** a bounded probe executes in the managed executor's network context and reports a safe result, timestamp and tested connection revision. Saving a connection alone is never represented as proof of connectivity.

**Failure scenario:** Wrong credentials, invalid TLS, denied destination, exhausted capacity and unavailable OpenBao have distinguishable typed categories without raw provider errors. A paused connection accepts no new execution admission.

### US03 — Use the same connection through SDK and UI workflows (P1)

**Given** the connection `orders-db`, **when** an author defines a parameterized PostgreSQL step through the visual editor or any official SDK, **then** both representations publish the same canonical operation and use the same server authorization, limits, credential resolution and execution semantics. A subsequent step can consume the bounded result.

**Failure scenario:** A connection edit after run admission cannot redirect a retry. Revoking access prevents subsequent credential resolution even for an older pinned revision. An unsupported driver or server capability is rejected explicitly.

### US04 — Execute ad hoc steps and workflows from the UI (P1)

**Given** an authorized user, **when** they choose a connection, enter an operation and parameters, and execute a step or supported workflow, **then** Sagat retains an immutable execution definition, creates a normal durable run and exposes its status, cancellation, attempts and authorized result. A single step needs no manual workflow-publication ceremony.

**Failure scenario:** A lost HTTP response and retry with the same operation key return the same run. Refreshing the page resumes inspection. Reusing the key with changed SQL, parameters or connections conflicts. A cancellation cannot be represented as rollback of a committed customer-database write.

### US05 — Rotate, revoke and recover credentials (P1)

**Given** an active connection, **when** its credential is rotated or migrated to OpenBao, **then** its logical reference survives and later authorized attempts resolve the selected current credential version. Secret values never appear in workflow documents, management responses, browser persistence, logs, journal or audit.

**Failure scenario:** Sealed/unavailable OpenBao fails closed with bounded retries; there is no automatic fallback to an older local credential. A credential already delivered to a process cannot be claimed to have disappeared merely because its reference was revoked.

## Functional requirements

- **FR-017-001 — Connection catalog:** Provide namespace-scoped create/list/detail/update/pause/delete APIs and UI. PostgreSQL settings include host, port, database, TLS verification/trust, credential reference and finite execution/pool limits. Reject embedded credential URLs and unrecognized settings. A supported-driver catalog advertises PostgreSQL only until another adapter passes its gates.
- **FR-017-002 — Reusable references:** Give connections stable identity, a namespace-unique logical name and immutable revisions. Names resolve only in trusted tenant/namespace context. New runs resolve and persist revisions atomically with admission; all attempts in that run retain them. An explicit revision remains possible. Cross-environment promotion resolves the logical name in the destination namespace and revalidates access.
- **FR-017-003 — Lifecycle:** Edits create new revisions with optimistic concurrency. Pause/revocation blocks new admission/resolution but does not claim to undo an in-flight effect. Retain referenced revisions; reject destructive removal while live dependencies require them and preserve tombstones/history. An edit invalidates the displayed probe status for the new revision.
- **FR-017-004 — Secret UI:** Allow authorized users to create/rotate a secret from the namespace secret screen and directly from the database connection form, or select an existing authorized reference. Reuse current secret-management/use permissions. Passwords and sensitive credential fields are write-only; forms clear values after submission/closure and never put them in URLs, local storage, query caches or telemetry.
- **FR-017-005 — OpenBao authority:** New secrets created by this feature MUST persist their values in OpenBao. Sagat retains scoped logical identifiers, provider/version metadata, authorization and lifecycle state. A backend-only adapter uses OpenBao's HTTP API through an application port. The browser, workflow IR and SDK builders receive neither OpenBao tokens nor caller-selectable provider paths/mounts. Credentials are delivered only to an authorized execution capability.
- **FR-017-006 — Authentication and isolation:** Authenticate Sagat workloads to OpenBao with a scoped workload identity, TLS verification and renewable/replaceable short-lived tokens. Derive provider paths from trusted tenant/namespace/secret identity. Root tokens are bootstrap-only. Prevent path traversal, cross-namespace access and broad provider-list passthrough; bounded token handling and outage behavior must be tested.
- **FR-017-007 — Partial saves and reconciliation:** Secret writes/rotation and Sagat metadata do not share a transaction. Persist operation identity, authorization context, intent and audit before provider I/O; activate only after verified provider success and current authorization. Recover an uncertain write by operation/version evidence before retry. Preserve the same operation key across UI retries. Never persist plaintext credential content in an outbox; a crash before provider receipt may require secure resubmission, explicitly represented in the operation state.
- **FR-017-008 — Credential versions:** Pin connection settings separately from credential values. Initially use versioned static credentials in OpenBao KV v2; resolve the active authorized version per attempt and record only its opaque version metadata. Rotation of a stored value does not automatically change the password in PostgreSQL. Dynamic PostgreSQL credentials remain an extension requiring lease/renewal/revocation tests. No plaintext cache on disk or between attempts.
- **FR-017-009 — Existing secrets:** Preserve existing logical names, ACLs and workflow references. Credentials selected from the legacy store require explicit migration and verified cutover to OpenBao before this feature uses them. Migration is resumable, fences concurrent rotation/revocation, preserves required versions and has a verified rollback/recovery procedure. Each secret has one authoritative provider; no failure-driven provider switching. Unrelated legacy secrets keep their documented behavior until separately migrated.
- **FR-017-010 — Shared operation contract:** Define typed database operations, a connection reference, parameter bindings, timeout, result bounds and effect policy in one canonical contract. SDK builders in TypeScript, Go, Python, C# and Java and the Studio emit that contract. Neither UI nor SDK invents retry, authorization or serialization semantics. Existing contracts remain valid; unsupported capabilities fail before execution.
- **FR-017-011 — SQL execution scope:** Support parameterized queries and explicit writes. Values bind through the driver; dynamic identifiers require a separately validated allowlist. Start with one statement per database step, no arbitrary multi-statement scripts, DDL, transaction-control commands or administrative commands. Statement classification is not a security boundary: database roles and transaction/resource settings must enforce access, including function-call cases. Unsupported SQL/types fail with safe typed errors.
- **FR-017-012 — Managed executor:** Run database I/O in an isolated managed worker/executor with the standard lease, fencing, heartbeat, cancellation and result protocol. UI/API/scheduler processes never execute submitted SQL or connection probes. The executor has only its allowed target credentials, no orchestration-database credentials, and finite CPU/memory/network/concurrency budgets. Arbitrary SDK workers cannot claim its reserved operations or resolve its connection credentials.
- **FR-017-013 — Network scope:** Allow explicitly approved customer database destinations, including private networks required by deployment, only through a bounded managed-executor policy. Preserve general script/webhook SSRF restrictions. Deny metadata/link-local/loopback and platform state stores; enforce resolved-address and actual-connection checks, ports, TLS identity and network isolation. A namespace administrator cannot grant access beyond the deployment's allowed destinations.
- **FR-017-014 — Connection probe:** Execute a fixed platform-controlled probe through the same destination, TLS, credential, executor and quota path as a step. It accepts no arbitrary SQL and changes no business data. Persist audited job identity and a safe result with tested revision/time. A successful probe is evidence at that instant, not a continuing health guarantee.
- **FR-017-015 — Ad hoc admission:** Ad hoc steps and workflows require explicit execution rights and immutable retained definition/version identity. Use the existing durable admission transaction for run, initial tasks, journal, quotas, audit and outbox. Tag origin/actor for UI filtering. Same-key same-body replay returns the original run after current authorization; changed content conflicts. No untracked browser-to-database execution path.
- **FR-017-016 — Output contract:** Return bounded typed columns/rows, affected-row metadata and explicit truncation/overflow status using one five-language representation for decimals, large integers, timestamps, nulls and binary values. Results used by later steps remain retained workflow content under existing quotas. Writes that succeeded externally but cannot retain their result enter the existing reconciliation/manual-review policy; they are not blindly repeated.
- **FR-017-017 — Result access:** Provide a bounded, audited database-result view for explicitly authorized users of the execution. Connection-use, workflow-authoring and ordinary run-read rights do not automatically reveal arbitrary database values. Approve a dedicated result disclosure contract before enabling the view. Preserve existing structural inspection semantics, no-store responses, safe rendering, result retention and secret-output checks. General exports/raw payload reveal are out of scope.
- **FR-017-018 — External effects:** Treat writes as at-least-once attempts. Require a documented idempotency/reconciliation contract for retryable writes; a key carried by Sagat alone does not deduplicate SQL. Unknown outcomes wait for reconciliation or manual review, including ad hoc execution. Target-local receipt/effect atomicity must be proven if offered; never promise a transaction spanning Sagat, OpenBao and a customer database. Compensation is an explicit operation with its own outcome.
- **FR-017-019 — Authorization:** Separate management, use, ad hoc execution, write execution and result reading. Reuse secret ACLs, worker bindings and current-role checks; record the exact role matrix in the reviewed contracts. Revalidate connection/secret authorization at publication, run admission and capability use, including receipt replay and ad hoc actor ownership. Foreign resources remain indistinguishable from missing ones.
- **FR-017-020 — Observability and audit:** Audit catalog changes, secret lifecycle/access, probes, ad hoc starts, result reads and denials with bounded metadata. Emit OTel spans/durations and counters for provider access, probes, database execution, unknown outcomes and pool pressure. No SQL text, parameters, result rows, credential values or provider response bodies enter logs/spans/audit. Metric labels are closed sets, not connection/run/tenant identifiers.
- **FR-017-021 — Limits and failures:** Enforce finite SQL/request/parameter/result sizes, row counts, connect/query deadlines, pool size/idle lifetime, per-connection/namespace/tenant concurrency, operation backlog and retained bytes. Apply the minimum of platform and namespace ceilings. Use canonical typed errors with code, retryability and trace correlation, distinguishing refusal, TLS/authentication, unavailable provider/database, quota, unsupported operation and unknown outcome without leaking diagnostics.
- **FR-017-022 — Delivery and recovery:** Ship reproducible PostgreSQL/OpenBao development and production configuration, pinned images/dependencies, bootstrap/unseal and workload-auth instructions, provider outage/rotation/migration/backup-restore runbooks and a happy/failure demo. Production defaults must not use dev-mode OpenBao, root tokens or disabled TLS verification. Product completion requires real-service and all-five SDK evidence.

## User experience

1. Namespace → Connections → New connection → PostgreSQL.
2. Enter name, host, port, database and TLS settings; select an authorized secret or choose Create secret.
3. Save, observe any partial-save state, then Test connection. Show revision and last test separately from enabled/paused state.
4. Workflow editor → Database step → select connection, query/write mode, parameter bindings and result/effect policy. The SDK expresses the same choices.
5. Connections → Execute step, or workflow editor → Run ad hoc. Show the admitted run immediately and follow persisted progress. Authorized result access is a separate action.

Illustrative shape only, NOT an accepted API or currently runnable workflow:

```yaml
id: find-order
type: database
connectionRef: orders-db
operation: query
statement: SELECT id, status FROM orders WHERE id = $1
parameters:
  - expression: input.orderId
limits:
  timeoutMs: 5000
  maxRows: 1
```

Driver-specific syntax stays in the adapter/operation declaration. Never place the password or an OpenBao address/token in this document. An output consumed by another step requires no additional destination; an external write is its own operation.

## Non-functional requirements

- **NFR-017-SEC:** Zero tested cross-tenant/namespace access and zero credential/token canary leakage; private-network and executor capability refusals block release.
- **NFR-017-DUR:** Preserve atomic engine state/task/timer/subscription/outbox changes, stable request/effect identities and fenced results; no claimed exactly-once external effects.
- **NFR-017-COMP:** Existing workflows and protocol clients continue working. Connection revisions cannot redirect existing runs; provider migration preserves logical references. Unsupported SQL types and capabilities fail explicitly across all SDKs.
- **NFR-017-OBS:** Every accepted operation has persisted correlation/audit and bounded OTel evidence. Export failures do not determine database or engine commit success.
- **NFR-017-PERF:** Select finite default/maximum limits in the implementation plan and measure p50/p95/p99 admission, secret resolution, connection acquisition and result retention on a declared environment. No unmeasured throughput/SLO commitment.

## Verification and success criteria

All evidence below is required future evidence, not completed tests.

| Criterion | Required automated proof | Requirements |
|---|---|---|
| SC-017-001 | Real browser creates a connection and OpenBao secret, reloads without values, reuses a secret, edits with revision conflict and distinguishes save from probe | FR-017-001..007,014 |
| SC-017-002 | Real OpenBao verifies scoped paths, workload auth, versions, rotation, sealed/expired-token failures and no silent fallback | FR-017-005..009,019 |
| SC-017-003 | All five SDKs and a UI-authored workflow execute the same PostgreSQL operations and produce equal typed results/errors | FR-017-010..012,016,021 |
| SC-017-004 | Ad hoc single-step and multi-step runs survive lost admission response, duplicate click, reload, restart, lost broker hint and cancellation races | FR-017-015,018 |
| SC-017-005 | Two tenants/two namespaces cannot manage, use, probe, resolve or read each other's resources; revoked author/actor/executor fails closed | FR-017-003,006,012..014,017,019 |
| SC-017-006 | Crash at every provider-write/metadata/connection-save boundary resumes or requests secure resubmission without orphan activation or duplicate rotation | FR-017-007,009 |
| SC-017-007 | Crash after customer commit/before completion, stale lease, timeout/signal race, lost broker delivery, failed compensation and restart retain explicit effects/unknown outcomes | FR-017-012,015,018 |
| SC-017-008 | Connection revision remains pinned across edits/retries while credential rotation and revocation take effect according to the declared policy | FR-017-002,003,008,009 |
| SC-017-009 | Private-network allow/deny, TLS, DNS change, sensitive destination and privileged-function tests use the production executor/network profile | FR-017-011..014 |
| SC-017-010 | Result access/no-store/redaction, byte/row/pool/quota limits, SQL/secret canaries and audit failure tests pass; real OTel export contains bounded metadata only | FR-017-016,017,019..021 |
| SC-017-011 | Restore/migration drill and reproducible happy/failure demos preserve existing references and expose all skipped/unproven guarantees | FR-017-009,022 |

## Structure, commands and coding conventions

Extend existing modules where semantics belong: `crates/domain/`, `crates/application/`, `crates/infra-postgres/`, `crates/infra-secrets/`, `crates/api-http/`, `crates/api-grpc/`, `frontend/apps/studio/`, all `sdks/`, `infra/docker/`, `infra/helm/` and `docs/runbooks/`. An executor process/adapter boundary is necessary for submitted SQL; its exact packaging belongs in the reviewed plan. No SQLx, HTTP framework, OpenBao client or JSON business representation enters the Rust domain.

Future feature suites belong under `tests/{contract,integration,e2e,fault}/database-connections/`, with shared SDK scenarios in `tests/contract/sdk-conformance/`. Canonical schemas remain in `specs/001-platform-foundation/contracts/`. Use Rust snake_case, existing typed identities and the canonical safe error envelope; create no empty mapper abstractions. Follow existing `SecretValue` redacted Debug/zeroizing ownership for credential bytes.

Existing executable commands, listed for planning rather than as evidence of this feature:

```sh
cargo build --workspace --locked
cargo fmt --all -- --check
cargo test -p sagat-tests --test contract_platform_foundation --locked
cargo test -p sagat-tests --test integration_security_multitenancy --locked
python3 scripts/check-architecture.py
npm --prefix frontend ci
npm --prefix frontend run typecheck
npm --prefix frontend run test
npm --prefix frontend run build
npm --prefix frontend run dev
```

Integration tests require Docker/PostgreSQL and the repository's test configuration. Feature-specific targets, OpenBao auth/deployment checks and five-language commands are recorded in [verification.md](verification.md) and the linked runbooks.

## Boundaries and accepted design

**Always:** follow immutable definitions/revisions, tenant scope, audited authorization, durable state and real-service tests. Update spec/contracts/tests/docs in the same implementation delivery.

**Accepted design:** shared database operation schema, executor capability/network policy, database-result disclosure, exact role matrix, OpenBao provisioning reconciliation/migration and finite limits are recorded in ADR-0035, the plan and canonical contracts. Changes require corresponding contract, evidence and dependency updates.

**Never:** store plaintext credentials in workflow/operation persistence; expose raw OpenBao routing or tokens; execute submitted SQL in the control plane/scheduler; infer idempotency from a SELECT prefix or an application key alone; silently redirect retries; present pending support as shipped.

Out of scope: more database engines in this first delivery, Kafka target redesign, arbitrary scripts with direct database sockets, automatic schema/database provisioning, cross-step/customer-database distributed transactions, automatic account/password creation through OpenBao's database engine, full platform key/telemetry migration, and bulk result export.

The user approved implementation on 2026-10-16. `plan.md` and `tasks.md` record the execution sequence; machine-readable contracts are updated with their implementation. No task or success criterion is complete merely because this document exists. See the [requirements checklist](checklists/requirements.md).
