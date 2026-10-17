# Feature 017 — Baseline, source notes and confirmed choices

Baseline investigation: 2026-10-16. The repository observations below describe the starting revision.
Implementation and executed verification are recorded in [verification.md](verification.md).

## User choices

- Configure existing database connections and create/select their secrets in Sagat's UI.
- Reuse connections in SDK-authored workflows, visual workflows and ad hoc workflows/steps.
- First delivery: PostgreSQL, with a boundary for additional drivers.
- Integrate OpenBao now. After discussing alternatives and API support, the user explicitly confirmed continuing with OpenBao.

## Repository evidence

The feature worktree was cut from fetched `origin/develop` at `6ede1c7`. The original workspace remains on `feature/workflow-primitives`; its changes were not copied. Identifier 017 avoids the 013/014 drafts and the existing feature/016 reservation visible in local branches.

- [Integration UI](../../frontend/apps/studio/src/features/integrations/IntegrationsPage.tsx) and [events runbook](../../docs/runbooks/events.md) configure inbound brokers and separate outbound subscriptions. Neither is a generic database-connection API.
- [Secret service](../../crates/application/src/secrets.rs), [secret types](../../crates/domain/src/secrets.rs) and [security runbook](../../docs/runbooks/security-governance.md) provide names, authorization, sealed local versions and lease-bound resolution. A cipher interface is not already an asynchronous OpenBao lifecycle adapter.
- [Workflow contract](../001-platform-foundation/contracts/workflow.schema.json) has activity/script secret declarations. No database node or generic connection-reference contract is accepted at this baseline.
- [ADR-0028](../../docs/adr/ADR-0028-authorized-structural-payload-inspection.md) authorizes structural inspection, not unrestricted SQL-result disclosure. The proposed database result action needs its own reviewed permission/disclosure contract.
- The separate local worktree `sagat-octopus-orchestrator-013-otel-openbao`, branch `feature/013-otel-openbao`, contains an uncommitted `specs/013-otel-openbao/spec.md` draft and no plan/tasks. It proposes OpenBao authority, workload identities, cross-system reconciliation and migration. Its proposed ADR-0022 number collides with the integrated supervisor-denial ADR; do not link that number as accepted or silently copy its constitution changes.
- Separate work on secret worker pools, result retention, payload classification and workflow primitives is not assumed merged. Reconcile exact capabilities during planning instead of relying on branch names as proof.

## Official sources consulted

### OpenBao interfaces and secret lifecycle

- [HTTP API](https://openbao.org/api-docs/): JSON/HTTP operations under `/v1/`, authenticated over TLS. Proposal: the Rust adapter exposes a small application-owned interface; it does not forward arbitrary browser-selected paths. Pin the deployed version and verify required endpoints rather than assume full compatibility from the `/v1` prefix.
- [Official Go API client](https://pkg.go.dev/github.com/openbao/openbao/api/v2): available for Go integrations. The Rust service can call the HTTP API directly; a Go sidecar is not required merely to use the provider.
- [KV v2](https://openbao.org/docs/secrets/kv/kv-v2/): supports secret versions, version-aware writes and distinct data/metadata access. Proposal: use this engine for initial user-supplied credentials and test reconciliation instead of treating provider writes as PostgreSQL transactions.
- [Lease semantics](https://openbao.org/docs/concepts/lease/): dynamic credentials have provider leases; ordinary KV values do not. Proposal: keep Sagat task leases, provider authentication tokens and future database-credential leases distinct.
- [PostgreSQL engine](https://openbao.org/docs/secrets/databases/postgresql/): supports dynamic and static roles. This enables a later credential-generation mode, but it does not prove Sagat integration or authorize automatic account provisioning in feature 017.

### Alternative considered at the user's request

OpenBao's [MPL-2.0 license](https://github.com/openbao/openbao/blob/main/LICENSE) and database capability fit an infrastructure backend behind Sagat's own UI. Infisical offers [self-hosted deployment](https://infisical.com/docs/self-hosting/overview), but its [license](https://github.com/Infisical/infisical/blob/main/LICENSE) distinguishes MIT code from Enterprise directories, and its [current plans](https://infisical.com/pricing) list dynamic secrets as a paid capability. These facts support retaining OpenBao for this project; they do not establish a universal product ranking or a performance comparison.

## Design risks to resolve in the plan

1. **Provider operation evidence:** Define how the same-key/same-content write is recognized after a lost response without persisting plaintext or leaking credential equality through public metadata. New values that never reached OpenBao require secure resubmission after a crash.
2. **Network enforcement:** The existing general egress path rejects private addresses. Customer database access needs an explicit deployment-scoped executor route; a preflight DNS check alone is insufficient.
3. **SQL semantics:** Read-only mode, functions, type conversion, database permissions and unknown writes need real database proofs. An SQL prefix check is not an authorization boundary.
4. **Result disclosure:** Finalize who can inspect bounded database results and which retained outputs are eligible; preserve generic run-inspection restrictions.
5. **Identity and deployment:** Pin OpenBao and PostgreSQL driver versions; select production workload authentication, trust, seal/bootstrap/recovery and finite pool/timeout limits. Dev fixtures cannot become production defaults.
6. **Migration concurrency:** Prevent local/OpenBao double authority while rotation, revocation and active runs race cutover. Retire legacy versions only after their dependencies and restore behavior are verified.

## Evidence status

Implementation is authorized. Actual checks are recorded in `verification.md`; tasks and success criteria remain open until their own evidence exists. The adapter fixture pins OpenBao 2.7.1, verified against the official release and KV v2 documentation. Workload identity renewal uses the OpenBao Agent token sink; production rejects root and non-expiring tokens at lookup-self.

## Target driver implementation decision (2026-10-17)

Use tokio-postgres 0.7.18 with an already pinned TCP/TLS socket and sqlparser 0.63.0 for a conservative single-statement AST subset. Inspecting dependency sources identified DEBUG query logging, INFO notices and an unbounded individual frame codec; the adapter adds a pre-allocation frame/aggregate budget, discards asynchronous notices and the Sagat log layer suppresses the driver and SQL-parser targets. Use fresh sessions per attempt under distributed lease capacity and a local semaphore; zero idle lifetime avoids credential retention and untrusted session-state reuse. This refines the bounded pool implementation without changing ADR-0033 authority/execution semantics. See [adapter contract](contracts/postgres-executor.md) for sources, result format and limitations.

## Reconciliation with the separate 013 draft

Feature 017 implements the async `SecretProvider` boundary, fixed KV-v2 credential paths,
durable provisioning receipts, token-file identity, explicit per-secret migration and bounded
cleanup. The separate unmerged 013 worktree remains owned by its author and was not modified.
When that draft is resumed, depend on 017's accepted ADR-0033 and provider contracts; reuse these
interfaces/migrations rather than create a second authority or duplicate schema. Its wider
signing-key migration, dynamic database credentials and telemetry replacement require their own
approved scope and evidence. The existing mixed local/remote signing consumers now fail closed
during provider outage; this compatibility fix does not migrate all signing keys.
