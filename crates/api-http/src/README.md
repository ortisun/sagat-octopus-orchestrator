# crates/api-http/src

## Overview

Axum HTTP adapter exposing authentication, state/configuration, router composition, typed errors, server limits, SSE/blob/Studio support and deployment-gated control-plane capabilities. Business operations delegate to application services.

## Interface

- The public facade exports AppState, HttpLimits, router/router_with_execution/router_with_governance/router_with_governance_and_approvals and protect, plus authentication, concurrency, execution, governance, rate-limit and native-server families. [lib.rs:AppState] [lib.rs:HttpLimits] [lib.rs:router] [lib.rs:router_with_execution] [lib.rs:router_with_governance] [lib.rs:router_with_governance_and_approvals] [lib.rs:protect]
- Core supplies authentication/audit; Authenticated, authenticate_bearer and parse_bearer are re-exported. Admission, Caller and ApiJson remain internal handler helpers. [core.rs:Core] [extract.rs:Authenticated] [extract.rs:authenticate_bearer] [extract.rs:parse_bearer] [admission.rs:Admission] [extract.rs:Caller] [extract.rs:ApiJson]
- Exec/ExecState configure execution ports, optional publishing, payload inspection/disclosure, database results and stream/blob limits. Governance/GovState configure quotas, reveal grants, bindings and egress plus optional operations, secrets, connections, events, outbound, connectors and scripts. [exec.rs:Exec] [exec.rs:ExecState] [governance.rs:Governance] [governance.rs:GovState]
- HttpBlobTransfer/BlobTransfers, TenantConcurrency/TenantPermit/TenantAtCapacity and RateLimitConfig/RateLimitConfigError/RateLimiter expose transfer, concurrency and rate admission. [blobs.rs:HttpBlobTransfer] [blobs.rs:BlobTransfers] [concurrency.rs:TenantConcurrency] [concurrency.rs:TenantPermit] [concurrency.rs:TenantAtCapacity] [limits.rs:RateLimitConfig] [limits.rs:RateLimitConfigError] [limits.rs:RateLimiter] [limits.rs:DEFAULT_MAX_TRACKED_KEYS] [limits.rs:MAX_RETRY_AFTER]
- ErrorCode/ErrorParts/ApiError define response errors; MAX_PENDING_WRITES/AuditOutcome/spawn_audit_access_denied define bounded independent denial auditing. [error.rs:ErrorCode] [error.rs:ErrorParts] [error.rs:ApiError] [denial.rs:MAX_PENDING_WRITES] [denial.rs:AuditOutcome] [denial.rs:spawn_audit_access_denied]
- ServerConfig, HttpTlsConfig::from_pem, serve and serve_tls provide native transport. StreamLimits/StreamRegistry/StreamRefused/StreamSlot provide SSE admission; StudioConfig/StudioLimits/StudioOidc/StudioError/studio_router/validate_trace_url_template provide static Studio hosting. [server.rs:ServerConfig] [server.rs:HttpTlsConfig] [server.rs:serve] [server.rs:serve_tls] [streams.rs:StreamLimits] [streams.rs:StreamRegistry] [streams.rs:StreamRefused] [streams.rs:StreamSlot] [studio.rs:StudioConfig] [studio.rs:StudioLimits] [studio.rs:StudioOidc] [studio.rs:StudioError] [studio.rs:studio_router] [studio.rs:validate_trace_url_template]
- Private DTO modules cover identity/workspace/namespace/version, chain inspection/commands, declared operations, approvals and database connection configuration. [dto.rs:WorkspaceBody] [dto.rs:NamespaceBody] [dto.rs:AuthContextBody] [dto.rs:VersionBody] [dto_chains.rs:ChainInspectionBody] [dto_declared_operations.rs:DeclaredUpdateResponseBody] [dto_primitives.rs:ApprovalDecisionReceiptBody] [dto_database.rs:ConfigBody] [dto_database.rs:PutConnectionBody] [dto_database.rs:ConnectionBody] [dto_database.rs:DatabaseDriverBody]
- Private event DTOs cover endpoints/events, outbound subscriptions/delivery, connectors and dead letters. Execution DTOs cover workflows/validation/version/run/journal/telemetry/runtime graphs and compensation projections. [dto_events.rs:EndpointBody] [dto_events.rs:EventBody] [dto_events.rs:SubscriptionBody] [dto_events.rs:BrokerSubscriptionBody] [dto_events.rs:ConnectorBody] [dto_events.rs:DeadLetterBody] [dto_exec.rs:WorkflowBody] [dto_exec.rs:StartRunBody] [dto_exec.rs:RunBody] [dto_exec.rs:runtime_graph] [dto_exec.rs:graph_changes_body]
- Private governance/operations DTOs cover quotas, bindings, secret metadata, egress, alerts, workers and audits; secret request Debug redacts values. Script DTOs cover authoring/version/build/test/execution/runner records and artifact decoding; versioning DTOs cover policies, requests, decisions, activation and promotion. [dto_gov.rs:SetSecretBody] [dto_gov.rs:SecretBody] [dto_ops.rs:AlertBody] [dto_ops.rs:WorkerBody] [dto_ops.rs:AuditRecordBody] [dto_scripts.rs:CreateScriptBody] [dto_scripts.rs:VersionBody] [dto_scripts.rs:BuildAssignmentBody] [dto_scripts.rs:RecordExecutionBody] [dto_scripts.rs:decode_artifact] [dto_versioning.rs:PolicyBody] [dto_versioning.rs:RequestBody] [dto_versioning.rs:ActivationBody] [dto_versioning.rs:PromotionBody]
- [auth](./auth/README.md) — Public OIDC/JWT authentication boundary: configuration, bounded JWKS retrieval/cache, token verification and deployment-controlled webhook identity profiles. [auth/config.rs:AuthConfig]
- [routes](./routes/README.md) — Axum handler families mounted beneath /v1. Most handlers and route-local DTOs are internal because routes is crate-private; reveal_grants is separately exposed as a public module. Mounting depends on router composition and configured services. [routes/chains.rs:routes]

