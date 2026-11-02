# crates/infra-sandbox/src

## Overview

Docker-isolated build/run adapter, artifact/lockfile validation and JSON-line supervisor protocol for the five implemented script languages.

## Interface

- `Images`, `GoTarget` and `DockerSandbox` expose local-engine setup, Go target/probe, leftovers/sweep and Sandbox probe/toolchain/build/run operations; `artifact_contract`/`artifact_policy` inspect signed artifacts. [docker.rs:Images] [docker.rs:GoTarget] [docker.rs:DockerSandbox] [docker.rs:artifact_contract] [docker.rs:artifact_policy]
- Lockfile constants, `ApprovedPackage`, `packages`, `accept` and `approved` define the closed Newtonsoft.Json catalog. `Message`, `parse` and protocol run/build/fetch/secret/refusal frame constructors define supervisor framing. [lockfile.rs:ApprovedPackage] [lockfile.rs:packages] [lockfile.rs:accept] [lockfile.rs:approved] [protocol.rs:Message] [protocol.rs:parse] [protocol.rs:run_message] [protocol.rs:run_binary_message] [protocol.rs:run_v3_message] [protocol.rs:build_message] [protocol.rs:fetch_reply] [protocol.rs:secret_reply] [protocol.rs:refusal]

## Invariants

- Container configuration disables network, privilege/capabilities, uses UID 65534, a read-only root, no-new-privileges/private IPC/no log driver, fixed resource limits and bounded noexec/nosuid/nodev /tmp. [docker.rs:DockerSandbox::config]
- Execution bounds host calls (100), channel lines (400), retained denials (32 of 200 characters), output/stderr and wall/setup/engine time. The 20-second engine-call ceiling is not evidence of tested wedged-engine recovery. [docker.rs:MAX_HOST_CALLS] [docker.rs:MAX_CHANNEL_LINES] [docker.rs:MAX_DENIED] [docker.rs:MAX_DENIED_LEN] [docker.rs:MAX_LINE] [docker.rs:ENGINE_CALL]
- Artifacts are revalidated by language and input/output schema. Go targets must match the native runner; pool homogeneity remains an external constraint because claims name language only. C# accepts the exact net10.0 Newtonsoft.Json 13.0.4 catalog; other lockfile support is explicitly limited. [docker.rs:parse_artifact] [docker.rs:run] [docker.rs:GoTarget] [lockfile.rs:accept] [lockfile.rs:approved]

## Side Effects

- The adapter inspects/pulls images and creates, starts, attaches, samples, kills/removes and sweeps Docker containers; drop guards asynchronously request removal. [docker.rs:DockerSandbox::ensure_image] [docker.rs:DockerSandbox::drive] [docker.rs:DockerSandbox::sample_memory] [docker.rs:DockerSandbox::sweep] [docker.rs:Removal::drop]
- Source/artifact/input frames travel over container stdin/stdout; host fetch/secret requests invoke HostCalls. Retained logs/results are redacted, artifacts/SBOMs are built in memory and metrics/spans report outcomes. [docker.rs:DockerSandbox::drive_created] [docker.rs:DockerSandbox::serve_fetch] [docker.rs:shown_log] [docker.rs:redact_value] [docker.rs:build]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-infra-sandbox --locked
```

These commands were not run in this documentation-only pass.
