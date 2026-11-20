# inline

## Overview

Public Java sagat-inline-v1 script and mediated host-call contract.

## Interface

- `InlineScript.run(Object, Octopus)` is the script entry contract. [runners/java/api/sagat/inline/InlineScript.java:InlineScript.run]
- `Octopus` exposes the idempotency key, `fetch`, `secret`, and `log`. [runners/java/api/sagat/inline/Octopus.java:Octopus]
- `Response` carries immutable status, headers, and body. [runners/java/api/sagat/inline/Response.java:Response]

## Invariants

- The source identifies this entry point as the stable `sagat-inline-v1` contract. [runners/java/api/sagat/inline/InlineScript.java:InlineScript]

## Side Effects

- Implementations mediate fetch, leased-secret access, and logging; these files define the interface without performing those operations themselves. [runners/java/api/sagat/inline/Octopus.java:Octopus]

## Verification

From the repository root, with Docker available for the container integration suite and a private `CARGO_TARGET_DIR` outside the checkout:

```sh
cargo test -p sagat-infra-sandbox --locked
cargo test -p sagat-tests --test integration_inline_code_sandbox_containers --locked
```

The crate checks and container integration target are declared in `crates/infra-sandbox/Cargo.toml` and `tests/Cargo.toml`.
