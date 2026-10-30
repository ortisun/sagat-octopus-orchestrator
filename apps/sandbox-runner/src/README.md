# sandbox-runner source

## Overview

Remote script build/test/step runner over HTTP and worker gRPC, delegating compilation and execution to a supplied Sandbox.

## Interface

- `api` exports typed `RunnerError`, `Credentials`, job/report/result types, incident types, `ControlPlane` and `policy_of`. [api.rs:RunnerError] [api.rs:Credentials] [api.rs:ControlPlane] [api.rs:IncidentTarget] [api.rs:policy_of]
- `RunnerConfig` / `NamespaceTarget` configure identity, endpoints, namespace targets, concurrency, policy ceilings, keys/images and language capabilities. `Runner` exposes startup, claims, build/test processing, step polling/execution and serving with `StepTask` / `StepOutcome`. [config.rs:RunnerConfig] [config.rs:NamespaceTarget] [lib.rs:Runner] [lib.rs:StepTask] [lib.rs:StepOutcome]
- `ScriptHost` provides closed/open/live host-call implementations with declared secret lookup. Public CLI helpers parse serve/healthcheck/help/version; binary startup supplies `DockerSandbox`. [host.rs:ScriptHost] [cli.rs:Command] [cli.rs:parse] [main.rs:serve]

## Invariants

- Outside development, control-plane endpoints must use HTTPS. Exactly one token source is required, concurrency is nonzero and required Go support needs a target. Credential files are reread per request; HTTP disables redirects/proxies and uses a deadline. [config.rs:RunnerConfig] [api.rs:Credentials] [api.rs:ControlPlane]
- Before execution, artifacts must match the requested digest and tenant-scoped signature/policy. An empty trust-key configuration uses signing-key bootstrap; later unknown keys are refused. [lib.rs:Runner::load] [lib.rs:Runner::verify]
- The runner reserves semaphore room before polling, heartbeats work, suppresses stale reporting and bounds report retries with stable task-derived keys. [lib.rs:Runner::poll_steps] [lib.rs:Runner::run_step] [lib.rs:ENGINE_TRIES]
- Live fetch loads namespace egress policy per request, strips disallowed headers, bounds retained responses and requires denial incident recording. Secret lookup is limited to configured names. [host.rs:ScriptHost] [host.rs:script_may_set] [host.rs:MAX_RESPONSE_BYTES]

## Side Effects

- Makes authenticated HTTP and gRPC calls and reads token/CA files. Calls the supplied sandbox for probes, builds and execution; the binary connects Docker and periodically sweeps old runner containers. [api.rs:ControlPlane] [lib.rs:Runner::client] [lib.rs:Runner::process_build] [lib.rs:Runner::run_step] [main.rs:serve]
- Mediates script fetch/secret access, records incidents/execution reports and emits operational tracing/metrics. Startup optionally binds an admin metrics/liveness listener and manages telemetry lifecycle. [host.rs:ScriptHost] [lib.rs:Runner::observe] [main.rs:run] [main.rs:axum_live]

## Verification

From the repository root, with a private Cargo target directory outside the checkout:

```sh
cargo test -p sagat-sandbox-runner --locked
```

This is the manifest-derived package check; it was not run during this documentation pass.
