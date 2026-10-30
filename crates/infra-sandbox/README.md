# crates/infra-sandbox

## Overview

sagat-infra-sandbox builds and runs untrusted scripts in isolated Docker containers and embeds language shims.

## Interface

- DockerSandbox, artifact_contract, lockfile acceptance and protocol Message expose the host boundary. Embedded TypeScript/Python shims provide build/runtime entry points. [src/docker.rs:DockerSandbox] [src/docker.rs:artifact_contract] [src/lockfile.rs:accept] [src/protocol.rs:Message] [shims/build.mjs:say] [shims/build.py:say] [shims/runtime.mjs:start] [shims/runtime.py:_main]
- [shims](./shims/README.md) — Container-resident TypeScript/Python build/runtime shims embedded by the Rust sandbox and connected through JSON-line frames. [shims/build.mjs:reject]
- [src](./src/README.md) — Docker-isolated build/run adapter, artifact/lockfile validation and JSON-line supervisor protocol for the five implemented script languages. [src/docker.rs:Images]

## Invariants

- Containers disable network/capabilities, use a read-only root and fixed resources; host calls, lines, output and wall time are bounded. A configured engine-call ceiling does not demonstrate tested wedged-engine recovery. [src/docker.rs:DockerSandbox::config] [src/docker.rs:MAX_HOST_CALLS] [src/docker.rs:MAX_CHANNEL_LINES] [src/docker.rs:ENGINE_CALL]

## Side Effects

- The adapter pulls images and creates/runs/samples/kills/removes containers; shims execute user entry points and exchange framed stdio. [src/docker.rs:DockerSandbox::ensure_image] [src/docker.rs:DockerSandbox::drive] [shims/runtime.mjs:start]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-infra-sandbox --locked
```

These commands were not run in this documentation-only pass.
