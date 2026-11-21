# consumer

## Overview

Independent non-publishable consumer Cargo workspace. [Cargo.toml:workspace]

## Interface

- Package sagat-rust-clean-consumer depends on the SDK through a local parent path. [Cargo.toml:name] [Cargo.toml:dependencies]

- [src](./src/README.md) — component contract.

## Invariants

- Publishing is disabled and the nested workspace is independent. [Cargo.toml:publish] [Cargo.toml:workspace]

## Side Effects

- None

## Verification

Run from the repository root:

```sh
cargo run --locked --manifest-path sdks/rust/consumer/Cargo.toml
```

Use a private `CARGO_TARGET_DIR` outside the repository, as required by [`sdks/AGENTS.md`](../../AGENTS.md).

These commands are documented from the existing build configuration; they were not executed during this documentation update.
