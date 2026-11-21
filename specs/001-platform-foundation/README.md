# Platform-foundation specification contracts

## Overview

Feature 001 design artifacts and shared wire/schema contracts. Feature status remains defined by spec.md, plan.md and tasks.md.

## Interface

The child contracts define the source boundaries in this directory.

## Subdirectories & Boundaries

- [contracts](./contracts/README.md) — Canonical design schemas and worker protocol.

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
