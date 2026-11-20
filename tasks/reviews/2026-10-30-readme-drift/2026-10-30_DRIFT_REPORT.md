# EGM Drift Report — 203 bootstrap READMEs

Checkout: `develop@888074e6237b2a1ce9c85f83ec5703b081c65dd6`. Scope: the 203 files and SHA-256 values in [inventory.json](./inventory.json). Four distinct findings affect six READMEs; repeated channel descriptions share one finding. All findings remain open.

This is a static source/document comparison. Every target was loaded in full by the checker; source references, links, named interfaces and shared-contract copies were screened, with focused semantic reads of behavior, guards and channel/configuration boundaries. “No drift detected” records no discrepancy found within that static scope. It does not certify every function, compile every sample, reproduce historical test/advisory records or establish runtime, security, performance or release acceptance. Full-reading by a checker is distinguished from exhaustive manual source review. Method, hashes, coverage and limits are recorded in [execution_manifest.json](./execution_manifest.json); findings are also [structured](./findings.json).

## README.md

- **DRIFT-001 — Major: Script language entry points disagree with the root guide.** [README.md:320](../../../README.md#L320). The root guide directs C#/Go/Java authors to Studio/the script API after the TypeScript/Python-only CLI. Studio disables Go and Java, while server authoring validation accepts all five. The referenced OpenAPI createScript description and its generated TypeScript comment still say only TypeScript/Python are built and reject the other three.
  Evidence: [frontend/apps/studio/src/features/scripts/languages.ts:40](../../../frontend/apps/studio/src/features/scripts/languages.ts#L40), [frontend/apps/studio/src/features/scripts/CreateScriptDialog.tsx:67](../../../frontend/apps/studio/src/features/scripts/CreateScriptDialog.tsx#L67), [crates/domain/src/scripts.rs:109](../../../crates/domain/src/scripts.rs#L109), [crates/application/src/scripts.rs:958](../../../crates/application/src/scripts.rs#L958), [apps/cli/src/script.rs:61](../../../apps/cli/src/script.rs#L61), [specs/001-platform-foundation/contracts/openapi.yaml:3138](../../../specs/001-platform-foundation/contracts/openapi.yaml#L3138), [frontend/apps/studio/src/api/schema.d.ts:2207](../../../frontend/apps/studio/src/api/schema.d.ts#L2207).
  Correction: State the current entry-point matrix explicitly: CLI TypeScript/Python; Studio TypeScript/Python/C#; API authoring accepts all five subject to configured runner/toolchain capabilities. Align canonical OpenAPI availability text and regenerate derived declarations. Track any Studio expansion as implementation work rather than imply it already exists.
  Closure: Root instructions and contract availability text agree with the actual selectors/validators; genuine execution acceptance remains a separate gate.

## apps/README.md

No drift detected.

## apps/cli/README.md

No drift detected.

## apps/cli/src/README.md

No drift detected.

## apps/control-plane/README.md

No drift detected.

## apps/control-plane/src/README.md

No drift detected.

## apps/database-runner/README.md

No drift detected.

## apps/database-runner/src/README.md

No drift detected.

## apps/dev-idp/README.md

No drift detected.

## apps/dev-idp/src/README.md

No drift detected.

## apps/execution-engine/README.md

No drift detected.

## apps/execution-engine/src/README.md

No drift detected.

## apps/sandbox-runner/README.md

No drift detected.

## apps/sandbox-runner/src/README.md

No drift detected.

## crates/README.md

No drift detected.

## crates/api-grpc/README.md

No drift detected.

## crates/api-grpc/src/README.md

No drift detected.

## crates/api-http/README.md

No drift detected.

## crates/api-http/src/README.md

No drift detected.

## crates/api-http/src/auth/README.md

No drift detected.

## crates/api-http/src/routes/README.md

No drift detected.

## crates/application/README.md

No drift detected.

## crates/application/src/README.md

No drift detected.

## crates/application/src/execution/README.md

No drift detected.

## crates/application/src/runs/README.md

No drift detected.

## crates/application/src/scripts/README.md

No drift detected.

## crates/application/src/worker/README.md

No drift detected.

## crates/domain/README.md

No drift detected.

## crates/domain/src/README.md

No drift detected.

## crates/domain/src/database/README.md

No drift detected.

## crates/domain/src/execution/README.md

No drift detected.

## crates/domain/src/workflow/README.md

No drift detected.

## crates/infra-blobstore/README.md

No drift detected.

## crates/infra-blobstore/src/README.md

No drift detected.

## crates/infra-connectors/README.md

No drift detected.

## crates/infra-connectors/src/README.md

No drift detected.

## crates/infra-database/README.md

No drift detected.

## crates/infra-database/src/README.md

No drift detected.

## crates/infra-egress/README.md

No drift detected.

## crates/infra-egress/src/README.md

No drift detected.

## crates/infra-expr/README.md

No drift detected.

## crates/infra-expr/src/README.md

No drift detected.

## crates/infra-ir/README.md

No drift detected.

## crates/infra-ir/src/README.md

No drift detected.

## crates/infra-nats/README.md

No drift detected.

## crates/infra-nats/src/README.md

No drift detected.

## crates/infra-postgres/README.md

No drift detected.

## crates/infra-postgres/migrations/README.md

No drift detected.

## crates/infra-postgres/src/README.md

No drift detected.

## crates/infra-postgres/src/database/README.md

No drift detected.

## crates/infra-postgres/src/execution/README.md

No drift detected.

## crates/infra-postgres/src/scripts/README.md

No drift detected.

## crates/infra-sandbox/README.md

No drift detected.

## crates/infra-sandbox/shims/README.md

No drift detected.

## crates/infra-sandbox/src/README.md

No drift detected.

## crates/infra-schema/README.md

No drift detected.

## crates/infra-schema/src/README.md

No drift detected.

## crates/infra-schema/src/classification/README.md

No drift detected.

## crates/infra-secrets/README.md

No drift detected.

## crates/infra-secrets/src/README.md

No drift detected.

## crates/scheduler/README.md

No drift detected.

## crates/scheduler/src/README.md

No drift detected.

## crates/scheduler/src/passes/README.md

No drift detected.

## crates/telemetry/README.md

No drift detected.

## crates/telemetry/src/README.md

No drift detected.

## examples/README.md

No drift detected.

## examples/hello-world/README.md

No drift detected.

## examples/hello-world/dotnet/README.md

No drift detected.

## examples/hello-world/go/README.md

No drift detected.

## examples/hello-world/java/README.md

No drift detected.

## examples/hello-world/java/src/README.md

No drift detected.

## examples/hello-world/java/src/main/README.md

No drift detected.

## examples/hello-world/java/src/main/java/README.md

No drift detected.

## examples/hello-world/python/README.md

No drift detected.

## examples/hello-world/typescript/README.md

No drift detected.

## examples/hello-world/typescript/src/README.md

No drift detected.

## examples/order-processing/README.md

No drift detected.

## examples/order-processing/dotnet/README.md

No drift detected.

## examples/order-processing/go/README.md

No drift detected.

## examples/order-processing/java/README.md

No drift detected.

## examples/order-processing/java/src/README.md

No drift detected.

## examples/order-processing/java/src/main/README.md

No drift detected.

## examples/order-processing/java/src/main/java/README.md

No drift detected.

## examples/order-processing/python/README.md

No drift detected.

## examples/order-processing/typescript/README.md

No drift detected.

## examples/order-processing/typescript/src/README.md

No drift detected.

## examples/payment-saga/README.md

No drift detected.

## examples/payment-saga/typescript/README.md

No drift detected.

## examples/payment-saga/typescript/src/README.md

No drift detected.

## frontend/README.md

No drift detected.

## frontend/apps/README.md

No drift detected.

## frontend/apps/studio/README.md

No drift detected.

## frontend/apps/studio/scripts/README.md

No drift detected.

## frontend/apps/studio/src/README.md

No drift detected.

## frontend/apps/studio/src/api/README.md

No drift detected.

## frontend/apps/studio/src/app/README.md

No drift detected.

## frontend/apps/studio/src/auth/README.md

No drift detected.

## frontend/apps/studio/src/features/README.md

No drift detected.

## frontend/apps/studio/src/features/connections/README.md

No drift detected.

## frontend/apps/studio/src/features/database/README.md

No drift detected.

## frontend/apps/studio/src/features/integrations/README.md

No drift detected.

## frontend/apps/studio/src/features/operations/README.md

No drift detected.

## frontend/apps/studio/src/features/runs/README.md

No drift detected.

## frontend/apps/studio/src/features/scripts/README.md

No drift detected.

## frontend/apps/studio/src/features/secrets/README.md

No drift detected.

## frontend/apps/studio/src/features/workflows/README.md

No drift detected.

## frontend/apps/studio/src/model/README.md

No drift detected.

## frontend/apps/studio/src/ui/README.md

No drift detected.

## infra/README.md

No drift detected.

## infra/docker/README.md

No drift detected.

## infra/docker/postgres-init/README.md

No drift detected.

## infra/dr/README.md

No drift detected.

## infra/openbao/README.md

No drift detected.

## infra/openbao/policies/README.md

No drift detected.

## runners/README.md

No drift detected.

## runners/dotnet/README.md

No drift detected.

## runners/dotnet/Sagat.DotnetBuild/README.md

No drift detected.

## runners/dotnet/Sagat.DotnetRuntime/README.md

No drift detected.

## runners/dotnet/Sagat.ScriptContract/README.md

No drift detected.

## runners/go/README.md

No drift detected.

## runners/java/README.md

No drift detected.

## runners/java/api/README.md

No drift detected.

## runners/java/api/sagat/README.md

No drift detected.

## runners/java/api/sagat/inline/README.md

No drift detected.

## runners/java/host/README.md

- **DRIFT-003 — Major: Java mediated calls are assigned to the wrong output channel.** [runners/java/host/README.md:22](../../../runners/java/host/README.md#L22). Three contracts say mediated fetch/secret requests and script logs go to stderr. RuntimeHost preserves the original System.out as its JSON protocol channel; OctopusHost.call sends requests through that preserved channel. Only ordinary script stdout and log values are directed to stderr.
  Evidence: [runners/java/host/sagat/host/RuntimeHost.java:29](../../../runners/java/host/sagat/host/RuntimeHost.java#L29), [runners/java/host/sagat/host/RuntimeHost.java:105](../../../runners/java/host/sagat/host/RuntimeHost.java#L105), [runners/java/host/sagat/host/RuntimeHost.java:159](../../../runners/java/host/sagat/host/RuntimeHost.java#L159), [runners/java/host/sagat/host/RuntimeHost.java:163](../../../runners/java/host/sagat/host/RuntimeHost.java#L163).
  Correction: In all three contracts, document fetch/secret requests on the preserved stdout JSON channel, replies on stdin, and ordinary script output/logs on stderr.
  Closure: All three Side Effects sections describe the actual channel split without changing the runner protocol.

## runners/java/host/sagat/README.md

- **DRIFT-003 — Major: Java mediated calls are assigned to the wrong output channel.** [runners/java/host/sagat/README.md:22](../../../runners/java/host/sagat/README.md#L22). Three contracts say mediated fetch/secret requests and script logs go to stderr. RuntimeHost preserves the original System.out as its JSON protocol channel; OctopusHost.call sends requests through that preserved channel. Only ordinary script stdout and log values are directed to stderr.
  Evidence: [runners/java/host/sagat/host/RuntimeHost.java:29](../../../runners/java/host/sagat/host/RuntimeHost.java#L29), [runners/java/host/sagat/host/RuntimeHost.java:105](../../../runners/java/host/sagat/host/RuntimeHost.java#L105), [runners/java/host/sagat/host/RuntimeHost.java:159](../../../runners/java/host/sagat/host/RuntimeHost.java#L159), [runners/java/host/sagat/host/RuntimeHost.java:163](../../../runners/java/host/sagat/host/RuntimeHost.java#L163).
  Correction: In all three contracts, document fetch/secret requests on the preserved stdout JSON channel, replies on stdin, and ordinary script output/logs on stderr.
  Closure: All three Side Effects sections describe the actual channel split without changing the runner protocol.

## runners/java/host/sagat/host/README.md

- **DRIFT-003 — Major: Java mediated calls are assigned to the wrong output channel.** [runners/java/host/sagat/host/README.md:23](../../../runners/java/host/sagat/host/README.md#L23). Three contracts say mediated fetch/secret requests and script logs go to stderr. RuntimeHost preserves the original System.out as its JSON protocol channel; OctopusHost.call sends requests through that preserved channel. Only ordinary script stdout and log values are directed to stderr.
  Evidence: [runners/java/host/sagat/host/RuntimeHost.java:29](../../../runners/java/host/sagat/host/RuntimeHost.java#L29), [runners/java/host/sagat/host/RuntimeHost.java:105](../../../runners/java/host/sagat/host/RuntimeHost.java#L105), [runners/java/host/sagat/host/RuntimeHost.java:159](../../../runners/java/host/sagat/host/RuntimeHost.java#L159), [runners/java/host/sagat/host/RuntimeHost.java:163](../../../runners/java/host/sagat/host/RuntimeHost.java#L163).
  Correction: In all three contracts, document fetch/secret requests on the preserved stdout JSON channel, replies on stdin, and ordinary script output/logs on stderr.
  Closure: All three Side Effects sections describe the actual channel split without changing the runner protocol.

## scripts/README.md

No drift detected.

## sdks/README.md

No drift detected.

## sdks/dotnet/README.md

No drift detected.

## sdks/dotnet/proto/README.md

No drift detected.

## sdks/dotnet/scripts/README.md

No drift detected.

## sdks/dotnet/src/README.md

No drift detected.

## sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/DependencyInjection/README.md

No drift detected.

## sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/Internal/README.md

No drift detected.

## sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/README.md

No drift detected.

## sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/Workers/README.md

- **DRIFT-004 — Nit: C# reporting uses five attempts, not a five-retry budget.** [sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/Workers/README.md:16](../../../sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/Workers/README.md#L16). The contract calls the report budget five safe retries. ReportAttempts is 5 and WithRetriesAsync permits another call only while n + 1 < ReportAttempts: at most five total attempts, comprising an initial call and four retries. Stale/non-retryable failures stop sooner.
  Evidence: [sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/Workers/WorkerHost.cs:21](../../../sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/Workers/WorkerHost.cs#L21), [sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/Workers/WorkerHost.cs:1136](../../../sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/Workers/WorkerHost.cs#L1136).
  Correction: Use “at most five total attempts (up to four retries), with a stable task-derived key.”
  Closure: The documented terminology distinguishes the initial attempt from retries; no runtime failure is inferred.

## sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/Workflows/README.md

No drift detected.

## sdks/go/README.md

No drift detected.

## sdks/go/client/README.md

No drift detected.

## sdks/go/internal/README.md

No drift detected.

## sdks/go/internal/backoff/README.md

No drift detected.

## sdks/go/internal/bound/README.md

No drift detected.

## sdks/go/internal/ids/README.md

No drift detected.

## sdks/go/internal/jsonx/README.md

No drift detected.

## sdks/go/internal/safelog/README.md

No drift detected.

## sdks/go/internal/tokenfail/README.md

No drift detected.

## sdks/go/internal/workerpb/README.md

No drift detected.

## sdks/go/otelbridge/README.md

No drift detected.

## sdks/go/proto/README.md

No drift detected.

## sdks/go/proto/sagat_octopus_orchestrator/README.md

No drift detected.

## sdks/go/proto/sagat_octopus_orchestrator/v1/README.md

No drift detected.

## sdks/go/scripts/README.md

No drift detected.

## sdks/go/worker/README.md

No drift detected.

## sdks/go/workflow/README.md

No drift detected.

## sdks/java/README.md

- **DRIFT-002 — Nit: Java installation notes deny an existing release pipeline.** [sdks/java/README.md:25](../../../sdks/java/README.md#L25). The installation section says no signing or release pipeline exists. The current workflow packages SDKs, collects artifacts, attests provenance and conditionally publishes GitHub Releases, including Java artifacts produced by package-sdks.sh.
  Evidence: [.github/workflows/release.yml:99](../../../.github/workflows/release.yml#L99), [.github/workflows/release.yml:159](../../../.github/workflows/release.yml#L159), [scripts/package-sdks.sh:364](../../../scripts/package-sdks.sh#L364).
  Correction: Describe the existing GitHub packaging/attestation/release path and distinguish it from deferred external Maven repository publication and Maven-specific signing. Retain local-install instructions.
  Closure: The README describes the existing local source pipeline without claiming Maven Central publication, a successful remote workflow, a published tag or a signing mechanism not inspected here.

## sdks/java/scripts/README.md

No drift detected.

## sdks/java/sdk/README.md

No drift detected.

## sdks/java/sdk/src/README.md

No drift detected.

## sdks/java/sdk/src/main/README.md

No drift detected.

## sdks/java/sdk/src/main/java/README.md

No drift detected.

## sdks/java/sdk/src/main/java/dev/README.md

No drift detected.

## sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/README.md

No drift detected.

## sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/README.md

No drift detected.

## sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/opentelemetry/README.md

No drift detected.

## sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/worker/README.md

No drift detected.

## sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/workflow/README.md

No drift detected.

## sdks/java/sdk/src/main/proto/README.md

No drift detected.

## sdks/python/README.md

No drift detected.

## sdks/python/proto/README.md

No drift detected.

## sdks/python/sagat_octopus_orchestrator/README.md

No drift detected.

## sdks/python/sagat_octopus_orchestrator/_proto/README.md

No drift detected.

## sdks/python/sagat_octopus_orchestrator/worker/README.md

No drift detected.

## sdks/python/scripts/README.md

No drift detected.

## sdks/rust/README.md

No drift detected.

## sdks/rust/consumer/README.md

No drift detected.

## sdks/rust/consumer/src/README.md

No drift detected.

## sdks/rust/proto/README.md

No drift detected.

## sdks/rust/src/README.md

No drift detected.

## sdks/rust/src/capabilities/README.md

No drift detected.

## sdks/rust/src/client/README.md

No drift detected.

## sdks/rust/src/workflow/README.md

No drift detected.

## sdks/typescript/README.md

No drift detected.

## sdks/typescript/examples/README.md

No drift detected.

## sdks/typescript/proto/README.md

No drift detected.

## sdks/typescript/scripts/README.md

No drift detected.

## sdks/typescript/src/README.md

No drift detected.

## sdks/typescript/src/worker/README.md

No drift detected.

## specs/001-platform-foundation/README.md

No drift detected.

## specs/001-platform-foundation/contracts/README.md

No drift detected.

## specs/README.md

No drift detected.

## templates/README.md

No drift detected.

## templates/inline-script/README.md

No drift detected.

## vendor/README.md

No drift detected.

## vendor/contracts/README.md

No drift detected.

## vendor/contracts/rskafka/README.md

No drift detected.

## vendor/contracts/rskafka/src/README.md

No drift detected.

## vendor/contracts/rskafka/src/client/README.md

No drift detected.

## vendor/contracts/rskafka/src/client/producer/README.md

No drift detected.

## vendor/contracts/rskafka/src/connection/README.md

No drift detected.

## vendor/contracts/rskafka/src/connection/transport/README.md

No drift detected.

## vendor/contracts/rskafka/src/protocol/README.md

No drift detected.

## vendor/contracts/rskafka/src/protocol/messages/README.md

No drift detected.