## Invariants

- The base router mounts /v1 health/readiness and authenticated identity/version/workspace/namespace routes. Execution composition adds workflow/run/database-execution routes; publication, payload inspection/disclosure, database-result and input-blob routes depend on configured services. [lib.rs:router] [lib.rs:compose] [exec.rs:ExecState]
- Governance composition mounts quota/binding/egress/operations/reveal-grant paths; connection, secret, event/webhook, outbound, connector and script/runner routes require their optional service. Approval, declared-operation and chain routes require router_with_governance_and_approvals. Studio is a separately returned router. [lib.rs:governance_routes] [lib.rs:router_with_governance_and_approvals] [studio.rs:studio_router]
- protect applies panic containment, request/body-idle timeouts, body-size limits and request-context/error-envelope middleware. Route groups add body limits and immediate global load shedding; probes remain outside the global in-flight gate. [lib.rs:protect] [lib.rs:router] [limits.rs:shed_excess]
- Bearer authentication verifies the token, charges a subject bucket before identity storage, resolves the session, charges the tenant bucket and retains a tenant permit in Caller/Authenticated. [extract.rs:authenticate_bearer] [extract.rs:Caller] [concurrency.rs:TenantPermit]
- RateLimiter bounds tracked keys and shares an overflow bucket after capacity. Defaults are tenant 100/s burst 200 and subject 20/s burst 40; retry guidance is capped at one hour. [limits.rs:RateLimitConfig::default] [limits.rs:RateLimitConfig::subject_default] [limits.rs:RateLimiter::with_capacity] [limits.rs:MAX_RETRY_AFTER]
- HTTP errors use a closed ErrorCode family, safe remediation, no-store/nosniff and correlation headers; framework errors are normalized to that envelope. [error.rs:ErrorCode] [error.rs:ErrorParts::synthesized] [context.rs:finalize]
- Run-input upload requires exactly one supported-header value, namespace authorization/admission, tenant/transfer permits and bounded timeout/token lifetime. [blobs.rs:header] [blobs.rs:upload] [blobs.rs:HttpBlobTransfer]
- SSE defaults are 2,000 process/200 tenant/20 subject streams, bounded polling, 15-second pings, 15-minute lifetime and 30-second authorization rechecks. Slots release on Drop. [streams.rs:StreamLimits::default] [streams.rs:StreamRegistry::try_open] [streams.rs:StreamSlot::drop]
- The native server uses HTTP/1.1, caps connections, bounds request-head/idle reads and stalled writes, and drains within shutdown_grace. TLS uses HTTP/1.1 ALPN; optional client certificates are revalidated per request and exposed as verified fingerprints to profile-specific handlers. [server.rs:serve_inner] [server.rs:serve_connection] [server.rs:StallGuard] [server.rs:HttpTlsConfig::from_pem] [server.rs:ClientIdentity::verified]
- Studio serves regular files whose canonical paths remain below its configured root, caps file/index sizes and concurrent/time-bounded transfers, and applies CSP, nosniff, same-origin and cache headers. [studio.rs:validate_index] [studio.rs:locate] [studio.rs:Transfer] [studio.rs:hardened]
- Authentication uses fixed issuer/audience, bounded tokens and JWKS retrieval/cache, RS256/ES256 keys and HTTPS trust roots outside development; webhook profiles are deployment-selected. [auth/verifier.rs:verify_claims] [auth/jwks.rs:HttpJwksFetcher] [auth/webhooks.rs:WebhookOidcVerifiers]

## Side Effects

- Handlers invoke application services/stores with audit context; injected ports perform storage/provider/broker effects. [exec.rs:Exec] [governance.rs:Governance] [admission.rs:admit]
- Authentication may retrieve JWKS over HTTP and update cache, metrics and logs. Middleware emits tracing/metrics and bounded denial-audit tasks. [auth/jwks.rs:HttpJwksFetcher::fetch] [auth/jwks.rs:JwksCache::refresh] [context.rs:request_context] [denial.rs:spawn_gated]
- Blob upload streams request bytes to BlobService; SSE/denial paths spawn Tokio tasks; Studio performs bounded filesystem reads and streaming. [blobs.rs:upload] [routes/execution.rs:run_stream] [denial.rs:spawn_gated] [studio.rs:file]
- serve/serve_tls accept sockets, optionally handshake TLS, spawn connection tasks and close/abort connections on deadlines or shutdown. [server.rs:serve_inner] [server.rs:serve_connection]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-api-http --locked
```

These commands were not run in this documentation-only pass.
