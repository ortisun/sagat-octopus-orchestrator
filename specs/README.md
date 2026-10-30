# Specification contract index

## Overview

Source-bearing schema contracts within the Spec Kit feature tree. The complete feature inventory remains in the [specification index](../SPECIFICATION-INDEX.md); this contract index does not change feature acceptance.

## Interface

The child contracts define the source boundaries in this directory.

## Subdirectories & Boundaries

- [001-platform-foundation](./001-platform-foundation/README.md) — Platform foundation and shared protocol contracts.

## Invariants

- No additional executable invariant is introduced by this grouping; source checks and contract requirements are documented in the linked components.

## Side Effects

- None from the directory grouping itself. The linked component contracts describe command execution, storage and network effects.

## Verification

Run from the repository root:

```sh
cargo test -p sagat-tests --test contract_platform_foundation --locked
```

These are verification entry points; no runtime test result is implied by this index.
