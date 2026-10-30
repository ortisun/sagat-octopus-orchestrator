# host

## Overview

Trusted Java compilation and runtime hosts exchanging newline-delimited JSON over standard I/O.

## Interface

- [sagat](sagat/README.md) — Trusted Java compilation and runtime hosts exchanging newline-delimited JSON over standard I/O.

## Invariants

- Build requests use the exact `op/source/lockfile/packages` envelope, require `op=build`, and reject lockfiles and packages. Compilation fixes Java 17 and disables annotation processing. [runners/java/host/sagat/host/BuildHost.java:build]
- Artifacts contain only `Script.class` and nested `Script$*.class`, at most 32 files and 1,400 KiB total. [runners/java/host/sagat/host/BuildHost.java:build]
- The runtime requires `target=jvm-17`, `encoding=base64`, canonical class bytes, and the artifact count/size bounds. [runners/java/host/sagat/host/RuntimeHost.java:decodeClasses]
- Mediated calls are serialized and assigned increasing IDs; unmatched replies are ignored and refused or closed-channel calls fail. [runners/java/host/sagat/host/RuntimeHost.java:OctopusHost.call]

## Side Effects

- Creates `/tmp/sagat-build`, writes source and class files, launches fixed `javac`, and emits built/rejected messages. [runners/java/host/sagat/host/BuildHost.java:build]
- Loads and executes submitted bytecode, redirects script stdout to stderr, and emits results/errors on the preserved JSON channel. [runners/java/host/sagat/host/RuntimeHost.java:RuntimeHost.main]
- Emits mediated fetch/secret requests and script log values to stderr. [runners/java/host/sagat/host/RuntimeHost.java:OctopusHost]

## Verification

From the repository root, with Docker available for the container integration suite and a private `CARGO_TARGET_DIR` outside the checkout:

```sh
cargo test -p sagat-infra-sandbox --locked
cargo test -p sagat-tests --test integration_inline_code_sandbox_containers --locked
```

The crate checks and container integration target are declared in `crates/infra-sandbox/Cargo.toml` and `tests/Cargo.toml`.
