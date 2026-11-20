# src

## Overview

Minimal consumer of the public SDK API. [main.rs:main]

## Interface

- Binary entry point main constructs the public Client. [main.rs:main]

## Invariants

- The executable asserts that Client Debug output omits the configured fixture token. [main.rs:assert]

## Side Effects

- Constructs a Client and panics if construction or the redaction assertion fails. [main.rs:main]

## Verification

Run from the repository root:

```sh
cargo run --locked --manifest-path sdks/rust/consumer/Cargo.toml
```

Use a private `CARGO_TARGET_DIR` outside the repository, as required by [`sdks/AGENTS.md`](../../../AGENTS.md).

These commands are documented from the existing build configuration; they were not executed during this documentation update.
