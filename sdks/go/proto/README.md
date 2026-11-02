# proto

## Overview

Directory boundary for sagat_octopus_orchestrator.

## Interface

- [sagat_octopus_orchestrator](./sagat_octopus_orchestrator/README.md) — component contract.

## Invariants

- None at this grouping level; child contracts describe their own checks.

## Side Effects

- None at this grouping level; runtime effects belong to the child components.

## Verification

Run from the repository root:

```sh
cd sdks/go && go test -race ./... && go vet ./...
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
